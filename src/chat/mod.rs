pub mod completion;

pub mod tools; 
// pub mod mcp;   //  Coming Soon :)


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