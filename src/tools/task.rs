//! task: spawn an isolated subagent with its own context window and a
//! restricted tool set. Only the final report comes back to the parent —
//! the point is to keep exploration noise out of the main thread.

use super::{schema_obj, Tool, ToolCtx, ToolOutput};
use crate::agent::Agent;
use crate::agent::state::{AgentState, BgTask};
use crate::perms::{PermissionEngine, PermissionMode};
use crate::ui::Ui;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub struct TaskTool;

const DEFAULT_TOOLS: &[&str] = &["read_file", "ls", "glob", "grep", "web_fetch"];

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &'static str {
        "task"
    }

    fn description(&self) -> &'static str {
        "Spawns a subagent that runs a self-contained job with a fresh context and returns only its final report. agent_type picks a preset: \"explore\" (default, read-only: read_file, ls, glob, grep, web_fetch) for research, \"build\" (explore + bash + bash_output) for jobs that must run builds/tests. Set run_in_background=true to start it without waiting and keep working — the final report arrives through bash_output like any background task. Use for broad exploration or parallelizable research so the results land in your context as a digest instead of dozens of raw tool results. The subagent cannot ask questions or spawn further subagents."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "description": {"type": "string", "description": "3-5 word summary of the job"},
                "prompt": {"type": "string", "description": "Complete, self-contained instructions for the subagent"},
                "agent_type": {"type": "string", "enum": ["explore", "build"], "description": "Tool preset: explore (read-only search, default) or build (adds bash + bash_output for running commands)"},
                "tools": {"type": "array", "items": {"type": "string"}, "description": "Extra tool names to enable beyond the preset"},
                "max_turns": {"type": "integer", "description": "Hard turn cap (default 16)"},
                "run_in_background": {"type": "boolean", "description": "Start the subagent and return immediately; poll its final report with bash_output (default false)"}
            }),
            &["description", "prompt"],
        )
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn concurrency_safe(&self) -> bool {
        true
    }

    fn perm_summary(&self, input: &Value) -> String {
        input
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("subagent")
            .to_string()
    }

    async fn execute(&self, input: Value, ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let description = match super::require_str(&input, "description") {
            Ok(d) => d,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let prompt = match super::require_str(&input, "prompt") {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let agent_type = super::opt_str(&input, "agent_type")
            .unwrap_or(None)
            .unwrap_or_else(|| "explore".to_string());
        let mut tool_names: Vec<String> = DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect();
        match agent_type.as_str() {
            "explore" => {}
            "build" => {
                for t in ["bash", "bash_output"] {
                    if !tool_names.iter().any(|n| n == t) {
                        tool_names.push(t.to_string());
                    }
                }
            }
            other => {
                return ToolOutput::err(format!(
                    "invalid agent_type '{other}' (use \"explore\" or \"build\", or omit for explore)"
                ));
            }
        }
        if let Some(extra) = input.get("tools").and_then(|v| v.as_array()) {
            for t in extra.iter().filter_map(|v| v.as_str()) {
                if !tool_names.iter().any(|n| n == t) {
                    tool_names.push(t.to_string());
                }
            }
        }
        let max_turns = super::opt_u64(&input, "max_turns")
            .unwrap_or(None)
            .unwrap_or(16)
            .clamp(2, 40) as u32;
        let run_in_background = super::opt_bool(&input, "run_in_background")
            .unwrap_or(None)
            .unwrap_or(false);
        // The subagent's system prompt states the (stable) workspace root;
        // its actual starting cwd rides the task prompt instead.
        let prompt = format!("(cwd: {})\n\n{prompt}", ctx.cwd.display());

        let registry = crate::tools::Registry::subset(&tool_names);
        if registry.names().is_empty() {
            return ToolOutput::err(format!("no valid tools in {tool_names:?}"));
        }
        let mut state = AgentState::new(ctx.workspace_root.clone());
        state.cwd.clone_from(&ctx.cwd);

        // Background mode: register a BgTask, run the subagent in a spawned
        // future, and return immediately — bash_output delivers the report.
        if run_in_background {
            let id = ctx.next_bg_id;
            let bg = BgTask {
                pid: None,
                command: format!("subagent: {description}"),
                started: chrono::Local::now(),
                output: Arc::new(Mutex::new(String::new())),
                done: Arc::new(AtomicBool::new(false)),
                exit: Arc::new(Mutex::new(None)),
            };
            ctx.effects.background = Some((id, bg.clone()));
            ctx.effects.bg_counter = Some(id);
            let provider = Arc::clone(&ctx.provider);
            let cfg = Arc::clone(&ctx.cfg);
            let model = ctx.cfg.model.clone();
            let cancel = Arc::clone(&ctx.cancel);
            let cwd = ctx.cwd.clone();
            let workspace_root = ctx.workspace_root.clone();
            let owned_tools = tool_names;
            tokio::spawn(async move {
                let registry = crate::tools::Registry::subset(&owned_tools);
                let mut state = AgentState::new(workspace_root);
                state.cwd = cwd;
                let mut sub = Agent::new(
                    provider,
                    cfg,
                    state,
                    Ui::quiet(),
                    None,
                    registry,
                    PermissionEngine::new(PermissionMode::Yolo, vec![], vec![], true),
                    model,
                    cancel,
                    true,
                );
                sub.set_turn_cap(max_turns);
                let (report, code) = match sub.run_turn(&prompt).await {
                    Ok(out) if out.interrupted => {
                        ("(subagent was interrupted before finishing)".to_string(), 1)
                    }
                    Ok(out) if out.final_text.trim().is_empty() => {
                        ("(subagent returned an empty report)".to_string(), 1)
                    }
                    Ok(out) => (out.final_text.trim().to_string(), 0),
                    Err(e) => (format!("subagent failed: {e}"), 1),
                };
                *bg.exit.lock().unwrap() = Some(code);
                *bg.output.lock().unwrap() = report;
                bg.done.store(true, Ordering::Relaxed);
            });
            return ToolOutput::ok(format!(
                "Background task #{id} started (subagent: {description}). Keep working; \
                 poll with bash_output ({{\"id\": {id}}}) for the final report."
            ));
        }

        let mut sub = Agent::new(
            Arc::clone(&ctx.provider),
            Arc::clone(&ctx.cfg),
            state,
            Ui::quiet(),
            None,
            registry,
            PermissionEngine::new(PermissionMode::Yolo, vec![], vec![], true),
            ctx.cfg.model.clone(),
            Arc::clone(&ctx.cancel),
            true,
        );
        sub.set_turn_cap(max_turns);
        match sub.run_turn(&prompt).await {
            Ok(out) => {
                if out.interrupted {
                    ToolOutput::err("subagent was interrupted before finishing")
                } else {
                    let report = out.final_text.trim();
                    if report.is_empty() {
                        ToolOutput::err("subagent returned an empty report")
                    } else {
                        ToolOutput::ok(format!("Subagent report ({description}):\n\n{report}"))
                    }
                }
            }
            Err(e) => ToolOutput::err(format!("subagent failed: {e}")),
        }
    }
}
