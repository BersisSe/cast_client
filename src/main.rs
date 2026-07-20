#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::Color32;
use eframe::egui::{self, Frame, Panel, RichText, ScrollArea};

use serde::{Deserialize, Serialize};

use crate::components::{edit_line, message_bubble, sidebar_row};
use crate::multiagent::types::CompletionRequest;
use crate::multiagent::{AIClient, AIConfig, CompletionEvent};
use crate::theme::custom_styling;
use crate::types::Message;
use crate::types::Selected;
use crate::types::{Conversation, MessageSender};
use std::sync::mpsc::{Receiver, Sender, channel};
use tokio_util::sync::CancellationToken;

mod components;
mod multiagent;
mod storage;
mod theme;
mod types;

use mimalloc::MiMalloc;
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() -> eframe::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    let _guard = rt.enter();
    std::thread::spawn(move || {
        rt.block_on(std::future::pending::<()>());
    });

    let icon_bytes = include_bytes!("./icon/AppIcon64.png");
    let image = image::load_from_memory(icon_bytes)
        .expect("Failed to open icon structure")
        .to_rgba8();

    let (width, height) = image.dimensions();

    let icon_data = egui::viewport::IconData {
        rgba: image.into_raw(),
        width,
        height,
    };
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([760.0, 520.0])
            .with_icon(icon_data)
            .with_app_id("cast_client")
            .with_title("Cast Client"),

        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "Cast Client",
        native_options,
        Box::new(|cc| Ok(Box::new(CastClient::new(cc)))),
    )
}

#[derive(Clone, Serialize, Deserialize)]
struct AppSettings {
    base_url: String,
    api_key: String,
    model: String,
    system_prompt: Option<String>,
}

struct CastClient {
    convos: Vec<Conversation>,
    active: Selected,
    input: String,

    tx: Sender<CompletionEvent>,
    rx: Receiver<CompletionEvent>,
    generating_convo: Option<usize>,
    active_cancel: Option<CancellationToken>,

    settings: AppSettings,
    show_settings: bool,
    client: AIClient,
    md_cache: egui_commonmark::CommonMarkCache,
    last_active_convo: Option<usize>,
}

impl CastClient {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.options_mut(|options| {
            options.reduce_texture_memory = true;
        });
        custom_styling(&cc.egui_ctx);
        let mut initial_settings = AppSettings {
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai/".to_string(),
            api_key: "".to_string(),
            model: "gemini-3.5-flash".to_string(),
            system_prompt: None,
        };

        if let Some(store) = cc.storage {
            if let Some(saved_settings) = eframe::get_value(store, "app_settings") {
                initial_settings = saved_settings;
            }
        }

        let ai_client = AIClient::new(AIConfig {
            base_url: initial_settings.base_url.clone(),
            api_key: initial_settings.api_key.clone(),
        });

        let (tx, rx) = channel();

        Self {
            convos: storage::load_conversations(),
            active: Selected::New,
            input: String::new(),
            tx,
            rx,
            generating_convo: None,
            active_cancel: None,
            last_active_convo: None,
            settings: initial_settings,
            client: ai_client,
            show_settings: false,
            md_cache: egui_commonmark::CommonMarkCache::default(),
        }
    }
    pub fn update_network_client(&mut self) {
        let config = AIConfig {
            api_key: self.settings.api_key.clone(),
            base_url: self.settings.base_url.clone(),
        };
        self.client.update_config(config);
    }
    pub fn render_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("API Configuration");
        ui.add_space(10.0);

        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing([12.0, 12.0])
            .show(ui, |ui| {
                ui.label("Base URL:");
                ui.text_edit_singleline(&mut self.settings.base_url)
                    .on_hover_text("e.g., https://api.openai.com/v1 or http://localhost:11434/v1");
                ui.end_row();

                ui.label("API Key:");
                ui.add(egui::TextEdit::singleline(&mut self.settings.api_key).password(true))
                    .on_hover_text("Your provider API key credential string");
                ui.end_row();

                ui.label("Model ID:");
                ui.text_edit_singleline(&mut self.settings.model)
                    .on_hover_text("e.g., gemini-3.5-flash, gpt-4o, or qwen2.5-coder:7b");
                ui.end_row();
                ui.label("System Prompt:");
                ui.add(
                    egui::TextEdit::multiline(self.settings.system_prompt.get_or_insert_with(String::new))
                        .hint_text("Using Default"),
                );
                ui.end_row();
            });

        ui.add_space(20.0);
        if ui.button("<- Back to Chat").clicked() {
            self.update_network_client();
            self.show_settings = false;
        }
    }
    pub fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("Cast Client")
                .heading()
                .strong()
                .color(Color32::WHITE),
        );
        ui.add_space(10.0);
        let settings_btn_text = if self.show_settings {
            "View Chats"
        } else {
            "Settings"
        };
        if ui.button(settings_btn_text).clicked() {
            self.show_settings = !self.show_settings;
        }
        ui.separator();
        ui.add_space(5.0);

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            if ui.button("New Chat").clicked() {
                self.active = Selected::New;
            }
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut delete_idx: Option<usize> = None;
                    let mut click_idx: Option<usize> = None;

                    for (index, convo) in self.convos.iter().enumerate() {
                        let title = &convo.title;
                        let active = matches!(self.active, Selected::Index(i) if i == index);
                        let (clicked, deleted) = sidebar_row(ui, title, active);
                        if clicked {
                            click_idx = Some(index);
                        }
                        if deleted {
                            delete_idx = Some(index);
                        }
                    }

                    if let Some(idx) = click_idx {
                        if matches!(self.active, Selected::Index(current) if current != idx) {
                            self.md_cache = egui_commonmark::CommonMarkCache::default();
                        }
                        self.active = Selected::Index(idx);
                        self.show_settings = false;
                    }

                    if let Some(idx) = delete_idx {
                        if self.generating_convo == Some(idx) {
                            if let Some(cancel) = self.active_cancel.take() {
                                cancel.cancel();
                            }
                            self.generating_convo = None;
                        }
                        self.convos.remove(idx);
                        match self.active {
                            Selected::Index(i) if i == idx => self.active = Selected::New,
                            Selected::Index(i) if i > idx => self.active = Selected::Index(i - 1),
                            _ => {}
                        }
                        storage::save_conversations(&self.convos);
                    }
                });
        });
    }
    pub fn render_content(&mut self, ui: &mut egui::Ui) {
        if self.show_settings {
            self.render_settings(ui);
        } else {
            match self.active {
                Selected::New => {
                    ui.with_layout(
                        egui::Layout::centered_and_justified(egui::Direction::TopDown),
                        |ui| {
                            ui.label(
                                RichText::new("Select a conversation or start a new one").heading(),
                            );
                        },
                    );
                }
                Selected::Index(idx) => self.render_chat(ui, idx),
            };
        }
    }
    pub fn render_chat(&mut self, ui: &mut egui::Ui, idx: usize) {
        let convo = &self.convos[idx];
        ScrollArea::vertical().show(ui, |ui| {
            for msg in &convo.messages {
                message_bubble(ui, msg, &mut self.md_cache);
            }
        });
    }
    pub fn render_bottom(&mut self, ui: &mut egui::Ui) {
        let mut cancel_triggered = false;
        let text = edit_line(
            ui,
            &mut self.input,
            self.generating_convo.is_some(),
            Some(&mut || {
                cancel_triggered = true;
            }),
        );
        if cancel_triggered {
            if let Some(cancel) = self.active_cancel.take() {
                cancel.cancel();
            }
            self.generating_convo = None;
        }
        if let Some(val) = text {
            if val.trim().is_empty() {
                return;
            }

            let idx = match self.active {
                Selected::Index(idx) => idx,
                Selected::New => {
                    self.convos.push(Conversation {
                        title: "New Chat".into(),
                        messages: Vec::new(),
                    });
                    const MAX_CONVERSATIONS: usize = 50;
                    if self.convos.len() > MAX_CONVERSATIONS {
                        self.convos.drain(0..self.convos.len() - MAX_CONVERSATIONS);
                    }
                    let new_idx = self.convos.len() - 1;
                    self.active = Selected::Index(new_idx);
                    new_idx
                }
            };
            let msg = Message::new(MessageSender::User, &val);
            self.convos[idx].messages.push(msg);
            self.convos[idx].messages.push(Message::new_ai_begin());

            if self.convos[idx].title == "New Chat" {
                let title = val.chars().take(50).collect::<String>();
                if !title.is_empty() {
                    self.convos[idx].title = title;
                }
            }

            const MAX_MESSAGES: usize = 100;
            let msg_len = self.convos[idx].messages.len();
            if msg_len > MAX_MESSAGES {
                self.convos[idx].messages.drain(0..msg_len - MAX_MESSAGES);
            }

            let messages = self.convos[idx].messages.clone();
            let request = CompletionRequest::new()
                .model(self.settings.model.clone())
                .messages(messages)
                .streaming(true)
                .system_prompt(self.settings.system_prompt.clone())
                .build();

            let client = self.client.clone();
            let ctx = ui.ctx().clone();
            let tx = self.tx.clone();
            let cancel = CancellationToken::new();
            self.active_cancel = Some(cancel.clone());
            self.generating_convo = Some(idx);

            tokio::spawn(async move {
                client.completion(request, tx, cancel, ctx).await;
            });
            storage::save_conversations(&self.convos)
        }
    }
}

impl eframe::App for CastClient {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Selected::Index(idx) = self.active {
            if self.last_active_convo != Some(idx) {
                self.md_cache = egui_commonmark::CommonMarkCache::default();
                self.last_active_convo = Some(idx);
            }
        }

        while let Ok(event) = self.rx.try_recv() {
            if let Some(idx) = self.generating_convo {
                match event {
                    CompletionEvent::Chunk(text) => {
                        if let Some(last) = self.convos[idx].messages.last_mut() {
                            last.ai_start = false;
                            if last.content.capacity() == 0 {
                                last.content.reserve(128);
                            }
                            last.content.push_str(&text);
                        }
                    }
                    CompletionEvent::Finished | CompletionEvent::Cancelled => {
                        self.generating_convo = None;
                        self.active_cancel = None;
                        storage::save_conversations(&self.convos);
                    }
                    CompletionEvent::Error(e) => {
                        self.convos[idx]
                            .messages
                            .push(Message::new_ai(&format!("Error: {e}")));
                        self.generating_convo = None;
                        self.active_cancel = None;
                    }
                }
            }
        }

        Panel::left("nav").resizable(true).show(ui, |ui| {
            self.render_sidebar(ui);
        });
        Panel::bottom("edit")
            .frame(Frame::new().inner_margin(10).fill(theme::BG_CONTENT))
            .show(ui, |ui| {
                self.render_bottom(ui);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            self.render_content(ui);
        });
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "app_settings", &self.settings);
    }
}
