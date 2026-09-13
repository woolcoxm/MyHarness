//! Minimal mock LSP server for tests: answers `initialize`, publishes one
//! deterministic error diagnostic for every didOpen/didChange. std-only.
//!
//! Protocol reminder (both directions):
//!   Content-Length: <bytes>\r\n\r\n<body>

use std::io::{Read, Write};

fn main() {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    let mut buf: Vec<u8> = Vec::new();

    loop {
        // Ensure we have one full frame.
        let header_end = match buf.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(i) => i,
            None => {
                if !read_more(&mut reader, &mut buf) {
                    return; // EOF: harness closed us
                }
                continue;
            }
        };
        let len = match parse_len(&buf[..header_end]) {
            Some(n) => n,
            None => {
                buf.clear();
                if !read_more(&mut reader, &mut buf) {
                    return;
                }
                continue;
            }
        };
        if buf.len() < header_end + 4 + len {
            if !read_more(&mut reader, &mut buf) {
                return;
            }
            continue;
        }
        let body = buf[header_end + 4..header_end + 4 + len].to_vec();
        buf.drain(..header_end + 4 + len);
        let Ok(msg) = serde_json::from_slice::<serde_json::Value>(&body) else { continue };

        let method = msg["method"].as_str().unwrap_or_default();
        if let Some(id) = msg.get("id") {
            // Requests: initialize / shutdown both succeed with a null-ish
            // result; anything else gets an empty result too.
            send(&mut writer, &serde_json::json!({
                "jsonrpc": "2.0", "id": id, "result": {}
            }));
            continue;
        }
        match method {
            "textDocument/didOpen" | "textDocument/didChange" => {
                let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("?").to_string();
                let name = uri.rsplit(['/', '\\']).next().unwrap_or("?").to_string();
                send(&mut writer, &serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/publishDiagnostics",
                    "params": {
                        "uri": uri,
                        "diagnostics": [{
                            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                            "severity": 1,
                            "message": format!("MOCK_LSP_ERROR: {name}")
                        }]
                    }
                }));
            }
            _ => {}
        }
    }
}

fn send(writer: &mut std::io::StdoutLock<'_>, value: &serde_json::Value) {
    let body = value.to_string();
    let _ = write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = writer.flush();
}

fn read_more(reader: &mut std::io::StdinLock<'_>, buf: &mut Vec<u8>) -> bool {
    let mut chunk = [0u8; 4096];
    match reader.read(&mut chunk) {
        Ok(0) | Err(_) => false,
        Ok(n) => {
            buf.extend_from_slice(&chunk[..n]);
            true
        }
    }
}

fn parse_len(headers: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(headers);
    for line in text.lines() {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                return value.trim().parse().ok();
            }
        }
    }
    None
}
