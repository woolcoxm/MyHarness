//! Anthropic Messages API client (also works with Anthropic-compatible
//! endpoints such as Z.ai's `https://api.z.ai/api/anthropic`, which is what
//! GLM coding models are heaviest-trained against).

use anyhow::Result;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::{
    send_with_retry, ContentBlock, EventRx, LlmRequest, Message, Provider, Role, SseBuffer,
    StreamEvent, ToolSchema,
};

pub struct AnthropicProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    /// Mark tools/system/last message with cache_control breakpoints.
    prompt_caching: bool,
}

impl AnthropicProvider {
    pub fn new(base_url: String, api_key: String, prompt_caching: bool) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        AnthropicProvider {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            prompt_caching,
        }
    }
}

fn serialize_messages(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            let blocks: Vec<Value> = m
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } if !text.is_empty() => {
                        Some(json!({"type": "text", "text": text}))
                    }
                    ContentBlock::ToolUse { id, name, input } => Some(json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input,
                    })),
                    ContentBlock::ToolResult { tool_use_id, content, images, is_error } => {
                        // With images the content becomes a block array
                        // (text + image blocks); plain text stays a string.
                        if images.is_empty() {
                            let mut v = json!({
                                "type": "tool_result",
                                "tool_use_id": tool_use_id,
                                "content": content,
                            });
                            if *is_error {
                                v["is_error"] = json!(true);
                            }
                            Some(v)
                        } else {
                            let mut blocks = vec![json!({"type": "text", "text": content})];
                            for img in images {
                                blocks.push(json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": img.media_type,
                                        "data": img.data,
                                    }
                                }));
                            }
                            let mut v = json!({
                                "type": "tool_result",
                                "tool_use_id": tool_use_id,
                                "content": blocks,
                            });
                            if *is_error {
                                v["is_error"] = json!(true);
                            }
                            Some(v)
                        }
                    }
                    _ => None,
                })
                .collect();
            json!({
                "role": if m.role == Role::Assistant { "assistant" } else { "user" },
                "content": blocks,
            })
        })
        .collect()
}

fn serialize_tools(tools: &[ToolSchema], cache_last: bool) -> Vec<Value> {
    let mut out: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect();
    // Anthropic prompt caching: a breakpoint on the last tool caches the
    // whole tool prefix across requests.
    if cache_last {
        if let Some(last) = out.last_mut() {
            last["cache_control"] = json!({"type": "ephemeral"});
        }
    }
    out
}

fn map_event(event: Option<&str>, data: &str) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return out;
    };
    match event {
        Some("message_start") => {
            out.push(StreamEvent::MessageStart);
            let input = v["message"]["usage"]["input_tokens"].as_u64().unwrap_or(0);
            out.push(StreamEvent::Usage(super::Usage { input_tokens: input, output_tokens: 0 }));
        }
        Some("content_block_start") => {
            if v["content_block"]["type"] == "tool_use" {
                out.push(StreamEvent::ToolUseStart {
                    id: v["content_block"]["id"].as_str().unwrap_or_default().to_string(),
                    name: v["content_block"]["name"].as_str().unwrap_or_default().to_string(),
                });
            }
        }
        Some("content_block_delta") => {
            let delta = &v["delta"];
            match delta["type"].as_str() {
                Some("text_delta") => out.push(StreamEvent::TextDelta(
                    delta["text"].as_str().unwrap_or_default().to_string(),
                )),
                Some("thinking_delta") => out.push(StreamEvent::ThinkingDelta(
                    delta["thinking"].as_str().unwrap_or_default().to_string(),
                )),
                Some("input_json_delta") => out.push(StreamEvent::ToolInputDelta(
                    delta["partial_json"].as_str().unwrap_or_default().to_string(),
                )),
                _ => {}
            }
        }
        Some("content_block_stop") => out.push(StreamEvent::BlockStop),
        Some("message_delta") => {
            let output = v["usage"]["output_tokens"].as_u64().unwrap_or(0);
            out.push(StreamEvent::Usage(super::Usage { input_tokens: 0, output_tokens: output }));
            out.push(StreamEvent::MessageDelta {
                stop_reason: v["delta"]["stop_reason"].as_str().map(str::to_string),
            });
        }
        Some("message_stop") => out.push(StreamEvent::MessageStop),
        _ => {}
    }
    out
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &'static str {
        "anthropic"
    }
    async fn stream(&self, req: &LlmRequest) -> Result<EventRx> {
        let url = format!("{}/v1/messages", self.base_url);
        // With caching, system becomes a block array so the breakpoint can
        // sit on it; otherwise it stays a plain string.
        let system = if self.prompt_caching {
            json!([{
                "type": "text",
                "text": req.system,
                "cache_control": {"type": "ephemeral"},
            }])
        } else {
            json!(req.system)
        };
        // Mark the last message's last content block so the conversation
        // prefix is cacheable too (3 breakpoints total, within the limit).
        let mut messages = serialize_messages(&req.messages);
        if self.prompt_caching {
            if let Some(last) = messages.last_mut() {
                if let Some(blocks) = last["content"].as_array_mut() {
                    if let Some(last_block) = blocks.last_mut() {
                        last_block["cache_control"] = json!({"type": "ephemeral"});
                    }
                }
            }
        }
        let body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "system": system,
            "messages": messages,
            "tools": serialize_tools(&req.tools, self.prompt_caching),
            "stream": true,
        });
        let request = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body);

        let resp = send_with_retry(request).await?;
        let mut stream = resp.bytes_stream();
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut sse = SseBuffer::new();
            let mut closed = false;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        for (event, data) in sse.feed(&bytes) {
                            if event.as_deref() == Some("error") {
                                let _ = tx.send(Err(anyhow::anyhow!("stream error: {data}"))).await;
                                closed = true;
                                break;
                            }
                            for ev in map_event(event.as_deref(), &data) {
                                let is_stop = matches!(ev, StreamEvent::MessageStop);
                                if tx.send(Ok(ev)).await.is_err() {
                                    return;
                                }
                                if is_stop {
                                    closed = true;
                                }
                            }
                        }
                        if closed {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(anyhow::anyhow!("stream read error: {e}"))).await;
                        break;
                    }
                }
            }
        });
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ContentBlock, Role};

    #[test]
    fn serializes_ir_to_wire_format() {
        let messages = vec![
            Message::user_text("hello"),
            Message {
                role: Role::Assistant,
                content: vec![
                    ContentBlock::Text { text: "calling".into() },
                    ContentBlock::ToolUse { id: "t1".into(), name: "bash".into(), input: serde_json::json!({"command": "ls"}) },
                ],
            },
            Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "t1".into(),
                    content: "files".into(),
                    images: Vec::new(),
                    is_error: false,
                }],
            },
        ];
        let wire = serde_json::to_value(serialize_messages(&messages)).unwrap();
        assert_eq!(wire[0]["role"], "user");
        assert_eq!(wire[1]["content"][1]["type"], "tool_use");
        assert_eq!(wire[1]["content"][1]["input"]["command"], "ls");
        assert_eq!(wire[2]["content"][0]["type"], "tool_result");
        assert!(wire[2]["content"][0].get("is_error").is_none(), "false is_error should be omitted");
    }

    #[test]
    fn maps_stream_events() {
        let events = map_event(Some("message_start"), r#"{"message":{"usage":{"input_tokens":10}}}"#);
        assert!(matches!(events[0], StreamEvent::MessageStart));
        let events = map_event(
            Some("content_block_delta"),
            r#"{"delta":{"type":"input_json_delta","partial_json":"{\"a\":"}}"#,
        );
        assert!(matches!(&events[0], StreamEvent::ToolInputDelta(s) if s == "{\"a\":"));
        let events = map_event(
            Some("content_block_delta"),
            r#"{"delta":{"type":"thinking_delta","thinking":"let me consider..."}}"#,
        );
        assert!(matches!(&events[0], StreamEvent::ThinkingDelta(s) if s == "let me consider..."));
        let events = map_event(
            Some("message_delta"),
            r#"{"delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}"#,
        );
        assert!(matches!(events[0], StreamEvent::Usage(u) if u.output_tokens == 7));
    }

    #[test]
    fn cache_breakpoints_marked_when_enabled() {
        let tools = vec![crate::llm::ToolSchema {
            name: "a".to_string(),
            description: String::new(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let plain = serde_json::to_value(serialize_tools(&tools, false)).unwrap();
        assert!(plain[0].get("cache_control").is_none());
        let cached = serde_json::to_value(serialize_tools(&tools, true)).unwrap();
        assert_eq!(cached[0]["cache_control"]["type"], "ephemeral");
    }
}
