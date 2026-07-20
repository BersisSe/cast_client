
use eframe::egui;
use egui::FontFamily::{Proportional, Monospace as EguiMonospace};
use egui::FontId;
use egui::TextStyle::*;
use egui::{Style, Visuals, Stroke, Color32, CornerRadius};
use std::collections::BTreeMap;

pub const BG_SIDEBAR: Color32 = Color32::from_rgb(0x14, 0x17, 0x1C);
pub const BG_CONTENT: Color32 = Color32::from_rgb(0x1A, 0x1E, 0x24);
pub const BG_BUBBLE_AI: Color32 = Color32::from_rgb(0x20, 0x24, 0x2B);
pub const BG_BUBBLE_USER: Color32 = Color32::from_rgb(0x23, 0x2A, 0x35);
pub const BORDER_HAIRLINE: Color32 = Color32::from_rgb(0x2E, 0x34, 0x40);
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xDC, 0xE1, 0xE8);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0x7A, 0x84, 0x94);
pub const ACCENT: Color32 = Color32::from_rgb(0x5B, 0x9D, 0xD9);

pub const CORNER_RADIUS: u8 = 6; 
pub const HAIRLINE_WIDTH: f32 = 1.0;

pub fn custom_styling(ctx: &egui::Context) {
    let mut style = Style::default();
    
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.visuals.override_text_color = Some(TEXT_PRIMARY);

    let mut visuals = Visuals::dark();
    visuals.selection.bg_fill = ACCENT;
    visuals.panel_fill = BG_CONTENT;
    visuals.window_fill = BG_CONTENT;
    visuals.extreme_bg_color = BG_SIDEBAR;

    visuals.window_corner_radius = CornerRadius::same(CORNER_RADIUS);

    let no_stroke = Stroke::NONE;
    
    visuals.widgets.noninteractive.bg_fill = BG_CONTENT;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER_HAIRLINE);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(CORNER_RADIUS);

    visuals.widgets.inactive.bg_fill = BG_BUBBLE_AI;
    visuals.widgets.inactive.bg_stroke = no_stroke; 
    visuals.widgets.inactive.corner_radius = CornerRadius::same(CORNER_RADIUS);

    visuals.widgets.hovered.bg_fill = BG_BUBBLE_USER;
    visuals.widgets.hovered.bg_stroke = no_stroke;
    visuals.widgets.hovered.corner_radius = CornerRadius::same(CORNER_RADIUS);

    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.bg_stroke = no_stroke;
    visuals.widgets.active.corner_radius = CornerRadius::same(CORNER_RADIUS);

    style.visuals = visuals;

    let text_styles: BTreeMap<_, _> = [
        (Heading, FontId::new(24.0, Proportional)),
        (Body, FontId::new(15.0, Proportional)),
        (Monospace, FontId::new(14.0, EguiMonospace)), 
        (Button, FontId::new(14.0, Proportional)),
        (Small, FontId::new(12.0, Proportional)),
    ]
    .into();

    style.text_styles = text_styles;

    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        "Lexend-Light".to_owned(),
        egui::FontData::from_static(include_bytes!("./fonts/Lexend-Light.ttf")).into(),
    );
    fonts.families.entry(Proportional).or_default().insert(0, "Lexend-Light".to_owned());

    fonts.font_data.insert(
        "JetBrains-Mono".to_owned(),
        egui::FontData::from_static(include_bytes!("./fonts/JetBrainsMono-Regular.ttf")).into(),
    );
    fonts.families.entry(EguiMonospace).or_default().insert(0, "JetBrains-Mono".to_owned());

    add_system_fallback_fonts(&mut fonts);
    ctx.set_fonts(fonts);
    ctx.set_style_of(egui::Theme::Dark, style);
}

fn add_system_fallback_fonts(fonts: &mut egui::FontDefinitions) {
    #[cfg(target_os = "windows")]
    {
        let candidates = [
            r"C:\Windows\Fonts\seguisym.ttf",
            r"C:\Windows\Fonts\seguiemj.ttf",
        ];
        for path in &candidates {
            if let Ok(data) = std::fs::read(path) {
                let name = "Fallback-SegoeSymbol".to_owned();
                fonts.font_data.insert(name.clone(), egui::FontData::from_owned(data).into());
                fonts.families.entry(Proportional).or_default().push(name.clone());
                fonts.families.entry(EguiMonospace).or_default().push(name);
                break;
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(data) = std::fs::read("/System/Library/Fonts/Apple Symbols.ttf") {
            let name = "Fallback-AppleSymbols".to_owned();
            fonts.font_data.insert(name.clone(), egui::FontData::from_owned(data).into());
            fonts.families.entry(Proportional).or_default().push(name.clone());
            fonts.families.entry(EguiMonospace).or_default().push(name);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ];
        for path in &candidates {
            if let Ok(data) = std::fs::read(path) {
                let name = "Fallback-Linux".to_owned();
                fonts.font_data.insert(name.clone(), egui::FontData::from_owned(data).into());
                fonts.families.entry(Proportional).or_default().push(name.clone());
                fonts.families.entry(EguiMonospace).or_default().push(name);
                break;
            }
        }
    }
}