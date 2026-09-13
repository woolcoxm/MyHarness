//! session_recall: search the user's past session transcripts (Goose's
//! chat-history recall). The session archive is the one dataset every
//! other harness leaves on the table — "how did we fix this last month"
//! becomes one tool call instead of re-deriving the answer.

use super::{require_str, schema_obj, Tool, ToolCtx, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct SessionRecallTool;

const MAX_RESULTS: usize = 8;
const SNIPPET: usize = 240;

#[async_trait]
impl Tool for SessionRecallTool {
    fn name(&self) -> &'static str {
        "session_recall"
    }

    fn description(&self) -> &'static str {
        "Searches your past myharness session transcripts for messages containing the query (case-insensitive substring) and returns up to 8 matches, newest first, each with the session id, role, date, and a snippet around the match. Use it to recall how a problem was solved before, decisions from earlier sessions, or prior context on a file. Current-session history is searched too. For fresh research prefer grep/read_file."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "query": {"type": "string", "description": "Substring to find in past session messages"}
            }),
            &["query"],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn concurrency_safe(&self) -> bool {
        true
    }

    fn perm_summary(&self, input: &Value) -> String {
        input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .chars()
            .take(60)
            .collect()
    }

    async fn execute(&self, input: Value, _ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let query = match require_str(&input, "query") {
            Ok(q) => q,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let needle = query.to_lowercase();
        let dir = _ctx.cfg.sessions_dir();
        let entries = crate::session::Session::list(&dir);
        if entries.is_empty() {
            return ToolOutput::err(format!(
                "no sessions in {} — nothing to recall yet",
                dir.display()
            ));
        }

        struct Hit {
            session: String,
            when: String,
            role: &'static str,
            snippet: String,
        }
        let mut hits: Vec<Hit> = Vec::new();
        // list() is newest-first, so the first hits are the freshest.
        'outer: for (id, mtime, _model, _first, _count) in &entries {
            let path = dir.join(format!("{id}.jsonl"));
            let Ok(events) = crate::session::Session::read_events(&path) else {
                continue;
            };
            let when = chrono::DateTime::<chrono::Local>::from(*mtime)
                .format("%Y-%m-%d %H:%M")
                .to_string();
            for ev in events {
                let crate::session::Event::Message(m) = ev else { continue };
                let text = m.text();
                if text.is_empty() {
                    continue;
                }
                if let Some(pos) = text.to_lowercase().find(&needle) {
                    let start = pos.saturating_sub(SNIPPET / 3);
                    let snippet: String = text
                        .chars()
                        .skip(start)
                        .take(SNIPPET)
                        .collect();
                    let role = match m.role {
                        crate::llm::Role::User => "user",
                        crate::llm::Role::Assistant => "assistant",
                    };
                    hits.push(Hit { session: id.clone(), when: when.clone(), role, snippet });
                    if hits.len() >= MAX_RESULTS {
                        break 'outer;
                    }
                }
            }
        }

        if hits.is_empty() {
            return ToolOutput::ok(format!(
                "no past session messages contain '{query}' (searched {} session(s))",
                entries.len()
            ));
        }
        let mut out = format!(
            "{} match(es) for '{query}' (newest first):\n\n",
            hits.len()
        );
        for (i, h) in hits.iter().enumerate() {
            out.push_str(&format!(
                "{}. [{}] {} session {}:\n   ...{}...\n\n",
                i + 1,
                h.when,
                h.role,
                h.session,
                h.snippet.replace('\n', " ")
            ));
        }
        out.push_str("Resume one with: myharness resume <session-id-prefix>");
        ToolOutput::ok(out)
    }
}
