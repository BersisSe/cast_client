use genai::chat::Tool;
use rmcp::model::Tool as McpTool;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    #[default]
    Stdio,
    Sse,
    StreamableHttp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    #[serde(default = "generate_id")]
    pub id: String,
    pub name: String,
    pub transport: McpTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_filter: Option<Vec<String>>,
}

fn generate_id() -> String {
    Uuid::new_v4().to_string()
}

fn default_true() -> bool {
    true
}

impl McpServerConfig {
    pub fn new_stdio(name: String, command: String, args: Vec<String>) -> Self {
        Self {
            id: generate_id(),
            name,
            transport: McpTransport::Stdio,
            command: Some(command),
            args,
            url: None,
            headers: HashMap::new(),
            enabled: true,
            tool_filter: None,
        }
    }

    pub fn new_sse(name: String, url: String) -> Self {
        Self {
            id: generate_id(),
            name,
            transport: McpTransport::Sse,
            command: None,
            args: Vec::new(),
            url: Some(url),
            headers: HashMap::new(),
            enabled: true,
            tool_filter: None,
        }
    }

    pub fn new_streamable_http(name: String, url: String) -> Self {
        Self {
            id: generate_id(),
            name,
            transport: McpTransport::StreamableHttp,
            command: None,
            args: Vec::new(),
            url: Some(url),
            headers: HashMap::new(),
            enabled: true,
            tool_filter: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self.transport {
            McpTransport::Stdio => {
                if self.command.is_none() || self.command.as_ref().unwrap().is_empty() {
                    return Err("Stdio transport requires a command".to_string());
                }
            }
            McpTransport::Sse | McpTransport::StreamableHttp => {
                if self.url.is_none() || self.url.as_ref().unwrap().is_empty() {
                    return Err("SSE/StreamableHTTP transport requires a URL".to_string());
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct McpServerTool {
    pub server_id: String,
    pub tool: McpTool,
}

impl McpServerTool {
    pub fn to_genai_tool(&self) -> Tool {
        let schema = serde_json::to_value(&self.tool.input_schema).unwrap_or(serde_json::json!({}));
        Tool::new(format!("mcp__{}__{}", self.server_id, self.tool.name))
            .with_description(self.tool.description.clone().unwrap_or_default())
            .with_schema(schema)
    }
}

pub fn convert_mcp_tools_to_genai(tools: &[McpServerTool]) -> Vec<Tool> {
    tools.iter().map(|t| t.to_genai_tool()).collect()
}
