use crate::chat::mcp::{McpServerConfig, McpServerTool, convert_mcp_tools_to_genai};
use rmcp::model::Tool as McpTool;
use serde_json::Map;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

struct StdioConnection {
    child: tokio::process::Child,
}

pub struct McpManager {
    stdio_connections: Arc<RwLock<HashMap<String, StdioConnection>>>,
    tools: Arc<RwLock<Vec<McpServerTool>>>,
    server_configs: Arc<RwLock<Vec<McpServerConfig>>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            stdio_connections: Arc::new(RwLock::new(HashMap::new())),
            tools: Arc::new(RwLock::new(Vec::new())),
            server_configs: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn initialize(&self, configs: Vec<McpServerConfig>) {
        info!("[MCP] Initializing {} servers", configs.len());
        *self.server_configs.write().await = configs.clone();

        for config in configs {
            let name = config.name.clone();
            let enabled = config.enabled;
            if enabled {
                info!("[MCP] Attempting to connect to server: {}", name);
                if let Err(e) = self.connect_server(config).await {
                    error!("[MCP] Failed to connect to MCP server {}: {}", name, e);
                }
            } else {
                info!("[MCP] Skipping disabled server: {}", name);
            }
        }
    }

    async fn connect_server(&self, config: McpServerConfig) -> Result<(), String> {
        let server_id = config.id.clone();
        let server_name = config.name.clone();

        match config.transport {
            crate::chat::mcp::McpTransport::Stdio => {
                self.connect_stdio(config, server_id, server_name).await
            }
            crate::chat::mcp::McpTransport::Sse => self.connect_sse(&server_name).await,
            crate::chat::mcp::McpTransport::StreamableHttp => {
                self.connect_streamable_http(&server_name).await
            }
        }
    }

    async fn connect_stdio(
        &self,
        config: McpServerConfig,
        server_id: String,
        server_name: String,
    ) -> Result<(), String> {
        let command = config.command.ok_or("No command for stdio transport")?;
        let args = config.args;

        info!("Spawning MCP server: {} {}", command, args.join(" "));

        let mut cmd = if command.trim().starts_with("npx") {
            // Run npx through cmd.exe - combine command and args into single string
            let full_command = format!("{} {}", command, args.join(" "));
            let mut cmd = Command::new("cmd.exe");
            cmd.arg("/c").arg(&full_command);
            cmd
        } else {
            let mut cmd = Command::new(&command);
            cmd.args(&args);
            cmd
        };

        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return Err(format!("Failed to spawn stdio process: {e}")),
        };

        info!("Spawned MCP server child with PID: {:?}", child.id());

        let mut stdin = match child.stdin.take() {
            Some(i) => i,
            None => return Err("Failed to get stdin".to_string()),
        };
        let stdout = match child.stdout.take() {
            Some(o) => o,
            None => return Err("Failed to get stdout".to_string()),
        };

        let mut reader = BufReader::new(stdout);

        // Initialize the MCP connection and list its tools
        let tools = self
            .mcp_initialize_and_list_tools_stdio(&mut stdin, &mut reader, &server_id, &server_name)
            .await?;

        // Store the connection for later cleanup
        let connection = StdioConnection { child };

        {
            let mut connections = self.stdio_connections.write().await;
            connections.insert(server_id.clone(), connection);
        }

        let tool_count = tools.len();

        {
            let mut tools_lock = self.tools.write().await;
            tools_lock.extend(tools);
        }

        info!(
            "Connected to MCP server '{}' with {} tools",
            server_name, tool_count
        );
        Ok(())
    }

    async fn connect_sse(&self, server_name: &str) -> Result<(), String> {
        Err(format!(
            "SSE transport not yet implemented for server {}",
            server_name
        ))
    }

    async fn connect_streamable_http(&self, server_name: &str) -> Result<(), String> {
        Err(format!(
            "StreamableHTTP transport not yet implemented for server {}",
            server_name
        ))
    }

    async fn mcp_initialize_and_list_tools_stdio(
        &self,
        stdin: &mut tokio::process::ChildStdin,
        reader: &mut BufReader<tokio::process::ChildStdout>,
        server_id: &str,
        server_name: &str,
    ) -> Result<Vec<McpServerTool>, String> {
        debug!("[MCP] Starting MCP handshake for {}", server_name);

        // Send initialize request
        let init_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "Cast Client",
                    "version": "0.2.0"
                }
            }
        });

        let request_str = serde_json::to_string(&init_request).unwrap() + "\n";
        debug!("[MCP] Sending initialize: {}", request_str.trim());

        stdin
            .write_all(request_str.as_bytes())
            .await
            .map_err(|e| format!("Failed to write initialize request: {e}"))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush stdin: {e}"))?;

        // Read initialize response
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("Failed to read initialize response: {e}"))?;

        debug!("[MCP] Received initialize response: {}", line.trim());

        let init_response: serde_json::Value = serde_json::from_str(line.trim())
            .map_err(|e| format!("Failed to parse initialize response: {e}, raw: {line}"))?;

        if let Some(error) = init_response.get("error") {
            return Err(format!("MCP initialize error: {}", error));
        }

        let _capabilities = init_response
            .get("result")
            .unwrap_or(&serde_json::Value::Null);
        debug!("[MCP] Server capabilities: {}", _capabilities);

        // Send initialized notification
        let initialized_notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });

        let notify_str = serde_json::to_string(&initialized_notification).unwrap() + "\n";
        stdin
            .write_all(notify_str.as_bytes())
            .await
            .map_err(|e| format!("Failed to write initialized notification: {e}"))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush stdin: {e}"))?;

        // Send tools/list request
        let list_tools_request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        });

        let request_str = serde_json::to_string(&list_tools_request).unwrap() + "\n";
        debug!("[MCP] Sending tools/list request");

        stdin
            .write_all(request_str.as_bytes())
            .await
            .map_err(|e| format!("Failed to write tools/list request: {e}"))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush stdin: {e}"))?;

        // Read tools/list response
        line.clear();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("Failed to read tools/list response: {e}"))?;

        debug!("[MCP] Received tools/list response: {}", line.trim());

        let list_response: serde_json::Value = serde_json::from_str(line.trim())
            .map_err(|e| format!("Failed to parse tools/list response: {e}, raw: {line}"))?;

        if let Some(error) = list_response.get("error") {
            return Err(format!("MCP tools/list error: {}", error));
        }

        let tools = list_response["result"]["tools"]
            .as_array()
            .ok_or("No tools array in response")?;

        debug!("[MCP] Found {} tools", tools.len());

        let server_tools: Vec<McpServerTool> = tools
            .iter()
            .filter_map(|tool| {
                let name = tool["name"].as_str()?;
                let description = tool["description"]
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let input_schema = tool["inputSchema"].clone();

                let schema_map: Map<String, serde_json::Value> =
                    if let serde_json::Value::Object(map) = input_schema {
                        map
                    } else {
                        Map::new()
                    };

                Some(McpServerTool {
                    server_id: server_id.to_string(),
                    tool: McpTool::new(name.to_string(), description, Arc::new(schema_map)),
                })
            })
            .collect();

        Ok(server_tools)
    }

    pub async fn disconnect_server(&self, server_id: &str) {
        let mut connections = self.stdio_connections.write().await;
        if let Some(mut conn) = connections.remove(server_id) {
            let _ = conn.child.kill().await;
        }

        let mut tools = self.tools.write().await;
        tools.retain(|t| t.server_id != server_id);
    }

    pub async fn update_server_config(&self, config: McpServerConfig) {
        info!("[MCP] update_server_config for: {}", config.name);
        let server_id = config.id.clone();
        let server_name = config.name.clone();
        let enabled = config.enabled;
        let was_enabled = {
            let configs = self.server_configs.read().await;
            configs
                .iter()
                .find(|c| c.id == server_id)
                .map(|c| c.enabled)
                .unwrap_or(false)
        };

        {
            let mut configs = self.server_configs.write().await;
            if let Some(idx) = configs.iter().position(|c| c.id == server_id) {
                configs[idx] = config.clone();
            } else {
                configs.push(config.clone());
            }
        }

        info!("[MCP] was_enabled={}, enabled={}", was_enabled, enabled);

        if was_enabled && !enabled {
            self.disconnect_server(&server_id).await;
        } else if !was_enabled && enabled {
            info!("[MCP] Connecting to new server: {}", server_name);
            if let Err(e) = self.connect_server(config).await {
                error!(
                    "[MCP] Failed to connect to MCP server {}: {}",
                    server_name, e
                );
            }
        } else if was_enabled && enabled {
            info!("[MCP] Reconnecting to server: {}", server_name);
            self.disconnect_server(&server_id).await;
            if let Err(e) = self.connect_server(config.clone()).await {
                error!(
                    "[MCP] Failed to reconnect to MCP server {}: {}",
                    server_name, e
                );
            }
        }
    }

    pub async fn remove_server(&self, server_id: &str) {
        self.disconnect_server(server_id).await;
        let mut configs = self.server_configs.write().await;
        configs.retain(|c| c.id != server_id);
    }

    pub async fn get_genai_tools(&self) -> Vec<genai::chat::Tool> {
        let tools = self.tools.read().await;
        convert_mcp_tools_to_genai(&tools)
    }

    pub async fn execute_tool(
        &self,
        _tool_name: &str,
        _arguments: serde_json::Value,
    ) -> Result<rmcp::model::CallToolResult, String> {
        Err("MCP tool execution not yet fully implemented".to_string())
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}
