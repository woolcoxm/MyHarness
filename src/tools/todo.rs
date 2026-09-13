//! todo_write: replaces the whole task list. The new list is echoed back in
//! the tool result (in-band, so it never invalidates the prompt-cache
//! prefix), and compaction folds the current list into the handoff summary —
//! multi-step work stays anchored without re-sending it every request.

use super::{Tool, ToolCtx, ToolOutput};
use crate::agent::state::Todo;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct TodoTool;

#[async_trait]
impl Tool for TodoTool {
    fn name(&self) -> &'static str {
        "todo_write"
    }

    fn description(&self) -> &'static str {
        "Replace the full task list. One item in_progress at a time."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {"type": "string"},
                            "status": {"type": "string", "enum": ["pending", "in_progress", "completed"]},
                            "priority": {"type": "string", "enum": ["high", "medium", "low"]}
                        },
                        "required": ["content", "status"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let Some(arr) = input.get("todos").and_then(|v| v.as_array()) else {
            return ToolOutput::err("missing required array parameter 'todos'");
        };
        let mut todos = Vec::new();
        for (i, item) in arr.iter().enumerate() {
            let content = item.get("content").and_then(|v| v.as_str()).unwrap_or_default().trim();
            if content.is_empty() {
                return ToolOutput::err(format!("todos[{i}].content is empty"));
            }
            let status = item.get("status").and_then(|v| v.as_str()).unwrap_or("pending").to_string();
            if !Todo::is_valid_status(&status) {
                return ToolOutput::err(format!(
                    "todos[{i}].status '{status}' invalid (pending | in_progress | completed)"
                ));
            }
            let priority = item
                .get("priority")
                .and_then(|v| v.as_str())
                .unwrap_or("medium")
                .to_string();
            if !Todo::is_valid_priority(&priority) {
                return ToolOutput::err(format!(
                    "todos[{i}].priority '{priority}' invalid (high | medium | low)"
                ));
            }
            todos.push(Todo {
                content: content.to_string(),
                status,
                priority,
            });
        }
        if todos.iter().filter(|t| t.status == "in_progress").count() > 1 {
            return ToolOutput::err("only one todo may be in_progress at a time");
        }
        let rendered = crate::agent::state::AgentState::render_todo_list(&todos);
        ctx.effects.todos = Some(todos);
        ToolOutput::ok(format!("Task list updated.\n\n{rendered}"))
    }
}
