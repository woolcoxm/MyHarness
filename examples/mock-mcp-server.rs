//! Minimal mock MCP server for tests and demos. Speaks newline-delimited
//! JSON-RPC on stdio: initialize, tools/list (one "echo" tool), tools/call
//! (returns the argument text uppercased).
//!
//! Used by tests/integration.rs via the built example binary; also handy for
//! manually verifying `[[mcp]]` config:
//!
//! ```toml
//! [[mcp]]
//! name = "mock"
//! command = "target/debug/examples/mock-mcp-server.exe"
//! ```

use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let id = v.get("id").cloned();
        let method = v["method"].as_str().unwrap_or_default().to_string();
        let Some(id) = id else { continue }; // notifications get no response
        let result = match method.as_str() {
            "initialize" => serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "mock-mcp", "version": "0.1.0"}
            }),
            "tools/list" => serde_json::json!({
                "tools": [{
                    "name": "echo",
                    "description": "Echoes the text back uppercased.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"text": {"type": "string"}},
                        "required": ["text"]
                    }
                }]
            }),
            "tools/call" => {
                let text = v["params"]["arguments"]["text"].as_str().unwrap_or_default();
                serde_json::json!({
                    "content": [{"type": "text", "text": format!("ECHO: {}", text.to_uppercase())}],
                    "isError": false
                })
            }
            _ => serde_json::json!({}),
        };
        let resp = serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result});
        let _ = writeln!(stdout, "{resp}");
        let _ = stdout.flush();
    }
}
