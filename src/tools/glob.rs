//! glob: fast filename matching. Cheap enough that the model can probe the
//! tree repeatedly instead of reading files blind.

use super::{schema_obj, Tool, ToolCtx, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct GlobTool;

const MAX_RESULTS: usize = 200;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &'static str {
        "glob"
    }

    fn description(&self) -> &'static str {
        "Returns file/directory paths matching a glob pattern (e.g. `src/**/*.rs`, `**/*.toml`). Sorted, capped at 200 matches. Use this to find files by name before reading."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "pattern": {"type": "string", "description": "Glob pattern (**, *, ? supported)"},
                "path": {"type": "string", "description": "Directory to search in (default: working directory)"}
            }),
            &["pattern"],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let pattern = match super::require_str(&input, "pattern") {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let base = match super::opt_str(&input, "path") {
            Ok(Some(p)) => super::resolve_path(&ctx.cwd, &p),
            _ => ctx.cwd.clone(),
        };
        if !base.is_dir() {
            return ToolOutput::err(format!("no such directory: {}", base.display()));
        }

        let full_pattern = {
            let abs = super::resolve_path(&ctx.cwd, &pattern);
            if abs.is_absolute() || pattern.contains(':') {
                abs.display().to_string().replace('\\', "/")
            } else {
                format!("{}/{}", base.display().to_string().replace('\\', "/"), pattern)
            }
        };

        let cwd_disp = ctx.cwd.clone();
        let handle = tokio::task::spawn_blocking(move || {
            let options = glob::MatchOptions {
                case_sensitive: true,
                require_literal_separator: false,
                require_literal_leading_dot: false,
            };
            let iter = match glob::glob_with(&full_pattern, options) {
                Ok(i) => i,
                Err(e) => return vec![format!("\x00INVALID_PATTERN:{e}")],
            };
            let mut hits: Vec<String> = Vec::new();
            for entry in iter {
                if let Ok(p) = entry {
                    hits.push(p.display().to_string().replace('\\', "/"));
                }
                if hits.len() > 5000 {
                    break;
                }
            }
            hits
        })
        .await
        .unwrap_or_default();

        if handle.len() == 1 && handle[0].starts_with("\x00INVALID_PATTERN:") {
            return ToolOutput::err(handle[0].trim_start_matches('\x00').replace("INVALID_PATTERN:", "invalid pattern: "));
        }
        let mut paths: Vec<std::path::PathBuf> = handle.into_iter().map(std::path::PathBuf::from).collect();
        paths.sort();
        paths.dedup();
        let total = paths.len();
        if total == 0 {
            return ToolOutput::ok("No matches.");
        }
        let mut out = paths
            .iter()
            .take(MAX_RESULTS)
            .map(|p| super::display_path(&cwd_disp, p))
            .collect::<Vec<_>>()
            .join("\n");
        if total > MAX_RESULTS {
            out.push_str(&format!("\n(+{} more matches)", total - MAX_RESULTS));
        }
        ToolOutput::ok(out)
    }
}
