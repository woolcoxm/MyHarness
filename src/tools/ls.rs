//! ls: one-level directory listing, directories first. Cheap orientation
//! for the model between glob probes.

use super::{schema_obj, Tool, ToolCtx, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct LsTool;

const MAX_ENTRIES: usize = 500;

#[async_trait]
impl Tool for LsTool {
    fn name(&self) -> &'static str {
        "ls"
    }

    fn description(&self) -> &'static str {
        "List directory contents."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "path": {"type": "string", "description": "Directory to list (default: working directory)"}
            }),
            &[],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let base = match super::opt_str(&input, "path") {
            Ok(Some(p)) => super::resolve_path(&ctx.cwd, &p),
            _ => ctx.cwd.clone(),
        };
        if !base.is_dir() {
            if base.is_file() {
                return ToolOutput::ok(base.display().to_string());
            }
            return ToolOutput::err(format!("no such directory: {}", base.display()));
        }
        let Ok(entries) = std::fs::read_dir(&base) else {
            return ToolOutput::err(format!("cannot read directory: {}", base.display()));
        };
        let mut dirs: Vec<String> = Vec::new();
        let mut files: Vec<String> = Vec::new();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                dirs.push(format!("{name}/"));
            } else {
                files.push(name);
            }
        }
        dirs.sort();
        files.sort();
        let total = dirs.len() + files.len();
        let mut lines: Vec<String> = dirs.into_iter().chain(files).take(MAX_ENTRIES).collect();
        if total > MAX_ENTRIES {
            lines.push(format!("(+{} more)", total - MAX_ENTRIES));
        }
        if lines.is_empty() {
            return ToolOutput::ok("(empty directory)");
        }
        ToolOutput::ok(format!("{}\n[{} entries in {}]", lines.join("\n"), total, base.display()))
    }
}
