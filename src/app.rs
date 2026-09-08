use eframe::egui::Color32;
use eframe::egui::{self, Frame, Panel, RichText, ScrollArea};

use genai::resolver::AuthData;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::chat::completion::poll_events;
use crate::chat::mcp_client::McpManager;
use crate::chat::tools::get_builtin_tool_defs;
use crate::chat::{self, ExecMode};
use crate::components::{
    edit_line, message_bubble, sidebar_row, thinking_bubble, tool_status_bubble,
};
use crate::theme::custom_styling;
use tracing::info;
use crate::tray::TrayHandles;
use crate::types::{CompletionEvent, Conversation, GenerationState, Selected};
use genai::Client;

use genai::adapter::AdapterKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    #[serde(default)]
    pub tool_preferences: HashMap<String, bool>,
    #[serde(default)]
    pub mcp_servers: Vec<crate::chat::mcp::McpServerConfig>,
}

pub struct CastClient {
    convos: Vec<Conversation>,
    active: Selected,
    input: String,

    tx: Sender<CompletionEvent>,
    rx: Receiver<CompletionEvent>,
    active_cancel: Option<CancellationToken>,

    settings: AppSettings,
    show_settings: bool,
    client: Client,
    md_cache: egui_commonmark::CommonMarkCache,
    last_active_convo: Option<usize>,

    tray_hidden: Arc<AtomicBool>,
    quit_requested: Arc<AtomicBool>,
    _tray: tray_icon::TrayIcon,

    exec_mode: ExecMode,
    generation: GenerationState,

    mcp_manager: Arc<McpManager>,
    show_add_mcp_modal: bool,
    add_mcp_form: AddMcpForm,
    built_adapter: AdapterKind,
    built_api_key: String,
}

#[derive(Default)]
struct AddMcpForm {
    name: String,
    transport: crate::chat::mcp::McpTransport,
    command: String,
    args: String,
    url: String,
    headers: String,
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
            tool_preferences: crate::storage::load_tool_preferences(),
            mcp_servers: crate::storage::load_mcp_servers(),
        };

        if let Some(store) = cc.storage
            && let Some(saved_settings) = eframe::get_value(store, "app_settings")
        {
            initial_settings = saved_settings;
        }
        let mut default_headers = HeaderMap::new();
        default_headers.insert(
            "X-OpenRouter-Title",
            HeaderValue::from_static("Cast Client"),
        );
        default_headers.insert("X-Title", HeaderValue::from_static("Cast Client"));

        let (tx, rx) = channel();
        let client = Self::build_client(&initial_settings);
        let built_adapter = initial_settings.adapter;
        let built_api_key = initial_settings.api_key.clone();

        let mcp_manager = Arc::new(McpManager::new());
        let mcp_manager_clone = mcp_manager.clone();
        let mcp_servers = initial_settings.mcp_servers.clone();
        tokio::spawn(async move {
            mcp_manager_clone.initialize(mcp_servers).await;
        });

        Self {
            convos: crate::storage::load_conversations(),
            active: Selected::New,
            input: String::new(),
            tx,
            rx,

            active_cancel: None,
            last_active_convo: None,
            settings: initial_settings,
            client,
            show_settings: false,
            md_cache: egui_commonmark::CommonMarkCache::default(),
            tray_hidden,
            quit_requested,
            _tray: tray_icon,
            generation: GenerationState::default(),
            exec_mode: ExecMode::default(),
            mcp_manager,
            show_add_mcp_modal: false,
            add_mcp_form: AddMcpForm::default(),
            built_adapter,
            built_api_key,
        }
    }

    fn build_client(settings: &AppSettings) -> Client {
        let mut default_headers = HeaderMap::new();
        default_headers.insert(
            "X-OpenRouter-Title",
            HeaderValue::from_static("Cast Client"),
        );
        default_headers.insert("X-Title", HeaderValue::from_static("Cast Client"));

        let reqwest_client = reqwest::Client::builder()
            .default_headers(default_headers)
            .build()
            .expect("Failed to build reqwest client");

        let api_key = settings.api_key.clone();
        Client::builder()
            .with_reqwest(reqwest_client)
            .with_adapter_kind(settings.adapter)
            .with_auth_resolver_fn(move |_service_id| {
                Ok(Some(AuthData::Key(api_key.clone())))
            })
            .build()
    }

    /// Rebuilds the genai client if the adapter or API key changed in Settings.
    fn sync_client_with_settings(&mut self) {
        if self.built_adapter != self.settings.adapter
            || self.built_api_key != self.settings.api_key
        {
            info!("[SETTINGS] Adapter or API key changed, rebuilding client");
            self.client = Self::build_client(&self.settings);
            self.built_adapter = self.settings.adapter;
            self.built_api_key = self.settings.api_key.clone();
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
                egui::ComboBox::from_id_salt("api_adapter_combo")
                    .selected_text(self.settings.adapter.to_string())
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
        ui.separator();
        ui.add_space(10.0);

        self.render_available_tools(ui);

        ui.add_space(20.0);
        ui.separator();
        ui.add_space(10.0);

        self.render_mcp_servers(ui);

        ui.add_space(20.0);
        if ui.button("<- Back to Chat").clicked() {
            self.show_settings = false;
        }
    }

    fn render_mcp_servers(&mut self, ui: &mut egui::Ui) {
        ui.heading("MCP Servers");
        ui.add_space(5.0);
        ui.label(
            egui::RichText::new("Model Context Protocol servers for extended tool capabilities. Add servers to enable additional tools in Agent mode.")
                .small()
                .color(crate::theme::TEXT_SECONDARY),
        );
        ui.add_space(10.0);

        // List existing servers
        let mut to_remove: Option<usize> = None;
        let mut to_toggle: Option<(usize, bool)> = None;

        for (idx, server) in self.settings.mcp_servers.iter().enumerate() {
            let mut enabled = server.enabled;

            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let checkbox_response = ui.checkbox(&mut enabled, "");
                            if checkbox_response.changed() {
                                to_toggle = Some((idx, enabled));
                            }

                            let status_color = if enabled {
                                crate::theme::ACCENT
                            } else {
                                crate::theme::TEXT_SECONDARY
                            };
                            let status_text = if enabled {
                                "🟢 Enabled"
                            } else {
                                "🔴 Disabled"
                            };
                            ui.label(
                                egui::RichText::new(server.name.clone())
                                    .strong()
                                    .color(crate::theme::TEXT_PRIMARY),
                            );
                            ui.label(egui::RichText::new(status_text).small().color(status_color));
                        });

                        ui.horizontal(|ui| {
                            let transport_str = match server.transport {
                                crate::chat::mcp::McpTransport::Stdio => "stdio",
                                crate::chat::mcp::McpTransport::Sse => "SSE",
                                crate::chat::mcp::McpTransport::StreamableHttp => "StreamableHTTP",
                            };
                            ui.label(
                                egui::RichText::new(format!("Transport: {}", transport_str))
                                    .small()
                                    .color(crate::theme::TEXT_SECONDARY),
                            );

                            match server.transport {
                                crate::chat::mcp::McpTransport::Stdio => {
                                    if let Some(cmd) = &server.command {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "Command: {} {}",
                                                cmd,
                                                server.args.join(" ")
                                            ))
                                            .small()
                                            .color(crate::theme::TEXT_SECONDARY),
                                        );
                                    }
                                }
                                crate::chat::mcp::McpTransport::Sse
                                | crate::chat::mcp::McpTransport::StreamableHttp => {
                                    if let Some(url) = &server.url {
                                        ui.label(
                                            egui::RichText::new(format!("URL: {}", url))
                                                .small()
                                                .color(crate::theme::TEXT_SECONDARY),
                                        );
                                    }
                                }
                            }
                        });
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("❌ Remove").clicked() {
                            to_remove = Some(idx);
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }

        if let Some(idx) = to_remove {
            let server_id = self.settings.mcp_servers[idx].id.clone();
            self.settings.mcp_servers.remove(idx);
            crate::storage::save_mcp_servers(&self.settings.mcp_servers);
            let mcp_manager = self.mcp_manager.clone();
            tokio::spawn(async move {
                mcp_manager.remove_server(&server_id).await;
            });
        }

        if let Some((idx, enabled)) = to_toggle {
            self.settings.mcp_servers[idx].enabled = enabled;
            crate::storage::save_mcp_servers(&self.settings.mcp_servers);
            let server_config = self.settings.mcp_servers[idx].clone();
            let mcp_manager = self.mcp_manager.clone();
            tokio::spawn(async move {
                mcp_manager.update_server_config(server_config).await;
            });
        }

        ui.add_space(10.0);
        if ui.button("➕ Add MCP Server").clicked() {
            self.show_add_mcp_modal = true;
            self.add_mcp_form = AddMcpForm::default();
        }
    }

    fn render_add_mcp_modal(&mut self, ui: &mut egui::Ui) {
        if !self.show_add_mcp_modal {
            return;
        }

        let mut close_modal = false;

        egui::Window::new("Add MCP Server")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.vertical(|ui| {
                    ui.add_space(10.0);

                    ui.label("Server Name:");
                    ui.text_edit_singleline(&mut self.add_mcp_form.name)
                        .on_hover_text("A friendly name for this MCP server");
                    ui.add_space(10.0);

                    ui.label("Transport:");
                    egui::ComboBox::from_id_salt("mcp_transport_combo")
                        .selected_text(match self.add_mcp_form.transport {
                            crate::chat::mcp::McpTransport::Stdio => "Stdio (Local Process)",
                            crate::chat::mcp::McpTransport::Sse => "SSE (Server-Sent Events)",
                            crate::chat::mcp::McpTransport::StreamableHttp => "Streamable HTTP",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.add_mcp_form.transport,
                                crate::chat::mcp::McpTransport::Stdio,
                                "Stdio (Local Process)",
                            );
                            ui.selectable_value(
                                &mut self.add_mcp_form.transport,
                                crate::chat::mcp::McpTransport::Sse,
                                "SSE (Server-Sent Events)",
                            );
                            ui.selectable_value(
                                &mut self.add_mcp_form.transport,
                                crate::chat::mcp::McpTransport::StreamableHttp,
                                "Streamable HTTP",
                            );
                        });
                    ui.add_space(10.0);

                    match self.add_mcp_form.transport {
                        crate::chat::mcp::McpTransport::Stdio => {
                            ui.label("Command:");
                            ui.text_edit_singleline(&mut self.add_mcp_form.command)
                                .on_hover_text("e.g., npx, docker, python");
                            ui.add_space(5.0);
                            ui.label("Arguments (space-separated):");
                            ui.text_edit_singleline(&mut self.add_mcp_form.args)
                                .on_hover_text("e.g., -y @modelcontextprotocol/server-filesystem /path/to/dir");
                        }
                        crate::chat::mcp::McpTransport::Sse | crate::chat::mcp::McpTransport::StreamableHttp => {
                            ui.label("URL:");
                            ui.text_edit_singleline(&mut self.add_mcp_form.url)
                                .on_hover_text("e.g., http://localhost:3000/sse or https://api.example.com/mcp");
                            ui.add_space(5.0);
                            ui.label("Headers (JSON):");
                            ui.add(
                                egui::TextEdit::multiline(&mut self.add_mcp_form.headers)
                                    .desired_rows(3)
                                    .hint_text("Optional JSON object, e.g., {\"Authorization\": \"Bearer token\"}"),
                            );
                        }
                    }

                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            close_modal = true;
                        }

                        let can_save = match self.add_mcp_form.transport {
                            crate::chat::mcp::McpTransport::Stdio => !self.add_mcp_form.name.is_empty() && !self.add_mcp_form.command.is_empty(),
                            crate::chat::mcp::McpTransport::Sse | crate::chat::mcp::McpTransport::StreamableHttp => !self.add_mcp_form.name.is_empty() && !self.add_mcp_form.url.is_empty(),
                        };

                        if ui.add_enabled(can_save, egui::Button::new("Add Server")).clicked() {
                            let mut headers = std::collections::HashMap::new();
                            if !self.add_mcp_form.headers.trim().is_empty()
                                && let Ok(parsed) = serde_json::from_str::<std::collections::HashMap<String, String>>(&self.add_mcp_form.headers) {
                                    headers = parsed;
                                }

                            let mut server = match self.add_mcp_form.transport {
                                crate::chat::mcp::McpTransport::Stdio => {
                                    let args: Vec<String> = self.add_mcp_form.args.split_whitespace().map(String::from).collect();
                                    crate::chat::mcp::McpServerConfig::new_stdio(
                                        self.add_mcp_form.name.clone(),
                                        self.add_mcp_form.command.clone(),
                                        args,
                                    )
                                }
                                crate::chat::mcp::McpTransport::Sse => {
                                    crate::chat::mcp::McpServerConfig::new_sse(
                                        self.add_mcp_form.name.clone(),
                                        self.add_mcp_form.url.clone(),
                                    )
                                }
                                crate::chat::mcp::McpTransport::StreamableHttp => {
                                    crate::chat::mcp::McpServerConfig::new_streamable_http(
                                        self.add_mcp_form.name.clone(),
                                        self.add_mcp_form.url.clone(),
                                    )
                                }
                            };

                            server.headers = headers;

                            // Validate
                            if let Err(e) = server.validate() {
                                tracing::error!("Invalid MCP server config: {e}");
                            } else {
                                self.settings.mcp_servers.push(server.clone());
                                crate::storage::save_mcp_servers(&self.settings.mcp_servers);
                                let mcp_manager = self.mcp_manager.clone();
                                tokio::spawn(async move {
                                    mcp_manager.update_server_config(server).await;
                                });
                                close_modal = true;
                            }
                        }
                    });
                });
            });

        if close_modal {
            self.show_add_mcp_modal = false;
            self.add_mcp_form = AddMcpForm::default();
        }
    }

    fn render_available_tools(&mut self, ui: &mut egui::Ui) {
        ui.heading("Available Tools");
        ui.add_space(5.0);
        ui.label(
            egui::RichText::new("Built-in tools available in Agent mode. Disable tools you don't want the AI to use.")
                .small()
                .color(crate::theme::TEXT_SECONDARY),
        );
        ui.add_space(10.0);

        let tool_defs = get_builtin_tool_defs();
        for tool in tool_defs {
            let enabled = self
                .settings
                .tool_preferences
                .get(tool.name)
                .copied()
                .unwrap_or(tool.enabled_by_default);
            let mut new_enabled = enabled;

            ui.horizontal(|ui| {
                let checkbox_response = ui.checkbox(&mut new_enabled, "");
                if checkbox_response.changed() {
                    self.settings
                        .tool_preferences
                        .insert(tool.name.to_string(), new_enabled);
                    crate::storage::save_tool_preferences(&self.settings.tool_preferences);
                }

                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(tool.name)
                            .strong()
                            .color(if new_enabled {
                                crate::theme::TEXT_PRIMARY
                            } else {
                                crate::theme::TEXT_SECONDARY
                            }),
                    );
                    ui.label(
                        egui::RichText::new(tool.description)
                            .small()
                            .color(crate::theme::TEXT_SECONDARY),
                    );
                });
            });
            ui.add_space(8.0);
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
                        if let GenerationState::Active { convo_idx, .. } = &self.generation
                            && *convo_idx == idx
                        {
                            if let Some(cancel) = self.active_cancel.take() {
                                cancel.cancel();
                            }

                            self.generation = GenerationState::Idle;
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
                message_bubble(ui, msg, &mut self.md_cache);
            }

            if let GenerationState::Active { convo_idx, phase } = &self.generation
                && *convo_idx == idx
            {
                match phase {
                    crate::types::GenerationPhase::Thinking => {
                        thinking_bubble(ui);
                    }

                    crate::types::GenerationPhase::ExecutingTool { tool_name } => {
                        tool_status_bubble(ui, tool_name);
                    }
                }
            }
        });
    }

    fn render_bottom(&mut self, ui: &mut egui::Ui) {
        let mut cancel_triggered = false;

        let text = edit_line(
            ui,
            &mut self.input,
            !matches!(self.generation, GenerationState::Idle),
            Some(&mut || {
                cancel_triggered = true;
            }),
            &mut self.exec_mode,
        );

        if cancel_triggered {
            if let Some(cancel) = self.active_cancel.take() {
                cancel.cancel();
            }

            self.generation = GenerationState::Idle;
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
                tool_preferences: &self.settings.tool_preferences,
                mcp_manager: &self.mcp_manager,
            };

            if let Some((idx, cancel)) = chat::completion::send_message(Some(val), ctx) {
                self.active_cancel = Some(cancel);
                self.generation = GenerationState::Active {
                    convo_idx: idx,
                    phase: crate::types::GenerationPhase::Thinking,
                };
            }
        }
    }
}

impl eframe::App for CastClient {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.sync_client_with_settings();

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

        if let Selected::Index(idx) = self.active
            && self.last_active_convo != Some(idx)
        {
            self.md_cache = egui_commonmark::CommonMarkCache::default();
            self.last_active_convo = Some(idx);
        }
        poll_events(
            &self.rx,
            &mut self.convos,
            &mut self.generation,
            &mut self.active_cancel,
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

        // Render MCP modal on top of everything
        self.render_add_mcp_modal(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "app_settings", &self.settings);
        crate::storage::save_tool_preferences(&self.settings.tool_preferences);
        crate::storage::save_mcp_servers(&self.settings.mcp_servers);
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
        .expect("Failed to load window icon")
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
