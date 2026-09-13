//! Event-sourced session persistence. One JSONL file per session; replaying
//! the events reconstructs the conversation, todos, cwd and usage so
//! `myharness resume` continues exactly where the session stopped.

use crate::llm::{Message, Usage};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use crate::agent::state::{AgentState, Todo};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Meta {
        id: String,
        started: String,
        model: String,
        cwd: String,
    },
    Message(Message),
    Compaction {
        summary: String,
        /// How many trailing messages were kept after the summary.
        kept: usize,
    },
    Todos {
        todos: Vec<Todo>,
    },
    FilesRead {
        files: Vec<String>,
    },
    /// Journal entries appended during a turn (append semantics).
    Journal {
        entries: Vec<crate::agent::state::EditJournalEntry>,
    },
    /// `/undo` popped this many journal entries.
    Undo {
        count: usize,
    },
    Cwd {
        path: String,
    },
    Clear,
}

pub struct Session {
    pub id: String,
    pub path: PathBuf,
    writer: BufWriter<std::fs::File>,
}

impl Session {
    pub fn create(dir: &std::path::Path, model: &str, cwd: &std::path::Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let id = format!(
            "{}-{}",
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            &uuid::Uuid::new_v4().simple().to_string()[..6]
        );
        let path = dir.join(format!("{id}.jsonl"));
        let mut session = Session {
            id: id.clone(),
            path: path.clone(),
            writer: BufWriter::new(std::fs::File::create(&path)?),
        };
        session.append(&Event::Meta {
            id,
            started: chrono::Local::now().to_rfc3339(),
            model: model.to_string(),
            cwd: cwd.display().to_string(),
        })?;
        Ok(session)
    }

    pub fn append(&mut self, event: &Event) -> Result<()> {
        let mut line = serde_json::to_string(event)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        self.writer.flush()?;
        Ok(())
    }

    /// Re-open an existing session file for appending.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let file = std::fs::OpenOptions::new().append(true).open(path)?;
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("resumed")
            .to_string();
        Ok(Session {
            id,
            path: path.to_path_buf(),
            writer: BufWriter::new(file),
        })
    }

    /// Read all events from a session file.
    pub fn read_events(path: &std::path::Path) -> Result<Vec<Event>> {
        let raw = std::fs::read_to_string(path)?;
        let mut events = Vec::new();
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(line) {
                Ok(ev) => events.push(ev),
                Err(e) => {
                    // Torn tail line (crash mid-write): stop replay there.
                    eprintln!("warning: skipping unreadable session line: {e}");
                }
            }
        }
        Ok(events)
    }

    /// Rebuild agent state by replaying events.
    pub fn replay(events: Vec<Event>, workspace_root: std::path::PathBuf) -> AgentState {
        let mut state = AgentState::new(workspace_root);
        for ev in events {
            match ev {
                Event::Meta { cwd, .. } => {
                    let p = PathBuf::from(&cwd);
                    if p.is_dir() {
                        state.cwd = p;
                    }
                }
                Event::Message(m) => state.messages.push(m),
                Event::Compaction { summary, kept } => {
                    let kept_msgs: Vec<Message> = state.messages.iter().rev().take(kept).rev().cloned().collect();
                    state.messages.clear();
                    state.messages.push(Message::user_text(format!(
                        "[Earlier conversation compacted. Handoff summary:]\n{summary}"
                    )));
                    state.messages.extend(kept_msgs);
                    state.compacted = true;
                }
                Event::Todos { todos } => state.todos = todos,
                Event::FilesRead { files } => {
                    state.files_read = files.iter().map(PathBuf::from).collect();
                    state.persisted_files_read = state.files_read.clone();
                }
                Event::Journal { entries } => {
                    state.edit_journal.extend(entries);
                    state.persisted_journal_len = state.edit_journal.len();
                }
                Event::Undo { count } => {
                    for _ in 0..count {
                        if state.edit_journal.pop().is_none() {
                            break;
                        }
                    }
                    state.persisted_journal_len = state.edit_journal.len();
                }
                Event::Cwd { path } => {
                    let p = PathBuf::from(path);
                    if p.is_dir() {
                        state.cwd = p;
                    }
                }
                Event::Clear => {
                    state.messages.clear();
                    state.todos.clear();
                    state.files_read.clear();
                    state.persisted_files_read.clear();
                    state.usage = Usage::default();
                }
            }
        }
        state
    }

    /// List sessions newest-first: (id, mtime, model, first user text, messages).
    pub fn list(dir: &std::path::Path) -> Vec<(String, std::time::SystemTime, String, String, usize)> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let path = e.path();
                if path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                    continue;
                }
                let mtime = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .to_string();
                let mut model = String::new();
                let mut first = String::new();
                let mut count = 0usize;
                if let Ok(events) = Self::read_events(&path) {
                    count = events.iter().filter(|e| matches!(e, Event::Message(_))).count();
                    for ev in &events {
                        match ev {
                            Event::Meta { model: m, .. } => model = m.clone(),
                            Event::Message(msg) if first.is_empty() => {
                                if let crate::llm::Role::User = msg.role {
                                    let t = msg.text();
                                    if !t.is_empty() && !t.starts_with('[') {
                                        first = t.chars().take(60).collect();
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                out.push((id, mtime, model, first, count));
            }
        }
        out.sort_by_key(|(_, mtime, ..)| std::cmp::Reverse(*mtime));
        out
    }
}
