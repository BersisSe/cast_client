use std::fs;
use std::path::PathBuf;

use crate::types::Conversation;

fn storage_dir() -> PathBuf {
    let base = dirs::data_dir().expect("Failed to get data directory");
    base.join("cast_client")
}

fn conversations_path() -> PathBuf {
    storage_dir().join("conversations.json")
}

pub fn load_conversations() -> Vec<Conversation> {
    let path = conversations_path();
    if !path.exists() {
        return Vec::new();
    }
    match fs::read_to_string(&path) {
        Ok(json) => serde_json::from_str(&json).unwrap_or_else(|e| {
            eprintln!("Failed to parse conversations file: {e}");
            Vec::new()
        }),
        Err(e) => {
            eprintln!("Failed to read conversations file: {e}");
            Vec::new()
        }
    }
}

pub fn save_conversations(convos: &[Conversation]) {
    let dir = storage_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("Failed to create storage directory: {e}");
        return;
    }
    let path = dir.join("conversations.json");
    let tmp_path = dir.join("conversations.json.tmp");

    match serde_json::to_string_pretty(convos) {
        Ok(json) => {
            if let Err(e) = fs::write(&tmp_path, &json) {
                eprintln!("Failed to write temp file: {e}");
                return;
            }
            if let Err(e) = fs::rename(&tmp_path, &path) {
                eprintln!("Failed to rename temp file: {e}");
                let _ = fs::remove_file(&tmp_path);
            }
        }
        Err(e) => eprintln!("Failed to serialize conversations: {e}"),
    }
}
