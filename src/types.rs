use std::fmt::Display;

use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageSender {
    User,
    AI,

}
impl Display for MessageSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageSender::User => write!(f, "User"),
            MessageSender::AI => write!(f, "AI"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub sender: MessageSender,
    pub content: String,
    pub ai_start: bool,
}

impl Message {
    pub fn new(sender: MessageSender, content: &str) -> Self {
        Self {
            sender,
            content: content.to_string(),
            
            ai_start: false,
        }
    }
    pub fn new_ai(content: &str) -> Self {
        Self {
            sender: MessageSender::AI,
            content: content.to_string(),
            
            ai_start: false,
        }
    }
    pub fn new_ai_begin() -> Self {
        Self {
            sender: MessageSender::AI,
            content: String::with_capacity(128),
            ai_start: true,
        }
    }
    pub fn display_text(&self) -> String {
        format!("{}: {}", self.sender, self.content)
    }
}

impl From<&Message> for serde_json::Value {
    fn from(msg: &Message) -> Self {
        let role = if msg.sender == MessageSender::User { "user" } else { "assistant" };
        serde_json::json!({
            "role": role,
            "content": msg.content
        })
    }
}


#[derive(Debug,Clone, Serialize, Deserialize)]
pub struct Conversation{
    pub title: String,
    pub messages: Vec<Message>
}

#[derive(Debug,Clone, Serialize, Deserialize)]
pub enum Selected{
    New,
    Index(usize)
}
