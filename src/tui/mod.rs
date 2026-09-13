//! The custom TUI frontend: `myharness tui`. One column, quiet chrome —
//! the transcript is the product, chrome serves it.
//!
//! Architecture (informed by mining Codex/OpenCode/pi/Claude Code — see
//! harness-re/IDEAS.md): an actor-shaped split where the **worker task**
//! owns the Agent (turns run to completion there) and the **frontend
//! loop** owns the terminal, merging four streams: crossterm input,
//! typed UiEvents, permission requests, and a completion signal.
//! Steering (pi's two-tier queue): Enter while a turn runs queues the
//! message; the agent injects it after the current tool batch, or starts
//! a follow-up round if the turn already finished.

pub mod input;
pub mod render;

use crate::agent::Agent;
use crate::ui::{handle_slash, ApprovalAnswer, SlashResult, Ui, UiEvent};
use anyhow::Result;
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEvent,
    KeyEventKind, KeyModifiers,
};
use crossterm::{execute, terminal::*};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Terminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// ASCII spinner (house style: every console, every font).
pub(crate) const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];
/// Rotating status verbs while the agent works (our own words — the idea
/// is Claude Code's, the vocabulary is ours).
pub(crate) const VERBS: [&str; 10] = [
    "thinking", "reading", "writing", "planning", "building",
    "testing", "pondering", "wiring", "polishing", "deliberating",
];

/// One of the two select-popups: reverse history search (ctrl+r) or the
/// command palette (ctrl+p) — same widget, different accept behavior.
pub struct Overlay {
    pub palette: bool,
    pub query: String,
    pub items: Vec<String>,
    pub sel: usize,
}

impl Overlay {
    fn filtered(&self) -> Vec<&String> {
        let q = self.query.to_lowercase();
        self.items
            .iter()
            .filter(|i| q.is_empty() || i.to_lowercase().contains(&q))
            .collect()
    }
}

enum WorkerMsg {
    Submit(String),
    Slash(String),
    Quit,
}

#[derive(Debug, Clone, Default)]
pub enum ToolState {
    #[default]
    Pending,
    Ok(String),
    Err(String),
    /// The call was stopped before running (permission/hook) — distinct
    /// from a failed tool (opencode's denied-vs-failed distinction).
    Denied(String),
}

#[derive(Debug, Clone)]
pub enum Item {
    User(String),
    Steered(String),
    Assistant(String),
    Thinking { text: String, closed: bool, secs: u64 },
    Tool {
        name: String,
        summary: String,
        state: ToolState,
        detail: Option<render::ToolDetail>,
    },
    System { text: String, warn: bool },
    TurnMeta { secs: f64, in_tok: u64, out_tok: u64 },
    Compaction,
}

/// A permission request being shown as a modal; `respond` answers it.
/// "always" is two-stage (opencode): the second keypress confirms the
/// exact pattern that would be remembered.
pub struct Modal {
    pub tool: String,
    pub arg: String,
    pub respond: tokio::sync::oneshot::Sender<ApprovalAnswer>,
    /// Some(pattern) once the user pressed `a` — shown for confirmation.
    pub confirm_always: Option<String>,
}

#[derive(PartialEq)]
enum Flow {
    Continue,
    Quit,
}

pub struct App {
    pub items: Vec<Item>,
    pub input: input::Input,
    pub busy: bool,
    pub streaming: bool,
    pub interrupting: bool,
    pub spinner_frame: usize,
    pub follow: bool,
    pub scroll: u16,
    pub steered_count: usize,
    pub model: String,
    pub mode: String,
    pub session: Option<String>,
    pub turns: u64,
    pub ctx_pct: u8,
    pub modal: Option<Modal>,
    pub overlay: Option<Overlay>,
    pub commands: Vec<String>,
    pub history_path: Option<std::path::PathBuf>,
    pub quit_pending: Option<Instant>,
    pub turn_started: Option<Instant>,
    pub thinking_started: Option<Instant>,
    pub no_color: bool,
    pub size: ratatui::layout::Rect,
}

impl App {
    fn new() -> Self {
        App {
            items: Vec::new(),
            input: input::Input::new(),
            busy: false,
            streaming: false,
            interrupting: false,
            spinner_frame: 0,
            follow: true,
            scroll: 0,
            steered_count: 0,
            model: String::new(),
            mode: String::new(),
            session: None,
            turns: 0,
            ctx_pct: 0,
            modal: None,
            overlay: None,
            commands: Vec::new(),
            history_path: None,
            quit_pending: None,
            turn_started: None,
            thinking_started: None,
            no_color: std::env::var_os("NO_COLOR").is_some(),
            size: ratatui::layout::Rect::default(),
        }
    }

    pub fn input_height(&self) -> usize {
        let width = self.size.width.saturating_sub(4) as usize; // borders + gutter
        let (lines, _) = self.input.render(width.max(8));
        2 + lines.len().min(6)
    }

    pub fn dim(&self) -> Style {
        Style::default().fg(if self.no_color { Color::Reset } else { Color::DarkGray })
    }

    pub fn accent(&self) -> Style {
        if self.no_color {
            Style::default()
        } else {
            Style::default().fg(Color::Cyan)
        }
    }

    pub fn user_style(&self) -> Style {
        if self.no_color {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        }
    }

    fn close_thinking(&mut self) {
        if let Some(Item::Thinking { closed, secs, .. }) = self.items.last_mut() {
            if !*closed {
                *closed = true;
                *secs = self
                    .thinking_started
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0);
                self.thinking_started = None;
            }
        }
    }

    fn end_streaming(&mut self) {
        self.close_thinking();
        if self.streaming {
            self.streaming = false;
        }
    }

    fn apply_ui(&mut self, ev: UiEvent) {
        match ev {
            UiEvent::Delta(t) => {
                self.end_streaming(); // freeze any prior assistant block
                match self.items.last_mut() {
                    Some(Item::Assistant(prev)) => prev.push_str(&t),
                    _ => self.items.push(Item::Assistant(t)),
                }
                self.streaming = true;
                self.interrupting = false;
            }
            UiEvent::Thinking(t) => match self.items.last_mut() {
                Some(Item::Thinking { text, closed: false, .. }) => text.push_str(&t),
                _ => {
                    self.end_streaming();
                    self.thinking_started = Some(Instant::now());
                    self.items.push(Item::Thinking { text: t, closed: false, secs: 0 });
                }
            },
            UiEvent::ToolStart { name, summary, input: json } => {
                self.end_streaming();
                let detail = render::tool_detail(&name, &json);
                self.items.push(Item::Tool {
                    name,
                    summary,
                    state: ToolState::Pending,
                    detail,
                });
            }
            UiEvent::ToolEnd { name, ok, first, .. } => {
                let state = if ok {
                    ToolState::Ok(first)
                } else if first.starts_with("permission denied")
                    || first.starts_with("blocked by PreToolUse hook")
                {
                    ToolState::Denied(first)
                } else {
                    ToolState::Err(first)
                };
                // Update the newest Pending tool with this name.
                for item in self.items.iter_mut().rev() {
                    if let Item::Tool { name: n, state: s, .. } = item {
                        if *n == name && matches!(s, ToolState::Pending) {
                            *s = state;
                            break;
                        }
                    }
                }
            }
            UiEvent::Info(t) => {
                if t.starts_with("context compacted") {
                    self.end_streaming();
                    self.items.push(Item::Compaction);
                } else if t.starts_with("hint fired") {
                    // The hint text itself arrives as the next system event.
                } else {
                    self.items.push(Item::System { text: t, warn: false });
                }
            }
            UiEvent::Warn(t) => self.items.push(Item::System { text: t, warn: true }),
            UiEvent::Ask { .. } => {
                // Handled at the loop level (carries the oneshot responder).
            }
            UiEvent::TurnEnd { turns, usage, ctx_est, window } => {
                self.busy = false;
                self.streaming = false;
                self.interrupting = false;
                self.steered_count = 0;
                self.turns = turns;
                self.ctx_pct = if window > 0 {
                    ((ctx_est as f64 / window as f64) * 100.0).clamp(0.0, 100.0) as u8
                } else {
                    0
                };
                let secs = self
                    .turn_started
                    .take()
                    .map(|t| t.elapsed().as_secs_f64())
                    .unwrap_or(0.0);
                self.items.push(Item::TurnMeta {
                    secs,
                    in_tok: usage.input_tokens,
                    out_tok: usage.output_tokens,
                });
            }
            UiEvent::Banner { model, mode, session } => {
                self.model = model;
                self.mode = mode;
                self.session = session;
            }
            UiEvent::Commands { items } => self.commands = items,
        }
    }
}

/// The agent-side worker: owns the Agent, runs turns to completion, shares
/// nothing with the frontend except channels.
async fn worker_loop(
    mut agent: Agent,
    mut rx: UnboundedReceiver<WorkerMsg>,
    done_tx: UnboundedSender<bool>,
) {
    agent.ui.banner(
        &agent.model,
        agent.perms.mode.name(),
        agent.session.as_ref().map(|s| s.id.as_str()),
    );
    // Palette source: every /<name> addressable thing.
    {
        let mut items: Vec<String> = [
            "help", "quit", "clear", "compact", "mode", "model", "todos",
            "undo", "usage", "info", "session", "skills", "commands",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        items.extend(agent.state.skills.iter().map(|s| s.name.clone()));
        items.extend(agent.state.commands.iter().map(|c| c.name.clone()));
        agent.ui.commands(&items);
    }
    while let Some(msg) = rx.recv().await {
        match msg {
            WorkerMsg::Submit(text) => {
                if let Err(e) = agent.run_turn(&text).await {
                    agent.ui.warn(&format!("turn failed: {e}"));
                }
                let _ = done_tx.send(false);
            }
            WorkerMsg::Slash(cmd) => {
                let verdict = handle_slash(&mut agent, &cmd).await;
                let _ = done_tx.send(verdict == SlashResult::Quit);
            }
            WorkerMsg::Quit => break,
        }
    }
}

pub async fn run(mut agent: Agent) -> Result<()> {
    let (ui_tx, mut ui_rx) = unbounded_channel::<UiEvent>();
    let (approval_tx, mut approval_rx) = unbounded_channel();
    let (steer_tx, steer_rx) = unbounded_channel::<String>();
    let (worker_tx, worker_rx) = unbounded_channel::<WorkerMsg>();
    let (done_tx, mut done_rx) = unbounded_channel::<bool>();
    let cancel = Arc::clone(&agent.cancel);

    let agent_cfg = Arc::clone(&agent.cfg);
    agent.ui = Ui::events(ui_tx);
    agent.set_approval_tx(approval_tx);
    agent.set_steering(steer_rx);

    // Terminal setup: alternate screen, raw mode, bracketed paste. A panic
    // hook restores the terminal so a crash never leaves a broken console.
    let mut stdout = std::io::stdout();
    enable_raw_mode()?;
    crossterm::execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        default_panic(info);
    }));
    let mut term = Terminal::new(CrosstermBackend::new(stdout))?;

    let worker = tokio::spawn(worker_loop(agent, worker_rx, done_tx));
    let mut app = App::new();
    app.history_path = Some(agent_cfg.data_dir.join("history.txt"));
    if let Some(hp) = &app.history_path {
        if let Ok(raw) = std::fs::read_to_string(hp) {
            app.input.load_history(&raw);
        }
    }
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(120));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let flow = loop {
        let _ = term.draw(|f| render::draw(f, &mut app));
        tokio::select! {
            maybe_ev = events.next() => {
                let Some(Ok(ev)) = maybe_ev else { break Flow::Quit };
                if handle_term_event(&mut app, ev, &worker_tx, &steer_tx, &cancel) == Flow::Quit {
                    break Flow::Quit;
                }
            }
            Some(ev) = ui_rx.recv() => app.apply_ui(ev),
            Some(req) = approval_rx.recv() => {
                app.modal = Some(Modal {
                    tool: req.tool,
                    arg: req.arg,
                    respond: req.respond,
                    confirm_always: None,
                });
            }
            Some(exit) = done_rx.recv() => {
                if exit {
                    break Flow::Quit;
                }
            }
            _ = tick.tick() => {
                if app.busy || app.streaming {
                    app.spinner_frame = app.spinner_frame.wrapping_add(1);
                }
                // Confirm-to-quit window expiry.
                if app.quit_pending.is_some_and(|t| t.elapsed() > Duration::from_secs(2)) {
                    app.quit_pending = None;
                }
            }
        }
    };

    // Teardown: interrupt any in-flight turn, tell the worker to stop, wait
    // briefly, restore the terminal no matter what.
    cancel.store(true, Ordering::SeqCst);
    let _ = worker_tx.send(WorkerMsg::Quit);
    let _ = tokio::time::timeout(Duration::from_secs(3), worker).await;
    restore_terminal()?;
    if let Flow::Quit = flow {
        println!("bye");
    }
    Ok(())
}

fn restore_terminal() -> Result<()> {
    let mut stdout = std::io::stdout();
    let _ = execute!(stdout, DisableBracketedPaste);
    let _ = execute!(stdout, LeaveAlternateScreen);
    let _ = disable_raw_mode();
    Ok(())
}

fn handle_term_event(
    app: &mut App,
    ev: Event,
    worker_tx: &UnboundedSender<WorkerMsg>,
    steer_tx: &UnboundedSender<String>,
    cancel: &AtomicBool,
) -> Flow {
    let Event::Key(key) = ev else {
        if let Event::Paste(s) = ev {
            app.input.insert_str(&s);
        }
        return Flow::Continue;
    };
    if key.kind != KeyEventKind::Press {
        return Flow::Continue;
    }

    // The overlay (history search / palette) owns the keyboard while open.
    // Actions that close the overlay are applied after the borrow ends.
    let mut picked: Option<(String, bool)> = None;
    let mut close_overlay = false;
    if let Some(ov) = app.overlay.as_mut() {
        match key.code {
            KeyCode::Esc => close_overlay = true,
            KeyCode::Up => ov.sel = ov.sel.saturating_sub(1),
            KeyCode::Down => {
                let n = ov.filtered().len();
                ov.sel = (ov.sel + 1).min(n.saturating_sub(1));
            }
            KeyCode::Backspace => {
                ov.query.pop();
                ov.sel = 0;
            }
            KeyCode::Enter => {
                if let Some(item) = ov.filtered().get(ov.sel) {
                    picked = Some(((*item).clone(), ov.palette));
                }
                close_overlay = true;
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    close_overlay = true; // ctrl+anything bails out
                } else {
                    ov.query.push(c);
                    ov.sel = 0;
                }
            }
            _ => {}
        }
    }
    if close_overlay {
        app.overlay = None;
    }
    let picked_was_some = picked.is_some();
    if let Some((item, palette)) = picked {
        if palette {
            app.busy = true;
            app.turn_started = Some(Instant::now());
            app.items.push(Item::System { text: format!("/{item}"), warn: false });
            let _ = worker_tx.send(WorkerMsg::Slash(item));
        } else {
            // History search: load the entry into the editor.
            app.input = input::Input::new();
            app.input.insert_str(&item);
        }
    }
    if close_overlay || picked_was_some {
        return Flow::Continue;
    }

    // The permission modal swallows everything except its own answers.
    // "always" is two-stage: show the exact pattern, confirm on second key.
    if let Some(mut modal) = app.modal.take() {
        let answer = match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if modal.confirm_always.is_some() {
                    Some(ApprovalAnswer::Always)
                } else {
                    Some(ApprovalAnswer::Once)
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                if modal.confirm_always.is_some() {
                    Some(ApprovalAnswer::Always)
                } else {
                    modal.confirm_always =
                        Some(crate::agent::always_pattern(&modal.tool, &modal.arg));
                    app.modal = Some(modal);
                    return Flow::Continue;
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                if modal.confirm_always.is_some() {
                    modal.confirm_always = None; // back to the main question
                    app.modal = Some(modal);
                    return Flow::Continue;
                }
                Some(ApprovalAnswer::Deny)
            }
            KeyCode::Esc => {
                if modal.confirm_always.is_some() {
                    modal.confirm_always = None;
                    app.modal = Some(modal);
                    return Flow::Continue;
                }
                Some(ApprovalAnswer::Deny)
            }
            KeyCode::Enter if modal.confirm_always.is_some() => Some(ApprovalAnswer::Always),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(ApprovalAnswer::Deny)
            }
            _ => {
                app.modal = Some(modal);
                return Flow::Continue;
            }
        };
        if let Some(a) = answer {
            let _ = modal.respond.send(a);
        }
        return Flow::Continue;
    }

    // Ctrl+C: interrupt a running turn, otherwise double-press to quit.
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char('c') = key.code {
            if app.busy {
                cancel.store(true, Ordering::SeqCst);
                app.interrupting = true;
            } else if app.quit_pending.is_some() {
                return Flow::Quit;
            } else {
                app.quit_pending = Some(Instant::now());
            }
            return Flow::Continue;
        }
        return ctrl_key(app, key, cancel);
    }

    match key.code {
        KeyCode::Enter if key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SHIFT) => {
            app.input.newline();
        }
        KeyCode::Enter => {
            let Some(text) = app.input.submit() else {
                return Flow::Continue;
            };
            if let Some(hp) = &app.history_path {
                let _ = std::fs::create_dir_all(hp.parent().unwrap_or(std::path::Path::new(".")));
                let _ = std::fs::write(hp, app.input.history_text());
            }
            if text == "/login" {
                app.items.push(Item::System {
                    text: "/login must run from the terminal: exit this TUI (ctrl+c) and run: myharness login".to_string(),
                    warn: true,
                });
                return Flow::Continue;
            }
            if app.busy {
                // Steering (pi): the agent injects this after the current
                // tool batch, or as a follow-up round if it already ended.
                let _ = steer_tx.send(text.clone());
                app.steered_count += 1;
                app.items.push(Item::Steered(text));
            } else if let Some(cmd) = text.strip_prefix('/') {
                app.busy = true;
                app.turn_started = Some(Instant::now());
                app.items.push(Item::System {
                    text: format!("/{cmd}"),
                    warn: false,
                });
                let _ = worker_tx.send(WorkerMsg::Slash(cmd.to_string()));
            } else {
                app.busy = true;
                app.turn_started = Some(Instant::now());
                app.items.push(Item::User(text.clone()));
                let _ = worker_tx.send(WorkerMsg::Submit(text));
            }
        }
        KeyCode::Esc => {
            if app.busy {
                cancel.store(true, Ordering::SeqCst);
                app.interrupting = true;
            } else {
                app.input = input::Input::new();
            }
        }
        KeyCode::Backspace => {
            app.input.backspace();
        }
        KeyCode::Delete => {
            app.input.delete();
        }
        KeyCode::Left => {
            app.input.left();
        }
        KeyCode::Right => {
            app.input.right();
        }
        KeyCode::Home => {
            app.input.home();
        }
        KeyCode::End => {
            app.input.end();
            app.follow = true;
            app.scroll = 0;
        }
        KeyCode::Up => {
            if app.input.is_single_line() {
                app.input.history_prev();
            } else {
                app.input.line_up();
            }
        }
        KeyCode::Down => {
            if app.input.is_single_line() {
                app.input.history_next();
            } else {
                app.input.line_down();
            }
        }
        KeyCode::PageUp => {
            app.follow = false;
            let page = app.size.height / 2;
            app.scroll = app.scroll.saturating_add(page.max(1));
        }
        KeyCode::PageDown => {
            let page = app.size.height / 2;
            app.scroll = app.scroll.saturating_sub(page.max(1));
            if app.scroll == 0 {
                app.follow = true;
            }
        }
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.follow = true;
            app.scroll = 0;
        }
        KeyCode::Char(c) => {
            app.input.insert_str(&c.to_string());
        }
        _ => {}
    }
    Flow::Continue
}

fn ctrl_key(app: &mut App, key: KeyEvent, _cancel: &AtomicBool) -> Flow {
    match key.code {
        KeyCode::Char('r') => {
            let items: Vec<String> = app
                .input
                .history_text()
                .lines()
                .rev()
                .map(str::to_string)
                .collect();
            app.overlay = Some(Overlay { palette: false, query: String::new(), items, sel: 0 });
        }
        KeyCode::Char('p') => {
            app.overlay = Some(Overlay {
                palette: true,
                query: String::new(),
                items: app.commands.clone(),
                sel: 0,
            });
        }
        KeyCode::Char('l') => {
            // Full redraw happens on the next draw anyway; nothing to do
            // beyond forcing a resize check.
        }
        KeyCode::Char('w') => {
            app.input.delete_word_left();
        }
        KeyCode::Char('u') => {
            app.input.clear_to_line_start();
        }
        KeyCode::Backspace => {
            app.input.delete_word_left();
        }
        _ => {}
    }
    Flow::Continue
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_end_marks_denied_distinct_from_error() {
        let mut app = App::new();
        app.apply_ui(UiEvent::ToolStart {
            name: "bash".into(),
            summary: "rm -rf /".into(),
            input: serde_json::json!({"command": "rm -rf /"}),
        });
        app.apply_ui(UiEvent::ToolEnd {
            name: "bash".into(),
            ok: false,
            first: "permission denied by user".into(),
            images: 0,
        });
        match &app.items[0] {
            Item::Tool { state, .. } => assert!(matches!(state, ToolState::Denied(_))),
            other => panic!("{other:?}"),
        }

        let mut app = App::new();
        app.apply_ui(UiEvent::ToolStart {
            name: "bash".into(),
            summary: "x".into(),
            input: serde_json::json!({}),
        });
        app.apply_ui(UiEvent::ToolEnd {
            name: "bash".into(),
            ok: false,
            first: "command timed out".into(),
            images: 0,
        });
        match &app.items[0] {
            Item::Tool { state, .. } => assert!(matches!(state, ToolState::Err(_))),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn deltas_append_to_open_assistant_block() {
        let mut app = App::new();
        app.apply_ui(UiEvent::Delta("hello ".into()));
        app.apply_ui(UiEvent::Delta("world".into()));
        assert_eq!(app.items.len(), 1);
        app.apply_ui(UiEvent::ToolStart { name: "ls".into(), summary: String::new(), input: serde_json::json!({}) });
        app.apply_ui(UiEvent::Delta("more".into()));
        assert_eq!(app.items.len(), 3); // assistant, tool, assistant
    }

    #[test]
    fn thinking_collapses_to_one_line() {
        let mut app = App::new();
        app.apply_ui(UiEvent::Thinking("deep ".into()));
        app.apply_ui(UiEvent::Thinking("thoughts".into()));
        app.apply_ui(UiEvent::Delta("answer".into()));
        match &app.items[0] {
            Item::Thinking { closed, .. } => assert!(closed),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn compaction_renders_as_rule() {
        let mut app = App::new();
        app.apply_ui(UiEvent::Info("context compacted: kept 6 recent messages".into()));
        assert!(matches!(app.items.last(), Some(Item::Compaction)));
    }

    #[test]
    fn turn_end_sets_context_percent_and_clears_busy() {
        let mut app = App::new();
        app.busy = true;
        app.turn_started = Some(Instant::now());
        app.apply_ui(UiEvent::TurnEnd {
            turns: 3,
            usage: crate::llm::Usage::default(),
            ctx_est: 100_000,
            window: 200_000,
        });
        assert!(!app.busy);
        assert_eq!(app.ctx_pct, 50);
        assert!(matches!(app.items.last(), Some(Item::TurnMeta { .. })));
    }

    #[test]
    fn transcript_lines_render_user_and_diff() {
        let mut app = App::new();
        app.apply_ui(UiEvent::Info("hello world".into()));
        app.items.push(Item::User("fix it".into()));
        app.items.push(Item::Tool {
            name: "edit_file".into(),
            summary: "src/lib.rs".into(),
            state: ToolState::Pending,
            detail: Some(render::ToolDetail::Diff {
                path: "src/lib.rs".into(),
                minus: vec!["old line".into()],
                plus: vec!["new line".into()],
            }),
        });
        let lines = render::build_transcript(&app, 80);
        let joined: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("== hello world"), "{joined}");
        assert!(joined.contains("you > fix it"), "{joined}");
        assert!(joined.contains("- old line"), "{joined}");
        assert!(joined.contains("+ new line"), "{joined}");
    }

    #[test]
    fn wrap_respects_width_and_long_words() {
        let out = render::wrap("aaa bbb ccc", 7);
        assert!(out.iter().all(|l| l.chars().count() <= 7), "{out:?}");
        assert_eq!(out.len(), 2, "{out:?}");
        // Words longer than the width are hard-split; wrap() clamps the
        // width to a minimum of 8.
        let hard = render::wrap("aaaaaaaaaaaaaaaaaaaa", 4);
        assert!(hard.iter().all(|l| l.chars().count() <= 8), "{hard:?}");
        assert_eq!(hard, vec!["aaaaaaaa", "aaaaaaaa", "aaaa"]);
        assert_eq!(render::wrap("", 10), vec![String::new()]);
    }

    /// Full-frame smoke test on ratatui's TestBackend: the layout must
    /// render without panicking and carry the transcript's markers.
    #[test]
    fn frame_renders_transcript_status_and_input() {
        let mut app = App::new();
        app.model = "mock-model".into();
        app.mode = "ask".into();
        app.apply_ui(UiEvent::Info("session start".into()));
        app.items.push(Item::User("fix the test".into()));
        app.apply_ui(UiEvent::ToolStart {
            name: "bash".into(),
            summary: "cargo test".into(),
            input: serde_json::json!({"command": "cargo test"}),
        });
        app.apply_ui(UiEvent::ToolEnd {
            name: "bash".into(),
            ok: true,
            first: "test result: ok".into(),
            images: 0,
        });
        app.apply_ui(UiEvent::Delta("all green".into()));
        app.apply_ui(UiEvent::TurnEnd {
            turns: 1,
            usage: crate::llm::Usage::default(),
            ctx_est: 10_000,
            window: 200_000,
        });
        app.input.insert_str("next task");

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        term.draw(|f| render::draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer();
        let mut screen = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                screen.push_str(buf[(x, y)].symbol());
            }
            screen.push('\n');
        }
        for expected in ["you > fix the test", "bash  cargo test", "ok  test result", "all green", "turn done", "mock-model", "> next task"] {
            assert!(screen.contains(expected), "missing {expected:?} in:\n{screen}");
        }
    }
}
