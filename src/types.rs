use genai::chat::ChatMessage;
use serde::{Deserialize, Serialize};


#[derive(Debug)]
pub enum CompletionEvent {
    Chunk(String),
    Finished,
    ToolCallStarted { tool_name: String },
    ToolTurnCompleted {
        tool_calls: Vec<genai::chat::ToolCall>,
        tool_responses: Vec<genai::chat::ToolResponse>,
    },
    Error(String),
    Cancelled,
}

#[derive(Debug,Clone, Serialize, Deserialize)]
pub struct Conversation{
    pub title: String,
    pub messages: Vec<Message>,
}

#[derive(Debug,Clone, Serialize, Deserialize)]
pub struct Message{
    /// The Genai Message
    pub message: ChatMessage,
    /// Number of tools Called
    pub tools_called: usize
}



#[derive(Debug,Clone, Serialize, Deserialize)]
pub enum Selected{
    New,
    Index(usize)
}


#[derive(Debug, Clone, Default)]
pub enum GenerationState {
    #[default]
    Idle,

    Active {
        convo_idx: usize,
        phase: GenerationPhase,
    },
}
#[derive(Debug, Clone)]
pub enum GenerationPhase {
    Thinking,
    ExecutingTool {
        tool_name: String,
    },
}
