use serde_json::{Value, json};

use crate::types::Message;

pub struct CompletionRequest {
    model: String,
    messages: Vec<Message>,
    streaming: bool,
}

pub struct CompReqBuilder {
    model: Option<String>,
    messages: Vec<Message>,
    streaming: bool,
}
impl CompReqBuilder {
    pub fn model(mut self, model: String) -> Self {
        self.model = Some(model);
        self
    }
    pub fn messages(mut self, msgs: Vec<Message>) -> Self {
        self.messages = msgs;
        self
    }
    pub fn streaming(mut self, is: bool) -> Self {
        self.streaming = is;
        self
    }
    pub fn build(self) -> CompletionRequest {
        let model = self.model.unwrap_or("gemini-3.5-flash".into());
        CompletionRequest { model, messages: self.messages, streaming: self.streaming }
    }
}


impl CompletionRequest {
    pub fn new() -> CompReqBuilder {
        CompReqBuilder { model: None, messages: vec![], streaming: true }
    }

    pub fn to_json(&self) -> Value {
        let msgs: Vec<Value> = self.messages.iter().map(|m| Into::<Value>::into(m)).collect();
        json!({
            "model": self.model,
            "messages": msgs,
            "stream": self.streaming,
            "reasoning_effort": "low",
        })
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming
    }
}