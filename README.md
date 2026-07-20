<div align="center">
  <img src="./src/icon/AppIcon64.png" alt="Cast Client Icon" />
  <h1>Cast Client</h1>
  <p>A Native AI Chat Client Written in Rust + egui</p>
</div>

## Features

- Streaming responses (SSE) with live markdown rendering
- Multiple conversations, persisted to disk
- Works with any OpenAI-compatible endpoint (Gemini, OpenAI, local models via
  Ollama, etc.) - not locked to one provider
- Single background thread for networking, not a full async worker pool
- GPU-accelerated UI via `egui`/`eframe`, no browser engine involved

## Memory footprint

| | Idle RAM |
|---|---|
| Gemini web app (Firefox tab) | ~580MB |
| OpenCode | ~670MB |
| Cast Client | ~110MB |

*Note*: These are just comparisons of memory usage Web app is much more capable than Cast and Open Code is just build for agentic coding. Cast is not trying to replace them.

## Building

```bash
cargo build --release
