use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

pub struct TrayHandles {
    pub hidden: Arc<AtomicBool>,
    pub quit_requested: Arc<AtomicBool>,
    pub icon: tray_icon::TrayIcon,
}

pub fn setup_tray(ctx: &egui::Context) -> TrayHandles {
    let hidden = Arc::new(AtomicBool::new(false));
    let quit_requested = Arc::new(AtomicBool::new(false));

    let menu = Menu::new();
    let quit_item = MenuItem::new("Quit", true, None);
    let quit_id = quit_item.id().clone();
    menu.append_items(&[&quit_item]).unwrap();

    {
        let tray_ctx = ctx.clone();
        let hidden = hidden.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let is_hidden = hidden.load(Ordering::SeqCst);
                hidden.store(!is_hidden, Ordering::SeqCst);
                tray_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(is_hidden));
                tray_ctx.request_repaint();
            }
        }));
    }
    {
        let quit_ctx = ctx.clone();
        let quit = quit_requested.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == quit_id {
                quit.store(true, Ordering::SeqCst);
                quit_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }));
    }

    let icon_bytes = include_bytes!("./icon/AppIcon64.png");
    let image = image::load_from_memory(icon_bytes)
        .expect("Failed to load tray icon")
        .to_rgba8();
    let (icon_w, icon_h) = image.dimensions();
    let tray_icon = tray_icon::Icon::from_rgba(image.into_raw(), icon_w, icon_h)
        .expect("Failed to create tray icon");

    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(tray_icon)
        .with_tooltip("Cast Client")
        .build()
        .expect("failed to build tray icon");

    TrayHandles {
        hidden,
        quit_requested,
        icon,
    }
}
