use futures_util::StreamExt;
use genai::Client;
use genai::chat::{
    ChatMessage, ChatOptions, ChatRequest, ChatRole, ChatStreamEvent, ToolCall, ToolResponse,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::chat::ExecMode;
use crate::chat::mcp_client::McpManager;
use crate::chat::tools::{execute_tool, get_enabled_builtin_tools};
use crate::types::{
    CompletionEvent, Conversation, GenerationPhase, GenerationState, Message, Selected,
};

pub const DEFAULT_SYSTEM_PROMPT: &str = "You are a smart AI assistant interacting with the user \
in a native app called Cast Client. \
You have tools available; use them when they help answer the request. \
Cast renders Markdown, so keep responses well formatted. \
Be concise and do not repeat yourself.";
const MAX_CONVERSATIONS: usize = 50;
const MAX_MESSAGES: usize = 100;

fn create_request(
    mode: ExecMode,
    messages: Vec<ChatMessage>,
    system: Option<String>,
    tool_preferences: &HashMap<String, bool>,
    mcp_tools: Vec<genai::chat::Tool>,
) -> ChatRequest {
    let mut tools = match mode {
        ExecMode::Chat => vec![],
        ExecMode::Agent => get_enabled_builtin_tools(tool_preferences),
    };

    tools.extend(mcp_tools);

    let system_p = system.as_deref().unwrap_or(DEFAULT_SYSTEM_PROMPT);

    ChatRequest::new(messages)
        .with_tools(tools)
        .with_system(system_p)
}

pub struct SendContext<'a> {
    pub convos: &'a mut Vec<Conversation>,
    pub active: &'a mut Selected,
    pub client: &'a Client,
    pub model: String,
    pub tx: &'a Sender<CompletionEvent>,
    pub mode: ExecMode,
    pub system: Option<String>,
    pub tool_preferences: &'a HashMap<String, bool>,
    pub mcp_manager: &'a Arc<McpManager>,
}

/// Pushes the user message onto the active
/// conversation, kicks off an agentic streaming loop on the tokio runtime, and
/// returns the cancellation token + conversation index.
pub fn send_message(
    text: Option<String>,
    ctx: SendContext<'_>,
) -> Option<(usize, CancellationToken)> {
    let val = text?;
    if val.trim().is_empty() {
        return None;
    }

    let SendContext {
        convos,
        active,
        client,
        model,
        tx,
        mode,
        system,
        tool_preferences,
        mcp_manager,
    } = ctx;

    let idx = match *active {
        Selected::Index(idx) => idx,
        Selected::New => {
            convos.push(Conversation {
                title: "New Chat".into(),
                messages: Vec::new(),
            });
            if convos.len() > MAX_CONVERSATIONS {
                convos.drain(0..convos.len() - MAX_CONVERSATIONS);
            }
            let new_idx = convos.len() - 1;
            *active = Selected::Index(new_idx);
            new_idx
        }
    };

    let user_msg = ChatMessage::user(val.clone());
    convos[idx].messages.push(Message::new(user_msg));

    if convos[idx].title == "New Chat" {
        let title = val.chars().take(50).collect::<String>();
        if !title.is_empty() {
            convos[idx].title = title;
        }
    }

    let msg_len = convos[idx].messages.len();
    if msg_len > MAX_MESSAGES {
        convos[idx].messages.drain(0..msg_len - MAX_MESSAGES);
    }

    let mut request_messages: Vec<ChatMessage> = convos[idx]
        .messages
        .iter()
        .map(|m| m.message.clone())
        .collect();
    if let Some(last) = request_messages.last()
        && matches!(last.role, ChatRole::Assistant)
        && last.content.is_empty()
    {
        request_messages.pop();
    }

    // Fetch MCP tools (async but we block here)
    let mcp_tools = {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(mcp_manager.get_genai_tools())
    };

    let request = create_request(mode, request_messages, system, tool_preferences, mcp_tools);

    let client = client.clone();
    let tx = tx.clone();
    let cancel = CancellationToken::new();
    let cancel_for_task = cancel.clone();
    let mcp_manager = mcp_manager.clone();
    tokio::spawn(async move {
        run_agent_loop(client, model, request, tx, cancel_for_task, mcp_manager).await;
    });

    crate::storage::save_conversations(convos);

    Some((idx, cancel))
}

/// Agentic streaming loop: streams chunks, accumulates tool calls, executes them,
async fn run_agent_loop(
    client: Client,
    model: String,
    mut request: ChatRequest,
    tx: Sender<CompletionEvent>,
    cancel: CancellationToken,
    mcp_manager: Arc<McpManager>,
) {
    const MAX_TOOL_TURNS: usize = 8;
    for turn in 0..MAX_TOOL_TURNS {
        if cancel.is_cancelled() {
            info!("[AGENT] Cancellation requested before API call");

            let _ = tx.send(CompletionEvent::Cancelled);
            return;
        }

        let options = ChatOptions {
            capture_tool_calls: Some(true),
            ..Default::default()
        };
        let mut stream = match client
            .exec_chat_stream(&model, request.clone(), Some(&options))
            .await
        {
            Ok(stream) => stream.stream,

            Err(err) => {
                error!("[AGENT] Failed to start stream: {err}");
                let _ = tx.send(CompletionEvent::Error(err.to_string()));

                return;
            }
        };

        let mut end_event = None;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = tx.send(CompletionEvent::Cancelled);
                    return;
                }

                event = stream.next() => {
                    let Some(event) = event else {
                        break;
                    };

                    match event {
                        Ok(ChatStreamEvent::Chunk(chunk)) => {
                            if !chunk.content.is_empty() {
                                let _ =
                                    tx.send(CompletionEvent::Chunk(chunk.content));
                            }
                        }

                        Ok(ChatStreamEvent::ToolCallChunk(_tool_chunk)) => {}

                        Ok(ChatStreamEvent::End(end)) => {
                            end_event = Some(end);
                            break;
                        }

                        Err(err) => {
                            error!("[AGENT] Stream error: {err}");
                            let _ = tx.send(
                                CompletionEvent::Error(err.to_string())
                            );

                            return;
                        }

                        other => {
                            debug!("[AGENT] Unhandled stream event: {other:?}");
                        }
                    }
                }
            }
        }

        let Some(end) = end_event else {
            error!("[AGENT] Stream ended without a terminal event");
            let _ = tx.send(CompletionEvent::Error(
                "Stream ended without a terminal event.".into(),
            ));

            return;
        };

        let Some(assistant_tool_message) = end.into_assistant_message_for_tool_use() else {
            let _ = tx.send(CompletionEvent::Finished);

            return;
        };

        let tool_calls: Vec<ToolCall> = assistant_tool_message
            .content
            .parts()
            .iter()
            .filter_map(|part| part.as_tool_call().cloned())
            .collect();

        if tool_calls.is_empty() {
            let _ = tx.send(CompletionEvent::Finished);

            return;
        }

        if tool_calls.len() > 8 {
            warn!(
                "[AGENT] Too many tool calls in one turn: {}",
                tool_calls.len()
            );

            let _ = tx.send(CompletionEvent::Error(format!(
                "Agent requested {} tools in one turn.",
                tool_calls.len()
            )));

            return;
        }

        let mut tool_responses = Vec::with_capacity(tool_calls.len());

        for call in tool_calls.iter() {
            if cancel.is_cancelled() {
                info!("[AGENT] Cancelled before tool execution");

                let _ = tx.send(CompletionEvent::Cancelled);

                return;
            }

            let _ = tx.send(CompletionEvent::ToolCallStarted {
                tool_name: call.fn_name.clone(),
            });

            let output = if call.fn_name.starts_with("mcp__") {
                match mcp_manager
                    .execute_tool(&call.fn_name, call.fn_arguments.clone())
                    .await
                {
                    Ok(result) => {
                        debug!("[MCP TOOL] Success: {}", call.fn_name);
                        serde_json::to_string(&result)
                            .unwrap_or_else(|_| "MCP tool executed".to_string())
                    }
                    Err(err) => {
                        error!("[MCP TOOL] Failure ({}): {err}", call.fn_name);
                        format!("MCP tool execution error: {}", err)
                    }
                }
            } else {
                match execute_tool(call).await {
                    Ok(result) => {
                        debug!("[TOOL] Success: {}", call.fn_name);
                        result
                    }
                    Err(err) => {
                        error!("[TOOL] Failure ({}): {err}", call.fn_name);
                        format!("Tool execution error: {}", err)
                    }
                }
            };

            tool_responses.push(ToolResponse::from_tool_call(call, output));
        }

        let _ = tx.send(CompletionEvent::ToolTurnCompleted {
            tool_calls: tool_calls.clone(),
        });
        request.messages.push(assistant_tool_message);

        request.messages.push(tool_responses.into());

        if turn + 1 >= MAX_TOOL_TURNS {
            warn!("[AGENT] Max tool turns ({MAX_TOOL_TURNS}) reached");

            let _ = tx.send(CompletionEvent::Error(
                "Agent stopped after reaching the maximum number of tool turns.".into(),
            ));

            return;
        }
    }
}

pub fn poll_events(
    rx: &std::sync::mpsc::Receiver<CompletionEvent>,
    convos: &mut [Conversation],
    generation: &mut GenerationState,
    active_cancel: &mut Option<CancellationToken>,
) {
    while let Ok(event) = rx.try_recv() {
        let convo_idx = match generation {
            GenerationState::Active { convo_idx, .. } => *convo_idx,
            GenerationState::Idle => {
                warn!(
                    "[POLL] Ignoring event while generation is idle: {:?}",
                    event
                );
                continue;
            }
        };

        match event {
            CompletionEvent::Chunk(text) => {
                use genai::chat::ContentPart;
                if let Some(last) = convos[convo_idx].messages.last_mut()
                    && matches!(last.message.role, ChatRole::Assistant)
                {
                    last.message.content.push(ContentPart::Text(text));
                } else {
                    convos[convo_idx]
                        .messages
                        .push(Message::new(ChatMessage::assistant(text)));
                }

                *generation = GenerationState::Active {
                    convo_idx,
                    phase: GenerationPhase::Thinking,
                };
            }

            CompletionEvent::ToolCallStarted { tool_name } => {
                if !convos[convo_idx]
                    .messages
                    .last()
                    .is_some_and(|msg| matches!(msg.message.role, ChatRole::Assistant))
                {
                    convos[convo_idx]
                        .messages
                        .push(Message::new(ChatMessage::assistant(String::new())));
                }

                *generation = GenerationState::Active {
                    convo_idx,
                    phase: GenerationPhase::ExecutingTool { tool_name },
                };
            }

            CompletionEvent::ToolTurnCompleted { tool_calls } => {
                if let Some(last) = convos[convo_idx].messages.last_mut()
                    && matches!(last.message.role, ChatRole::Assistant)
                {
                    last.tools_called = tool_calls.len();
                } else {
                    convos[convo_idx].messages.push(Message {
                        message: ChatMessage::assistant(String::new()),
                        tools_called: tool_calls.len(),
                        is_error: false,
                    });
                }

                *generation = GenerationState::Active {
                    convo_idx,
                    phase: GenerationPhase::Thinking,
                };
            }

            CompletionEvent::Finished => {
                *generation = GenerationState::Idle;
                *active_cancel = None;

                crate::storage::save_conversations(convos);
            }

            CompletionEvent::Cancelled => {
                *generation = GenerationState::Idle;
                *active_cancel = None;

                crate::storage::save_conversations(convos);
            }

            CompletionEvent::Error(e) => {
                error!("[POLL] Generation error for convo {convo_idx}: {e}");

                convos[convo_idx]
                    .messages
                    .push(Message::error(ChatMessage::assistant(format!(
                        "Error {}",
                        e
                    ))));

                *generation = GenerationState::Idle;
                *active_cancel = None;

                crate::storage::save_conversations(convos);
            }
        }
    }
}
