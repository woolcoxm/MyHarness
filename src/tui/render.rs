//! TUI rendering: turns the App state into ratatui widgets. Pure where
//! possible — `build_transcript` and the wrap/diff helpers are exercised
//! by TestBackend tests without a terminal.
//!
//! House style: ASCII glyphs everywhere, color as a secondary channel
//! (dropped entirely under NO_COLOR), one column, quiet chrome.

use super::{App, Modal};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use serde_json::Value;

/// Extra rendering data for one tool call (a small diff preview).
#[derive(Debug, Clone)]
pub enum ToolDetail {
    Diff { path: String, minus: Vec<String>, plus: Vec<String> },
}

/// Cap on diff lines rendered per tool call.
const MAX_DIFF_LINES: usize = 40;

pub fn tool_detail(name: &str, input: &Value) -> Option<ToolDetail> {
    let path = input.get("path").and_then(Value::as_str).unwrap_or("").to_string();
    let cap = |s: &str| -> Vec<String> {
        s.lines()
            .take(MAX_DIFF_LINES)
            .map(str::to_string)
            .collect()
    };
    match name {
        "edit_file" => {
            let old = input.get("old_string").and_then(Value::as_str).unwrap_or("");
            let new = input.get("new_string").and_then(Value::as_str).unwrap_or("");
            Some(ToolDetail::Diff { path, minus: cap(old), plus: cap(new) })
        }
        "write_file" => {
            let content = input.get("content").and_then(Value::as_str).unwrap_or("");
            Some(ToolDetail::Diff { path, minus: Vec::new(), plus: cap(content) })
        }
        _ => None,
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    app.size = f.area();
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(app.input_height().clamp(3, 10) as u16),
        Constraint::Length(1),
    ])
    .split(f.area());

    draw_transcript(f, app, chunks[0]);
    draw_status(f, app, chunks[1]);
    draw_input(f, app, chunks[2]);
    draw_keys(f, app, chunks[3]);
    if app.overlay.is_some() {
        draw_overlay(f, app, chunks[2]);
    }
    if app.modal.is_some() {
        draw_modal(f, app);
    }
}

/// History search / command palette: a filterable list sitting over the
/// input box, selection highlighted.
fn draw_overlay(f: &mut Frame, app: &App, area: Rect) {
    let Some(ov) = &app.overlay else { return };
    let title = if ov.palette { " palette (enter runs, esc cancels) " } else { " history (enter edits, esc cancels) " };
    let block = Block::default().borders(Borders::ALL).title(title).border_style(app.accent());
    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    let prompt = if ov.query.is_empty() { "filter: (all)" } else { &format!("filter: {query}", query = ov.query) };
    lines.push(Line::styled(prompt.to_string(), Style::default()));
    lines.push(Line::from(""));
    let filtered = ov.filtered();
    let max_rows = (inner.height.saturating_sub(3)) as usize;
    let start = ov.sel.saturating_sub(max_rows.saturating_sub(1));
    for (i, item) in filtered.iter().enumerate().skip(start).take(max_rows) {
        let style = if i == ov.sel {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        let shown: String = item.chars().take((inner.width as usize).saturating_sub(4)).collect();
        let prefix = if ov.palette { "/" } else { "" };
        lines.push(Line::styled(format!("{prefix}{shown}"), style));
    }
    if filtered.is_empty() {
        lines.push(Line::styled("(no matches)", app.dim()));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_transcript(f: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width as usize;
    let lines = build_transcript(app, width);
    let total = lines.len() as u16;
    let visible = area.height;
    // Follow-tail unless the user scrolled up.
    let scroll = if app.follow {
        total.saturating_sub(visible)
    } else {
        app.scroll.min(total.saturating_sub(visible))
    };
    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
    if !app.follow && total > visible {
        let more = total - visible - scroll;
        let hint = format!("-- {more} line(s) above · ctrl+g to follow --");
        let spans = Line::styled(hint, app.dim());
        // Drawn over the status bar's right side, where it can't collide
        // with the primary status content.
        let mut bar = Rect { y: area.bottom(), height: 1, ..area };
        bar.width = bar.width.saturating_sub(1);
        f.render_widget(
            Paragraph::new(spans).alignment(ratatui::layout::Alignment::Right),
            bar,
        );
    }
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let mut spans: Vec<Span> = Vec::new();
    let sep = Span::raw(" | ");
    spans.push(Span::styled(app.model.clone(), app.accent()));
    spans.push(Span::raw(format!(" {}", app.mode)));
    spans.push(sep.clone());
    spans.push(Span::raw(format!("turn {}", app.turns)));
    spans.push(sep.clone());
    let ctx = format!("ctx {}%", app.ctx_pct);
    let style = if app.no_color || app.ctx_pct < 75 {
        Style::default()
    } else if app.ctx_pct < 90 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    };
    spans.push(Span::styled(ctx, style));
    if app.busy {
        spans.push(sep.clone());
        let spin = super::SPINNER[app.spinner_frame % super::SPINNER.len()];
        let verb = if app.streaming {
            "streaming"
        } else {
            super::VERBS[(app.spinner_frame / 10) % super::VERBS.len()]
        };
        spans.push(Span::styled(format!("{spin} {verb}..."), app.accent()));
    }
    if app.steered_count > 0 {
        spans.push(sep.clone());
        spans.push(Span::styled(format!("{} queued", app.steered_count), app.dim()));
    }
    if app.interrupting {
        spans.push(sep.clone());
        spans.push(Span::styled("interrupting...", Style::default().fg(Color::Red)));
    }
    if app.quit_pending.is_some() {
        spans.push(Span::styled(
            "ctrl+c again to quit",
            Style::default().fg(if app.no_color { Color::Reset } else { Color::Yellow }),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let border = if app.modal.is_some() {
        Style::default()
    } else if app.busy {
        Style::default().fg(if app.no_color { Color::Reset } else { Color::Yellow })
    } else {
        Style::default()
    };
    let block = Block::default().borders(Borders::ALL).border_style(border);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let width = inner.width as usize;
    let (lines, (crow, ccol)) = app.input.render(width);
    let text: Vec<Line> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let prefix = if i == 0 { "> " } else { "  " };
            Line::from(format!("{prefix}{l}"))
        })
        .collect();
    f.render_widget(Paragraph::new(text), inner);
    // Place the terminal cursor at the editor position.
    let x = inner.x + ccol as u16 + 2; // + prompt gutter
    let y = inner.y + crow as u16;
    if x < inner.right() && y < inner.bottom() {
        f.set_cursor_position((x, y));
    }
}

fn draw_keys(f: &mut Frame, app: &App, area: Rect) {
    let hint = if app.modal.is_some() {
        "y allow once · a always (session) · n/esc deny"
    } else if app.busy {
        "enter steer · alt+enter newline · esc interrupt · ctrl+c quit"
    } else {
        "enter send · alt+enter newline · up/down history · pgup/pgdn scroll · ctrl+g follow · /commands"
    };
    f.render_widget(Paragraph::new(Line::styled(hint, app.dim())), area);
}

fn draw_modal(f: &mut Frame, app: &App) {
    let Some(Modal { tool, arg, .. }) = &app.modal else { return };
    let area = centered(f.area(), 60, 14);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" permission: {tool} "))
        .border_style(if app.no_color { Style::default() } else { Style::default().fg(Color::Yellow) });
    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);

    let width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    let wrapped = wrap(arg, width);
    for l in wrapped.into_iter().take(inner.height.saturating_sub(2) as usize) {
        lines.push(Line::styled(l, Style::default()));
    }
    if let Some(pattern) = &app.modal.as_ref().and_then(|m| m.confirm_always.clone()) {
        lines.push(Line::styled("remember for this session:", Style::default()));
        lines.push(Line::styled(
            format!("  {tool}: {pattern}"),
            if app.no_color { Style::default() } else { Style::default().fg(Color::Yellow) },
        ));
        lines.push(Line::styled(
            "[a/y] confirm always   [n/esc] back   (no other keys)",
            if app.no_color { Style::default() } else { Style::default().fg(Color::Cyan) },
        ));
    } else {
        lines.push(Line::styled(
            "[y] allow once   [a] always this session   [n] deny",
            if app.no_color { Style::default() } else { Style::default().fg(Color::Cyan) },
        ));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn centered(whole: Rect, pct_w: u16, max_h: u16) -> Rect {
    let w = (whole.width * pct_w / 100).max(30);
    let h = max_h.min(whole.height.saturating_sub(2));
    let x = whole.x + (whole.width.saturating_sub(w)) / 2;
    let y = whole.y + (whole.height.saturating_sub(h)) / 2;
    Rect { x, y, width: w, height: h }
}

/// The whole transcript as styled lines, pre-wrapped to `width`.
pub fn build_transcript(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let push_wrapped = |s: &str, style: Style, out: &mut Vec<Line<'static>>| {
        for l in wrap(s, width) {
            out.push(Line::styled(l, style));
        }
    };
    for item in &app.items {
        use super::Item::*;
        match item {
            User(t) => push_wrapped(&format!("you > {t}"), app.user_style(), &mut out),
            Steered(t) => push_wrapped(
                &format!("you + (steered) {t}"),
                app.dim(),
                &mut out,
            ),
            Assistant(t) => {
                push_wrapped(t, Style::default(), &mut out);
                out.push(Line::from(""));
            }
            Thinking { text, closed, secs } => {
                if *closed {
                    let words = text.split_whitespace().count();
                    out.push(Line::styled(
                        format!("~ thought ({words} words, {secs}s)"),
                        app.dim(),
                    ));
                } else {
                    for l in wrap(&format!("~ {text}"), width) {
                        out.push(Line::styled(l, app.dim()));
                    }
                }
            }
            Tool { name, summary, state, detail } => {
                let (mark, tail, style) = match state {
                    super::ToolState::Pending => ("*", "...".to_string(), app.dim()),
                    super::ToolState::Ok(first) => {
                        ("*", format!("ok  {first}"), Style::default())
                    }
                    super::ToolState::Err(first) => (
                        "!",
                        format!("ERR {first}"),
                        Style::default().fg(if app.no_color { Color::Reset } else { Color::Red }),
                    ),
                    // Denied is deliberately distinct from failed (opencode):
                    // the model was stopped, not the tool.
                    super::ToolState::Denied(first) => (
                        "x",
                        format!("denied  {first}"),
                        Style::default().fg(if app.no_color { Color::Reset } else { Color::DarkGray }),
                    ),
                };
                let head = if summary.is_empty() {
                    format!("  {mark} {name}")
                } else {
                    format!("  {mark} {name}  {summary}")
                };
                out.push(Line::styled(format!("{head}  {tail}"), style));
                if let Some(ToolDetail::Diff { path, minus, plus }) = detail {
                    out.push(Line::styled(format!("    {path}"), app.dim()));
                    for l in minus {
                        for w in wrap(&format!("    - {l}"), width) {
                            out.push(Line::styled(
                                w,
                                Style::default().fg(if app.no_color { Color::Reset } else { Color::Red }),
                            ));
                        }
                    }
                    for l in plus {
                        for w in wrap(&format!("    + {l}"), width) {
                            out.push(Line::styled(
                                w,
                                Style::default().fg(if app.no_color { Color::Reset } else { Color::Green }),
                            ));
                        }
                    }
                }
            }
            System { text, warn } => {
                let (p, style) = if *warn {
                    ("!!", Style::default().fg(if app.no_color { Color::Reset } else { Color::Yellow }))
                } else {
                    ("==", app.dim())
                };
                push_wrapped(&format!("{p} {text}"), style, &mut out);
            }
            TurnMeta { secs, in_tok, out_tok } => {
                out.push(Line::styled(
                    format!("-- turn done · {secs:.1}s · {in_tok} in / {out_tok} out --"),
                    app.dim(),
                ));
                out.push(Line::from(""));
            }
            Compaction => {
                out.push(Line::styled(
                    "--------------- context compacted ---------------",
                    app.dim(),
                ));
            }
        }
    }
    out
}

/// Greedy word wrap, falling back to hard char wrap for long words.
/// Empty input produces one empty line.
pub fn wrap(s: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    for raw in s.split('\n') {
        if raw.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in raw.split(' ') {
            // Hard-split words wider than the line.
            let mut pieces: Vec<String> = Vec::new();
            let mut cur = String::new();
            for c in word.chars() {
                if cur.chars().count() >= width {
                    pieces.push(std::mem::take(&mut cur));
                }
                cur.push(c);
            }
            pieces.push(cur);
            for (i, piece) in pieces.iter().enumerate() {
                if line.is_empty() {
                    line.push_str(piece);
                } else if line.chars().count() + 1 + piece.chars().count() <= width {
                    line.push(' ');
                    line.push_str(piece);
                } else {
                    out.push(std::mem::take(&mut line));
                    if i == 0 {
                        line.push_str(piece);
                    } else {
                        // continuation of a hard-split word
                        line.push_str(piece.trim_start());
                    }
                }
            }
        }
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}
