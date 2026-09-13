//! Token estimation. Deliberately heuristic (ASCII ~= 0.3 tokens/char, other
//! scripts ~= 0.8): it only guards the compaction threshold, and the real
//! usage numbers reported by the API are displayed separately.

use crate::llm::{ContentBlock, Message, Role};

pub fn estimate_str(s: &str) -> u64 {
    let mut tokens = 0.0f64;
    for c in s.chars() {
        tokens += if c.is_ascii() { 0.3 } else { 0.8 };
    }
    tokens as u64
}

pub fn estimate_messages(system: &str, messages: &[Message]) -> u64 {
    let mut total = estimate_str(system) + 800; // tool schemas + framing overhead
    for m in messages {
        total += 32;
        for b in &m.content {
            match b {
                ContentBlock::Text { text } => total += estimate_str(text),
                ContentBlock::ToolUse { name, input, .. } => {
                    total += estimate_str(name) + estimate_str(&input.to_string());
                }
                ContentBlock::ToolResult { content, images, .. } => {
                    total += estimate_str(content);
                    // Images: ~1 token per 1400 bytes of raw data.
                    for img in images {
                        total += (img.data.len() as f64 * 0.75 / 1400.0) as u64;
                    }
                }
            }
        }
    }
    total
}

/// Render a transcript for the compaction summarizer.
pub fn render_transcript(messages: &[Message]) -> String {
    let mut s = String::new();
    for m in messages {
        let role = if m.role == Role::Assistant { "assistant" } else { "user" };
        for b in &m.content {
            match b {
                ContentBlock::Text { text } => {
                    s.push_str(&format!("[{role}] {}\n", truncate(text, 4000)));
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    s.push_str(&format!("[{role} tool_call {name}] {}\n", truncate(&input.to_string(), 600)));
                }
                ContentBlock::ToolResult { tool_use_id, content, images, is_error } => {
                    let tag = if *is_error { "tool_error" } else { "tool_result" };
                    let note = if images.is_empty() {
                        String::new()
                    } else {
                        format!(" (+{} image{})", images.len(), if images.len() > 1 { "s" } else { "" })
                    };
                    s.push_str(&format!(
                        "[{tag} for {tool_use_id}]{note} {}\n",
                        truncate(content, 800)
                    ));
                }
            }
        }
    }
    s
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max / 2).collect();
    let tail: String = s.chars().skip(s.chars().count() - max / 4).collect();
    format!("{head}\n...[truncated]...\n{tail}")
}
