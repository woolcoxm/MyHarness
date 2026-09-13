//! monitor: run a command on a schedule until its output matches (Claude
//! Code's Monitor tool, on our background-task registry). Each check's
//! output is compared against `pattern` (substring) or non-emptiness; on
//! match the task finishes with the output and the model hears about it
//! through the normal background completion notices — poll with
//! bash_output like any background task.

use super::{opt_str, opt_u64, schema_obj, Tool, ToolCtx, ToolOutput};
use crate::agent::state::BgTask;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct MonitorTool;

#[async_trait]
impl Tool for MonitorTool {
    fn name(&self) -> &'static str {
        "monitor"
    }

    fn description(&self) -> &'static str {
        "Run a command every N seconds until output matches. Background task."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "command": {"type": "string", "description": "Command whose output is checked each round"},
                "pattern": {"type": "string", "description": "Substring that finishes the monitor (default: any non-empty output matches)"},
                "interval_s": {"type": "integer", "description": "Seconds between checks (default 10, max 600)"},
                "max_checks": {"type": "integer", "description": "Check budget before giving up (default 30)"}
            }),
            &["command"],
        )
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn perm_summary(&self, input: &Value) -> String {
        input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .chars()
            .take(100)
            .collect()
    }

    async fn execute(&self, input: Value, ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let command = match super::require_str(&input, "command") {
            Ok(c) => c,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let pattern = opt_str(&input, "pattern").unwrap_or(None);
        let interval_s = opt_u64(&input, "interval_s")
            .unwrap_or(None)
            .unwrap_or(10)
            .clamp(1, 600);
        let max_checks = opt_u64(&input, "max_checks")
            .unwrap_or(None)
            .unwrap_or(30)
            .clamp(1, 1000);

        let id = ctx.next_bg_id;
        let task = spawn_monitor(ctx, command.clone(), pattern.clone(), interval_s, max_checks);
        ctx.effects.background = Some((id, task));
        ctx.effects.bg_counter = Some(id);
        let what = pattern
            .as_deref()
            .unwrap_or("non-empty output");
        ToolOutput::ok(format!(
            "Monitor #{id} started: `{command}` every {interval_s}s (up to {max_checks} \
             checks) until it sees {what}. Keep working — you'll be notified when it \
             matches; poll with bash_output ({{\"id\": {id}}}) anytime."
        ))
    }
}

/// The check loop: run, compare, sleep — writes findings into the shared
/// buffer so bash_output can show progress (last few check summaries).
fn spawn_monitor(
    ctx: &ToolCtx<'_>,
    command: String,
    pattern: Option<String>,
    interval_s: u64,
    max_checks: u64,
) -> BgTask {
    let output = Arc::new(Mutex::new(String::new()));
    let done = Arc::new(AtomicBool::new(false));
    let exit = Arc::new(Mutex::new(None::<i32>));
    let shell = ctx.cfg.resolve_shell();
    let cancel = Arc::clone(&ctx.cancel);

    let out_buf = Arc::clone(&output);
    let exit_ref = Arc::clone(&exit);
    let done_ref = Arc::clone(&done);
    let cmd_for_task = command.clone();
    tokio::spawn(async move {
        let mut checks = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            checks += 1;
            let text = run_one(&shell, &command).await;
            let matched = match &pattern {
                Some(p) => text.contains(p.as_str()),
                None => !text.trim().is_empty(),
            };
            let lines: Vec<&str> = text.lines().collect();
            let tail: String = if lines.len() > 3 {
                lines[lines.len() - 3..].join(" / ")
            } else {
                lines.join(" / ")
            }
            .chars()
            .take(300)
            .collect::<String>();
            if matched {
                let mut buf = out_buf.lock().unwrap();
                buf.clear();
                buf.push_str(&format!(
                    "monitor matched after {checks} check(s) (every {interval_s}s):\n\n--- output ---\n{text}"
                ));
                drop(buf);
                *exit_ref.lock().unwrap() = Some(0);
                break;
            }
            {
                let mut buf = out_buf.lock().unwrap();
                // Keep the running log bounded: recent summaries only.
                let lines: Vec<&str> = buf.lines().collect();
                if lines.len() > 20 {
                    let keep: String = lines[lines.len() - 10..].join("\n");
                    buf.clear();
                    buf.push_str("(earlier checks truncated)\n");
                    buf.push_str(&keep);
                    buf.push('\n');
                }
                buf.push_str(&format!("check {checks}/{max_checks}: no match ({tail})\n"));
            }
            if checks >= max_checks {
                let mut buf = out_buf.lock().unwrap();
                buf.push_str(&format!(
                    "\nmonitor gave up after {max_checks} check(s) without a match"
                ));
                drop(buf);
                *exit_ref.lock().unwrap() = Some(1);
                break;
            }
            tokio::time::sleep(Duration::from_secs(interval_s)).await;
        }
        done_ref.store(true, Ordering::Relaxed);
    });

    BgTask {
        pid: None,
        command: format!("monitor: {cmd_for_task}"),
        started: chrono::Local::now(),
        output,
        done,
        exit,
    }
}

async fn run_one(shell: &crate::config::Shell, command: &str) -> String {
    use crate::config::Shell;
    let mut cmd = match shell {
        Shell::Posix(bash) => {
            let mut c = tokio::process::Command::new(bash);
            c.arg("-c").arg(command);
            c
        }
        Shell::PowerShell => {
            let mut c = tokio::process::Command::new("powershell");
            c.args(["-NoProfile", "-NonInteractive", "-Command"]).arg(command);
            c
        }
        Shell::Cmd => {
            let mut c = tokio::process::Command::new("cmd");
            c.args(["/D", "/C"]).arg(command);
            c
        }
    };
    cmd.stdin(std::process::Stdio::null());
    match tokio::time::timeout(Duration::from_secs(60), cmd.output()).await {
        Ok(Ok(o)) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.trim().is_empty() {
                stdout.to_string()
            } else {
                format!("{stdout}\n{stderr}")
            }
        }
        Ok(Err(e)) => format!("(failed to run: {e})"),
        Err(_) => "(check timed out after 60s)".to_string(),
    }
}
