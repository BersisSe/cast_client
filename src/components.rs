use eframe::egui::{self, Stroke, StrokeKind};
use egui_commonmark::CommonMarkCache;

use super::theme;
use crate::types::{Message, MessageSender};

pub fn message_bubble(ui: &mut egui::Ui, msg: &Message, cache: &mut CommonMarkCache) {
    let align = if msg.sender == MessageSender::User {
        egui::Layout::right_to_left(egui::Align::Min)
    } else {
        egui::Layout::left_to_right(egui::Align::Min)
    };

    ui.with_layout(align, |ui| {
        let max_width = (ui.available_width() * 0.78).min(700.0);
        ui.set_max_width(max_width);

        let fill = if msg.sender == MessageSender::User {
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
                render_label(ui, msg, cache);
            });
    });
}

fn render_label(ui: &mut egui::Ui, msg: &Message, cache: &mut CommonMarkCache) {
    if msg.ai_start {
        ui.label(egui::RichText::new("Thinking..").color(theme::TEXT_SECONDARY));
        ui.spinner();
    } else {
        egui_commonmark::CommonMarkViewer::new().show(ui, cache, &msg.content);
    }
}

pub fn edit_line(ui: &mut egui::Ui, input: &mut String, sending_disabled: bool, on_cancel: Option<&mut dyn FnMut()>) -> Option<String> {
    let mut submitted: Option<String> = None;

    egui::Frame::new()
        .fill(theme::BG_CONTENT)
        .inner_margin(6)
        .corner_radius(theme::CORNER_RADIUS)
        .stroke(Stroke::new(theme::HAIRLINE_WIDTH, theme::BORDER_HAIRLINE))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let button_width = 64.0;
                let field_width = ui.available_width() - button_width - 8.0;
                let response = ui.add_enabled(
                    !sending_disabled,
                    egui::TextEdit::singleline(input)
                        .hint_text(if sending_disabled {
                            "Waiting for response..."
                        } else {
                            "Message..."
                        })
                        .desired_width(field_width.max(0.0)),
                );

                let enter_pressed =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

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

                    if (enter_pressed || send_clicked) && !input.trim().is_empty() {
                        submitted = Some(std::mem::take(input));
                        response.request_focus();
                    }
                }
            });
        });

    submitted
}

pub fn sidebar_row(ui: &mut egui::Ui, title: &str, active: bool) -> bool {
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

    response.clicked()
}
