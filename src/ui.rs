//! Line-oriented UI. Deliberately ASCII-only and TUI-free: it renders
//! correctly in every Windows console, pipes, and CI logs, and it can't
//! wedge the terminal state.
//!
//! The sink is one of three: stdout (REPL), quiet (`-p` / subagents), or a
//! channel (`serve` mode) where every event becomes a JSON-RPC notification
//! line instead of terminal output.

use crate::agent::Agent;
use crate::tools::ToolOutput;
use anyhow::Result;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone)]
pub struct Ui {
    pub quiet: bool,
    /// serve mode: events are emitted as JSON-RPC notification lines.
    pub channel: Option<UnboundedSender<String>>,
    at_line_start: bool,
    /// A thinking run just printed without a trailing newline; the next
    /// real output starts on a fresh line.
    thinking_open: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

fn notify(tx: &UnboundedSender<String>, method: &str, params: &str) {
    let _ = tx.send(format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"{method}\",\"params\":{params}}}"
    ));
}

impl Ui {
    pub fn new() -> Self {
        Ui { quiet: false, channel: None, at_line_start: true, thinking_open: false }
    }

    pub fn quiet() -> Self {
        Ui { quiet: true, channel: None, at_line_start: true, thinking_open: false }
    }

    /// serve mode: all events become JSON notification lines on the channel.
    pub fn channel(tx: UnboundedSender<String>) -> Self {
        Ui { quiet: true, channel: Some(tx), at_line_start: true, thinking_open: false }
    }

    fn out(&mut self, s: &str) {
        if self.quiet {
            return;
        }
        println!("{s}");
    }

    fn ensure_newline(&mut self) {
        if (!self.at_line_start || self.thinking_open) && !self.quiet {
            println!();
        }
        self.at_line_start = true;
        self.thinking_open = false;
    }

    /// Streamed assistant text: printed as it arrives.
    pub fn assistant_delta(&mut self, s: &str) {
        if let Some(tx) = &self.channel {
            notify(tx, "mh/turn.delta", &format!("{{\"text\":{}}}", serde_json::json!(s)));
            return;
        }
        if self.quiet {
            return;
        }
        if self.thinking_open {
            println!();
            self.thinking_open = false;
        }
        print!("{s}");
        let _ = std::io::stdout().flush();
        self.at_line_start = s.ends_with('\n');
    }

    /// Streamed model reasoning: shown with a `~` prefix so the operator can
    /// watch the model think; never enters the context.
    pub fn thinking_delta(&mut self, s: &str) {
        if let Some(tx) = &self.channel {
            notify(tx, "mh/turn.thinking", &format!("{{\"text\":{}}}", serde_json::json!(s)));
            return;
        }
        if self.quiet {
            return;
        }
        for line in s.split_inclusive('\n') {
            if self.at_line_start && !line.trim().is_empty() {
                print!("  ~ ");
            }
            print!("{line}");
            self.at_line_start = line.ends_with('\n');
        }
        let _ = std::io::stdout().flush();
        self.thinking_open = true;
    }

    pub fn end_stream(&mut self) {
        self.ensure_newline();
    }

    pub fn tool_start(&mut self, name: &str, summary: &str) {
        if let Some(tx) = &self.channel {
            notify(
                tx,
                "mh/tool.start",
                &format!(
                    "{{\"name\":{},\"summary\":{}}}",
                    serde_json::json!(name),
                    serde_json::json!(summary)
                ),
            );
            return;
        }
        if self.quiet {
            return;
        }
        self.ensure_newline();
        if summary.is_empty() {
            println!("  * {name}");
        } else {
            println!("  * {name}({summary})");
        }
        self.at_line_start = true;
    }

    pub fn tool_end(&mut self, name: &str, result: &ToolOutput) {
        if let Some(tx) = &self.channel {
            let first = result.content.lines().next().unwrap_or("").chars().take(120).collect::<String>();
            notify(
                tx,
                "mh/tool.end",
                &format!(
                    "{{\"name\":{},\"ok\":{},\"first\":{}}}",
                    serde_json::json!(name),
                    !result.is_error,
                    serde_json::json!(first)
                ),
            );
            return;
        }
        if self.quiet {
            return;
        }
        let first = result.content.lines().next().unwrap_or("").chars().take(120).collect::<String>();
        let images = if result.images.is_empty() {
            String::new()
        } else {
            format!(" [+{} img]", result.images.len())
        };
        if result.is_error {
            println!("    ! {first}{images}");
        } else {
            println!("    ok  {first}{images}");
        }
        self.at_line_start = true;
    }

    pub fn permission_ask(&mut self, name: &str, arg: &str) {
        if self.channel.is_some() || self.quiet {
            return;
        }
        self.ensure_newline();
        println!("  !! {name} wants to run: {arg}");
        print!("     allow? [y]es / [n]o / [a]lways: ");
        let _ = std::io::stdout().flush();
        self.at_line_start = false;
    }

    pub fn info(&mut self, s: &str) {
        if let Some(tx) = &self.channel {
            notify(tx, "mh/info", &format!("{{\"text\":{}}}", serde_json::json!(s)));
            return;
        }
        self.ensure_newline();
        self.out(&format!("== {s}"));
    }

    pub fn warn(&mut self, s: &str) {
        if let Some(tx) = &self.channel {
            notify(tx, "mh/warn", &format!("{{\"text\":{}}}", serde_json::json!(s)));
            return;
        }
        if self.quiet {
            eprintln!("!! {s}");
            return;
        }
        self.ensure_newline();
        println!("!! {s}");
    }

    pub fn turn_footer(&mut self, turns: u64, input_tokens: u64, output_tokens: u64) {
        if self.channel.is_some() || self.quiet {
            return;
        }
        self.ensure_newline();
        println!(
            "-- turn {turns} | {in_tok} in / {out_tok} out (cumulative)",
            in_tok = format_tokens(input_tokens),
            out_tok = format_tokens(output_tokens),
        );
        println!();
    }

    pub fn banner(&mut self, model: &str, mode: &str, session: Option<&str>) {
        if self.channel.is_some() || self.quiet {
            return;
        }
        println!("myharness {} | model {} | mode {}", env!("CARGO_PKG_VERSION"), model, mode);
        if let Some(s) = session {
            println!("session {s} | /help for commands");
        }
        println!();
    }
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

const HELP: &str = "\
/help                 this help
/quit                 exit (also /exit, Ctrl-D)
/clear                wipe the conversation context
/compact              compact the context now into a handoff summary
/mode [name]          show or set permission mode: plan | ask | auto-edit | yolo
/model [name]         show or switch the model
/todos                show the current task list
/undo [n]             restore the last n file edits (default 1)
/usage                token/request usage for this session
/info                 model, mode, shell, cwd, session details
/skills               list discovered skills (SKILL.md packs)
/<skill> [args]       run a discovered skill by name
/commands             list slash-command files (.agents/commands/*.md)";

pub async fn repl(mut agent: Agent) -> Result<()> {
    let busy = Arc::new(AtomicBool::new(false));
    let cancel = Arc::clone(&agent.cancel);

    // Ctrl-C: first press interrupts the running turn; second exits.
    let busy_flag = Arc::clone(&busy);
    let cancel_task = Arc::clone(&cancel);
    tokio::spawn(async move {
        loop {
            if tokio::signal::ctrl_c().await.is_err() {
                return;
            }
            if busy_flag.swap(false, Ordering::SeqCst) {
                cancel_task.store(true, Ordering::SeqCst);
                eprintln!("\n(interrupting... press Ctrl-C again to force quit)");
            } else {
                std::process::exit(130);
            }
        }
    });

    agent.ui.banner(
        &agent.model,
        agent.perms.mode.name(),
        agent.session.as_ref().map(|s| s.id.as_str()),
    );

    let history_path = agent.cfg.data_dir.join("history.txt");
    let mut editor = rustyline::DefaultEditor::new()?;
    let _ = std::fs::create_dir_all(&agent.cfg.data_dir);
    let _ = editor.load_history(&history_path);

    loop {
        let line = match editor.readline("mh> ") {
            Ok(l) => l,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        };
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        let _ = editor.add_history_entry(&trimmed);
        let _ = editor.save_history(&history_path);

        if let Some(cmd) = trimmed.strip_prefix('/') {
            if handle_slash(&mut agent, cmd).await == SlashResult::Quit {
                break;
            }
            continue;
        }

        busy.store(true, Ordering::SeqCst);
        let result = agent.run_turn(&trimmed).await;
        busy.store(false, Ordering::SeqCst);
        cancel.store(false, Ordering::SeqCst);
        if let Err(e) = result {
            agent.ui.warn(&format!("turn failed: {e}"));
        }
    }
    println!("bye");
    Ok(())
}

#[derive(PartialEq)]
enum SlashResult {
    Continue,
    Quit,
}

async fn handle_slash(agent: &mut Agent, cmd: &str) -> SlashResult {
    let mut parts = cmd.split_whitespace();
    let name = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("");
    match name {
        "quit" | "exit" | "q" => return SlashResult::Quit,
        "help" | "h" | "?" => agent.ui.out(HELP),
        "clear" => agent.clear(),
        "compact" => {
            if let Err(e) = agent.compact_now().await {
                agent.ui.warn(&format!("compaction failed: {e}"));
            }
        }
        "mode" => {
            if arg.is_empty() {
                agent.ui.info(&format!(
                    "mode: {} (plan | ask | auto-edit | yolo)",
                    agent.perms.mode.name()
                ));
            } else {
                match crate::perms::PermissionMode::parse(arg) {
                    Ok(m) => {
                        agent.perms.set_mode(m);
                        agent.ui.info(&format!("mode set to {}", m.name()));
                    }
                    Err(e) => agent.ui.warn(&e.to_string()),
                }
            }
        }
        "plan" => {
            agent.perms.set_mode(crate::perms::PermissionMode::Plan);
            agent.ui.info("mode set to plan (read-only; mutations blocked)");
        }
        "yolo" => {
            agent.perms.set_mode(crate::perms::PermissionMode::Yolo);
            agent.ui.warn("mode set to yolo — mutations run without asking");
        }
        "model" => {
            if arg.is_empty() {
                agent.ui.info(&format!("model: {} (restart or /model <name> to change)", agent.model));
            } else {
                agent.model = arg.to_string();
                agent.ui.info(&format!("model set to {arg}"));
            }
        }
        "todos" => {
            let t = agent.state.render_todos();
            agent.ui.out(if t.is_empty() { "(no todos)" } else { &t });
        }
        "undo" => {
            let n: usize = arg.parse().unwrap_or(1).clamp(1, 100);
            let undone = agent.undo(n);
            if undone == 0 {
                agent.ui.info("nothing to undo (no journaled edits)");
            }
        }
        "usage" => {
            let u = &agent.state.usage;
            agent.ui.info(&format!(
                "requests: {} | tokens in: {} | tokens out: {} | turns: {} | compacted: {}",
                agent.state.requests,
                u.input_tokens,
                u.output_tokens,
                agent.state.turns,
                agent.state.compacted
            ));
        }
        "info" => {
            let shell = format!("{:?}", agent.cfg.resolve_shell());
            agent.ui.info(&format!(
                "model {} via {} | mode {} | shell {} | cwd {} | files read {} | session {}",
                agent.model,
                agent.provider.name(),
                agent.perms.mode.name(),
                shell,
                agent.state.cwd.display(),
                agent.state.files_read.len(),
                agent.session.as_ref().map(|s| s.path.display().to_string()).unwrap_or_else(|| "(none)".to_string()),
            ));
        }
        "session" => {
            agent.ui.info(&agent
                .session
                .as_ref()
                .map(|s| format!("{} ({})", s.id, s.path.display()))
                .unwrap_or_else(|| "(no session file)".to_string()));
        }
        "skills" => {
            if agent.state.skills.is_empty() {
                agent.ui.info(
                    "no skills discovered — put SKILL.md packs in .agents/skills/<name>/ (workspace) or ~/.agents/skills/<name>/",
                );
            } else {
                let list = crate::skills::render_list(&agent.state.skills);
                agent.ui.out(list.trim_end());
            }
        }
        "commands" => {
            if agent.state.commands.is_empty() {
                agent.ui.info(
                    "no command files discovered — put markdown files in .agents/commands/ (workspace) or ~/.agents/commands/",
                );
            } else {
                let list = crate::commands::render_list(&agent.state.commands);
                agent.ui.out(list.trim_end());
            }
        }
        other => {
            // Unknown commands fall through to skills and command files:
            // /<name> [args] either loads a skill through the model or
            // expands a command file's prompt template.
            if agent.state.skills.iter().any(|s| s.name == other) {
                let rest = cmd.strip_prefix(other).unwrap_or("").trim();
                let task = if rest.is_empty() {
                    format!("Use the '{other}' skill.")
                } else {
                    format!("Use the '{other}' skill: {rest}")
                };
                if let Err(e) = agent.run_turn(&task).await {
                    agent.ui.warn(&format!("turn failed: {e}"));
                }
            } else if let Some(c) = agent.state.commands.iter().find(|c| c.name == other) {
                let rest = cmd.strip_prefix(other).unwrap_or("").trim();
                let prompt = crate::commands::expand(&c.body, rest);
                if let Err(e) = agent.run_turn(&prompt).await {
                    agent.ui.warn(&format!("turn failed: {e}"));
                }
            } else {
                agent.ui.warn(&format!("unknown command /{other} — try /help"));
            }
        }
    }
    SlashResult::Continue
}
