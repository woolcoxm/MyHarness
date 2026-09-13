//! edit_file: exact-string replacement. Uniqueness of old_string is enforced
//! so an edit can never silently land in the wrong place — the failure modes
//! (0 matches / N matches) come back with actionable guidance instead.

use super::{canon, display_path, resolve_path, require_str, schema_obj, Tool, ToolCtx, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn description(&self) -> &'static str {
        "Exact-string replacement. old_string must match exactly and be unique (or replace_all). File must have been read."
        }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "path": {"type": "string", "description": "File path to edit"},
                "old_string": {"type": "string", "description": "Exact text to replace (must be unique unless replace_all is true)"},
                "new_string": {"type": "string", "description": "Replacement text"},
                "replace_all": {"type": "boolean", "description": "Replace every occurrence (default false)"}
            }),
            &["path", "old_string", "new_string"],
        )
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn perm_summary(&self, input: &Value) -> String {
        input.get("path").and_then(|v| v.as_str()).unwrap_or("?").to_string()
    }

    async fn execute(&self, input: Value, ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let path = match require_str(&input, "path") {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let old_string = match require_str(&input, "old_string") {
            Ok(s) => s,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let new_string = input
            .get("new_string")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let replace_all = super::opt_bool(&input, "replace_all").unwrap_or(None).unwrap_or(false);

        if old_string == new_string {
            return ToolOutput::err("old_string and new_string are identical; nothing to do");
        }
        let resolved = resolve_path(&ctx.cwd, &path);
        let key = canon(&ctx.cwd, &path);
        if !ctx.files_read.contains(&key) {
            return ToolOutput::err(format!(
                "{} has not been read this session; read it with read_file before editing",
                display_path(&ctx.cwd, &resolved)
            ));
        }
        if let Err(msg) = super::stale_check(ctx, &key) {
            return ToolOutput::err(msg);
        }
        let content = match std::fs::read_to_string(&resolved) {
            Ok(c) => c,
            Err(e) => return ToolOutput::err(format!("cannot read {}: {e}", resolved.display())),
        };
        let count = content.match_indices(&old_string).count();
        if count == 0 {
            return ToolOutput::err(format!(
                "old_string not found in {}. Check exact whitespace/indentation, or re-read the file — it may have changed.",
                display_path(&ctx.cwd, &resolved)
            ));
        }
        if count > 1 && !replace_all {
            return ToolOutput::err(format!(
                "old_string occurs {count} times in {}; add surrounding context lines to make it unique, or set replace_all=true",
                display_path(&ctx.cwd, &resolved)
            ));
        }
        let updated = if replace_all {
            content.replace(&old_string, &new_string)
        } else {
            content.replacen(&old_string, &new_string, 1)
        };
        // Journal the pre-edit state so /undo can restore it.
        ctx.effects.journal.push(super::snapshot_for_journal(ctx, &resolved));
        if let Err(e) = std::fs::write(&resolved, updated.as_bytes()) {
            return ToolOutput::err(format!("write failed: {e}"));
        }
        if let Some((m, l)) = super::stat_file(&resolved) {
            ctx.effects.file_stats.push((key, m, l));
        }
        ToolOutput::ok(format!(
            "Replaced {} occurrence(s) in {} (now {} lines)",
            if replace_all { count } else { 1 },
            display_path(&ctx.cwd, &resolved),
            updated.lines().count()
        ))
    }
}
