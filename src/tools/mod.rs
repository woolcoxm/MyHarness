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
pub mod js_check;
pub mod ls;
pub mod monitor;
pub mod net_guard;
pub mod read_file;
pub mod repo_map;
pub mod session_recall;
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
    /// Updated read fingerprints (mtime_ms, len) — recorded by reads and
    /// refreshed by writes so the stale guard stays coherent.
    pub file_stats: Vec<(PathBuf, u64, u64)>,
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
        for (f, m, l) in self.file_stats {
            state.file_stats.insert(f, (m, l));
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
    /// Read-guard fingerprints: (mtime_ms, len) as of the last read.
    pub file_stats: &'a HashMap<PathBuf, (u64, u64)>,
    /// Live background tasks (read view).
    pub background: &'a HashMap<u32, BgTask>,
    /// Id to use for the next background task.
    pub next_bg_id: u32,
    /// Set when the user interrupted; long-running tools should abort.
    pub cancel: Arc<AtomicBool>,
    /// Where pre-edit backups land (data_dir/checkpoints/<session>).
    pub checkpoint_dir: PathBuf,
    /// Where oversized outputs spill (data_dir/artifacts/<session>).
    pub artifacts_dir: PathBuf,
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
                Arc::new(session_recall::SessionRecallTool),
                Arc::new(monitor::MonitorTool),
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
    if let Ok(c) = resolved.canonicalize() {
        return strip_verbatim(c);
    }
    // Not yet existing: canonicalize the deepest existing ancestor and
    // re-append the missing tail, so the key matches what canonicalize()
    // will produce once the file exists. A lexical fallback can't do this:
    // Windows 8.3 short-name components (RUNNER~1 vs runneradmin) only
    // expand through the filesystem.
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut anc = resolved.as_path();
    while !anc.exists() {
        match anc.parent() {
            Some(parent) => {
                if let Some(name) = anc.file_name() {
                    tail.push(name.to_os_string());
                }
                anc = parent;
            }
            None => return resolved,
        }
    }
    match anc.canonicalize() {
        Ok(mut c) => {
            for comp in tail.iter().rev() {
                c.push(comp);
            }
            strip_verbatim(c)
        }
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

/// Hard cap for a spilled artifact file.
const MAX_SPILL: usize = 2_000_000;

/// Head+tail budget with a lossless escape hatch: over-budget output is
/// spilled whole to the session's artifacts directory and the model gets
/// head+tail plus a pointer it can `read_file` with offset/limit. Spill is
/// best-effort — a failed write falls back to plain truncation, never
/// failing the tool.
pub(crate) fn budget_output(
    ctx: &ToolCtx<'_>,
    label: &str,
    text: &str,
    max: usize,
    head: usize,
    tail: usize,
) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    match spill_artifact(&ctx.artifacts_dir, label, text) {
        Some(path) => truncate_with_pointer(text, head, tail, &path),
        None => truncate_middle(text, max, head, tail),
    }
}

/// Write the full text (capped) under `dir` with a unique name.
pub(crate) fn spill_artifact(dir: &Path, label: &str, text: &str) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let name = format!("{label}-{}.txt", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let path = dir.join(name);
    let mut body: String = text.chars().take(MAX_SPILL).collect();
    if text.chars().count() > MAX_SPILL {
        body.push_str("\n... [spill capped at 2,000,000 chars] ...");
    }
    std::fs::write(&path, body).ok()?;
    Some(path)
}

fn truncate_with_pointer(text: &str, head: usize, tail: usize, path: &Path) -> String {
    let total = text.chars().count();
    let chars: Vec<char> = text.chars().collect();
    let head = head.min(total);
    let tail = tail.min(total.saturating_sub(head));
    let h: String = chars[..head].iter().collect();
    let t: String = chars[total - tail..].iter().collect();
    let dropped = total - head - tail;
    format!(
        "{h}\n... [{dropped} chars truncated; full output ({total} chars) saved to {} — \
         read_file it with offset/limit to see the middle] ...\n{t}",
        path.display()
    )
}

pub(crate) fn schema_obj(props: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": props,
        "required": required.iter().filter(|r| !r.is_empty()).collect::<Vec<_>>(),
    })
}

/// (mtime_ms, len) for an existing file.
pub(crate) fn stat_file(p: &Path) -> Option<(u64, u64)> {
    let md = std::fs::metadata(p).ok()?;
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Some((mtime, md.len()))
}

/// Stale-context guard (Cline's tracker, enforced at the mutation gate): a
/// file that changed on disk since it was last read must be re-read before
/// editing — the failure direction is an extra round-trip, never a lost
/// external edit. Reads recorded before the tracker existed are exempt.
pub(crate) fn stale_check(ctx: &ToolCtx<'_>, key: &Path) -> Result<(), String> {
    let Some((mtime, len)) = ctx.file_stats.get(key) else {
        return Ok(());
    };
    match stat_file(key) {
        Some((m2, l2)) if m2 == *mtime && l2 == *len => Ok(()),
        Some(_) => Err(format!(
            "{} changed on disk since it was last read (edited outside the harness?); \
             read_file it again so the edit applies to the current content",
            key.display()
        )),
        // Vanished: the read/write below fails loudly on its own.
        None => Ok(()),
    }
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

    #[test]
    fn spill_artifact_writes_full_text() {
        let dir = tempfile::tempdir().unwrap();
        let text = "x".repeat(50_000);
        let path = spill_artifact(dir.path(), "bash", &text).expect("spill must succeed");
        assert!(path.to_string_lossy().contains("bash-"));
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(on_disk.chars().count(), 50_000, "nothing may be lost in the spill");
        let out = truncate_with_pointer(&text, 10, 10, &path);
        assert!(out.starts_with("xxxxxxxxxx"));
        assert!(out.ends_with("xxxxxxxxxx"));
        assert!(out.contains("full output (50000 chars) saved to"));
        assert!(out.contains(&path.display().to_string()));
    }

    #[test]
    fn spill_artifact_fails_cleanly_on_unwritable_dir() {
        // A directory path whose parent is a regular file cannot be created.
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "i am a file").unwrap();
        assert!(spill_artifact(&blocker.join("sub"), "bash", "huge").is_none());
    }

    #[test]
    fn spill_caps_at_2m_chars() {
        let dir = tempfile::tempdir().unwrap();
        let text = "y".repeat(MAX_SPILL + 5_000);
        let path = spill_artifact(dir.path(), "bash", &text).unwrap();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains("spill capped at 2,000,000 chars"));
    }
}
