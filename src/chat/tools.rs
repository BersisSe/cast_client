use genai::chat::{Tool, ToolCall};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltinTool {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: serde_json::Value,
    pub enabled_by_default: bool,
}

pub fn get_builtin_tool_defs() -> Vec<BuiltinTool> {
    vec![
        BuiltinTool {
            name: "read_file",
            description: "Read the text content of a file from the local filesystem",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative or absolute file path" }
                },
                "required": ["path"]
            }),
            enabled_by_default: true,
        },
        BuiltinTool {
            name: "write_file",
            description: "Write content to a file on the local filesystem (creates or overwrites)",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Target file path" },
                    "content": { "type": "string", "description": "Text content to write" }
                },
                "required": ["path", "content"]
            }),
            enabled_by_default: true,
        },
        BuiltinTool {
            name: "list_directory",
            description: "List files and subdirectories inside a directory path",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path (defaults to current directory '.')" }
                }
            }),
            enabled_by_default: true,
        },
        BuiltinTool {
            name: "web_fetch",
            description: "Fetch raw text/HTML response from a web URL",
            schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Full URL including protocol, e.g. https://example.com" }
                },
                "required": ["url"]
            }),
            enabled_by_default: true,
        },
    ]
}

pub fn get_enabled_builtin_tools(preferences: &HashMap<String, bool>) -> Vec<Tool> {
    get_builtin_tool_defs()
        .into_iter()
        .filter(|def| {
            preferences
                .get(def.name)
                .copied()
                .unwrap_or(def.enabled_by_default)
        })
        .map(|def| {
            Tool::new(def.name)
                .with_description(def.description)
                .with_schema(def.schema)
        })
        .collect()
}

/// Executes a tool call asynchronously and returns the result as a string.
pub async fn execute_tool(call: &ToolCall) -> Result<String, String> {
    match call.fn_name.as_str() {
        "read_file" => {
            let path = call
                .fn_arguments
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required 'path' argument".to_string())?;

            tokio::fs::read_to_string(path)
                .await
                .map_err(|e| format!("Failed to read '{path}': {e}"))
        }

        "write_file" => {
            let path = call
                .fn_arguments
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required 'path' argument".to_string())?;

            let content = call
                .fn_arguments
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required 'content' argument".to_string())?;

            tokio::fs::write(path, content)
                .await
                .map_err(|e| format!("Failed to write to '{path}': {e}"))?;

            Ok(format!(
                "Successfully wrote {} bytes to '{path}'",
                content.len()
            ))
        }

        "list_directory" => {
            let path = call
                .fn_arguments
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or(".");

            let mut entries = tokio::fs::read_dir(path)
                .await
                .map_err(|e| format!("Failed to read directory '{path}': {e}"))?;

            let mut items = Vec::new();
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_dir = entry.file_type().await.is_ok_and(|ft| ft.is_dir());

                if is_dir {
                    items.push(format!("📁 {name}/"));
                } else {
                    items.push(format!("📄 {name}"));
                }
            }

            if items.is_empty() {
                Ok(format!("Directory '{path}' is empty."))
            } else {
                Ok(items.join("\n"))
            }
        }

        "web_fetch" => {
            let url = call
                .fn_arguments
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required 'url' argument".to_string())?;

            let res = reqwest::get(url)
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?;

            let text = res
                .text()
                .await
                .map_err(|e| format!("Failed to read body: {e}"))?;

            // Limit response size to 8KB.
            if text.len() > 8000 {
                Ok(format!(
                    "{}...\n\n[Content truncated at 8000 characters]",
                    &text[..8000]
                ))
            } else {
                Ok(text)
            }
        }

        _ => Err(format!("Unknown tool: {}", call.fn_name)),
    }
}
