use eframe::egui::{self, Stroke, StrokeKind};
use egui_commonmark::CommonMarkCache;
use genai::chat::{ChatMessage, ChatRole};

use crate::chat::ExecMode;
use crate::types::Message;

use super::theme;

pub fn message_bubble(ui: &mut egui::Ui, msg: &Message, cache: &mut CommonMarkCache) {
    let is_user = matches!(msg.message.role, ChatRole::User);
    let align = if is_user {
        egui::Layout::right_to_left(egui::Align::Min)
    } else {
        egui::Layout::left_to_right(egui::Align::Min)
    };

    ui.with_layout(align, |ui| {
        let max_width = (ui.available_width() * 0.78).min(700.0);
        ui.set_max_width(max_width);

        let fill = if is_user {
            theme::BG_BUBBLE_USER
        } else {
            theme::BG_BUBBLE_AI
        };

        egui::Frame::new()
            .fill(fill)
            .inner_margin(10)
            .corner_radius(theme::CORNER_RADIUS)
            .stroke(Stroke::new(theme::HAIRLINE_WIDTH, theme::BORDER_HAIRLINE))
            .show(ui, |ui| {
                render_label(ui, &msg.message, msg.tools_called, cache);
            });
    });
}
fn render_label(
    ui: &mut egui::Ui,
    msg: &ChatMessage,
    tools_used: usize,
    cache: &mut CommonMarkCache,
) {
    if matches!(msg.role, ChatRole::Tool) {
        return;
    }

    if matches!(msg.role, ChatRole::Assistant)
        && !msg.content.tool_calls().is_empty()
    {
        return;
    }

    let text = msg.content.texts().join("");

    if text.is_empty() && matches!(msg.role, ChatRole::Assistant) {
        ui.label(
            egui::RichText::new("Thinking..")
                .color(theme::TEXT_SECONDARY),
        );
        ui.spinner();
        return;
    }

    if matches!(msg.role, ChatRole::Assistant) {
        egui_commonmark::CommonMarkViewer::new().show(ui, cache, &text);

        if tools_used > 0 {
            let label = format!(
                "🔧 {} tool{} called",
                tools_used,
                if tools_used == 1 { "" } else { "s" }
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(label)
                    .small()
                    .color(theme::TEXT_SECONDARY),
            );
        }

        return;
    }

    ui.label(
        egui::RichText::new(text)
            .color(theme::TEXT_PRIMARY),
    );
}

pub fn edit_line(
    ui: &mut egui::Ui,
    input: &mut String,
    sending_disabled: bool,
    on_cancel: Option<&mut dyn FnMut()>,
    dropped_files: &mut Vec<egui::DroppedFile>,
    exec_mode: &mut ExecMode, 
) -> Option<String> {
    let mut submitted: Option<String> = None;

    if !sending_disabled
        && ui
            .ctx()
            .input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter))
    {
        input.push('\n');
    }
    ui.ctx().input(|i| {
        if !i.raw.dropped_files.is_empty() {
            dropped_files.extend(i.raw.dropped_files.clone());
        }
    });

    egui::Frame::new()
        .fill(theme::BG_CONTENT)
        .inner_margin(6)
        .corner_radius(theme::CORNER_RADIUS)
        .stroke(Stroke::new(theme::HAIRLINE_WIDTH, theme::BORDER_HAIRLINE))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(exec_mode, ExecMode::Chat, "💬 Chat");
                    ui.selectable_value(exec_mode, ExecMode::Agent, "🤖 Agent");

                    ui.separator();

                    if *exec_mode == ExecMode::Chat {
                        ui.label(
                            egui::RichText::new("Tools disabled")
                                .small()
                                .color(theme::TEXT_SECONDARY),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new("Tools active")
                                .small()
                                .color(theme::ACCENT),
                        );
                    }
                });

                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    let button_width = 64.0;
                    let field_width = ui.available_width() - button_width - 8.0;

                    let response = ui.add_enabled(
                        !sending_disabled,
                        egui::TextEdit::multiline(input)
                            .hint_text(if sending_disabled {
                                "Waiting for response..."
                            } else {
                                "Message..."
                            })
                            .desired_width(field_width.max(0.0))
                            .desired_rows(2),
                    );

                    let enter_to_submit = !sending_disabled
                        && response.has_focus()
                        && ui
                            .ctx()
                            .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));

                    if sending_disabled {
                        if let Some(cancel_fn) = on_cancel {
                            if ui.button("Cancel").clicked() {
                                cancel_fn();
                            }
                        }
                    } else {
                        let send_clicked = ui
                            .add_enabled(!sending_disabled, egui::Button::new("Send"))
                            .clicked();

                        if (enter_to_submit || send_clicked) && !input.trim().is_empty() {
                            submitted = Some(std::mem::take(input));
                            response.request_focus();
                        }
                    }
                });
            });
        });

    submitted
}

pub fn sidebar_row(ui: &mut egui::Ui, title: &str, active: bool) -> (bool, bool) {
    let desired_size = egui::vec2(ui.available_width(), 34.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

    let text_color = if active {
        theme::TEXT_PRIMARY
    } else if response.hovered() {
        theme::TEXT_PRIMARY
    } else {
        theme::TEXT_SECONDARY
    };

    if active {
        let bar_width = 3.0;
        let bar_rect = egui::Rect::from_min_size(rect.min, egui::vec2(bar_width, rect.height()));
        ui.painter().rect_filled(bar_rect, 0.0, theme::ACCENT);
    } else if response.hovered() {
        ui.painter().rect_stroke(
            rect,
            theme::CORNER_RADIUS as f32,
            Stroke::new(theme::HAIRLINE_WIDTH, theme::BORDER_HAIRLINE),
            StrokeKind::Inside,
        );
    }

    let text_pos = rect.min + egui::vec2(14.0, rect.height() / 2.0);
    ui.painter().text(
        text_pos,
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(15.0),
        text_color,
    );

    let btn_id = response.id.with("del");
    let mut deleted = false;
    if response.hovered() || active {
        let btn_size = 20.0;
        let btn_rect = egui::Rect::from_min_size(
            rect.right_top() + egui::vec2(-btn_size - 4.0, (rect.height() - btn_size) / 2.0),
            egui::vec2(btn_size, btn_size),
        );
        let btn_response = ui.interact(btn_rect, btn_id, egui::Sense::click());
        if btn_response.clicked() {
            deleted = true;
        }
        let btn_bg = if btn_response.hovered() {
            egui::Color32::from_rgb(200, 60, 60)
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(btn_rect, 4.0, btn_bg);
        ui.painter().text(
            btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            egui::FontId::proportional(14.0),
            if btn_response.hovered() {
                egui::Color32::WHITE
            } else {
                theme::TEXT_SECONDARY
            },
        );
    }

    (response.clicked() && !deleted, deleted)
}
pub fn thinking_bubble(ui: &mut egui::Ui) {
    ui.with_layout(
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            let max_width = (ui.available_width() * 0.78).min(700.0);
            ui.set_max_width(max_width);

            egui::Frame::new()
                .fill(crate::theme::BG_BUBBLE_AI)
                .inner_margin(10)
                .corner_radius(crate::theme::CORNER_RADIUS)
                .stroke(egui::Stroke::new(
                    crate::theme::HAIRLINE_WIDTH,
                    crate::theme::BORDER_HAIRLINE,
                ))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Thinking...")
                                .color(crate::theme::TEXT_SECONDARY),
                        );
                        ui.spinner();
                    });
                });
        },
    );
}

pub fn tool_status_bubble(ui: &mut egui::Ui, tool_name: &str) {
    ui.with_layout(
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            let max_width = (ui.available_width() * 0.78).min(700.0);
            ui.set_max_width(max_width);

            egui::Frame::new()
                .fill(crate::theme::BG_BUBBLE_AI)
                .inner_margin(10)
                .corner_radius(crate::theme::CORNER_RADIUS)
                .stroke(egui::Stroke::new(
                    crate::theme::HAIRLINE_WIDTH,
                    crate::theme::BORDER_HAIRLINE,
                ))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("🔧");

                        ui.label(
                            egui::RichText::new(format!(
                                "Executing tool `{tool_name}`..."
                            ))
                            .italics()
                            .color(crate::theme::TEXT_SECONDARY),
                        );

                        ui.spinner();
                    });
                });
        },
    );
}