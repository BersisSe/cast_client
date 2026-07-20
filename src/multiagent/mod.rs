use eframe::egui::Context;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use reqwest::Client;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub mod types;

#[derive(Debug, Clone)]
pub struct AIConfig {
    pub base_url: String,
    pub api_key: String,
}

#[derive(Debug, Clone)]
pub struct AIClient {
    config: AIConfig,
    inner: Client,
}

#[derive(Debug)]
pub enum CompletionEvent {
    Chunk(String),
    Finished,
    Error(String),
    Cancelled,
}

impl AIClient {
    pub fn new(config: AIConfig) -> Self {
        let inner = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .expect("failed to build reqwest client");

        Self { config, inner }
    }
    pub fn update_config(&mut self, config: AIConfig) {
        self.config = config;
    }

    pub async fn completion(
        &self,
        body: String,
        streaming: bool,
        tx: std::sync::mpsc::Sender<CompletionEvent>,
        cancel: CancellationToken,
        ctx: Context,
    ) {
        let url = format!(
            "{}chat/completions",
            self.config.base_url.trim_end_matches('/').to_string() + "/"
        );

        let send_result = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = tx.send(CompletionEvent::Cancelled);
                ctx.request_repaint();
                return;
            }
            result = self.inner.post(&url)
                .bearer_auth(&self.config.api_key)
                .header("Content-Type", "application/json")
                .body(body)
                .send() => result,
        };

        let resp = match send_result {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(CompletionEvent::Error(e.to_string()));
                ctx.request_repaint();
                return;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let _ = tx.send(CompletionEvent::Error(format!("HTTP {status}: {text}")));
            ctx.request_repaint();
            return;
        }

        if !streaming {
            match resp.json::<serde_json::Value>().await {
                Ok(value) => {
                    if let Some(text) = value["choices"][0]["message"]["content"].as_str() {
                        let _ = tx.send(CompletionEvent::Chunk(text.to_string()));
                        ctx.request_repaint();
                    }
                    let _ = tx.send(CompletionEvent::Finished);
                    ctx.request_repaint();
                }
                Err(e) => {
                    let _ = tx.send(CompletionEvent::Error(e.to_string()));
                    ctx.request_repaint();
                }
            }
            return;
        }

        let mut stream = resp.bytes_stream().eventsource();

        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = tx.send(CompletionEvent::Cancelled);
                    ctx.request_repaint();
                    return;
                }
                next = stream.next() => next,
            };

            let Some(event) = next else {
                break;
            };

            match event {
                Ok(ev) => {
                    if ev.data == "[DONE]" {
                        break;
                    }
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&ev.data) {
                        if let Some(text) = value["choices"][0]["delta"]["content"].as_str() {
                            let _ = tx.send(CompletionEvent::Chunk(text.to_string()));
                            ctx.request_repaint();
                        }
                        if let Some(finish_reason) = value["choices"][0]["finish_reason"].as_str() {
                            if !finish_reason.is_empty() {
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(CompletionEvent::Error(e.to_string()));
                    ctx.request_repaint();
                    return;
                }
            }
        }

        let _ = tx.send(CompletionEvent::Finished);
        ctx.request_repaint();
    }
}
