//! grep: regex content search with gitignore-aware walking (via the `ignore`
//! crate, the same engine ripgrep uses for traversal). Runs on the blocking
//! pool so big trees don't stall the async runtime.

use super::{schema_obj, Tool, ToolCtx, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct GrepTool;

const MAX_MATCHES: usize = 200;
const MAX_LINE: usize = 400;
const SKIP_DIRS: &[&str] = &["target", "node_modules", "dist", "build", ".venv", "__pycache__", ".next", "vendor"];

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn description(&self) -> &'static str {
        "Content search (ripgrep engine). output_mode: content | files | count."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "pattern": {"type": "string", "description": "Regular expression to search for"},
                "path": {"type": "string", "description": "File or directory to search (default: working directory)"},
                "glob": {"type": "string", "description": "Glob filter for file names, e.g. \"*.rs\""},
                "ignore_case": {"type": "boolean", "description": "Case-insensitive matching (default false)"},
                "output_mode": {"type": "string", "enum": ["content", "files", "count"], "description": "content: path:line:text (default) | files: matching paths | count: path:match-count"}
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
        if !base.exists() {
            return ToolOutput::err(format!("no such path: {}", base.display()));
        }
        let glob_filter = super::opt_str(&input, "glob").unwrap_or(None);
        let ignore_case = super::opt_bool(&input, "ignore_case").unwrap_or(None).unwrap_or(false);
        let output_mode = super::opt_str(&input, "output_mode").unwrap_or(None).unwrap_or_else(|| "content".to_string());
        let files_only = match output_mode.as_str() {
            "content" => false,
            "files" => true,
            "count" => true,
            other => {
                return ToolOutput::err(format!(
                    "invalid output_mode '{other}' (use \"content\", \"files\", or \"count\")"
                ));
            }
        };
        let count_only = output_mode == "count";

        let re = match regex::RegexBuilder::new(&pattern)
            .case_insensitive(ignore_case)
            .size_limit(1_000_000)
            .build()
        {
            Ok(r) => r,
            Err(e) => return ToolOutput::err(format!("invalid regex: {e}")),
        };

        let cwd_disp = ctx.cwd.clone();
        let base_clone = base.clone();
        let handle = tokio::task::spawn_blocking(move || {
            let mut matches: Vec<String> = Vec::new();
            // (display path, match count) in first-seen order, for files/count modes.
            let mut per_file: Vec<(String, usize)> = Vec::new();
            let mut files_scanned = 0usize;
            let mut total_matches = 0usize;
            let mut truncated = false;

            let mut builder = ignore::WalkBuilder::new(&base_clone);
            builder.hidden(true).git_ignore(true).git_global(true).git_exclude(true).parents(true);
            builder.filter_entry(|e| {
                e.file_type()
                    .map(|ft| !ft.is_dir() || !SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
                    .unwrap_or(true)
            });
            if let Some(ov) = glob_filter.as_ref().and_then(|g| {
                ignore::overrides::OverrideBuilder::new(&base_clone)
                    .add(g)
                    .ok()
                    .and_then(|b| b.build().ok())
            }) {
                builder.overrides(ov);
            }
            for entry in builder.build().flatten() {
                if total_matches >= MAX_MATCHES {
                    truncated = true;
                    break;
                }
                let Some(ft) = entry.file_type() else { continue };
                if ft.is_dir() {
                    continue;
                }
                let path = entry.path();
                let Ok(meta) = std::fs::metadata(path) else { continue };
                if meta.len() > 5_000_000 {
                    continue;
                }
                files_scanned += 1;
                let Ok(text) = std::fs::read_to_string(path) else { continue };
                let disp = super::display_path(&cwd_disp, path);
                let mut file_count = 0usize;
                for (i, line) in text.lines().enumerate() {
                    if re.is_match(line) {
                        file_count += 1;
                        total_matches += 1;
                        if !files_only && matches.len() < MAX_MATCHES {
                            let shown: String = line.trim().chars().take(MAX_LINE).collect();
                            matches.push(format!("{}:{}:{}", disp, i + 1, shown));
                        }
                        if total_matches >= MAX_MATCHES {
                            truncated = true;
                            break;
                        }
                    }
                }
                if file_count > 0 {
                    per_file.push((disp, file_count));
                }
            }
            (matches, per_file, files_scanned, total_matches, truncated)
        })
        .await;

        let (matches, per_file, files_scanned, total_matches, truncated) =
            handle.unwrap_or((Vec::new(), Vec::new(), 0, 0, false));
        if per_file.is_empty() {
            return ToolOutput::ok(format!("No matches (searched {files_scanned} files)."));
        }
        let lines: Vec<String> = if files_only {
            per_file
                .into_iter()
                .map(|(p, n)| if count_only { format!("{p}:{n}") } else { format!("{p} ({n} match{})", if n == 1 { "" } else { "es" }) })
                .collect()
        } else {
            matches
        };
        let mut out = lines.join("\n");
        out.push_str(&format!(
            "\n\n[{} matches in {} files scanned{}]",
            if truncated { format!("first {}", MAX_MATCHES) } else { total_matches.to_string() },
            files_scanned,
            if truncated { ", capped" } else { "" }
        ));
        ToolOutput::ok(out)
    }
}
