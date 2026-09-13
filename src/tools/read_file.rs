//! read_file: `cat -n`-formatted line output, the format coding models are
//! most reliably trained on. Line numbers make edit_file conversations
//! unambiguous. Image files are returned as image blocks so the model can
//! actually look at them (screenshots, diagrams, rendered UIs).

use super::{canon, opt_u64, require_str, schema_obj, truncate_middle, Tool, ToolCtx, ToolOutput};
use crate::llm::ContentImage;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;

pub struct ReadFileTool;

const DEFAULT_LIMIT: u64 = 2000;
const MAX_LINE_LEN: usize = 2000;
const MAX_OUTPUT: usize = 60_000;
const MAX_IMAGE_BYTES: usize = 4_000_000;

fn image_media_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Reads a file and returns it to you. Text files come back line-numbered in `cat -n` format (`     1\ttext`); use offset (1-based line) and limit for large files (default limit 2000). Image files (png/jpg/gif/webp) are returned as images you can see — use this for screenshots and UI verification. Reading a file is required before edit_file or overwriting via write_file."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "path": {"type": "string", "description": "File path, relative to the working directory or absolute"},
                "offset": {"type": "integer", "description": "1-based line number to start reading from"},
                "limit": {"type": "integer", "description": "Maximum number of lines to return (default 2000)"}
            }),
            &["path"],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let path = match require_str(&input, "path") {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let offset = opt_u64(&input, "offset").unwrap_or(None).unwrap_or(1).max(1);
        let limit = opt_u64(&input, "limit").unwrap_or(None).unwrap_or(DEFAULT_LIMIT).clamp(1, 5000);

        let resolved = super::resolve_path(&ctx.cwd, &path);
        if resolved.is_dir() {
            return ToolOutput::err(format!(
                "{} is a directory; use ls to list it",
                super::display_path(&ctx.cwd, &resolved)
            ));
        }
        let bytes = match std::fs::read(&resolved) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return ToolOutput::err(format!(
                    "file not found: {} (use glob or ls to locate files)",
                    super::display_path(&ctx.cwd, &resolved)
                ));
            }
            Err(e) => return ToolOutput::err(format!("cannot read {}: {e}", resolved.display())),
        };
        ctx.effects.files_read.push(canon(&ctx.cwd, &path));

        // Images: return as image blocks.
        if let Some(media_type) = image_media_type(&resolved) {
            if bytes.len() > MAX_IMAGE_BYTES {
                return ToolOutput::err(format!(
                    "image is {} MB; cap is {} MB — resize it first",
                    bytes.len() / 1_000_000,
                    MAX_IMAGE_BYTES / 1_000_000
                ));
            }
            let data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
            return ToolOutput::ok(format!(
                "Image {}: {} KB, returned as an image you can see.",
                super::display_path(&ctx.cwd, &resolved),
                bytes.len() / 1024
            ))
            .with_images(vec![ContentImage {
                media_type: media_type.to_string(),
                data,
            }]);
        }

        if bytes.contains(&0) {
            return ToolOutput::err(format!(
                "{} is binary and cannot be displayed (images: png/jpg/gif/webp are supported)",
                resolved.display()
            ));
        }

        let text = String::from_utf8_lossy(&bytes);
        let total_lines = text.lines().count() as u64;
        let mut out = String::new();
        for (i, line) in text.lines().enumerate() {
            let lineno = i as u64 + 1;
            if lineno < offset {
                continue;
            }
            if lineno >= offset + limit {
                let skipped = total_lines - lineno + 1;
                out.push_str(&format!(
                    "... [{skipped} more lines; re-read with offset={lineno}] ...\n"
                ));
                break;
            }
            let line = line.trim_end_matches('\r');
            let shown = if line.chars().count() > MAX_LINE_LEN {
                let truncated: String = line.chars().take(MAX_LINE_LEN).collect();
                format!("{truncated} ... [line truncated]")
            } else {
                line.to_string()
            };
            out.push_str(&format!("{lineno:>6}\t{shown}\n"));
        }
        if total_lines == 0 {
            out.push_str("(empty file)\n");
        }
        ToolOutput::ok(truncate_middle(&out, MAX_OUTPUT, 40_000, 15_000))
    }
}
