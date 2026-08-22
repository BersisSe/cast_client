use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::chat::mcp::McpServerConfig;
use crate::types::Conversation;

fn storage_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        dirs::data_dir()
            .expect("Failed to get data directory")
            .join("cast_client")
    })
}

fn conversations_path() -> &'static PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| storage_dir().join("conversations.json"))
}

fn tool_preferences_path() -> &'static PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| storage_dir().join("tool_preferences.json"))
}

fn mcp_servers_path() -> &'static PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| storage_dir().join("mcp_servers.json"))
}

/// Loads a JSON document, returning `None` if the file is missing or corrupt.
fn load_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    if !path.exists() {
        return None;
    }
    let json = match fs::read_to_string(path) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("Failed to read {}: {e}", path.display());
            return None;
        }
    };
    match serde_json::from_str(&json) {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("Failed to parse {}: {e}", path.display());
            None
        }
    }
}

/// Saves a JSON document atomically via a temp file + rename.
fn save_json<T: Serialize + ?Sized>(path: &Path, value: &T) {
    if let Err(e) = fs::create_dir_all(path.parent().unwrap_or(Path::new("."))) {
        eprintln!("Failed to create storage directory: {e}");
        return;
    }
    let tmp_path = path.with_extension("json.tmp");

    let json = match serde_json::to_string_pretty(value) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("Failed to serialize {}: {e}", path.display());
            return;
        }
    };
    if let Err(e) = fs::write(&tmp_path, &json) {
        eprintln!("Failed to write temp file: {e}");
        return;
    }
    if let Err(e) = fs::rename(&tmp_path, path) {
        eprintln!("Failed to rename temp file: {e}");
        let _ = fs::remove_file(&tmp_path);
    }
}

pub fn load_conversations() -> Vec<Conversation> {
    load_json(conversations_path()).unwrap_or_default()
}

pub fn save_conversations(convos: &[Conversation]) {
    save_json(conversations_path(), convos);
}

pub fn load_tool_preferences() -> HashMap<String, bool> {
    load_json(tool_preferences_path()).unwrap_or_default()
}

pub fn save_tool_preferences(preferences: &HashMap<String, bool>) {
    save_json(tool_preferences_path(), preferences);
}

pub fn load_mcp_servers() -> Vec<McpServerConfig> {
    load_json(mcp_servers_path()).unwrap_or_default()
}

pub fn save_mcp_servers(servers: &[McpServerConfig]) {
    save_json(mcp_servers_path(), servers);
}
