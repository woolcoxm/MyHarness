//! Hooks: lifecycle commands with the Claude-Code-compatible contract —
//! JSON payload on stdin, exit code 2 blocks (PreToolUse: deny; Stop: force
//! one more round), or a JSON decision object on stdout
//! (`{"decision":"allow"|"deny","reason":"..."}`, `permissionDecision`
//! accepted as an alias).
//!
//! Semantics in this harness:
//! - PreToolUse runs before permission checks. A hook `deny` always denies;
//!   a hook `allow` skips the prompt but not deny rules.
//! - PostToolUse observes results; its verdict is logged only.
//! - Stop runs when the model ends its turn; exit 2 / deny pushes the reason
//!   back to the model and forces continuation (max 3 times per turn).

use crate::config::{Config, HookDef, HookEvent};
use anyhow::Result;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum HookVerdict {
    Proceed,
    Block(String),
}

pub struct HookContext {
    pub session_id: String,
    pub transcript_path: String,
    pub cwd: String,
    pub permission_mode: String,
}

fn matches(h: &HookDef, event: HookEvent, tool: Option<&str>) -> bool {
    h.event == event && h.tool.as_deref().map(|t| t == tool.unwrap_or("")).unwrap_or(true)
}

async fn run_hook(h: &HookDef, payload: &Value, cwd: &str) -> Result<(i32, String, String)> {
    let mut cmd = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        // /D (skip autorun) must precede /C on current Windows builds.
        c.args(["/D", "/C"]).arg(&h.command);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(&h.command);
        c
    };
    cmd.current_dir(cwd)
        .env("MYHARNESS_HOOK", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let timeout = Duration::from_millis(h.timeout_ms.unwrap_or(10_000).clamp(1_000, 60_000));
    let mut child = cmd.spawn()?;
    use tokio::io::AsyncWriteExt;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.to_string().as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => Ok((
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        )),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => Err(anyhow::anyhow!("hook timed out after {}ms", timeout.as_millis())),
    }
}

/// Interpret one hook result: exit 2 or an explicit deny/block decision.
fn interpret(code: i32, stdout: &str, stderr: &str) -> HookVerdict {
    if code == 2 {
        let reason = if stderr.is_empty() { "blocked by hook (exit 2)" } else { stderr };
        return HookVerdict::Block(reason.to_string());
    }
    if let Ok(v) = serde_json::from_str::<Value>(stdout) {
        let decision = v
            .get("decision")
            .or_else(|| v.get("permissionDecision"))
            .and_then(|d| d.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let reason = v
            .get("reason")
            .or_else(|| v.get("permissionDecisionReason"))
            .and_then(|r| r.as_str())
            .unwrap_or("blocked by hook")
            .to_string();
        if decision == "deny" || decision == "block" {
            return HookVerdict::Block(reason);
        }
    }
    HookVerdict::Proceed
}

async fn run_matching(
    cfg: &Config,
    event: HookEvent,
    tool: Option<&str>,
    payload: &Value,
    cwd: &str,
) -> HookVerdict {
    for h in cfg.hooks.iter().filter(|h| matches(h, event, tool)) {
        match run_hook(h, payload, cwd).await {
            Ok((code, stdout, stderr)) => {
                if let HookVerdict::Block(reason) = interpret(code, &stdout, &stderr) {
                    return HookVerdict::Block(reason);
                }
            }
            Err(e) => {
                // A broken hook must not take the agent down; it also must
                // not silently allow — treat as block with the error.
                return HookVerdict::Block(format!("hook failed: {e}"));
            }
        }
    }
    HookVerdict::Proceed
}

pub async fn pre_tool_use(
    cfg: &Config,
    ctx: &HookContext,
    tool: &str,
    input: &Value,
) -> HookVerdict {
    let payload = json!({
        "hook": "PreToolUse",
        "session_id": ctx.session_id,
        "transcript_path": ctx.transcript_path,
        "cwd": ctx.cwd,
        "permission_mode": ctx.permission_mode,
        "tool_name": tool,
        "tool_input": input,
    });
    run_matching(cfg, HookEvent::PreToolUse, Some(tool), &payload, &ctx.cwd).await
}

pub async fn post_tool_use(
    cfg: &Config,
    ctx: &HookContext,
    tool: &str,
    input: &Value,
    content: &str,
    is_error: bool,
) {
    let payload = json!({
        "hook": "PostToolUse",
        "session_id": ctx.session_id,
        "transcript_path": ctx.transcript_path,
        "cwd": ctx.cwd,
        "permission_mode": ctx.permission_mode,
        "tool_name": tool,
        "tool_input": input,
        "tool_response": {"content": content, "is_error": is_error},
    });
    let _ = run_matching(cfg, HookEvent::PostToolUse, Some(tool), &payload, &ctx.cwd).await;
}

/// Returns Block(reason) when a Stop hook wants the model to continue.
pub async fn stop(cfg: &Config, ctx: &HookContext) -> HookVerdict {
    let payload = json!({
        "hook": "Stop",
        "session_id": ctx.session_id,
        "transcript_path": ctx.transcript_path,
        "cwd": ctx.cwd,
        "permission_mode": ctx.permission_mode,
    });
    run_matching(cfg, HookEvent::Stop, None, &payload, &ctx.cwd).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpret_exit_2_and_json_decisions() {
        assert!(matches!(interpret(2, "", "no rm -rf"), HookVerdict::Block(r) if r == "no rm -rf"));
        assert!(matches!(
            interpret(
                0,
                r#"{"decision":"deny","reason":"not on Fridays"}"#,
                ""
            ),
            HookVerdict::Block(r) if r == "not on Fridays"
        ));
        // Claude-style key alias.
        assert!(matches!(
            interpret(0, r#"{"permissionDecision":"deny"}"#, ""),
            HookVerdict::Block(_)
        ));
        assert!(matches!(interpret(0, r#"{"decision":"allow"}"#, ""), HookVerdict::Proceed));
        assert!(matches!(interpret(0, "plain stdout", ""), HookVerdict::Proceed));
    }
}
