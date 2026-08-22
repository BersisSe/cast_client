use genai::chat::ChatMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub enum CompletionEvent {
    Chunk(String),
    Finished,
    ToolCallStarted {
        tool_name: String,
    },
    ToolTurnCompleted {
        tool_calls: Vec<genai::chat::ToolCall>,
    },
    Error(String),
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub title: String,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub message: ChatMessage,
    pub tools_called: usize,
    /// True if this message represents an error surfaced to the user
    #[serde(default)]
    pub is_error: bool,
}

impl Message {
    pub fn new(message: ChatMessage) -> Self {
        Self {
            message,
            tools_called: 0,
            is_error: false,
        }
    }

    pub fn error(message: ChatMessage) -> Self {
        Self {
            message,
            tools_called: 0,
            is_error: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Selected {
    New,
    Index(usize),
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
    ExecutingTool { tool_name: String },
}
