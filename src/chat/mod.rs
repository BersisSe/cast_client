pub mod completion;

pub mod mcp;
pub mod mcp_client;
pub mod tools;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum ExecMode {
    #[default]
    Chat,
    Agent,
}
