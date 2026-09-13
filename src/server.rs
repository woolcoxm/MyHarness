//! `myharness serve`: a JSON-RPC 2.0 endpoint over stdio (one JSON object
//! per line, both directions) so TUIs, IDEs, and scripts can drive the
//! harness as a thin client — the Codex app-server pattern, line-framed.
//!
//! Methods:
//! - `initialize` → server info
//! - `session.list` → saved sessions (same data as the CLI subcommand)
//! - `turn.run` {input} → runs one turn; `mh/*` notifications stream out
//!   while it runs; the response carries {final_text, interrupted}
//! - `turn.cancel` → interrupts the running turn
//!
//! Requests are handled concurrently (turn.cancel must work during a
//! turn.run), but turns themselves serialize on the agent lock. Permission
//! prompts fail closed — no human is attached to this Ui, so allow rules
//! are the only way a gated tool runs.

use crate::agent::state::AgentState;
use crate::agent::Agent;
use crate::config::Config;
use crate::llm::Provider;
use crate::perms::{PermissionEngine, PermissionMode};
use crate::session::Session;
use crate::tools::Registry;
use crate::ui::Ui;
use anyhow::Result;
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

pub async fn serve(cfg: Config, provider: Arc<dyn Provider>) -> Result<()> {
    let cfg = Arc::new(cfg);
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    // Single writer task: notifications and responses interleave through one
    // channel so stdout lines never tear.
    let writer = tokio::spawn(async move {
        let mut out = tokio::io::BufWriter::new(tokio::io::stdout());
        while let Some(line) = rx.recv().await {
            let _ = out.write_all(line.as_bytes()).await;
            let _ = out.write_all(b"\n").await;
            let _ = out.flush().await;
        }
    });

    let ui_tx = tx.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let agent: Arc<tokio::sync::Mutex<Option<Agent>>> = Arc::new(tokio::sync::Mutex::new(None));

    // Build an agent on demand: a fresh session, or a resume of an existing
    // one (id prefix; empty/omitted = most recent). Returns (agent, resumed
    // message count) so callers can report what they got.
    let build_agent = |resume: Option<&str>| -> Result<(Agent, usize)> {
        let root = std::env::current_dir()?;
        let (state, session, resumed) = match resume {
            Some(prefix) => {
                let path = find_session(&cfg, prefix)?;
                let events = Session::read_events(&path)?;
                let mut state = Session::replay(events, root.clone());
                let mut session = Session::open(&path)?;
                let resumed = state.messages.len();
                crate::session::cwd_note_if_diverged(&mut state, &mut session, &root);
                (state, session, resumed)
            }
            None => (AgentState::new(root.clone()), Session::create(&cfg.sessions_dir(), &cfg.model, &root)?, 0),
        };
        let agent = Agent::new(
            Arc::clone(&provider),
            Arc::clone(&cfg),
            state,
            Ui::channel(ui_tx.clone()),
            Some(session),
            Registry::full(),
            PermissionEngine::new(PermissionMode::AutoEdit, cfg.allow_rules.clone(), cfg.deny_rules.clone(), true),
            cfg.model.clone(),
            Arc::clone(&cancel),
            false,
        );
        Ok((agent, resumed))
    };

    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut line = String::new();
    loop {
        line.clear();
        if stdin.read_line(&mut line).await? == 0 {
            break; // stdin closed: client is done
        }
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(error_response(Value::Null, -32700, &format!("parse error: {e}")));
                continue;
            }
        };
        let id = req.get("id").cloned();
        let Some(method) = req.get("method").and_then(|m| m.as_str()).map(str::to_string) else {
            continue; // a response or malformed frame; nothing to answer
        };
        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

        match method.as_str() {
            "initialize" => {
                let _ = tx.send(ok_response(
                    id.unwrap_or(Value::Null),
                    json!({"name": "myharness", "version": env!("CARGO_PKG_VERSION")}),
                ));
            }
            "session.list" => {
                let entries: Vec<Value> = Session::list(&cfg.sessions_dir())
                    .into_iter()
                    .map(|(id, mtime, model, first, count)| {
                        let modified = mtime
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        json!({"id": id, "modified_unix": modified, "model": model, "first_message": first, "messages": count})
                    })
                    .collect();
                let _ = tx.send(ok_response(id.unwrap_or(Value::Null), json!(entries)));
            }
            // Attach to an existing session (id prefix; omitted = latest) or
            // start a fresh one explicitly. Without either, the first
            // turn.run creates a fresh session implicitly.
            "session.attach" | "session.new" => {
                let rid = id.unwrap_or(Value::Null);
                let resume: Option<String> = if method == "session.attach" {
                    Some(params.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string())
                } else {
                    None
                };
                // Detach any current agent, build the requested one, and
                // store it — all under the lock so a concurrent turn.run
                // task can't slip a default session in between.
                let mut guard = agent.lock().await;
                if let Some(a) = guard.as_mut() {
                    a.ui = Ui::quiet();
                }
                *guard = None;
                match build_agent(resume.as_deref()) {
                    Ok((a, resumed)) => {
                        let session_id = a.session.as_ref().map(|s| s.id.clone()).unwrap_or_default();
                        let messages = a.state.messages.len();
                        *guard = Some(a);
                        let _ = tx.send(ok_response(
                            rid,
                            json!({"session_id": session_id, "resumed_messages": resumed, "messages": messages}),
                        ));
                    }
                    Err(e) => {
                        let _ = tx.send(error_response(rid, -32002, &format!("attach failed: {e}")));
                    }
                }
            }
            "turn.cancel" => {
                cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = tx.send(ok_response(id.unwrap_or(Value::Null), json!({"cancelling": true})));
            }
            "turn.run" => {
                let input = params.get("input").and_then(|i| i.as_str()).unwrap_or_default().to_string();
                if input.trim().is_empty() {
                    let _ = tx.send(error_response(id.unwrap_or(Value::Null), -32602, "turn.run requires a non-empty string 'input'"));
                    continue;
                }
                let tx = tx.clone();
                let agent = Arc::clone(&agent);
                let cancel = Arc::clone(&cancel);
                let cfg = Arc::clone(&cfg);
                let provider = Arc::clone(&provider);
                let rid = id.unwrap_or(Value::Null);
                // Handle on a task so turn.cancel arrives while the turn runs.
                tokio::spawn(async move {
                    // Lazily construct the fresh default session and run the
                    // turn under ONE lock hold — releasing between ensure and
                    // run would let shutdown swap the UI to quiet mid-flight
                    // (observed: deltas silently vanished).
                    let mut guard = agent.lock().await;
                    if guard.is_none() {
                        match build_default(&cfg, &provider, &cancel, &tx).await {
                            Ok(a) => *guard = Some(a),
                            Err(e) => {
                                let _ = tx.send(error_response(rid, -32603, &format!("session init failed: {e}")));
                                return;
                            }
                        }
                    }
                    let Some(a) = guard.as_mut() else {
                        let _ = tx.send(error_response(rid, -32603, "session was detached mid-turn (session.attach raced); retry turn.run"));
                        return;
                    };
                    let session_id = a.session.as_ref().map(|s| s.id.clone()).unwrap_or_default();
                    let outcome = a.run_turn(&input).await;
                    drop(guard);
                    cancel.store(false, std::sync::atomic::Ordering::SeqCst);
                    match outcome {
                        Ok(out) => {
                            let _ = tx.send(ok_response(
                                rid,
                                json!({"final_text": out.final_text, "interrupted": out.interrupted, "session_id": session_id}),
                            ));
                        }
                        Err(e) => {
                            let _ = tx.send(error_response(rid, -32603, &format!("turn failed: {e}")));
                        }
                    }
                });
            }
            other => {
                let _ = tx.send(error_response(
                    id.unwrap_or(Value::Null),
                    -32601,
                    &format!("unknown method '{other}' (initialize | session.list | session.attach | session.new | turn.run | turn.cancel)"),
                ));
            }
        }
    }
    // Shutdown: swap out the agent's notification sender (it owns one) and
    // drop the builder closure (its ui_tx capture is another one) so every
    // Sender drops, the writer drains its queue, and serve returns.
    {
        let mut guard = agent.lock().await;
        if let Some(a) = guard.as_mut() {
            a.ui = Ui::quiet();
        }
    }
    drop(agent);
    drop(ui_tx); // a Sender clone on the stack; the writer waits for it
    drop(tx);
    let _ = writer.await;
    Ok(())
}

/// Fresh-session agent for implicit first turns (same shape as the
/// build_agent closure, minus the resume arm).
async fn build_default(
    cfg: &Arc<Config>,
    provider: &Arc<dyn Provider>,
    cancel: &Arc<AtomicBool>,
    tx: &mpsc::UnboundedSender<String>,
) -> Result<Agent> {
    let root = std::env::current_dir()?;
    let state = AgentState::new(root.clone());
    let session = Session::create(&cfg.sessions_dir(), &cfg.model, &root)?;
    Ok(Agent::new(
        Arc::clone(provider),
        Arc::clone(cfg),
        state,
        Ui::channel(tx.clone()),
        Some(session),
        Registry::full(),
        PermissionEngine::new(PermissionMode::AutoEdit, cfg.allow_rules.clone(), cfg.deny_rules.clone(), true),
        cfg.model.clone(),
        Arc::clone(cancel),
        false,
    ))
}

/// Resolve a session id prefix (empty = latest) to its JSONL path.
fn find_session(cfg: &Config, prefix: &str) -> Result<std::path::PathBuf> {
    let dir = cfg.sessions_dir();
    let entries = Session::list(&dir);
    if entries.is_empty() {
        anyhow::bail!("no sessions found in {}", dir.display());
    }
    if prefix.is_empty() {
        let newest = entries.first().unwrap();
        return Ok(dir.join(format!("{}.jsonl", newest.0)));
    }
    entries
        .iter()
        .find(|(sid, ..)| sid.starts_with(prefix))
        .map(|(sid, ..)| dir.join(format!("{sid}.jsonl")))
        .ok_or_else(|| anyhow::anyhow!("no session id matching '{prefix}'"))
}

fn ok_response(id: Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_shapes() {
        let ok: Value = serde_json::from_str(&ok_response(json!(1), json!({"x": true}))).unwrap();
        assert_eq!(ok["id"], 1);
        assert_eq!(ok["result"]["x"], true);
        assert!(ok.get("error").is_none());
        let err = serde_json::from_str::<Value>(&error_response(Value::Null, -32601, "nope")).unwrap();
        assert_eq!(err["error"]["code"], -32601);
        assert_eq!(err["error"]["message"], "nope");
    }
}
