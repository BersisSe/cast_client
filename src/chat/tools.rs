use genai::chat::{Tool, ToolCall};
use serde_json::json;

/// Returns the schema definitions for all tools available to Agent Mode.
pub fn get_available_tools() -> Vec<Tool> {
    vec![
        Tool::new("get_weather")
            .with_description("Get live current weather for a city using wttr.in")
            .with_schema(json!({
                "type": "object",
                "properties": {
                    "city": { "type": "string", "description": "The city name, e.g. London, Tokyo" }
                },
                "required": ["city"]
            })),
        Tool::new("read_file")
            .with_description("Read the text content of a file from the local filesystem")
            .with_schema(json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative or absolute file path" }
                },
                "required": ["path"]
            })),
        Tool::new("write_file")
            .with_description("Write content to a file on the local filesystem (creates or overwrites)")
            .with_schema(json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Target file path" },
                    "content": { "type": "string", "description": "Text content to write" }
                },
                "required": ["path", "content"]
            })),
        Tool::new("list_directory")
            .with_description("List files and subdirectories inside a directory path")
            .with_schema(json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path (defaults to current directory '.')" }
                }
            })),
        Tool::new("web_fetch")
            .with_description("Fetch raw text/HTML response from a web URL")
            .with_schema(json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Full URL including protocol, e.g. https://example.com" }
                },
                "required": ["url"]
            })),
    ]
}

/// Executes a tool call asynchronously and returns the result as a string.
pub async fn execute_tool(call: &ToolCall) -> Result<String, String> {
    match call.fn_name.as_str() {
        "get_weather" => {
            let city = call
                .fn_arguments
                .get("city")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required 'city' argument".to_string())?;

            let url = format!("https://wttr.in/{}?format=3", city);
            let res = reqwest::get(&url)
                .await
                .map_err(|e| format!("Weather fetch error: {e}"))?;

            let text = res
                .text()
                .await
                .map_err(|e| format!("Failed to read response: {e}"))?;

            Ok(text.trim().to_string())
        }

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

            Ok(format!("Successfully wrote {} bytes to '{path}'", content.len()))
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
                let is_dir = entry.file_type().await.map_or(false, |ft| ft.is_dir());

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

            // Limit response size to 8KB so it doesn't overflow context
            if text.len() > 8000 {
                Ok(format!("{}...\n\n[Content truncated at 8000 characters]", &text[..8000]))
            } else {
                Ok(text)
            }
        }

        _ => Err(format!("Unknown tool: {}", call.fn_name)),
    }
}