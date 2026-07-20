use serde::Serialize;

use crate::types::Message;
use crate::types::MessageSender;

#[derive(Serialize)]
struct ChatBody<'a> {
    model: &'a str,
    messages: Vec<MessageBody<'a>>,
    stream: bool,
    reasoning_effort: &'a str,
}

#[derive(Serialize)]
struct MessageBody<'a> {
    role: &'a str,
    content: &'a str,
}

pub fn build_chat_body(model: &str, messages: &[Message], system_prompt: &Option<String>) -> String {
    let system_content = get_system_prompt(system_prompt);

    let mut msgs = Vec::with_capacity(messages.len() + 1);
    msgs.push(MessageBody { role: "system", content: system_content });
    msgs.extend(messages.iter().map(|m| MessageBody {
        role: if m.sender == MessageSender::User { "user" } else { "assistant" },
        content: &m.content,
    }));

    let body = ChatBody {
        model,
        messages: msgs,
        stream: true,
        reasoning_effort: "low",
    };

    serde_json::to_string(&body).expect("serialization should not fail")
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
