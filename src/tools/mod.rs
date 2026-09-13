//! Tool trait, registry and shared input/path helpers.
//!
//! Tools never prompt and never touch the UI: they receive parsed input plus
//! a `ToolCtx` and return a `ToolOutput` the agent feeds back to the model.
//! Permissions are enforced by the agent before dispatch.
//!
//! `ToolCtx` deliberately holds **copies** of the session state a tool needs
//! plus a `&mut ToolEffects` collector; the agent merges effects into the
//! real state after execution. That is what makes it safe to run several
//! concurrency-safe tools in parallel.

pub mod bash;
pub mod bash_output;
pub mod edit_file;
pub mod glob;
pub mod grep;
pub mod ls;
pub mod read_file;
pub mod repo_map;
pub mod skill;
pub mod task;
pub mod todo;
pub mod web_fetch;
pub mod web_search;
pub mod write_file;

use crate::agent::state::{AgentState, BgTask, EditJournalEntry, Todo};
use crate::config::Config;
use crate::llm::{ContentImage, Provider, ToolSchema};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
    /// Images returned to the model (anthropic protocol carries them in the
    /// tool_result; openai protocol degrades to a text note).
    pub images: Vec<ContentImage>,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        ToolOutput { content: content.into(), is_error: false, images: Vec::new() }
    }

    pub fn err(content: impl Into<String>) -> Self {
        ToolOutput { content: content.into(), is_error: true, images: Vec::new() }
    }

    pub fn with_images(mut self, images: Vec<ContentImage>) -> Self {
        self.images = images;
        self
    }
}

/// Side effects a tool wants applied to session state. Collected during
/// execution, merged by the agent afterwards (sequentially or after a
/// parallel batch).
#[derive(Debug, Default, Clone)]
pub struct ToolEffects {
    pub files_read: Vec<PathBuf>,
    pub new_cwd: Option<PathBuf>,
    pub todos: Option<Vec<Todo>>,
    /// (id, task) registered by background bash; agent inserts into the map.
    pub background: Option<(u32, BgTask)>,
    /// Next background-task id (monotonic counter).
    pub bg_counter: Option<u32>,
    /// File mutations recorded for /undo (write_file/edit_file only).
    pub journal: Vec<EditJournalEntry>,
}

impl ToolEffects {
    pub fn merge_into(self, state: &mut AgentState) {
        for f in self.files_read {
            state.files_read.insert(f);
        }
        if let Some(cwd) = self.new_cwd {
            state.cwd = cwd;
        }
        if let Some(t) = self.todos {
            state.todos = t;
        }
        if let Some((id, task)) = self.background {
            state.background.insert(id, task);
        }
        if let Some(c) = self.bg_counter {
            state.bg_counter = state.bg_counter.max(c);
        }
        state.edit_journal.extend(self.journal);
    }
}

pub struct ToolCtx<'a> {
    pub cwd: PathBuf,
    pub workspace_root: PathBuf,
    pub cfg: Arc<Config>,
    pub provider: Arc<dyn Provider>,
    /// Read guards: which files were read this session.
    pub files_read: &'a HashSet<PathBuf>,
    /// Live background tasks (read view).
    pub background: &'a HashMap<u32, BgTask>,
    /// Id to use for the next background task.
    pub next_bg_id: u32,
    /// Set when the user interrupted; long-running tools should abort.
    pub cancel: Arc<AtomicBool>,
    /// Where pre-edit backups land (data_dir/checkpoints/<session>).
    pub checkpoint_dir: PathBuf,
    /// Journal position for unique backup names.
    pub journal_next: usize,
    /// Current turn number (labels journal entries).
    pub turns: u64,
    pub effects: &'a mut ToolEffects,
}

/// Back up a file (or record its absence) before a mutating tool changes
/// it, and return the journal entry to push into effects.
pub(crate) fn snapshot_for_journal(ctx: &ToolCtx<'_>, path: &Path) -> EditJournalEntry {
    let seq = ctx.journal_next;
    let fname = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let safe: String = fname
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '_' })
        .collect();
    let entry_path = path.display().to_string();
    if path.exists() {
        let backup = ctx.checkpoint_dir.join(format!("{seq:04}_{safe}"));
        if std::fs::create_dir_all(&ctx.checkpoint_dir).is_ok()
            && std::fs::copy(path, &backup).is_ok()
        {
            return EditJournalEntry {
                path: entry_path,
                backup: backup.display().to_string(),
                existed: true,
                turn: ctx.turns,
            };
        }
        // Backup failed: still journal (existed=true) so undo reports it.
        return EditJournalEntry {
            path: entry_path,
            backup: String::new(),
            existed: true,
            turn: ctx.turns,
        };
    }
    EditJournalEntry {
        path: entry_path,
        backup: String::new(),
        existed: false,
        turn: ctx.turns,
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn schema(&self) -> Value;
    fn is_read_only(&self) -> bool;
    /// Tools that must be approval-gated even though they are read-only
    /// (e.g. network egress). Plan mode still allows them — they cannot
    /// modify state — but ask/auto-edit modes prompt like mutations.
    fn requires_approval(&self) -> bool {
        !self.is_read_only()
    }
    /// Whether several instances may run concurrently. Defaults to
    /// read-only-ness; tools that only emit effects can opt in.
    fn concurrency_safe(&self) -> bool {
        self.is_read_only()
    }
    /// Argument shown in permission prompts and the transcript line.
    fn perm_summary(&self, _input: &Value) -> String {
        String::new()
    }
    async fn execute(&self, input: Value, ctx: &mut ToolCtx<'_>) -> ToolOutput;
}

pub struct Registry {
    tools: Vec<Arc<dyn Tool>>,
}

impl Registry {
    pub fn full() -> Self {
        Registry {
            tools: vec![
                Arc::new(read_file::ReadFileTool),
                Arc::new(write_file::WriteFileTool),
                Arc::new(edit_file::EditFileTool),
                Arc::new(bash::BashTool),
                Arc::new(bash_output::BashOutputTool),
                Arc::new(glob::GlobTool),
                Arc::new(grep::GrepTool),
                Arc::new(ls::LsTool),
                Arc::new(todo::TodoTool),
                Arc::new(skill::SkillTool),
                Arc::new(task::TaskTool),
                Arc::new(web_fetch::WebFetchTool),
                Arc::new(web_search::WebSearchTool),
                Arc::new(repo_map::RepoMapTool),
            ],
        }
    }

    /// A restricted registry for subagents: only named tools, and never
    /// `task` itself (no recursive spawning).
    pub fn subset(names: &[String]) -> Self {
        let full = Self::full();
        Registry {
            tools: full
                .tools
                .into_iter()
                .filter(|t| t.name() != "task" && names.iter().any(|n| n == t.name()))
                .collect(),
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name).map(Arc::clone)
    }

    /// Add a dynamically provided tool (e.g. bridged from MCP).
    pub fn add_tool(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools
            .iter()
            .map(|t| ToolSchema {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.schema(),
            })
            .collect()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.tools.iter().map(|t| t.name()).collect()
    }
}

// ---- input helpers ---------------------------------------------------------

pub(crate) fn require_str(input: &Value, key: &str) -> Result<String> {
    match input.get(key).and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => Ok(s.to_string()),
        _ => bail!("missing required string parameter '{key}'"),
    }
}

pub(crate) fn opt_str(input: &Value, key: &str) -> Result<Option<String>> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .map(|s| s.to_string())
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("parameter '{key}' must be a string")),
    }
}

pub(crate) fn opt_u64(input: &Value, key: &str) -> Result<Option<u64>> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()).map(Some))
            .ok_or_else(|| anyhow::anyhow!("parameter '{key}' must be a number")),
    }
}

pub(crate) fn opt_bool(input: &Value, key: &str) -> Result<Option<bool>> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_bool()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("parameter '{key}' must be a boolean")),
    }
}

// ---- path helpers ----------------------------------------------------------

/// Resolve a user/model-supplied path against the session cwd, lexically
/// normalized (no filesystem access, so not-yet-existing targets work).
pub(crate) fn resolve_path(cwd: &Path, p: &str) -> PathBuf {
    let expanded = if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return lexical_clean(&home.join(rest));
        }
        PathBuf::from(p)
    } else {
        PathBuf::from(p)
    };
    if expanded.is_absolute() {
        lexical_clean(&expanded)
    } else {
        lexical_clean(&cwd.join(expanded))
    }
}

/// Canonicalize if possible, else lexically clean (file may not exist yet).
/// Windows `canonicalize()` returns `\\?\`-prefixed verbatim paths for
/// existing files while the lexical fallback has no prefix — guard keys
/// must match regardless of whether the file existed at computation time,
/// so the verbatim prefix is stripped (dunce-style).
pub(crate) fn canon(cwd: &Path, p: &str) -> PathBuf {
    let resolved = resolve_path(cwd, p);
    match resolved.canonicalize() {
        Ok(c) => strip_verbatim(c),
        Err(_) => resolved,
    }
}

fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.as_os_str().to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest.to_string());
    }
    p
}

pub(crate) fn lexical_clean(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        use std::path::Component;
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Format a path for display: relative to cwd when inside it.
pub(crate) fn display_path(cwd: &Path, p: &Path) -> String {
    p.strip_prefix(cwd)
        .map(|r| r.display().to_string().replace('\\', "/"))
        .unwrap_or_else(|_| p.display().to_string().replace('\\', "/"))
}

/// Head+tail truncation preserving a visible marker, used by tools that can
/// produce large outputs (bash, read_file, grep, web_fetch).
pub(crate) fn truncate_middle(s: &str, max: usize, head: usize, tail: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let h: String = chars[..head].iter().collect();
    let t: String = chars[chars.len() - tail..].iter().collect();
    let dropped = chars.len() - head - tail;
    format!("{h}\n... [{dropped} chars truncated] ...\n{t}")
}

pub(crate) fn schema_obj(props: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": props,
        "required": required.iter().filter(|r| !r.is_empty()).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_clean_removes_dots() {
        let p = lexical_clean(Path::new("/a/b/../c/./d"));
        assert_eq!(p, Path::new("/a/c/d"));
    }    #[test]
    fn resolve_relative_and_tilde() {
        let cwd = Path::new("/w/root");
        assert_eq!(resolve_path(cwd, "src/x.rs"), Path::new("/w/root/src/x.rs"));
        assert_eq!(resolve_path(cwd, "/abs/y.rs"), Path::new("/abs/y.rs"));
        let home = resolve_path(cwd, "~/z.rs");
        assert!(home.starts_with(dirs::home_dir().unwrap_or_default()));
    }

    #[test]
    fn canon_matches_before_and_after_creation() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("later.txt");
        // Key computed before the file exists (lexical)...
        let before = canon(dir.path(), "later.txt");
        std::fs::write(&target, "x").unwrap();
        // ...and after (canonicalized) must be identical.
        let after = canon(dir.path(), "later.txt");
        assert_eq!(before, after, "guard keys must not depend on file existence");
    }

    #[test]
    fn truncate_middle_caps_output() {
        let s = "a".repeat(1000);
        let out = truncate_middle(&s, 100, 60, 20);
        assert!(out.contains("chars truncated]"));
        assert!(out.chars().count() < 120);
    }
}
