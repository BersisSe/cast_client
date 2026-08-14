use eframe::egui::Color32;
use eframe::egui::{self, Frame, Panel, RichText, ScrollArea};

use genai::resolver::AuthData;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::chat::completion::poll_events;
use crate::chat::{self, ExecMode};
use crate::components::{edit_line, message_bubble, sidebar_row, thinking_bubble, tool_status_bubble};
use crate::theme::custom_styling;
use crate::tray::TrayHandles;
use crate::types::{ActiveConvoData, CompletionEvent, Conversation, Selected};
use genai::Client;

use genai::adapter::AdapterKind;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub adapter: AdapterKind,
    pub api_key: String,
    pub model: String,
    pub system_prompt: Option<String>,
}

pub struct CastClient {
    convos: Vec<Conversation>,
    active: Selected,
    input: String,

    tx: Sender<CompletionEvent>,
    rx: Receiver<CompletionEvent>,
    generating_convo: Option<usize>,
    active_cancel: Option<CancellationToken>,
    active_tool: Option<String>,

    settings: AppSettings,
    show_settings: bool,
    client: Client,
    md_cache: egui_commonmark::CommonMarkCache,
    last_active_convo: Option<usize>,

    tray_hidden: Arc<AtomicBool>,
    quit_requested: Arc<AtomicBool>,
    _tray: tray_icon::TrayIcon,

    attachments: Vec<egui::DroppedFile>,

    exec_mode: ExecMode,
}

impl CastClient {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        let TrayHandles {
            hidden: tray_hidden,
            quit_requested,
            icon: tray_icon,
        } = crate::tray::setup_tray(&ctx);
        cc.egui_ctx.options_mut(|options| {
            options.reduce_texture_memory = true;
            options.warn_on_id_clash = false;
        });
        custom_styling(&cc.egui_ctx);

        let mut initial_settings = AppSettings {
            adapter: AdapterKind::OpenRouter,
            api_key: "".to_string(),
            model: "gemini-3.5-flash".to_string(),
            system_prompt: None,
        };

        if let Some(store) = cc.storage {
            if let Some(saved_settings) = eframe::get_value(store, "app_settings") {
                initial_settings = saved_settings;
            }
        }
        let mut default_headers = HeaderMap::new();
        default_headers.insert("HTTP-Referer", HeaderValue::from_static("Cast Client"));
        default_headers.insert(
            "X-OpenRouter-Title",
            HeaderValue::from_static("Cast Client"),
        );
        default_headers.insert("X-Title", HeaderValue::from_static("Cast Client"));

        // 2. Build the underlying reqwest client with your default headers
        let reqwest_client = reqwest::Client::builder()
            .default_headers(default_headers)
            .build()
            .expect("Reqwest Client Failed to build");
        let (tx, rx) = channel();
        let apikey = initial_settings.api_key.clone();
        let client = Client::builder()
            .with_reqwest(reqwest_client)
            .with_adapter_kind(initial_settings.adapter)
            .with_auth_resolver_fn(|_service_id| Ok(Some(AuthData::Key(apikey))))
            .build();

        Self {
            convos: crate::storage::load_conversations(),
            active: Selected::New,
            input: String::new(),
            tx,
            rx,
            generating_convo: None,
            active_cancel: None,
            last_active_convo: None,
            settings: initial_settings,
            client,
            show_settings: false,
            md_cache: egui_commonmark::CommonMarkCache::default(),
            tray_hidden,
            quit_requested,
            attachments: Vec::new(),
            _tray: tray_icon,
            active_tool: None,
            exec_mode: ExecMode::default(),
        }
    }

    fn render_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("API Configuration");
        ui.add_space(10.0);

        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing([12.0, 12.0])
            .show(ui, |ui| {
                ui.label("API Adapter");

                // 2. Ardından ComboBox'ı ekliyoruz (from_label yerine from_id_source kullanarak)
                egui::ComboBox::from_id_salt("api_adapter_combo")
                    .selected_text(&self.settings.adapter.to_string())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.settings.adapter,
                            AdapterKind::OpenRouter,
                            "Open Router",
                        );
                        ui.selectable_value(
                            &mut self.settings.adapter,
                            AdapterKind::OpenAI,
                            "OpenAI",
                        );
                        ui.selectable_value(
                            &mut self.settings.adapter,
                            AdapterKind::Gemini,
                            "Gemini",
                        );
                        ui.selectable_value(
                            &mut self.settings.adapter,
                            AdapterKind::Anthropic,
                            "Anthropic",
                        );
                        ui.selectable_value(
                            &mut self.settings.adapter,
                            AdapterKind::Ollama,
                            "Ollama",
                        );
                        ui.selectable_value(&mut self.settings.adapter, AdapterKind::Xai, "Xai");
                        ui.selectable_value(&mut self.settings.adapter, AdapterKind::Groq, "Groq");
                        ui.selectable_value(
                            &mut self.settings.adapter,
                            AdapterKind::Moonshot,
                            "Moonshot",
                        );
                    });
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
                    egui::TextEdit::multiline(
                        self.settings.system_prompt.get_or_insert_with(String::new),
                    )
                    .hint_text("Using Default"),
                );
                ui.end_row();
            });

        ui.add_space(20.0);
        if ui.button("<- Back to Chat").clicked() {
            self.show_settings = false;
        }
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
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
                        crate::storage::save_conversations(&self.convos);
                    }
                });
        });
    }

    fn render_content(&mut self, ui: &mut egui::Ui) {
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

    fn render_chat(&mut self, ui: &mut egui::Ui, idx: usize) {
        let convo = &self.convos[idx];

        ScrollArea::vertical().show(ui, |ui| {
            for msg in &convo.messages {
                message_bubble(ui, msg, &mut self.md_cache, self.active_tool.as_deref());
            }

            // Generation UI is NOT part of Conversation.messages.
            if self.generating_convo == Some(idx) {
                if let Some(tool_name) = &self.active_tool {
                    tool_status_bubble(ui, tool_name);
                } else {
                    thinking_bubble(ui);
                }
            }
        });
    }

    fn render_bottom(&mut self, ui: &mut egui::Ui) {
        let mut cancel_triggered = false;
        if !self.attachments.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let mut to_remove = None;

                for (i, file) in self.attachments.iter().enumerate() {
                    let name = file
                        .path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Attached File".to_string());

                    let chip_res = ui
                        .scope(|ui| {
                            ui.visuals_mut().widgets.inactive.bg_fill = crate::theme::BG_CONTENT;
                            ui.add(egui::Button::new(format!("📎 {} ❌", name)).small())
                        })
                        .inner;

                    if chip_res.clicked() {
                        to_remove = Some(i);
                    }
                }

                if let Some(idx) = to_remove {
                    self.attachments.remove(idx);
                }
            });
            ui.add_space(4.0);
        }

        let text = edit_line(
            ui,
            &mut self.input,
            self.generating_convo.is_some(),
            Some(&mut || {
                cancel_triggered = true;
            }),
            &mut self.attachments,
            &mut self.exec_mode,
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

            let ctx = chat::completion::SendContext {
                convos: &mut self.convos,
                active: &mut self.active,
                client: &self.client,
                model: self.settings.model.clone(),
                tx: &self.tx,
                mode: self.exec_mode,
                system: self.settings.system_prompt.clone(),
            };

            if let Some((idx, cancel)) = chat::completion::send_message(Some(val), ctx) {
                self.active_cancel = Some(cancel);
                self.generating_convo = Some(idx);
            }
        }
    }
}

impl eframe::App for CastClient {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            if self.quit_requested.load(Ordering::SeqCst) {
                return;
            }
            self.tray_hidden.store(true, Ordering::SeqCst);
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        if let Selected::Index(idx) = self.active {
            if self.last_active_convo != Some(idx) {
                self.md_cache = egui_commonmark::CommonMarkCache::default();
                self.last_active_convo = Some(idx);
            }
        }
        // WHILE POLL
        poll_events(
            &self.rx,
            &mut self.convos,
            &mut self.generating_convo,
            &mut self.active_cancel,
            &mut self.active_tool,
        );

        Panel::left("nav").resizable(true).show(ui, |ui| {
            self.render_sidebar(ui);
        });
        Panel::bottom("edit")
            .frame(Frame::new().inner_margin(10).fill(crate::theme::BG_CONTENT))
            .show(ui, |ui| {
                if !self.show_settings {
                    self.render_bottom(ui);
                }
            });
        egui::CentralPanel::default().show(ui, |ui| {
            self.render_content(ui);
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "app_settings", &self.settings);
    }
}

pub fn run() -> eframe::Result<()> {
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
