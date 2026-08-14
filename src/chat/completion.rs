use futures_util::StreamExt;
use genai::Client;
use genai::chat::{
    ChatMessage, ChatOptions, ChatRequest, ChatRole, ChatStreamEvent, ToolCall, ToolResponse,
};
use std::sync::mpsc::Sender;
use tokio_util::sync::CancellationToken;

use crate::chat::ExecMode;
use crate::chat::tools::execute_tool;
use crate::types::{CompletionEvent, Conversation, Selected};

pub const DEFAULT_SYSTEM_PROMPT: &str =
    "You are an Smart AI Assistant Currently interacting with the user
in a native app called Cast Client. 
- Cast gives you tools to use if you think you need them you use them.
- Cast has Markdown Rendering Capabilities So you keep your responses well formatted.
- Cast is lightweight and Token Efficincy is key, you do not repeat what you say.
";
const MAX_CONVERSATIONS: usize = 50;
const MAX_MESSAGES: usize = 100;

fn create_request(
    mode: ExecMode,
    messages: Vec<ChatMessage>,
    system: Option<String>,
) -> ChatRequest {
    let tools = match mode {
        ExecMode::Chat => vec![],
        ExecMode::Agent => crate::chat::tools::get_available_tools(),
    };

    let system_p = system.as_deref().unwrap_or(DEFAULT_SYSTEM_PROMPT);

    ChatRequest::new(messages)
        .with_tools(tools)
        .with_system(system_p)
}

/// Everything `send_message` needs, grouped to keep the call site small.
pub struct SendContext<'a> {
    pub convos: &'a mut Vec<Conversation>,
    pub active: &'a mut Selected,
    pub client: &'a Client,
    pub model: String,
    pub tx: &'a Sender<CompletionEvent>,
    pub mode: ExecMode,
    pub system: Option<String>,
}

/// Pushes the user message (+ empty assistant placeholder) onto the active
/// conversation, kicks off an agentic streaming loop on the tokio runtime, and
/// returns the cancellation token + conversation index.
pub fn send_message(text: Option<String>, ctx: SendContext) -> Option<(usize, CancellationToken)> {
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
    convos[idx].messages.push(user_msg);

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

    let mut request_messages = convos[idx].messages.clone();
    if let Some(last) = request_messages.last() {
        if matches!(last.role, ChatRole::Assistant) && last.content.is_empty() {
            request_messages.pop();
        }
    }

    let request = create_request(mode, request_messages, system);

    let client = client.clone();
    let tx = tx.clone();
    let cancel = CancellationToken::new();
    let cancel_for_task = cancel.clone();
    tokio::spawn(async move {
        run_agent_loop(client, model, request, tx, cancel_for_task).await;
    });

    crate::storage::save_conversations(convos);

    Some((idx, cancel))
}

/// Agentic streaming loop: streams chunks, accumulates tool calls, executes them,
/// appends results using `From` traits, and continues until the final text response.
async fn run_agent_loop(
    client: Client,
    model: String,
    mut request: ChatRequest,
    tx: Sender<CompletionEvent>,
    cancel: CancellationToken,
) {
    const MAX_TOOL_TURNS: usize = 10;

    println!("\n==================================================");
    println!("[AGENT] Starting agent loop");
    println!("[AGENT] Model: {}", model);
    println!("[AGENT] Initial messages: {}", request.messages.len());
    println!("==================================================\n");

    for turn in 0..MAX_TOOL_TURNS {
        let turn_number = turn + 1;

        println!("\n");
        println!("==================================================");
        println!("[AGENT] STARTING TURN {}", turn_number);
        println!("==================================================");

        if cancel.is_cancelled() {
            println!("[AGENT] Cancellation requested before API call");

            let _ = tx.send(CompletionEvent::Cancelled);
            return;
        }

        // --------------------------------------------------
        // DEBUG: Dump request history
        // --------------------------------------------------

        println!(
            "[AGENT] Request contains {} messages",
            request.messages.len()
        );

        for (i, msg) in request.messages.iter().enumerate() {
            println!("\n[AGENT] MESSAGE #{}", i);
            println!("[AGENT]   role: {:?}", msg.role);
            println!("[AGENT]   content: {:#?}", msg.content);
        }

        println!("\n--------------------------------------------------");
        println!("[AGENT] Sending request to model (turn {})", turn_number);
        println!("--------------------------------------------------");

        // --------------------------------------------------
        // API REQUEST
        // --------------------------------------------------
        let options = ChatOptions {
            capture_tool_calls: Some(true),
            ..Default::default()
        };
        let mut stream = match client
            .exec_chat_stream(&model, request.clone(), Some(&options))
            .await
        {
            Ok(stream) => {
                println!("[AGENT] API request accepted");
                stream.stream
            }

            Err(err) => {
                println!("[AGENT] !!! API REQUEST FAILED !!!");
                println!("[AGENT] Error: {}", err);

                let _ = tx.send(CompletionEvent::Error(err.to_string()));

                return;
            }
        };

        println!("[AGENT] Stream started");

        // --------------------------------------------------
        // STREAM STATE
        // --------------------------------------------------

        let mut end_event = None;
        let mut chunk_count = 0usize;
        let mut tool_chunk_count = 0usize;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    println!(
                        "[AGENT] Cancellation received while streaming turn {}",
                        turn_number
                    );

                    let _ = tx.send(CompletionEvent::Cancelled);
                    return;
                }

                event = stream.next() => {
                    let Some(event) = event else {
                        println!(
                            "[AGENT] WARNING: Stream ended without ChatStreamEvent::End"
                        );

                        break;
                    };

                    match event {
                        Ok(ChatStreamEvent::Chunk(chunk)) => {
                            chunk_count += 1;

                            println!(
                                "[STREAM] Text chunk #{}: {:?}",
                                chunk_count,
                                chunk.content
                            );

                            if !chunk.content.is_empty() {
                                let _ = tx.send(
                                    CompletionEvent::Chunk(chunk.content)
                                );
                            }
                        }

                        Ok(ChatStreamEvent::ToolCallChunk(tool_chunk)) => {
                            tool_chunk_count += 1;

                            println!(
                                "[STREAM] ToolCallChunk #{}",
                                tool_chunk_count
                            );

                            println!(
                                "[STREAM]   tool: {}",
                                tool_chunk.tool_call.fn_name
                            );

                            println!(
                                "[STREAM]   call_id: {}",
                                tool_chunk.tool_call.call_id
                            );

                            println!(
                                "[STREAM]   arguments: {}",
                                tool_chunk.tool_call.fn_arguments
                            );
                        }

                        Ok(ChatStreamEvent::End(end)) => {
                            println!("\n[STREAM] ===== END EVENT =====");

                            println!(
                                "[STREAM] captured_usage: {:?}",
                                end.captured_usage
                            );

                            println!(
                                "[STREAM] captured_stop_reason: {:?}",
                                end.captured_stop_reason
                            );

                            println!(
                                "[STREAM] captured_content: {:#?}",
                                end.captured_content
                            );

                            println!(
                                "[STREAM] captured_reasoning: {:?}",
                                end.captured_reasoning_content
                            );

                            println!(
                                "[STREAM] captured_response_id: {:?}",
                                end.captured_response_id
                            );

                            println!(
                                "[STREAM] =======================\n"
                            );

                            end_event = Some(end);
                            break;
                        }

                        Err(err) => {
                            println!("[STREAM] !!! STREAM ERROR !!!");
                            println!("[STREAM] {}", err);

                            let _ = tx.send(
                                CompletionEvent::Error(err.to_string())
                            );

                            return;
                        }

                        other => {
                            println!(
                                "[STREAM] Other event: {:?}",
                                other
                            );
                        }
                    }
                }
            }
        }

        println!(
            "[AGENT] Stream statistics: text_chunks={}, tool_call_chunks={}",
            chunk_count, tool_chunk_count
        );

        // --------------------------------------------------
        // DID WE GET AN END EVENT?
        // --------------------------------------------------

        let Some(end) = end_event else {
            println!("[AGENT] !!! NO END EVENT - STOPPING AGENT !!!");

            let _ = tx.send(CompletionEvent::Error(
                "Stream ended without a terminal event.".into(),
            ));

            return;
        };

        // --------------------------------------------------
        // EXTRACT COMPLETED TOOL CALLS
        // --------------------------------------------------

        println!("[AGENT] Extracting completed tool calls...");

        let Some(assistant_tool_message) = end.into_assistant_message_for_tool_use() else {
            println!("[AGENT] No tool calls found in final stream content.");

            println!("[AGENT] Model has finished normally.");

            let _ = tx.send(CompletionEvent::Finished);

            return;
        };

        println!("[AGENT] genai produced an assistant tool-use message");

        println!(
            "[AGENT] Assistant tool message: {:#?}",
            assistant_tool_message
        );

        // --------------------------------------------------
        // EXTRACT TOOL CALLS
        // --------------------------------------------------

        let tool_calls: Vec<ToolCall> = assistant_tool_message
            .content
            .parts()
            .iter()
            .filter_map(|part| part.as_tool_call().cloned())
            .collect();

        println!("[AGENT] Completed tool calls: {}", tool_calls.len());

        if tool_calls.is_empty() {
            println!("[AGENT] Tool-use message contained zero calls.");

            let _ = tx.send(CompletionEvent::Finished);

            return;
        }

        // --------------------------------------------------
        // DEBUG TOOL CALLS
        // --------------------------------------------------

        for (i, call) in tool_calls.iter().enumerate() {
            println!("\n[TOOL] CALL #{}", i + 1);
            println!("[TOOL]   name: {}", call.fn_name);
            println!("[TOOL]   call_id: {}", call.call_id);
            println!("[TOOL]   arguments: {}", call.fn_arguments);

            if let Some(signatures) = &call.thought_signatures {
                println!("[TOOL]   thought_signatures: {:?}", signatures);
            }
        }

        // --------------------------------------------------
        // SAFETY LIMIT
        // --------------------------------------------------

        if tool_calls.len() > 8 {
            println!("[AGENT] !!! TOO MANY TOOL CALLS: {} !!!", tool_calls.len());

            let _ = tx.send(CompletionEvent::Error(format!(
                "Agent requested {} tools in one turn.",
                tool_calls.len()
            )));

            return;
        }

        // --------------------------------------------------
        // EXECUTE TOOLS
        // --------------------------------------------------

        let mut tool_responses = Vec::with_capacity(tool_calls.len());

        for (i, call) in tool_calls.iter().enumerate() {
            if cancel.is_cancelled() {
                println!("[AGENT] Cancelled before tool execution");

                let _ = tx.send(CompletionEvent::Cancelled);

                return;
            }

            println!("\n[TOOL] ========================================");
            println!("[TOOL] EXECUTING TOOL #{}", i + 1);
            println!("[TOOL] name: {}", call.fn_name);
            println!("[TOOL] call_id: {}", call.call_id);
            println!("[TOOL] args: {}", call.fn_arguments);
            println!("[TOOL] ========================================");

            let _ = tx.send(CompletionEvent::ToolCallStarted {
                tool_name: call.fn_name.clone(),
            });

            let output = match execute_tool(call).await {
                Ok(result) => {
                    println!("[TOOL] SUCCESS: {}", call.fn_name);

                    result
                }

                Err(err) => {
                    println!("[TOOL] FAILURE: {}", err);

                    format!("Tool execution error: {}", err)
                }
            };

            println!("[TOOL] RESULT ({}) chars:", output.len());

            println!("[TOOL] {:?}", output);

            tool_responses.push(ToolResponse::from_tool_call(call, output));
        }

        // --------------------------------------------------
        // SEND UI EVENT
        // --------------------------------------------------

        println!("\n[AGENT] Sending ToolTurnCompleted to UI");

        let _ = tx.send(CompletionEvent::ToolTurnCompleted {
            tool_calls: tool_calls.clone(),
            tool_responses: tool_responses.clone(),
        });

        // --------------------------------------------------
        // UPDATE API HISTORY
        // --------------------------------------------------

        println!("[AGENT] Adding assistant tool-use message to request history");

        request.messages.push(assistant_tool_message);

        println!(
            "[AGENT] Adding {} tool responses to request history",
            tool_responses.len()
        );

        request.messages.push(tool_responses.into());

        println!(
            "[AGENT] Request history is now {} messages",
            request.messages.len()
        );

        if turn + 1 >= MAX_TOOL_TURNS {
            println!("\n[AGENT] !!! MAX TOOL TURNS REACHED !!!");

            let _ = tx.send(CompletionEvent::Error(
                "Agent stopped after reaching the maximum number of tool turns.".into(),
            ));

            return;
        }

        println!("\n[AGENT] Turn {} complete.", turn_number);

        println!("[AGENT] Starting another model turn...");
    }

    println!("[AGENT] Agent loop exited.");
}

pub fn poll_events(
    rx: &std::sync::mpsc::Receiver<CompletionEvent>,
    convos: &mut [Conversation],
    generating_convo: &mut Option<usize>,
    active_cancel: &mut Option<CancellationToken>,
    active_tool: &mut Option<String>,
) {
    while let Ok(event) = rx.try_recv() {
        if let Some(idx) = *generating_convo {
            match event {
                CompletionEvent::Chunk(text) => {
                    use genai::chat::ContentPart;

                    *active_tool = None;

                    if let Some(last) = convos[idx].messages.last_mut() {
                        if matches!(last.role, ChatRole::Assistant) {
                            last.content.push(ContentPart::Text(text));
                        } else {
                            convos[idx].messages.push(ChatMessage::assistant(text));
                        }
                    } else {
                        convos[idx].messages.push(ChatMessage::assistant(text));
                    }
                }

                CompletionEvent::ToolCallStarted { tool_name } => {
                    *active_tool = Some(tool_name);
                }

                CompletionEvent::ToolTurnCompleted { .. } => {
                    *active_tool = None;
                }

                CompletionEvent::Finished => {
                    *generating_convo = None;
                    *active_cancel = None;
                    *active_tool = None;

                    crate::storage::save_conversations(convos);
                }

                CompletionEvent::Cancelled => {
                    *generating_convo = None;
                    *active_cancel = None;
                    *active_tool = None;

                    crate::storage::save_conversations(convos);
                }

                CompletionEvent::Error(e) => {
                    convos[idx]
                        .messages
                        .push(ChatMessage::assistant(format!("Error: {e}")));

                    *generating_convo = None;
                    *active_cancel = None;
                    *active_tool = None;
                }
            }
        }
    }
}
