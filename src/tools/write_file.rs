//! write_file: create or overwrite. Overwriting an existing file requires
//! that the file was read this session — blind overwrites are the single
//! most common way agents destroy work.

use super::{canon, display_path, resolve_path, require_str, schema_obj, Tool, ToolCtx, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "Creates a file (parent directories are created automatically) or overwrites an existing one. An existing file can only be overwritten after it was read with read_file in this session."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "path": {"type": "string", "description": "File path to write"},
                "content": {"type": "string", "description": "Full file contents to write"}
            }),
            &["path", "content"],
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
        let content = match require_str(&input, "content") {
            Ok(c) => c,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let resolved = resolve_path(&ctx.cwd, &path);
        let key = canon(&ctx.cwd, &path);
        if resolved.exists() && !ctx.files_read.contains(&key) {
            return ToolOutput::err(format!(
                "{} already exists but was not read in this session; read it with read_file before overwriting (this guard prevents blind clobbers)",
                display_path(&ctx.cwd, &resolved)
            ));
        }
        if let Some(parent) = resolved.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return ToolOutput::err(format!("cannot create directory {}: {e}", parent.display()));
            }
        }
        // Journal the pre-write state so /undo can restore it.
        ctx.effects.journal.push(super::snapshot_for_journal(ctx, &resolved));
        let lines = content.lines().count();
        if let Err(e) = std::fs::write(&resolved, content.as_bytes()) {
            return ToolOutput::err(format!("write failed: {e}"));
        }
        ctx.effects.files_read.push(key);
        ToolOutput::ok(format!(
            "Wrote {} lines to {}",
            lines,
            display_path(&ctx.cwd, &resolved)
        ))
    }
}
