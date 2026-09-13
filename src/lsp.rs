//! Minimal LSP client: spawn a language server over stdio, speak just
//! enough JSON-RPC (Content-Length framing) to open changed files and
//! collect the diagnostics it publishes. One client per configured server,
//! started lazily on the first relevant edit and killed with the harness
//! (kill_on_drop). This is deliberately NOT a general LSP implementation —
//! no completions, no workspace edits — just the feedback loop.

use crate::config::LspServerConfig;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin};
use tokio::sync::mpsc;

/// One published diagnostic, already reduced to what the model needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    pub path: String,
    pub line: u32,
    pub severity: u8, // 1 = error, 2 = warning
    pub message: String,
}

pub struct LspClient {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::UnboundedReceiver<Value>,
    next_id: i64,
    opened: HashSet<String>,
}

impl LspClient {
    /// Spawn the server and run the initialize handshake.
    pub async fn start(cfg: &LspServerConfig, root: &Path) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(&cfg.command);
        cmd.args(&cfg.args)
            .current_dir(root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn().with_context(|| format!("spawning LSP server '{}'", cfg.command))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("LSP server has no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("LSP server has no stdout"))?;
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(reader_loop(stdout, tx));

        let mut client = LspClient { child, stdin, rx, next_id: 1, opened: HashSet::new() };
        let root_uri = path_to_uri(root);
        let _resp = client
            .request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootUri": root_uri,
                    "capabilities": {},
                }),
                10_000,
            )
            .await
            .context("LSP initialize failed (server may not speak LSP over stdio)")?;
        client.notify("initialized", json!({})).await;
        Ok(client)
    }

    /// didOpen the first time a file is seen, full-text didChange after.
    pub async fn open_or_change(&mut self, uri: &str, language_id: &str, text: &str) -> Result<()> {
        let params = if self.opened.insert(uri.to_string()) {
            json!({
                "textDocument": {"uri": uri, "languageId": language_id, "version": 1, "text": text}
            })
        } else {
            json!({
                "textDocument": {"uri": uri, "version": 2},
                "contentChanges": [{"text": text}]
            })
        };
        let method = if params.get("contentChanges").is_some() {
            "textDocument/didChange"
        } else {
            "textDocument/didOpen"
        };
        self.notify(method, params).await;
        Ok(())
    }

    /// Drain published diagnostics until every awaited uri has been heard
    /// from (servers publish per didOpen/didChange) or `wait_ms` elapses.
    /// Only errors and warnings are kept.
    pub async fn collect_diagnostics(&mut self, wait_ms: u64, awaited: &HashSet<String>) -> Vec<Diag> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(wait_ms);
        let mut heard: HashSet<String> = HashSet::new();
        let mut diags = Vec::new();
        while heard.len() < awaited.len() {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let Ok(msg) = tokio::time::timeout(remaining, self.rx.recv()).await else { break };
            let Some(msg) = msg else { break };
            if msg.get("method").and_then(|m| m.as_str()) != Some("textDocument/publishDiagnostics") {
                continue;
            }
            let uri = msg["params"]["uri"].as_str().unwrap_or_default().to_string();
            heard.insert(uri.clone());
            let path = uri_to_path(&uri);
            for d in msg["params"]["diagnostics"].as_array().into_iter().flatten() {
                let severity = d["severity"].as_u64().unwrap_or(1) as u8;
                if severity > 2 {
                    continue;
                }
                let line = d["range"]["start"]["line"].as_u64().unwrap_or(0) as u32 + 1;
                let message = d["message"].as_str().unwrap_or("").trim().to_string();
                if !message.is_empty() {
                    diags.push(Diag { path: path.clone(), line, severity, message });
                }
            }
        }
        diags.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
        diags
    }

    /// True if the underlying process has exited.
    pub fn is_dead(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    async fn request(&mut self, method: &str, params: Value, timeout_ms: u64) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).await;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let msg = tokio::time::timeout(remaining, self.rx.recv())
                .await
                .map_err(|_| anyhow!("timeout waiting for LSP response to '{method}'"))?
                .ok_or_else(|| anyhow!("LSP server closed stdout before responding to '{method}'"))?;
            if msg.get("id").and_then(|i| i.as_i64()) == Some(id) {
                if let Some(err) = msg.get("error") {
                    return Err(anyhow!("LSP error on '{method}': {err}"));
                }
                return Ok(msg["result"].clone());
            }
            // Notifications arriving mid-request are dropped; diagnostics
            // are re-published on the next change.
        }
    }

    async fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params})).await;
    }

    async fn send(&mut self, value: &Value) {
        let body = value.to_string();
        let frame = format!("Content-Length: {}\r\n\r\n{body}", body.len());
        let _ = self.stdin.write_all(frame.as_bytes()).await;
        let _ = self.stdin.flush().await;
    }
}

/// Parse Content-Length framed JSON from the server's stdout into a channel.
async fn reader_loop<R: AsyncReadExt + Unpin>(mut stdout: R, tx: mpsc::UnboundedSender<Value>) {
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    loop {
        // Read until we have headers + body for one message.
        let header_end = match find_header_end(&buf) {
            Some(i) => i,
            None => {
                if read_more(&mut stdout, &mut buf).await.is_err() {
                    return;
                }
                continue;
            }
        };
        let len = match parse_content_length(&buf[..header_end]) {
            Some(n) => n,
            None => {
                // Malformed framing; drop the buffer and try to resync.
                buf.clear();
                if read_more(&mut stdout, &mut buf).await.is_err() {
                    return;
                }
                continue;
            }
        };
        if buf.len() < header_end + 4 + len {
            if read_more(&mut stdout, &mut buf).await.is_err() {
                return;
            }
            continue;
        }
        let body_range = header_end + 4..header_end + 4 + len;
        let body = buf[body_range].to_vec();
        buf.drain(..header_end + 4 + len);
        if let Ok(v) = serde_json::from_slice::<Value>(&body) {
            let _ = tx.send(v);
        }
    }
}

async fn read_more<R: AsyncReadExt + Unpin>(stdout: &mut R, buf: &mut Vec<u8>) -> std::io::Result<()> {
    let mut chunk = [0u8; 4096];
    let n = stdout.read(&mut chunk).await?;
    if n == 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof"));
    }
    buf.extend_from_slice(&chunk[..n]);
    Ok(())
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(headers);
    for line in text.lines() {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
}

/// Absolute path → `file:///` URI (forward slashes, percent-encoded per char
/// where needed).
pub fn path_to_uri(p: &Path) -> String {
    let s = p.display().to_string().replace('\\', "/");
    let mut out = String::from("file:///");
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' | ':' => out.push(c),
            _ => {
                for b in c.to_string().as_bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

/// `file:///` URI → filesystem path (percent-decoded, native separators).
pub fn uri_to_path(uri: &str) -> String {
    let rest = uri.trim_start_matches("file:///");
    let decoded = crate::tools::web_search::percent_decode(rest);
    if cfg!(windows) {
        decoded.replace('/', "\\")
    } else {
        decoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn uri_round_trip_windows() {
        let uri = path_to_uri(Path::new("C:\\work\\my proj\\a b.rs"));
        assert!(uri.starts_with("file:///C:/work/my%20proj/a%20b.rs"), "{uri}");
        assert_eq!(uri_to_path(&uri), "C:\\work\\my proj\\a b.rs");
    }

    #[test]
    #[cfg(unix)]
    fn uri_round_trip_unix() {
        let uri = path_to_uri(Path::new("/home/u/my proj/a.rs"));
        assert!(uri.starts_with("file:///home/u/my%20proj/a.rs"), "{uri}");
        assert_eq!(uri_to_path(&uri), "/home/u/my proj/a.rs");
    }

    #[test]
    fn content_length_parsing() {
        assert_eq!(parse_content_length(b"Content-Length: 18"), Some(18));
        assert_eq!(parse_content_length(b"content-length: 7"), Some(7));
        assert_eq!(parse_content_length(b"No Header"), None);
    }
}
