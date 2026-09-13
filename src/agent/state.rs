//! Mutable per-session agent state, reconstructed on resume.

use crate::llm::{Message, Usage};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Todo {
    pub content: String,
    /// pending | in_progress | completed
    pub status: String,
    /// high | medium | low
    #[serde(default = "default_priority")]
    pub priority: String,
}

fn default_priority() -> String {
    "medium".to_string()
}

impl Todo {
    pub fn render_line(&self) -> String {
        let mark = match self.status.as_str() {
            "completed" => "[x]",
            "in_progress" => "[~]",
            _ => "[ ]",
        };
        if self.priority == "high" {
            format!("- {mark} (high) {}", self.content)
        } else {
            format!("- {mark} {}", self.content)
        }
    }

    pub fn is_valid_status(s: &str) -> bool {
        matches!(s, "pending" | "in_progress" | "completed")
    }

    pub fn is_valid_priority(s: &str) -> bool {
        matches!(s, "high" | "medium" | "low")
    }
}

/// A background command started with `bash(run_in_background=true)`.
/// Output is kept in memory (capped) and polled via the `bash_output` tool.
#[derive(Debug, Clone)]
pub struct BgTask {
    pub pid: Option<u32>,
    pub command: String,
    pub started: chrono::DateTime<chrono::Local>,
    /// In-memory combined stdout+stderr so far (capped).
    pub output: Arc<Mutex<String>>,
    pub done: Arc<AtomicBool>,
    /// Exit code once finished.
    pub exit: Arc<Mutex<Option<i32>>>,
}

/// One recorded file mutation for `/undo`: the file's pre-edit backup (or
/// the fact that it did not exist, so undo deletes it).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditJournalEntry {
    pub path: String,
    pub backup: String,
    pub existed: bool,
    pub turn: u64,
}

#[derive(Debug, Clone)]
pub struct AgentState {
    pub messages: Vec<Message>,
    pub todos: Vec<Todo>,
    pub cwd: PathBuf,
    pub workspace_root: PathBuf,
    pub files_read: HashSet<PathBuf>,
    /// Mirror of files_read as last persisted, so we only write when changed.
    pub persisted_files_read: HashSet<PathBuf>,
    pub background: HashMap<u32, BgTask>,
    pub bg_counter: u32,
    /// Append-only journal of file mutations, newest last.
    pub edit_journal: Vec<EditJournalEntry>,
    /// Journal length as last persisted to the session log.
    pub persisted_journal_len: usize,
    pub usage: Usage,
    pub requests: u64,
    pub turns: u64,
    pub compacted: bool,
    pub limit_notice_sent: bool,
    /// Repo instructions from the nearest AGENTS.md (capped).
    pub project_context: Option<String>,
    /// One-glance repository layout for the system prompt.
    pub repo_layout: Option<String>,
    /// Discovered SKILL.md packs (user + workspace `.agents/skills`).
    pub skills: Vec<crate::skills::Skill>,
    /// Discovered slash-command files (user + workspace `.agents/commands`).
    pub commands: Vec<crate::commands::Command>,
    /// Background tasks whose completion notice has been delivered to the
    /// model (runtime-only; tasks themselves do not survive resume).
    pub bg_noticed: std::collections::HashSet<u32>,
}

const MAX_PROJECT_CONTEXT: usize = 8_000;
const MAX_LAYOUT_ENTRIES: usize = 40;

/// Nearest-first AGENTS.md lookup from `root` upward.
pub fn load_project_context(root: &std::path::Path) -> Option<(PathBuf, String)> {
    let mut dir = Some(root.to_path_buf());
    while let Some(d) = dir {
        let p = d.join("AGENTS.md");
        if p.is_file() {
            if let Ok(s) = std::fs::read_to_string(&p) {
                let content = if s.chars().count() > MAX_PROJECT_CONTEXT {
                    let cut: String = s.chars().take(MAX_PROJECT_CONTEXT).collect();
                    format!("{cut}\n... [AGENTS.md truncated at 8000 chars] ...")
                } else {
                    s
                };
                return Some((p, content));
            }
        }
        dir = d.parent().map(PathBuf::from);
    }
    None
}

/// Cheap repo orientation: top-level directories with recursive file counts
/// plus root files. The interim stand-in for a tree-sitter repo map —
/// nowhere near symbol-level, but it answers "what lives where" for the
/// cost of one directory walk. Skips build/dependency dirs.
pub fn build_repo_layout(root: &std::path::Path) -> Option<String> {
    const SKIP: &[&str] = &["target", "node_modules", ".git", "dist", "build", ".venv", "__pycache__"];
    let entries = std::fs::read_dir(root).ok()?;
    let mut dirs: Vec<(String, usize)> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut it = entries.flatten().collect::<Vec<_>>();
    it.sort_by_key(|e| e.file_name());
    for e in it {
        let name = e.file_name().to_string_lossy().to_string();
        if SKIP.contains(&name.as_str()) {
            continue;
        }
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let mut count = 0usize;
            for entry in ignore::WalkBuilder::new(e.path())
                .hidden(true)
                .git_ignore(true)
                .parents(true)
                .filter_entry(|en| {
                    en.file_type()
                        .map(|ft| !ft.is_dir() || !SKIP.contains(&en.file_name().to_string_lossy().as_ref()))
                        .unwrap_or(true)
                })
                .build()
                .flatten()
            {
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    count += 1;
                    if count > 999 {
                        break;
                    }
                }
            }
            dirs.push((name, count));
        } else {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            let human = if size > 1_048_576 {
                format!(" ({} MB)", size / 1_048_576)
            } else if size > 1024 {
                format!(" ({} KB)", size / 1024)
            } else {
                String::new()
            };
            files.push(format!("{name}{human}"));
        }
    }
    if dirs.is_empty() && files.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = dirs
        .into_iter()
        .map(|(n, c)| format!("{n}/ — {c} files"))
        .chain(files)
        .collect();
    lines.truncate(MAX_LAYOUT_ENTRIES);
    Some(lines.join("\n"))
}

impl AgentState {
    pub fn new(workspace_root: PathBuf) -> Self {
        let cwd = workspace_root.clone();
        let project_context = load_project_context(&cwd).map(|(_, c)| c);
        let repo_layout = build_repo_layout(&cwd);
        let skills = crate::skills::discover(&cwd);
        let commands = crate::commands::discover(&cwd);
        AgentState {
            messages: Vec::new(),
            todos: Vec::new(),
            cwd,
            workspace_root,
            files_read: HashSet::new(),
            persisted_files_read: HashSet::new(),
            background: HashMap::new(),
            bg_counter: 0,
            edit_journal: Vec::new(),
            persisted_journal_len: 0,
            usage: Usage::default(),
            requests: 0,
            turns: 0,
            compacted: false,
            limit_notice_sent: false,
            project_context,
            repo_layout,
            skills,
            commands,
            bg_noticed: std::collections::HashSet::new(),
        }
    }

    pub fn render_todos(&self) -> String {
        Self::render_todo_list(&self.todos)
    }

    pub fn render_todo_list(todos: &[Todo]) -> String {
        if todos.is_empty() {
            return String::new();
        }
        let mut s = String::from("Current task list:\n");
        for t in todos {
            s.push_str(&t.render_line());
            s.push('\n');
        }
        s
    }
}
