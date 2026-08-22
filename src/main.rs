#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod chat;
mod components;
mod storage;
mod theme;
mod tray;
mod types;

use mimalloc::MiMalloc;
use tracing_subscriber::fmt::init as init_tracing;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() -> eframe::Result<()> {
    init_tracing();
    app::run()
}
