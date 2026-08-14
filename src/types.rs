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

pub enum ActiveConvoData{
    generating = Option<usize>
}


#[derive(Debug,Clone, Serialize, Deserialize)]
pub struct Conversation{
    pub title: String,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug,Clone, Serialize, Deserialize)]
pub enum Selected{
    New,
    Index(usize)
}
