//! Line-oriented UI. Deliberately ASCII-only and TUI-free: it renders
//! correctly in every Windows console, pipes, and CI logs, and it can't
//! wedge the terminal state.
//!
//! The sink is one of three: stdout (REPL), quiet (`-p` / subagents), or a
//! channel (`serve` mode) where every event becomes a JSON-RPC notification
//! line instead of terminal output.

use crate::agent::Agent;
use crate::tools::ToolOutput;
use serde_json::Value;
use std::io::Write;
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc::UnboundedSender;

/// Typed UI events, consumed by the TUI frontend. The stdout sink prints
/// them, the channel sink serializes them as JSON-RPC notifications
/// (serve mode), and the events sink hands them to the TUI verbatim.
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// Streamed assistant text delta.
    Delta(String),
    /// Streamed model reasoning delta (display-only).
    Thinking(String),
    ToolStart { name: String, summary: String, input: Value },
    ToolEnd { name: String, ok: bool, first: String, images: usize },
    Info(String),
    Warn(String),
    /// A permission prompt was surfaced (TUI answers asynchronously).
    Ask { tool: String, arg: String },
    /// One run-to-completion turn finished (cumulative usage + context
    /// estimate vs window, for the TUI's context bar).
    TurnEnd { turns: u64, usage: crate::llm::Usage, ctx_est: u64, window: u64 },
    Banner { model: String, mode: String, session: Option<String> },
    /// Everything addressable as /<name> (slash commands, skills, command
    /// files) — palette source for the TUI.
    Commands { items: Vec<String> },
}

/// A permission request routed to a frontend that owns the terminal (TUI):
/// the agent blocks on the oneshot until the user answers.
pub struct ApprovalRequest {
    pub tool: String,
    pub arg: String,
    pub respond: tokio::sync::oneshot::Sender<ApprovalAnswer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalAnswer {
    /// Allow this one call.
    Once,
    /// Allow and remember a session rule (like REPL "always").
    Always,
    Deny,
}

#[derive(Debug, Clone)]
pub struct Ui {
    pub quiet: bool,
    /// serve mode: events are emitted as JSON-RPC notification lines.
    pub channel: Option<UnboundedSender<String>>,
    /// tui mode: events are emitted typed for the frontend to render.
    pub events: Option<UnboundedSender<UiEvent>>,
    at_line_start: bool,
    /// A thinking run just printed without a trailing newline; the next
    /// real output starts on a fresh line.
    thinking_open: bool,
    /// When true, all output goes to stderr instead of stdout.
    use_stderr: bool,
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
        Ui { quiet: false, channel: None, events: None, at_line_start: true, thinking_open: false, use_stderr: false }
    }

    pub fn quiet() -> Self {
        Ui { quiet: true, channel: None, events: None, at_line_start: true, thinking_open: false, use_stderr: false }
    }

    /// Redirect all output to stderr (used by autonomous mode so stdout
    /// stays clean for the final answer and terminal wrapping is avoided).
    pub fn redirect_to_stderr(&mut self) {
        self.use_stderr = true;
    }

    fn out_to(&self, s: &str) {
        if self.use_stderr {
            eprintln!("{s}");
        } else {
            println!("{s}");
        }
    }

    /// serve mode: all events become JSON notification lines on the channel.
    pub fn channel(tx: UnboundedSender<String>) -> Self {
        Ui { quiet: true, channel: Some(tx), events: None, at_line_start: true, thinking_open: false, use_stderr: false }
    }

    /// tui mode: all events are sent typed on the channel.
    pub fn events(tx: UnboundedSender<UiEvent>) -> Self {
        Ui { quiet: true, channel: None, events: Some(tx), at_line_start: true, thinking_open: false, use_stderr: false }
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
        if let Some(tx) = &self.events {
            let _ = tx.send(UiEvent::Delta(s.to_string()));
            return;
        }
        if let Some(tx) = &self.channel {
            notify(tx, "mh/turn.delta", &format!("{{\"text\":{}}}", serde_json::json!(s)));
            return;
        }
        if self.quiet || self.use_stderr {
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
        if let Some(tx) = &self.events {
            let _ = tx.send(UiEvent::Thinking(s.to_string()));
            return;
        }
        if let Some(tx) = &self.channel {
            notify(tx, "mh/turn.thinking", &format!("{{\"text\":{}}}", serde_json::json!(s)));
            return;
        }
        if self.quiet || self.use_stderr {
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

    pub fn tool_start(&mut self, name: &str, summary: &str, input: &Value) {
        if let Some(tx) = &self.events {
            let _ = tx.send(UiEvent::ToolStart {
                name: name.to_string(),
                summary: summary.to_string(),
                input: input.clone(),
            });
            return;
        }
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
        if self.use_stderr {
            let label = if summary.is_empty() { name.to_string() } else { format!("{name}({summary})") };
            eprintln!("  > {label}");
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
        let first = result.content.lines().next().unwrap_or("").chars().take(120).collect::<String>();
        if let Some(tx) = &self.events {
            let _ = tx.send(UiEvent::ToolEnd {
                name: name.to_string(),
                ok: !result.is_error,
                first,
                images: result.images.len(),
            });
            return;
        }
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
        if self.use_stderr {
            let first = result.content.lines().next().unwrap_or("").chars().take(100).collect::<String>();
            let mark = if result.is_error { "!" } else { "ok" };
            eprintln!("    {mark} {first}");
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
        if let Some(tx) = &self.events {
            let _ = tx.send(UiEvent::Ask { tool: name.to_string(), arg: arg.to_string() });
            return;
        }
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
        if let Some(tx) = &self.events {
            let _ = tx.send(UiEvent::Info(s.to_string()));
            return;
        }
        if let Some(tx) = &self.channel {
            notify(tx, "mh/info", &format!("{{\"text\":{}}}", serde_json::json!(s)));
            return;
        }
        if self.use_stderr {
            eprintln!("== {s}");
            return;
        }
        self.ensure_newline();
        self.out(&format!("== {s}"));
    }

    pub fn warn(&mut self, s: &str) {
        if let Some(tx) = &self.events {
            let _ = tx.send(UiEvent::Warn(s.to_string()));
            return;
        }
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

    /// Tell the TUI which /<name> commands exist (slash + skills + files).
    pub fn commands(&mut self, items: &[String]) {
        if let Some(tx) = &self.events {
            let _ = tx.send(UiEvent::Commands { items: items.to_vec() });
        }
    }

    pub fn turn_footer(&mut self, turns: u64, usage: &crate::llm::Usage, ctx_est: u64, window: u64) {
        if let Some(tx) = &self.events {
            let _ = tx.send(UiEvent::TurnEnd {
                turns,
                usage: *usage,
                ctx_est,
                window,
            });
            return;
        }
        if self.channel.is_some() || self.quiet {
            return;
        }
        self.ensure_newline();
        let cache = if usage.cache_read_tokens > 0 || usage.cache_creation_tokens > 0 {
            format!(
                " (cache: {} read, {} write)",
                format_tokens(usage.cache_read_tokens),
                format_tokens(usage.cache_creation_tokens)
            )
        } else {
            String::new()
        };
        println!(
            "-- turn {turns} | {} in{cache} / {} out (cumulative)",
            format_tokens(usage.input_tokens),
            format_tokens(usage.output_tokens),
        );
        println!();
    }

    pub fn banner(&mut self, model: &str, mode: &str, session: Option<&str>) {
        if let Some(tx) = &self.events {
            let _ = tx.send(UiEvent::Banner {
                model: model.to_string(),
                mode: mode.to_string(),
                session: session.map(str::to_string),
            });
            return;
        }
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

// REPL removed — TUI is the default interactive frontend.


#[derive(PartialEq)]
pub enum SlashResult {
    Continue,
    Quit,
}

/// Shared slash-command handler (REPL and TUI frontends). Unknown `/name`
/// falls through to skills and `.agents/commands` templates by running a
/// turn; output rides the Ui events either way.
pub async fn handle_slash(agent: &mut Agent, cmd: &str) -> SlashResult {
    let mut parts = cmd.split_whitespace();
    let name = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("");
    match name {
        "quit" | "exit" | "q" => return SlashResult::Quit,
        "help" | "h" | "?" => agent.ui.out(HELP),
        "clear" => agent.clear(),
        "compact" => {
            if let Err(e) = agent.compact_now(true).await {
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
                "requests: {} | tokens in: {} (cache: {} read, {} write) | tokens out: {} | turns: {} | compacted: {}",
                agent.state.requests,
                u.input_tokens,
                u.cache_read_tokens,
                u.cache_creation_tokens,
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
        "memory" => {
            let rest = cmd.strip_prefix(name).unwrap_or("").trim();
            if rest == "clear" {
                match agent.zero_mem.as_mut() {
                    Some(zm) => {
                        zm.clear();
                        agent.ui.info("zero-mem store cleared");
                    }
                    None => agent.ui.info("zero-mem is disabled"),
                }
            } else if let Some(q) = rest.strip_prefix("search ") {
                match agent.zero_mem.as_ref() {
                    Some(zm) => {
                        let hits = zm.retrieve(q, &std::collections::HashSet::new());
                        if hits.is_empty() {
                            agent.ui.info(&format!("no memories match '{q}'"));
                        } else {
                            for h in hits {
                                agent.ui.info(&format!(
                                    "({} days ago, {}) {}",
                                    h.when, h.role, h.snippet
                                ));
                            }
                        }
                    }
                    None => agent.ui.info("zero-mem is disabled"),
                }
            } else {
                match agent.zero_mem.as_ref() {
                    Some(zm) => agent.ui.info(&zm.status()),
                    None => agent.ui.info("zero-mem is disabled ([zero_mem] enabled = false)"),
                }
            }
        }
        "tools" => {
            let all_tools = agent.registry.names();
            let rest = cmd.strip_prefix(name).unwrap_or("").trim();
            let parts: Vec<&str> = rest.split_whitespace().collect();
            match parts.first().copied().unwrap_or("") {
                "toggle" | "on" | "off" => {
                    let action = parts[0];
                    let mut changed = Vec::new();
                    for tool_name in &parts[1..] {
                        let exists = all_tools.contains(tool_name);
                        if !exists {
                            agent.ui.warn(&format!("unknown tool '{tool_name}'"));
                            continue;
                        }
                        match action {
                            "on" => { agent.state.disabled_tools.remove(*tool_name); changed.push(format!("[x] {tool_name}")); }
                            "off" => { agent.state.disabled_tools.insert(tool_name.to_string()); changed.push(format!("[ ] {tool_name}")); }
                            _ => {
                                // toggle
                                if agent.state.disabled_tools.contains(*tool_name) {
                                    agent.state.disabled_tools.remove(*tool_name);
                                    changed.push(format!("[x] {tool_name}"));
                                } else {
                                    agent.state.disabled_tools.insert(tool_name.to_string());
                                    changed.push(format!("[ ] {tool_name}"));
                                }
                            }
                        }
                    }
                    if !changed.is_empty() {
                        agent.ui.info(&format!("{} tool(s) updated:
{}", changed.len(), changed.join("
")));
                    }
                }
                "reset" => {
                    agent.state.disabled_tools.clear();
                    agent.ui.info("all tools re-enabled");
                }
                _ => {
                    // List all tools with checkbox markers
                    let mut lines = vec!["Tools (toggle with /tools toggle <name>):".to_string(), String::new()];
                    for tool in &all_tools {
                        let mark = if agent.state.disabled_tools.contains(*tool) { "[ ]" } else { "[x]" };
                        lines.push(format!("  {mark} {tool}"));
                    }
                    let disabled = agent.state.disabled_tools.len();
                    if disabled > 0 {
                        lines.push(String::new());
                        lines.push(format!("{disabled} tool(s) disabled — saving ~{disabled} × 400 tok/request"));
                    }
                    agent.ui.out(&lines.join("
"));
                }
            }
        }
        "login" => {
            agent.ui.info("Setting up myharness credentials...");
            agent.ui.info("");
            agent.ui.info("Which provider are you using?");
            agent.ui.info("  1. Z.ai Coding Plan (subscription - recommended for GLM models)");
            agent.ui.info("  2. Standard Z.ai API (pay per token)");
            agent.ui.info("  3. Custom OpenAI-compatible endpoint");
            agent.ui.info("");
            agent.ui.info("Enter your choice [1-3]: ");
            let mut choice = String::new();
            let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
            if stdin.read_line(&mut choice).await.is_err() {
                agent.ui.warn("could not read input");
                return SlashResult::Continue;
            }
            let (name, default_url, default_model, protocol) = match choice.trim() {
                "1" => (
                    "coding-plan",
                    "https://api.z.ai/api/coding/paas/v4".to_string(),
                    "glm-5.3",
                    "openai",
                ),
                "2" => (
                    "standard-api",
                    "https://api.z.ai/api/anthropic".to_string(),
                    "glm-5.3",
                    "anthropic",
                ),
                "3" => {
                    agent.ui.info("Enter base URL (e.g. https://api.example.com/v1):");
                    let mut url = String::new();
                    let _ = stdin.read_line(&mut url).await;
                    agent.ui.info("Enter model name:");
                    let mut model = String::new();
                    let _ = stdin.read_line(&mut model).await;
                    (
                        "custom",
                        url.trim().to_string(),
                        if model.trim().is_empty() { "glm-5.3" } else { Box::leak(model.trim().to_string().into_boxed_str()) },
                        "openai",
                    )
                }
                _ => {
                    agent.ui.warn("invalid choice, aborting");
                    return SlashResult::Continue;
                }
            };
            agent.ui.info("Paste your API key:");
            let mut key = String::new();
            if stdin.read_line(&mut key).await.is_err() {
                agent.ui.warn("could not read key");
                return SlashResult::Continue;
            }
            let key = key.trim().to_string();
            if key.is_empty() {
                agent.ui.warn("empty key, aborting");
                return SlashResult::Continue;
            }
            let mut providers = std::collections::HashMap::new();
            providers.insert(
                name.to_string(),
                crate::auth::ProviderAuth {
                    api_key: key,
                    base_url: default_url,
                    model: default_model.to_string(),
                    protocol: protocol.to_string(),
                },
            );
            match crate::auth::save(name, providers) {
                Ok(()) => {
                    agent.ui.info(&format!(
                        "Credentials saved to {} (obfuscated at rest)",
                        crate::auth::auth_path().map(|p| p.display().to_string()).unwrap_or_default()
                    ));
                    match agent.reload_credentials() {
                        Ok(()) => agent.ui.info("Credentials loaded — you're ready to go."),
                        Err(e) => agent.ui.warn(&format!("credentials saved but reload failed: {e}. Restart myharness.")),
                    }
                }
                Err(e) => agent.ui.warn(&format!("failed to save credentials: {e}")),
            }
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
