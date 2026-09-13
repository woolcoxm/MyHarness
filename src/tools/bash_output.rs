//! bash_output: poll a background command started with
//! `bash(run_in_background=true)` — running state, exit code, and the tail
//! of its captured output.

use super::{budget_output, opt_u64, schema_obj, Tool, ToolCtx, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct BashOutputTool;

const MAX_REPORT: usize = 20_000;

#[async_trait]
impl Tool for BashOutputTool {
    fn name(&self) -> &'static str {
        "bash_output"
    }

    fn description(&self) -> &'static str {
        "Returns the status and output of a background task — a bash command started with run_in_background=true, or a subagent started with task run_in_background=true (its final report arrives here). If the task is still running you get the output so far — keep working and poll again later. Huge output is head+tail truncated with the full text saved to a file you can read_file for the middle. For bash tasks the pid is included so you can kill a stuck one via bash (taskkill /PID <pid> /T /F on Windows, kill on Unix)."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "id": {"type": "integer", "description": "Background task id returned by bash"}
            }),
            &["id"],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let Some(id) = opt_u64(&input, "id").unwrap_or(None) else {
            return ToolOutput::err("missing required integer parameter 'id'");
        };
        let Some(task) = ctx.background.get(&(id as u32)) else {
            let known: Vec<u32> = ctx.background.keys().copied().collect();
            return ToolOutput::err(format!(
                "no background task #{id} (running tasks: {known:?})"
            ));
        };
        let running = !task.done.load(std::sync::atomic::Ordering::Relaxed);
        let exit = *task.exit.lock().unwrap();
        let output = task.output.lock().unwrap().clone();

        let state = match (running, exit) {
            (true, _) => "still running".to_string(),
            (false, Some(code)) => format!("finished, exit code {code}"),
            (false, None) => "finished (exit code unknown)".to_string(),
        };
        let report = if output.trim().is_empty() {
            format!(
                "Task #{} ({}): {state}, pid {}, started {}.\n(no output yet)",
                id,
                task.command.chars().take(120).collect::<String>(),
                task.pid.map(|p| p.to_string()).unwrap_or_else(|| "?".into()),
                task.started.format("%H:%M:%S")
            )
        } else {
            format!(
                "Task #{} ({}): {state}, pid {}, started {}.\n\n--- output ---\n{}",
                id,
                task.command.chars().take(120).collect::<String>(),
                task.pid.map(|p| p.to_string()).unwrap_or_else(|| "?".into()),
                task.started.format("%H:%M:%S"),
                budget_output(ctx, "bash_output", &output, MAX_REPORT, 4_000, 14_000).trim_end()
            )
        };
        if running {
            ToolOutput::ok(format!("{report}\n\n(still running — poll again later)"))
        } else {
            ToolOutput::ok(report)
        }
    }
}
