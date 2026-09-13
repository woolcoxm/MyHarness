//! OpenAI-compatible chat-completions client with streaming tool calls.
//! Works with `https://api.z.ai/api/paas/v4` (GLM) and any other
//! OpenAI-shaped endpoint.

use anyhow::Result;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::{
    send_with_retry, ContentBlock, EventRx, LlmRequest, Message, Provider, Role, SseBuffer,
    StreamEvent,
};

pub struct OpenAiProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl OpenAiProvider {
    pub fn new(base_url: String, api_key: String) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        OpenAiProvider {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        }
    }
}

fn serialize_messages(system: &str, messages: &[Message]) -> Vec<Value> {
    let mut out = vec![json!({"role": "system", "content": system})];
    for m in messages {
        match m.role {
            Role::User => {
                // Tool results become `role: "tool"` messages; text stays a
                // normal user message. Order is preserved.
                for b in &m.content {
                    if let ContentBlock::ToolResult { tool_use_id, content, images, is_error } = b {
                        let mut body = if *is_error {
                            format!("ERROR: {content}")
                        } else {
                            content.clone()
                        };
                        // chat.completions tool messages are text-only; note
                        // the images instead of dropping them silently.
                        if !images.is_empty() {
                            body.push_str(&format!(
                                "\n({} image{} returned; this protocol cannot display them — ask for a textual description if needed)",
                                images.len(),
                                if images.len() > 1 { "s" } else { "" }
                            ));
                        }
                        out.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_use_id,
                            "content": body,
                        }));
                    }
                }
                let text = m.text();
                if !text.trim().is_empty() {
                    out.push(json!({"role": "user", "content": text}));
                }
            }
            Role::Assistant => {
                let text = m.text();
                let calls: Vec<Value> = m
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolUse { id, name, input } => Some(json!({
                            "id": id,
                            "type": "function",
                            "function": {"name": name, "arguments": input.to_string()},
                        })),
                        _ => None,
                    })
                    .collect();
                let mut v = json!({"role": "assistant"});
                if !text.is_empty() {
                    v["content"] = json!(text);
                }
                if !calls.is_empty() {
                    v["tool_calls"] = json!(calls);
                }
                out.push(v);
            }
        }
    }
    out
}

#[async_trait]
impl Provider for OpenAiProvider {
    fn name(&self) -> &'static str {
        "openai"
    }

    async fn stream(&self, req: &LlmRequest) -> Result<EventRx> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = json!({
            "model": req.model,
            "messages": serialize_messages(&req.system, &req.messages),
            "tools": req.tools.iter().map(|t| json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                }
            })).collect::<Vec<_>>(),
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        let request = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body);

        let resp = send_with_retry(request).await?;
        let mut stream = resp.bytes_stream();
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut sse = SseBuffer::new();
            let mut started = false;
            // tool-call fragments arrive per-index; track which indexes we
            // have already announced with ToolUseStart.
            let mut open_indexes: Vec<usize> = Vec::new();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        for (_event, data) in sse.feed(&bytes) {
                            if data == "[DONE]" {
                                let _ = tx.send(Ok(StreamEvent::MessageStop)).await;
                                return;
                            }
                            let Ok(v) = serde_json::from_str::<Value>(&data) else {
                                continue;
                            };
                            if !started {
                                started = true;
                                let _ = tx.send(Ok(StreamEvent::MessageStart)).await;
                            }
                            if let Some(usage) = v.get("usage").filter(|u| !u.is_null()) {
                                let _ = tx.send(Ok(StreamEvent::Usage(super::Usage {
                                    input_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0),
                                    output_tokens: usage["completion_tokens"].as_u64().unwrap_or(0),
                                    cache_read_tokens: usage["prompt_tokens_details"]["cached_tokens"]
                                        .as_u64()
                                        .unwrap_or(0),
                                    ..Default::default()
                                }))).await;
                            }
                            let Some(choice) = v["choices"].get(0) else { continue };
                            let delta = &choice["delta"];
                            // GLM reasoning arrives as reasoning_content.
                            if let Some(thinking) = delta["reasoning_content"].as_str() {
                                if !thinking.is_empty() {
                                    let _ = tx.send(Ok(StreamEvent::ThinkingDelta(thinking.to_string()))).await;
                                }
                            }
                            if let Some(text) = delta["content"].as_str() {
                                if !text.is_empty() {
                                    let _ = tx.send(Ok(StreamEvent::TextDelta(text.to_string()))).await;
                                }
                            }
                            if let Some(tcs) = delta["tool_calls"].as_array() {
                                for tc in tcs {
                                    let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                                    let id = tc["id"].as_str();
                                    let name = tc["function"]["name"].as_str();
                                    if (id.is_some() || name.is_some()) && !open_indexes.contains(&idx) {
                                        open_indexes.push(idx);
                                        let _ = tx.send(Ok(StreamEvent::ToolUseStart {
                                            id: id.map(str::to_string).unwrap_or_else(|| format!("call_{idx}")),
                                            name: name.unwrap_or_default().to_string(),
                                        })).await;
                                    }
                                    if let Some(args) = tc["function"]["arguments"].as_str() {
                                        if !args.is_empty() {
                                            let _ = tx.send(Ok(StreamEvent::ToolInputDelta(args.to_string()))).await;
                                        }
                                    }
                                }
                            }
                            if let Some(finish) = choice["finish_reason"].as_str() {
                                for _ in &open_indexes {
                                    let _ = tx.send(Ok(StreamEvent::BlockStop)).await;
                                }
                                open_indexes.clear();
                                let stop = match finish {
                                    "tool_calls" => "tool_use".to_string(),
                                    "length" => "max_tokens".to_string(),
                                    _ => "end_turn".to_string(),
                                };
                                let _ = tx.send(Ok(StreamEvent::MessageDelta { stop_reason: Some(stop) })).await;
                            }
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
    fn tool_results_become_tool_messages() {
        let messages = vec![Message {
            role: Role::User,
            content: vec![
                ContentBlock::ToolResult { tool_use_id: "t9".into(), content: "ok".into(), images: Vec::new(), is_error: false },
                ContentBlock::Text { text: "and this".into() },
            ],
        }];
        let wire = serialize_messages("sys", &messages);
        assert_eq!(wire[0]["role"], "system");
        assert_eq!(wire[1]["role"], "tool");
        assert_eq!(wire[1]["tool_call_id"], "t9");
        assert_eq!(wire[2]["role"], "user");
        assert_eq!(wire[2]["content"], "and this");
    }

    #[test]
    fn assistant_tool_calls_stringify_arguments() {
        let messages = vec![Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "c1".into(),
                name: "read_file".into(),
                input: serde_json::json!({"path": "x.rs"}),
            }],
        }];
        let wire = serialize_messages("sys", &messages);
        assert_eq!(wire[1]["tool_calls"][0]["function"]["name"], "read_file");
        assert_eq!(wire[1]["tool_calls"][0]["function"]["arguments"], r#"{"path":"x.rs"}"#);
        assert!(wire[1].get("content").is_none(), "empty text should be omitted");
    }
}
