//! skill: load a discovered SKILL.md instruction pack into context. Only the
//! name + description list rides the system prompt; the full body arrives
//! here, on demand, so a skill costs context only when used. Skills are
//! re-discovered from disk on every call — editing a SKILL.md takes effect
//! without restarting the session.

use super::{schema_obj, truncate_middle, Tool, ToolCtx, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct SkillTool;

const MAX_BODY: usize = 16_000;

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &'static str {
        "skill"
    }

    fn description(&self) -> &'static str {
        "Loads a skill (reusable instruction pack) by exact name and returns its full instructions; follow them for the rest of the task. Only names listed under 'Available skills' in the system prompt exist — never guess one. Optional args are appended for skills that take parameters."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "name": {"type": "string", "description": "Exact skill name from the available-skills list"},
                "args": {"type": "string", "description": "Optional task arguments appended to the instructions"}
            }),
            &["name"],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn perm_summary(&self, input: &Value) -> String {
        input
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    async fn execute(&self, input: Value, ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let name = match super::require_str(&input, "name") {
            Ok(n) => n,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let skills = crate::skills::discover(&ctx.cwd);
        let Some(sk) = skills.iter().find(|s| s.name == name) else {
            let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
            return ToolOutput::err(format!(
                "unknown skill '{name}' (available: {names:?}; use one of those exact names)"
            ));
        };
        let mut out = truncate_middle(&sk.body, MAX_BODY, 14_000, 1_000);
        if let Ok(Some(args)) = super::opt_str(&input, "args") {
            if !args.trim().is_empty() {
                out.push_str(&format!("\n\n# Task args\n{}", args.trim()));
            }
        }
        ToolOutput::ok(out)
    }
}
