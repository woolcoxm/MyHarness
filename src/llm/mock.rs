//! Scripted provider used by tests and by `--provider mock` for offline
//! end-to-end runs of the real binary. Responses are consumed in order from a
//! JSON script:
//!
//! ```json
//! [
//!   {"text": "hi", "tool_calls": [{"name": "bash", "input": {"command": "pwd"}}]},
//!   {"text": "all done"}
//! ]
//! ```
//!
//! When the script runs dry, the mock answers with a bare final message so
//! runaway loops terminate quickly.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use super::{EventRx, LlmRequest, Provider, StreamEvent};

#[derive(Clone)]
pub struct MockProvider {
    pub script: ScriptQueue,
}

/// Shared, cloneable handle over a FIFO step queue.
#[derive(Clone, Default)]
pub struct ScriptQueue {
    inner: Arc<Mutex<VecDeque<Value>>>,
}

impl ScriptQueue {
    pub fn from_json(script: Vec<Value>) -> Self {
        ScriptQueue {
            inner: Arc::new(Mutex::new(script.into())),
        }
    }

    fn pop(&self) -> Option<Value> {
        self.inner.lock().unwrap().pop_front()
    }

    pub fn remaining(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

impl MockProvider {
    pub fn new(script: Vec<Value>) -> Self {
        MockProvider {
            script: ScriptQueue::from_json(script),
        }
    }

    /// Parse a script file (JSON array) from disk.
    pub fn from_file(path: &std::path::Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        Ok(MockProvider::new(serde_json::from_str(&raw)?))
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn stream(&self, req: &LlmRequest) -> Result<EventRx> {
        let (tx, rx) = mpsc::channel(64);
        // Compaction requests are answered with a canned summary and must
        // NOT consume a script step.
        let is_compaction = req.system.contains("SUMMARIZE");
        let step = if is_compaction {
            serde_json::json!({})
        } else {
            self.script
                .pop()
                .unwrap_or_else(|| serde_json::json!({"text": "(mock: script exhausted)"}))
        };
        let text = if is_compaction {
            "(compacted) The user asked for a task; work is underway.".to_string()
        } else {
            step["text"].as_str().unwrap_or_default().to_string()
        };
        let thinking = if is_compaction {
            None
        } else {
            step["thinking"].as_str().map(str::to_string)
        };
        let tool_calls: Vec<Value> = if is_compaction {
            Vec::new()
        } else {
            step["tool_calls"].as_array().cloned().unwrap_or_default()
        };
        let input_tokens: u64 = serde_json::to_string(&req.messages).unwrap_or_default().len() as u64 / 4;
        tokio::spawn(async move {
            let _ = tx.send(Ok(StreamEvent::MessageStart)).await;
            let _ = tx.send(Ok(StreamEvent::Usage(super::Usage {
                input_tokens,
                output_tokens: text.len() as u64 / 4,
                ..Default::default()
            }))).await;
            if let Some(think) = thinking {
                for word in think.split(' ') {
                    let _ = tx.send(Ok(StreamEvent::ThinkingDelta(format!("{word} ")))).await;
                }
            }
            if !text.is_empty() {
                for word in text.split(' ') {
                    let _ = tx.send(Ok(StreamEvent::TextDelta(format!("{word} ")))).await;
                }
            }
            for (i, tc) in tool_calls.iter().enumerate() {
                let _ = tx.send(Ok(StreamEvent::ToolUseStart {
                    id: format!("mock_{i}"),
                    name: tc["name"].as_str().unwrap_or_default().to_string(),
                })).await;
                let input = tc["input"].clone();
                let _ = tx.send(Ok(StreamEvent::ToolInputDelta(input.to_string()))).await;
                let _ = tx.send(Ok(StreamEvent::BlockStop)).await;
            }
            let stop = step["stop_reason"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if tool_calls.is_empty() { "end_turn".to_string() } else { "tool_use".to_string() }
                });
            let _ = tx.send(Ok(StreamEvent::MessageDelta { stop_reason: Some(stop) })).await;
            let _ = tx.send(Ok(StreamEvent::MessageStop)).await;
        });
        Ok(rx)
    }
}
