use serde_json::{Value, json};

use crate::types::Message;

pub struct CompletionRequest {
    model: String,
    messages: Vec<Message>,
    streaming: bool,
    system_prompt: Option<String>,
}

pub struct CompReqBuilder {
    model: Option<String>,
    messages: Vec<Message>,
    streaming: bool,
    system_prompt: Option<String>
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
     pub fn system_prompt(mut self, prompt: Option<String>) -> Self {
        self.system_prompt = prompt;
        self
    }
    pub fn build(self) -> CompletionRequest {
        let model = self.model.unwrap_or("gemini-3.5-flash".into());
        CompletionRequest {
            model,
            messages: self.messages,
            streaming: self.streaming,
            system_prompt: self.system_prompt,
        }
    }
}

impl CompletionRequest {
    pub fn new() -> CompReqBuilder {
        CompReqBuilder {
            model: None,
            messages: vec![],
            streaming: true,
            system_prompt: None,
        }
    }

    pub fn to_json(&self) -> Value {
        let system = json!({
            "role": "system",
            "content": get_system_prompt(&self.system_prompt)
        });
        let mut msgs: Vec<Value> = vec![system];
        msgs.extend(self.messages.iter().map(|m| Into::<Value>::into(m)));
        
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

fn get_system_prompt<'a>(prompt: &'a Option<String>) -> &'a str {
    match prompt {
        Some(s) if !s.trim().is_empty() => s.as_str(),
        _ => {
            "You are an assistant running inside Cast Client, a lightweight native \
            chat application. Keep responses concise and well-formatted in markdown."
        }
    }
}
