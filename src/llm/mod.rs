//! Unified LLM message model and the `Provider` trait.
//!
//! The harness keeps one internal representation of a conversation
//! (`Message` / `ContentBlock`) and each provider serializes it to its own
//! wire format. Invariants the rest of the code relies on:
//! - `ToolUse` blocks only appear in `Assistant` messages.
//! - `ToolResult` blocks only appear in `User` messages, and each one answers
//!   a `ToolUse` from the immediately preceding assistant message.

pub mod anthropic;
pub mod mock;
pub mod openai;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<ContentImage>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

/// An image attached to a tool result (base64-encoded bytes + mime type).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContentImage {
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn tool_results(blocks: Vec<ContentBlock>) -> Self {
        Message {
            role: Role::User,
            content: blocks,
        }
    }

    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Input tokens served from the prompt cache (billed far below fresh
    /// input). Anthropic `cache_read_input_tokens`; OpenAI
    /// `prompt_tokens_details.cached_tokens`. Defaults to 0 for old sessions.
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Input tokens written to the prompt cache (billed at a small premium).
    /// Anthropic `cache_creation_input_tokens`.
    #[serde(default)]
    pub cache_creation_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    pub max_tokens: u32,
    pub temperature: f32,
}

/// Events emitted while streaming one assistant turn.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    MessageStart,
    TextDelta(String),
    /// Model reasoning/thinking output. Displayed to the operator, never
    /// fed back into the context (providers don't accept it on input).
    ThinkingDelta(String),
    ToolUseStart { id: String, name: String },
    ToolInputDelta(String),
    BlockStop,
    MessageDelta {
        #[allow(dead_code)] // kept for protocol completeness; turn end is
        // decided by the absence of tool calls, which is provider-agnostic
        stop_reason: Option<String>,
    },
    Usage(Usage),
    MessageStop,
}

/// A stream of provider events. Implemented as a channel receiver so the
/// trait stays object-safe (`Arc<dyn Provider>` works everywhere, including
/// inside subagents spawned by the `task` tool).
pub type EventRx = mpsc::Receiver<Result<StreamEvent>>;

#[async_trait]
pub trait Provider: Send + Sync {
    async fn stream(&self, req: &LlmRequest) -> Result<EventRx>;
    fn name(&self) -> &'static str;
}

/// Minimal SSE frame splitter shared by the streaming providers. Feed it raw
/// bytes; it returns complete `(event, data)` frames. Payloads are JSON, so
/// normalizing CRLF inside the buffer cannot corrupt them (raw control
/// characters never appear unescaped inside JSON strings).
pub(crate) struct SseBuffer {
    buf: String,
}

impl SseBuffer {
    pub fn new() -> Self {
        SseBuffer { buf: String::new() }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<(Option<String>, String)> {
        self.buf.push_str(&String::from_utf8_lossy(bytes));
        self.buf = self.buf.replace("\r\n", "\n");
        let mut frames = Vec::new();
        while let Some(idx) = self.buf.find("\n\n") {
            let block: String = self.buf.drain(..idx + 2).collect();
            let mut event: Option<String> = None;
            let mut data = String::new();
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("event:") {
                    event = Some(v.trim().to_string());
                } else if let Some(v) = line.strip_prefix("data:") {
                    data.push_str(v.strip_prefix(' ').unwrap_or(v));
                    data.push('\n');
                }
            }
            if data.ends_with('\n') {
                data.pop();
            }
            if event.is_some() || !data.is_empty() {
                frames.push((event, data));
            }
        }
        frames
    }
}

impl Default for SseBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Retry policy shared by providers: connection errors and 429/5xx responses
/// are retried with exponential backoff (honoring Retry-After when present).
/// Once the stream is established, failures propagate to the caller.
pub(crate) async fn send_with_retry(request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    let mut delay = 0.5f64;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let rb = request
            .try_clone()
            .ok_or_else(|| anyhow::anyhow!("request is not cloneable"))?;
        match rb.send().await {
            Ok(resp) if resp.status().is_success() => return Ok(resp),
            Ok(resp) => {
                let status = resp.status();
                let retry_after = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<f64>().ok());
                let body = resp.text().await.unwrap_or_default();
                let retryable = status.as_u16() == 429
                    || status.as_u16() == 408
                    || status.is_server_error();
                if !retryable || attempt >= 5 {
                    anyhow::bail!("API error {}: {}", status, body.chars().take(2000).collect::<String>());
                }
                let wait = retry_after.unwrap_or(delay);
                tokio::time::sleep(std::time::Duration::from_secs_f64(wait)).await;
                delay *= 2.0;
            }
            Err(e) => {
                if attempt >= 5 {
                    anyhow::bail!("connection error after {attempt} attempts: {e}");
                }
                tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
                delay *= 2.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_splits_frames_across_chunks() {
        let mut sse = SseBuffer::new();
        // One anthropic-style stream split at awkward boundaries.
        let a = b"event: message_start\ndata: {\"message\":{\"usage\":{\"input_to";
        let b = b"kens\":42}}}\n\nevent: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n";
        let c = b"event: message_stop\ndata: {}\n\n";
        let mut frames = sse.feed(a);
        assert!(frames.is_empty(), "partial frame must not emit");
        frames = sse.feed(b);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0.as_deref(), Some("message_start"));
        assert!(frames[0].1.contains("\"input_tokens\":42"));
        assert_eq!(frames[1].0.as_deref(), Some("content_block_delta"));
        frames = sse.feed(c);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0.as_deref(), Some("message_stop"));
    }

    #[test]
    fn sse_handles_crlf() {
        let mut sse = SseBuffer::new();
        let frames = sse.feed(b"event: ping\r\ndata: {}\r\n\r\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0.as_deref(), Some("ping"));
    }
}
