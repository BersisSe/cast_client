pub mod completion;

// Planned as tools/MCP land:
pub mod tools; // tool schema + local dispatch
// pub mod mcp;   // MCP server connections, tool discovery


#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExecMode {
    Chat,
    Agent,
}

impl Default for ExecMode {
    fn default() -> Self {
        Self::Chat
    }
}