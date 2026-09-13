//! Context compaction: when the estimated request size passes the configured
//! share of the context window, older turns are summarized into a handoff
//! brief and only a recent tail is kept verbatim.

use crate::agent::state::AgentState;
use crate::agent::tokens::{estimate_messages, render_transcript};
use crate::llm::{ContentBlock, LlmRequest, Message, Provider};
use anyhow::Result;
use std::sync::Arc;

pub const COMPACTION_PROMPT: &str = "SUMMARIZE — you are compacting a conversation between a developer and a coding agent. Produce a handoff brief a fresh agent could continue from exactly where this one stopped. Include: (1) the user's goal and constraints, (2) decisions made and why, (3) files created/modified with paths, (4) the current plan / next steps, (5) any verbatim snippets, commands or error messages that must survive word-for-word. Be dense; no preamble.";

/// Keep this many trailing messages after compaction. Boundaries are adjusted
/// so a tool_result message is never separated from its tool_use.
const TAIL: usize = 6;

fn is_tool_result_only(m: &Message) -> bool {
    !m.content.is_empty()
        && m.content.iter().all(|b| matches!(b, ContentBlock::ToolResult { .. }))
}

pub fn tail_start(messages: &[Message]) -> usize {
    let mut start = messages.len().saturating_sub(TAIL);
    while start > 0 && is_tool_result_only(&messages[start]) {
        start -= 1;
    }
    start
}

/// Returns (summary, kept messages). Falls back to mechanical truncation if
/// the summarization call fails.
pub async fn compact(
    provider: &Arc<dyn Provider>,
    model: &str,
    state: &AgentState,
) -> Result<(String, Vec<Message>)> {
    let transcript = render_transcript(&state.messages);
    let req = LlmRequest {
        model: model.to_string(),
        system: COMPACTION_PROMPT.to_string(),
        messages: vec![Message::user_text(transcript)],
        tools: Vec::new(),
        max_tokens: 2048,
        temperature: 0.1,
    };
    let summary = match collect_text(provider, &req).await {
        Ok(s) if !s.trim().is_empty() => s,
        Ok(_) => "(empty summary)".to_string(),
        Err(e) => format!("(summarization failed: {e}; older context dropped mechanically)"),
    };
    let start = tail_start(&state.messages);
    let kept: Vec<Message> = state.messages[start..].to_vec();
    Ok((summary, kept))
}

pub async fn collect_text(provider: &Arc<dyn Provider>, req: &LlmRequest) -> Result<String> {
    let mut rx = provider.stream(req).await?;
    let mut text = String::new();
    while let Some(ev) = rx.recv().await {
        match ev? {
            crate::llm::StreamEvent::TextDelta(s) => text.push_str(&s),
            crate::llm::StreamEvent::MessageStop => break,
            _ => {}
        }
    }
    Ok(text)
}

pub fn should_compact(state: &AgentState, system: &str, ratio: f32, window: u64) -> bool {
    estimate_messages(system, &state.messages) > (ratio as f64 * window as f64) as u64
}

/// Current estimated context size (system + messages), shared by the
/// compaction threshold and the TUI context bar.
pub fn should_compact_size(state: &AgentState, system: &str) -> u64 {
    estimate_messages(system, &state.messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ContentBlock, Role};

    fn tr(id: &str) -> Message {
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: id.to_string(),
                content: "r".to_string(),
                images: Vec::new(),
                is_error: false,
            }],
        }
    }

    #[test]
    fn tail_never_splits_tool_pair() {
        // [user, assistant(tool), tool_result, user, assistant, user]
        let messages = vec![
            Message::user_text("a"),
            Message { role: Role::Assistant, content: vec![ContentBlock::ToolUse { id: "1".into(), name: "t".into(), input: serde_json::json!({}) }] },
            tr("1"),
            Message::user_text("b"),
            Message { role: Role::Assistant, content: vec![ContentBlock::Text { text: "c".into() }] },
            Message::user_text("d"),
        ];
        let start = tail_start(&messages);
        // The tail must not begin at the bare tool_result message.
        assert_ne!(start, 2);
        assert!(start <= 2);
        assert!(!matches!(messages[start].content.first(), Some(ContentBlock::ToolResult { .. })));
    }
}
