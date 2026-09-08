use crate::chat::mcp::{McpServerConfig, McpServerTool, convert_mcp_tools_to_genai};
use rmcp::model::{CallToolRequestParams, Tool as McpTool};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::ServiceExt;
use serde_json::{Value, json};
use serde_json::Map;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

struct StdioConnection {
    child: tokio::process::Child,
    stdin: Mutex<tokio::process::ChildStdin>,
    stdout: Mutex<BufReader<tokio::process::ChildStdout>>,
    next_id: Mutex<u64>,
}

struct SseConnection {
    client: reqwest::Client,
    endpoint_url: String,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    next_id: Mutex<u64>,
    cancel: CancellationToken,
}

enum RemoteConnection {
    Streamable(RunningService<RoleClient, ()>),
    Sse(SseConnection),
}

pub struct McpManager {
    stdio_connections: Arc<RwLock<HashMap<String, StdioConnection>>>,
    remote_connections: Arc<RwLock<HashMap<String, RemoteConnection>>>,
    tools: Arc<RwLock<Vec<McpServerTool>>>,
    server_configs: Arc<RwLock<Vec<McpServerConfig>>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            stdio_connections: Arc::new(RwLock::new(HashMap::new())),
            remote_connections: Arc::new(RwLock::new(HashMap::new())),
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
            crate::chat::mcp::McpTransport::Sse => {
                self.connect_sse(config, server_id, server_name).await
            }
            crate::chat::mcp::McpTransport::StreamableHttp => {
                self.connect_streamable_http(config, server_id, server_name).await
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
            .stdio_handshake_and_list_tools(&mut stdin, &mut reader, &server_id, &server_name)
            .await?;

        // Store the connection for later tool calls
        let connection = StdioConnection {
            child,
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(reader),
            next_id: Mutex::new(3), // 1 = initialize, 2 = tools/list
        };

        self.stdio_connections
            .write()
            .await
            .insert(server_id.clone(), connection);

        let tool_count = tools.len();
        self.tools.write().await.extend(tools);

        info!(
            "Connected to MCP server '{}' with {} tools",
            server_name, tool_count
        );
        Ok(())
    }

    async fn connect_streamable_http(
        &self,
        config: McpServerConfig,
        server_id: String,
        server_name: String,
    ) -> Result<(), String> {
        let url = config.url.clone().ok_or("No URL for streamable http transport")?;

        info!("[MCP] Connecting via StreamableHTTP to {}", url);

        let mut transport_config = StreamableHttpClientTransportConfig::with_uri(url);
        if !config.headers.is_empty() {
            let mut headers = HashMap::new();
            for (name, value) in &config.headers {
                let name = http::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
                let value = http::HeaderValue::from_str(value)
                    .map_err(|e| format!("Invalid header value for '{name}': {e}"))?;
                headers.insert(name, value);
            }
            transport_config.custom_headers = headers;
        }

        let transport =
            StreamableHttpClientTransport::<reqwest::Client>::from_config(transport_config);

        let service = ()
            .serve(transport)
            .await
            .map_err(|e| {
                let msg = format!("Failed to initialize StreamableHTTP connection: {e}");
                if msg.contains("404") {
                    format!("{msg}. Hint: the server's MCP endpoint is usually at a path like /mcp — try http://host:port/mcp")
                } else {
                    msg
                }
            })?;

        // List tools (handles pagination)
        let all_tools: Vec<McpTool> = service
            .peer()
            .list_all_tools()
            .await
            .map_err(|e| format!("Failed to list tools: {e}"))?;

        let mcp_server_tools: Vec<McpServerTool> = all_tools
            .into_iter()
            .map(|tool| McpServerTool {
                server_id: server_id.clone(),
                tool,
            })
            .collect();

        let tool_count = mcp_server_tools.len();
        self.tools.write().await.extend(mcp_server_tools);
        self.remote_connections
            .write()
            .await
            .insert(server_id, RemoteConnection::Streamable(service));

        info!(
            "Connected to MCP server '{}' (streamable-http) with {} tools",
            server_name, tool_count
        );
        Ok(())
    }

    async fn connect_sse(
        &self,
        config: McpServerConfig,
        server_id: String,
        server_name: String,
    ) -> Result<(), String> {
        let url = config.url.clone().ok_or("No URL for SSE transport")?;

        info!("[MCP] Connecting via SSE to {}", url);

        let mut header_map = reqwest::header::HeaderMap::new();
        for (name, value) in &config.headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            let value = reqwest::header::HeaderValue::from_str(value)
                .map_err(|e| format!("Invalid header value for '{name}': {e}"))?;
            header_map.insert(name, value);
        }

        let client = reqwest::Client::builder()
            .default_headers(header_map)
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

        use eventsource_stream::Eventsource;
use futures_util::StreamExt;

        let response = client
            .get(&url)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(|e| format!("SSE connection failed: {e}"))?;

        if !response.status().is_success() {
            return Err(format!("SSE endpoint returned {}", response.status()));
        }

        let mut event_stream = response.bytes_stream().eventsource();

        // Wait for the "endpoint" event that tells us where to POST messages
        let endpoint_url = loop {
            match tokio::time::timeout(REQUEST_TIMEOUT, event_stream.next()).await {
                Err(_) => return Err("Timed out waiting for SSE endpoint event".to_string()),
                Ok(None) => return Err("SSE stream closed before endpoint event".to_string()),
                Ok(Some(Err(e))) => return Err(format!("SSE stream error: {e}")),
                Ok(Some(Ok(event))) => {
                    debug!("[MCP-SSE] Event '{}': {}", event.event, event.data.trim());
                    if event.event == "endpoint" {
                        break resolve_endpoint(&url, &event.data)?;
                    }
                    // ignore other pre-handshake events (e.g. keepalives)
                }
            }
        };

        info!("[MCP-SSE] POST endpoint resolved: {}", endpoint_url);

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let cancel = CancellationToken::new();

        // Background task: route responses from the SSE stream to awaiting callers
        let reader_pending = pending.clone();
        let reader_cancel = cancel.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = reader_cancel.cancelled() => break,
                    event = event_stream.next() => match event {
                        Some(Ok(event)) => {
                            if event.event != "message" && !event.event.is_empty() {
                                continue;
                            }
                            let Ok(value) = serde_json::from_str::<Value>(&event.data) else {
                                continue;
                            };
                            let Some(id) = value.get("id").and_then(|i| i.as_u64()) else {
                                continue; // notification or parse-level noise
                            };
                            if let Some(tx) = reader_pending.lock().await.remove(&id) {
                                let _ = tx.send(value);
                            }
                        }
                        _ => break, // error or stream closed
                    },
                }
            }
        });

        let conn = SseConnection {
            client,
            endpoint_url,
            pending,
            next_id: Mutex::new(3), // 1 = initialize, 2 = tools/list
            cancel,
        };

        // Handshake: initialize -> initialized notification -> tools/list
        let init_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "Cast Client", "version": "0.2.0" }
            }
        });
        let init_response = sse_request(&conn, init_request).await?;
        if let Some(err) = init_response.get("error") {
            conn.cancel.cancel();
            return Err(format!("MCP initialize error: {err}"));
        }

        let initialized_notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        if let Err(e) = conn
            .client
            .post(&conn.endpoint_url)
            .json(&initialized_notification)
            .send()
            .await
        {
            conn.cancel.cancel();
            return Err(format!("Failed to send initialized notification: {e}"));
        }

        let list_request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        });
        let list_response = sse_request(&conn, list_request).await?;
        if let Some(err) = list_response.get("error") {
            conn.cancel.cancel();
            return Err(format!("MCP tools/list error: {err}"));
        }

        let tools = match list_response["result"]["tools"].as_array() {
            Some(t) => t.clone(),
            None => {
                conn.cancel.cancel();
                return Err("No tools array in tools/list response".to_string());
            }
        };

        let mcp_server_tools: Vec<McpServerTool> = tools
            .iter()
            .filter_map(|tool| {
                let name = tool["name"].as_str()?;
                let description = tool["description"].as_str().unwrap_or_default();
                let schema_map: Map<String, Value> = match tool["inputSchema"].clone() {
                    Value::Object(map) => map,
                    _ => Map::new(),
                };
                Some(McpServerTool {
                    server_id: server_id.clone(),
                    tool: McpTool::new(name.to_string(), description.to_string(), Arc::new(schema_map)),
                })
            })
            .collect();

        let tool_count = mcp_server_tools.len();
        self.tools.write().await.extend(mcp_server_tools);
        self.remote_connections
            .write()
            .await
            .insert(server_id, RemoteConnection::Sse(conn));

        info!(
            "Connected to MCP server '{}' (sse) with {} tools",
            server_name, tool_count
        );
        Ok(())
    }

    async fn stdio_handshake_and_list_tools(
        &self,
        stdin: &mut tokio::process::ChildStdin,
        reader: &mut BufReader<tokio::process::ChildStdout>,
        server_id: &str,
        server_name: &str,
    ) -> Result<Vec<McpServerTool>, String> {
        debug!("[MCP] Starting MCP handshake for {}", server_name);

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

        write_line(stdin, &init_request).await?;

        let line = read_line(reader).await?;
        debug!("[MCP] Received initialize response: {}", line);
        let init_response: Value = parse_jsonrpc(&line)?;

        if let Some(error) = init_response.get("error") {
            return Err(format!("MCP initialize error: {}", error));
        }

        let initialized_notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        write_line(stdin, &initialized_notification).await?;

        let list_tools_request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        });
        write_line(stdin, &list_tools_request).await?;

        let line = read_line(reader).await?;
        debug!("[MCP] Received tools/list response: {}", line);
        let list_response: Value = parse_jsonrpc(&line)?;

        if let Some(error) = list_response.get("error") {
            return Err(format!("MCP tools/list error: {}", error));
        }

        let tools = list_response["result"]["tools"]
            .as_array()
            .ok_or("No tools array in response")?
            .clone();

        debug!("[MCP] Found {} tools", tools.len());

        let server_tools: Vec<McpServerTool> = tools
            .iter()
            .filter_map(|tool| {
                let name = tool["name"].as_str()?;
                let description = tool["description"].as_str().unwrap_or_default();
                let schema_map: Map<String, Value> = match tool["inputSchema"].clone() {
                    Value::Object(map) => map,
                    _ => Map::new(),
                };
                Some(McpServerTool {
                    server_id: server_id.to_string(),
                    tool: McpTool::new(name.to_string(), description.to_string(), Arc::new(schema_map)),
                })
            })
            .collect();

        Ok(server_tools)
    }

    pub async fn disconnect_server(&self, server_id: &str) {
        if let Some(mut conn) = self.stdio_connections.write().await.remove(server_id) {
            let _ = conn.child.kill().await;
        }

        if let Some(remote) = self.remote_connections.write().await.remove(server_id) {
            match remote {
                RemoteConnection::Streamable(service) => {
                    let _ = service.cancel().await;
                }
                RemoteConnection::Sse(conn) => conn.cancel.cancel(),
            }
        }

        self.tools
            .write()
            .await
            .retain(|t| t.server_id != server_id);
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

    /// Executes an MCP tool. `full_name` is the genai-facing name:
    /// `mcp__{server_id}__{tool_name}`
    pub async fn execute_tool(
        &self,
        full_name: &str,
        arguments: Value,
    ) -> Result<rmcp::model::CallToolResult, String> {
        let rest = full_name
            .strip_prefix("mcp__")
            .ok_or_else(|| format!("Not an MCP tool: {full_name}"))?;
        let (server_id, tool_name) = rest
            .split_once("__")
            .ok_or_else(|| format!("Malformed MCP tool name: {full_name}"))?;

        // 1. Stdio?
        {
            let connections = self.stdio_connections.read().await;
            if let Some(conn) = connections.get(server_id) {
                return stdio_call(conn, tool_name, arguments).await;
            }
        }

        // 2. Remote (streamable-http or sse)?
        {
            let connections = self.remote_connections.read().await;
            match connections.get(server_id) {
                Some(RemoteConnection::Streamable(service)) => {
                    let mut params = CallToolRequestParams::new(tool_name.to_string());
                    params.arguments = arguments.as_object().cloned();
                    let result = service
                        .peer()
                        .call_tool(params)
                        .await
                        .map_err(|e| format!("MCP tool call failed: {e}"))?;
                    return Ok(result);
                }
                Some(RemoteConnection::Sse(conn)) => {
                    let id = allocate_id(&conn.next_id).await;
                    let request = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "method": "tools/call",
                        "params": {
                            "name": tool_name,
                            "arguments": arguments
                        }
                    });
                    let response = sse_request(conn, request).await?;
                    return jsonrpc_result_to_call_tool_result(response);
                }
                None => {}
            }
        }

        Err(format!("MCP server not connected: {server_id}"))
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------- helpers ----------

async fn write_line(stdin: &mut tokio::process::ChildStdin, value: &Value) -> Result<(), String> {
    let mut line = serde_json::to_string(value).map_err(|e| format!("Serialize failed: {e}"))?;
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| format!("Failed to write to stdin: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush stdin: {e}"))
}

async fn read_line(reader: &mut BufReader<tokio::process::ChildStdout>) -> Result<String, String> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .map_err(|e| format!("Failed to read from stdout: {e}"))?;
    if line.is_empty() {
        return Err("Server closed stdout".to_string());
    }
    Ok(line.trim().to_string())
}

fn parse_jsonrpc(line: &str) -> Result<Value, String> {
    serde_json::from_str(line).map_err(|e| format!("Failed to parse JSON-RPC message: {e}, raw: {line}"))
}

async fn allocate_id(next_id: &Mutex<u64>) -> u64 {
    let mut id = next_id.lock().await;
    let current = *id;
    *id += 1;
    current
}

async fn stdio_call(
    conn: &StdioConnection,
    tool_name: &str,
    arguments: Value,
) -> Result<rmcp::model::CallToolResult, String> {
    let id = allocate_id(&conn.next_id).await;
    let request = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    });

    {
        let mut stdin = conn.stdin.lock().await;
        write_line(&mut stdin, &request).await?;
    }

    let line = {
        let mut stdout = conn.stdout.lock().await;
        read_line(&mut stdout).await?
    };

    let response: Value = parse_jsonrpc(&line)?;
    jsonrpc_result_to_call_tool_result(response)
}

/// Sends a JSON-RPC request over an SSE connection's POST endpoint and awaits
/// the matching response arriving on the GET event stream.
async fn sse_request(conn: &SseConnection, request: Value) -> Result<Value, String> {
    let id = request["id"]
        .as_u64()
        .ok_or("Request has no numeric id")?;

    let (tx, rx) = oneshot::channel();
    conn.pending.lock().await.insert(id, tx);

    let post = conn.client.post(&conn.endpoint_url).json(&request);
    let response = match post.send().await {
        Ok(r) => r,
        Err(e) => {
            conn.pending.lock().await.remove(&id);
            return Err(format!("Failed to POST to SSE endpoint: {e}"));
        }
    };

    if !response.status().is_success() {
        conn.pending.lock().await.remove(&id);
        return Err(format!("SSE endpoint POST returned {}", response.status()));
    }

    match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) => Err("SSE response channel dropped".to_string()),
        Err(_) => {
            conn.pending.lock().await.remove(&id);
            Err("Timed out waiting for SSE response".to_string())
        }
    }
}

fn jsonrpc_result_to_call_tool_result(
    response: Value,
) -> Result<rmcp::model::CallToolResult, String> {
    if let Some(error) = response.get("error") {
        return Err(format!("MCP tool error: {error}"));
    }
    let result = response
        .get("result")
        .ok_or("No result in tool response")?;
    serde_json::from_value(result.clone())
        .map_err(|e| format!("Failed to parse CallToolResult: {e}"))
}

/// Resolves the POST endpoint from the SSE `endpoint` event data, which may be
/// an absolute URL or a path relative to the server base.
fn resolve_endpoint(base_url: &str, data: &str) -> Result<String, String> {
    let data = data.trim();
    if data.is_empty() {
        return Err("Empty endpoint event data".to_string());
    }
    if data.starts_with("http://") || data.starts_with("https://") {
        return Ok(data.to_string());
    }
    let origin_end = base_url.find("://").map(|i| {
        base_url[i + 3..]
            .find('/')
            .map(|j| i + 3 + j)
            .unwrap_or(base_url.len())
    });
    match origin_end {
        Some(end) => Ok(format!("{}/{}", &base_url[..end], data.trim_start_matches('/'))),
        None => Err(format!("Cannot resolve endpoint '{data}' against '{base_url}'")),
    }
}
