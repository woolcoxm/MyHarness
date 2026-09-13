//! TUI rendering: turns the App state into ratatui widgets.
//!
//! Design language: dark synthwave — deep navy background, electric cyan
//! for accents, magenta for highlights, soft green for success. Bordered
//! blocks with rounded corners, section headers, and color-coded content.

use super::{App, Modal};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;
use serde_json::Value;

// ── theme ───────────────────────────────────────────────────────────────────

/// Theme colors — dark synthwave palette.
pub mod theme {
    use ratatui::style::{Color, Modifier, Style};

    pub const BG: Color = Color::Rgb(13, 17, 23);
    pub const SURFACE: Color = Color::Rgb(22, 27, 34);
    pub const BORDER: Color = Color::Rgb(48, 54, 61);
    pub const BORDER_HI: Color = Color::Rgb(88, 166, 255);
    pub const TEXT: Color = Color::Rgb(201, 209, 217);
    pub const TEXT_DIM: Color = Color::Rgb(110, 118, 129);
    pub const ACCENT: Color = Color::Rgb(88, 166, 255); // blue
    pub const ACCENT2: Color = Color::Rgb(188, 140, 255); // purple
    pub const GREEN: Color = Color::Rgb(63, 185, 80);
    pub const RED: Color = Color::Rgb(248, 81, 73);
    pub const YELLOW: Color = Color::Rgb(210, 153, 34);
    pub const MAGENTA: Color = Color::Rgb(219, 97, 162);
    pub const CYAN: Color = Color::Rgb(86, 208, 213);
    pub const ORANGE: Color = Color::Rgb(255, 166, 87);

    pub fn text() -> Style { Style::default().fg(TEXT) }
    pub fn dim() -> Style { Style::default().fg(TEXT_DIM) }
    pub fn accent() -> Style { Style::default().fg(ACCENT) }
    pub fn accent_bold() -> Style { Style::default().fg(ACCENT).add_modifier(Modifier::BOLD) }
    pub fn green() -> Style { Style::default().fg(GREEN) }
    pub fn red() -> Style { Style::default().fg(RED) }
    pub fn yellow() -> Style { Style::default().fg(YELLOW) }
    pub fn magenta() -> Style { Style::default().fg(MAGENTA) }
    pub fn cyan() -> Style { Style::default().fg(CYAN) }
    pub fn bold() -> Style { Style::default().add_modifier(Modifier::BOLD) }
}

/// Extra rendering data for one tool call (a small diff preview).
#[derive(Debug, Clone)]
pub enum ToolDetail {
    Diff { path: String, minus: Vec<String>, plus: Vec<String> },
}

const MAX_DIFF_LINES: usize = 40;

pub fn tool_detail(name: &str, input: &Value) -> Option<ToolDetail> {
    let path = input.get("path").and_then(Value::as_str).unwrap_or("").to_string();
    let cap = |s: &str| -> Vec<String> {
        s.lines().take(MAX_DIFF_LINES).map(str::to_string).collect()
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
        Constraint::Length(1),  // header
        Constraint::Min(1),     // transcript
        Constraint::Length(1),  // status
        Constraint::Length(app.input_height().clamp(3, 10) as u16),
        Constraint::Length(1),  // key hints
    ])
    .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_transcript(f, app, chunks[1]);
    draw_status(f, app, chunks[2]);
    draw_input(f, app, chunks[3]);
    draw_keys(f, app, chunks[4]);
    if app.overlay.is_some() {
        draw_overlay(f, app, chunks[3]);
    }
    if app.modal.is_some() {
        draw_modal(f, app);
    }
}

// ── header ──────────────────────────────────────────────────────────────────

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let bg = if app.no_color { Color::Reset } else { theme::BG };
    let block = Block::default().style(Style::default().bg(bg));
    f.render_widget(block, area);

    let mut spans: Vec<Span> = Vec::new();
    // Logo
    if app.no_color {
        spans.push(Span::raw(" myharness "));
    } else {
        spans.push(Span::styled(" ⚡ ", Style::default().fg(theme::YELLOW)));
        spans.push(Span::styled("my", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled("harness", Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(" ", Style::default()));
    }
    // Mode badge
    let mode_color = match app.mode.as_str() {
        "yolo" => theme::RED,
        "plan" => theme::CYAN,
        "auto-edit" => theme::GREEN,
        _ => theme::ACCENT,
    };
    spans.push(Span::styled(
        format!(" {} ", app.mode),
        Style::default().fg(if app.no_color { Color::Reset } else { theme::BG })
            .bg(if app.no_color { Color::Reset } else { mode_color })
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw("  "));
    // Session info
    if let Some(sess) = &app.session {
        let short: String = sess.chars().take(12).collect();
        spans.push(Span::styled(short, app.dim()));
        spans.push(Span::raw("  "));
    }
    // Right-aligned: model + context
    let model_str = format!("{} ", app.model);
    let ctx_str = format!("ctx {}%", app.ctx_pct);
    let right_len = model_str.len() + ctx_str.len() + 4;
    let padding = area.width.saturating_sub(spans.iter().map(|s| s.content.len() as u16).sum::<u16>()).saturating_sub(right_len as u16);
    spans.push(Span::raw(" ".repeat(padding as usize)));

    let ctx_style = if app.no_color || app.ctx_pct < 75 {
        theme::dim()
    } else if app.ctx_pct < 90 {
        Style::default().fg(theme::YELLOW)
    } else {
        Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)
    };
    spans.push(Span::styled(ctx_str, ctx_style));
    spans.push(Span::styled(" │ ", Style::default().fg(theme::BORDER)));
    spans.push(Span::styled(model_str, app.accent()));

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── transcript ──────────────────────────────────────────────────────────────

fn draw_transcript(f: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width as usize;
    let lines = build_transcript(app, width);
    let total = lines.len() as u16;
    let visible = area.height;
    let scroll = if app.follow {
        total.saturating_sub(visible)
    } else {
        app.scroll.min(total.saturating_sub(visible))
    };
    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
    if !app.follow && total > visible {
        let more = total - visible - scroll;
        let hint = format!(" ↑ {more} lines above · ctrl+g ↓ ");
        let spans = Line::styled(hint, Style::default().fg(theme::ACCENT).bg(theme::SURFACE));
        let bar = Rect { y: area.y, height: 1, ..area };
        f.render_widget(
            Paragraph::new(spans).alignment(ratatui::layout::Alignment::Center),
            bar,
        );
    }
}

// ── status bar ──────────────────────────────────────────────────────────────

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let bg = if app.no_color { Color::Reset } else { theme::SURFACE };
    let bg_block = Block::default().style(Style::default().bg(bg));
    let mut spans: Vec<Span> = Vec::new();
    let sep = Span::styled(" │ ", Style::default().fg(theme::BORDER));

    // Spinner + verb
    if app.busy {
        let spin = super::SPINNER[app.spinner_frame % super::SPINNER.len()];
        let verb = if app.streaming {
            "streaming"
        } else {
            super::VERBS[(app.spinner_frame / 10) % super::VERBS.len()]
        };
        let spin_style = if app.no_color { theme::text() } else { Style::default().fg(theme::CYAN).add_modifier(Modifier::BOLD) };
        spans.push(Span::styled(format!(" {spin} "), spin_style));
        spans.push(Span::styled(format!("{verb}..."), app.accent()));
        spans.push(sep.clone());
    }

    spans.push(Span::styled(format!("turn {}", app.turns), app.dim()));

    if app.steered_count > 0 {
        spans.push(sep.clone());
        spans.push(Span::styled(
            format!("◆ {} steered", app.steered_count),
            if app.no_color { theme::dim() } else { Style::default().fg(theme::MAGENTA) },
        ));
    }
    if app.interrupting {
        spans.push(sep.clone());
        spans.push(Span::styled("⏸ interrupting...", theme::red()));
    }
    if app.quit_pending.is_some() {
        spans.push(sep.clone());
        spans.push(Span::styled(
            "ctrl+c again to quit",
            if app.no_color { theme::text() } else { Style::default().fg(theme::YELLOW) },
        ));
    }

    f.render_widget(bg_block, area);
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── input ───────────────────────────────────────────────────────────────────

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let border_style = if app.modal.is_some() || app.overlay.is_some() {
        Style::default().fg(theme::BORDER)
    } else if app.busy {
        // Thinking: yellow border pulses
        let flash = app.spinner_frame % 10 < 7;
        Style::default().fg(if app.no_color {
            Color::Reset
        } else if flash {
            theme::YELLOW
        } else {
            theme::BORDER
        })
    } else {
        Style::default().fg(if app.no_color { Color::Reset } else { theme::ACCENT })
    };

    let title = if app.busy {
        " agent working — enter to steer, esc to interrupt "
    } else {
        " your prompt "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(Span::styled(title, Style::default().fg(if app.no_color { Color::Reset } else { theme::TEXT_DIM })));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let width = inner.width as usize;
    let (lines, (crow, ccol)) = app.input.render(width);
    let text: Vec<Line> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let prefix = if i == 0 { "❯ " } else { "  " };
            Line::from(vec![
                Span::styled(prefix, if app.no_color { Style::default() } else { Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD) }),
                Span::styled(l.clone(), if app.no_color { Style::default() } else { Style::default().fg(theme::TEXT) }),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(text), inner);
    let x = inner.x + ccol as u16 + 2;
    let y = inner.y + crow as u16;
    if x < inner.right() && y < inner.bottom() {
        f.set_cursor_position((x, y));
    }
}

// ── key hints ───────────────────────────────────────────────────────────────

fn draw_keys(f: &mut Frame, app: &App, area: Rect) {
    let bg = if app.no_color { Color::Reset } else { theme::BG };
    let block = Block::default().style(Style::default().bg(bg));
    f.render_widget(block, area);

    let hint = if app.modal.is_some() {
        " y allow · a always · n deny · esc back"
    } else if app.overlay.is_some() {
        " ↑↓ navigate · enter select · esc cancel · type to filter"
    } else if app.busy {
        " enter steer · alt+enter newline · esc interrupt"
    } else {
        " enter send · ctrl+r history · ctrl+p palette · pgup scroll"
    };
    let spans: Vec<Span> = hint
        .split(" · ")
        .enumerate()
        .flat_map(|(i, part)| {
            let mut v = Vec::new();
            if i > 0 {
                v.push(Span::styled(" · ", Style::default().fg(theme::BORDER)));
            }
            let (key, desc) = part.split_once(' ').unwrap_or((part, ""));
            v.push(Span::styled(
                key,
                if app.no_color { theme::text() } else { Style::default().fg(theme::ACCENT2) },
            ));
            if !desc.is_empty() {
                v.push(Span::styled(format!(" {desc}"), app.dim()));
            }
            v
        })
        .collect();
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── modal ───────────────────────────────────────────────────────────────────

fn draw_modal(f: &mut Frame, app: &App) {
    let Some(Modal { tool, arg, .. }) = &app.modal else { return };
    let area = centered(f.area(), 60, 14);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(
            format!(" ⚠ permission: {tool} "),
            if app.no_color { theme::text() } else { Style::default().fg(theme::YELLOW).add_modifier(Modifier::BOLD) },
        ))
        .border_style(if app.no_color { Style::default() } else { Style::default().fg(theme::YELLOW) });
    let inner = block.inner(area);
    f.render_widget(Clear, area);
    // Semi-transparent background
    let bg_block = Block::default().style(Style::default().bg(theme::SURFACE));
    f.render_widget(bg_block, area);
    f.render_widget(block, area);

    let width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    let wrapped = wrap(arg, width);
    for l in wrapped.into_iter().take(inner.height.saturating_sub(5) as usize) {
        lines.push(Line::styled(l, theme::text()));
    }
    lines.push(Line::from(""));
    if let Some(pattern) = &app.modal.as_ref().and_then(|m| m.confirm_always.clone()) {
        lines.push(Line::styled("remember for this session:", theme::dim()));
        lines.push(Line::styled(
            format!("  {tool}: {pattern}"),
            if app.no_color { theme::text() } else { Style::default().fg(theme::YELLOW) },
        ));
        lines.push(Line::styled(
            "[a/y] confirm always   [n/esc] back",
            if app.no_color { theme::text() } else { Style::default().fg(theme::GREEN) },
        ));
    } else {
        lines.push(Line::styled(
            "  [y] allow once    [a] always this session    [n] deny",
            if app.no_color { theme::text() } else { Style::default().fg(theme::CYAN) },
        ));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

// ── overlay (palette / history search) ─────────────────────────────────────

fn draw_overlay(f: &mut Frame, app: &App, area: Rect) {
    let Some(ov) = &app.overlay else { return };
    // Make the overlay a bit taller than the input box
    let overlay_area = Rect {
        y: area.y.saturating_sub(8),
        height: area.height + 10,
        ..area
    };
    let title = if ov.palette {
        " ◈ command palette "
    } else {
        " ⌕ history search "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(title, app.accent_bold()))
        .border_style(app.accent());
    let inner = block.inner(overlay_area);
    f.render_widget(Clear, overlay_area);
    let bg = Block::default().style(Style::default().bg(theme::SURFACE));
    f.render_widget(bg, overlay_area);
    f.render_widget(block, overlay_area);

    let mut lines: Vec<Line> = Vec::new();
    let filter_label = if ov.query.is_empty() {
        "type to filter...".to_string()
    } else {
        format!("❯ {}", ov.query)
    };
    lines.push(Line::styled(
        filter_label,
        if app.no_color { theme::text() } else { Style::default().fg(theme::CYAN) },
    ));
    lines.push(Line::from(""));
    let filtered = ov.filtered();
    let max_rows = (inner.height.saturating_sub(3)) as usize;
    let start = ov.sel.saturating_sub(max_rows.saturating_sub(1));
    for (i, item) in filtered.iter().enumerate().skip(start).take(max_rows) {
        let style = if i == ov.sel {
            if app.no_color {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD)
            }
        } else {
            theme::text()
        };
        let prefix = if ov.palette { "/" } else { "" };
        let icon = if i == ov.sel { "▸ " } else { "  " };
        let shown: String = item.chars().take((inner.width as usize).saturating_sub(6)).collect();
        lines.push(Line::styled(format!("{icon}{prefix}{shown}"), style));
    }
    if filtered.is_empty() {
        lines.push(Line::styled("  (no matches)", app.dim()));
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

// ── transcript ──────────────────────────────────────────────────────────────

pub fn build_transcript(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let push_wrapped = |prefix: &str, _prefix_style: Style, s: &str, style: Style, out: &mut Vec<Line<'static>>| {
        let full = format!("{prefix}{s}");
        for (i, l) in wrap(&full, width).into_iter().enumerate() {
            if i == 0 {
                out.push(Line::styled(l, style));
            } else {
                // Continuation lines get indent
                let indent = " ".repeat(prefix.chars().count());
                out.push(Line::styled(format!("{indent}{l}"), style));
            }
        }
    };

    for item in &app.items {
        use super::Item::*;
        match item {
            User(t) => {
                out.push(Line::from(""));
                push_wrapped("▌ ", app.user_style(), t, app.user_style(), &mut out);
            }
            Steered(t) => {
                push_wrapped("◆ ", if app.no_color { theme::dim() } else { Style::default().fg(theme::MAGENTA) }, t, app.dim(), &mut out);
            }
            Assistant(t) => {
                push_wrapped("", theme::text(), t, theme::text(), &mut out);
                out.push(Line::from(""));
            }
            Thinking { text, closed, secs } => {
                if *closed {
                    let words = text.split_whitespace().count();
                    out.push(Line::styled(
                        format!("  💭 {} words · {}s", words, secs),
                        app.dim(),
                    ));
                } else {
                    let style = if app.no_color { app.dim() } else { Style::default().fg(theme::ACCENT2) };
                    for l in wrap(&format!("  💭 {}", text), width) {
                        out.push(Line::styled(l, style));
                    }
                }
            }
            Tool { name, summary, state, detail } => {
                let (icon, tail, style) = match state {
                    super::ToolState::Pending => ("⠿", "...".to_string(), app.dim()),
                    super::ToolState::Ok(first) => {
                        ("✓", first.clone(), if app.no_color { Style::default() } else { Style::default().fg(theme::GREEN) })
                    }
                    super::ToolState::Err(first) => (
                        "✗",
                        first.clone(),
                        if app.no_color { Style::default() } else { Style::default().fg(theme::RED) },
                    ),
                    super::ToolState::Denied(first) => (
                        "⊘",
                        first.clone(),
                        if app.no_color { Style::default() } else { Style::default().fg(theme::TEXT_DIM) },
                    ),
                };
                let icon_style = match state {
                    super::ToolState::Pending => app.dim(),
                    super::ToolState::Ok(_) => if app.no_color { theme::text() } else { Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD) },
                    super::ToolState::Err(_) => if app.no_color { theme::text() } else { Style::default().fg(theme::RED).add_modifier(Modifier::BOLD) },
                    super::ToolState::Denied(_) => app.dim(),
                };
                let name_style = if app.no_color { theme::text() } else { Style::default().fg(theme::ACCENT) };
                out.push(Line::from(vec![
                    Span::styled(format!("  {icon} "), icon_style),
                    Span::styled(name.clone(), name_style),
                    Span::styled(
                        if summary.is_empty() { String::new() } else { format!(" {summary}") },
                        app.dim(),
                    ),
                    Span::styled(format!("  {tail}"), style),
                ]));
                if let Some(ToolDetail::Diff { path, minus, plus }) = detail {
                    out.push(Line::styled(
                        format!("    📄 {path}"),
                        if app.no_color { app.dim() } else { Style::default().fg(theme::CYAN) },
                    ));
                    for l in minus {
                        for w in wrap(&format!("    ─ {l}"), width) {
                            out.push(Line::styled(w, if app.no_color { app.dim() } else { Style::default().fg(Color::Rgb(248, 81, 73)) }));
                        }
                    }
                    for l in plus {
                        for w in wrap(&format!("    ┼ {l}"), width) {
                            out.push(Line::styled(w, if app.no_color { theme::text() } else { Style::default().fg(Color::Rgb(63, 185, 80)) }));
                        }
                    }
                }
            }
            System { text, warn } => {
                let (icon, style) = if *warn {
                    ("⚠", if app.no_color { theme::text() } else { Style::default().fg(theme::YELLOW) })
                } else {
                    ("·", app.dim())
                };
                push_wrapped(&format!("{icon} "), style, text, style, &mut out);
            }
            TurnMeta { secs, in_tok, out_tok } => {
                out.push(Line::styled(
                    format!(
                        "  ⏱ {}s · ↑{} ↓{} ────────────────────────────",
                        format_secs(*secs),
                        fmt_tokens(*in_tok),
                        fmt_tokens(*out_tok),
                    ),
                    app.dim(),
                ));
                out.push(Line::from(""));
            }
            Compaction => {
                out.push(Line::styled(
                    "  ═══════════ context compacted ═══════════",
                    if app.no_color { app.dim() } else { Style::default().fg(theme::ACCENT2) },
                ));
                out.push(Line::from(""));
            }
        }
    }
    out
}

fn format_secs(secs: f64) -> String {
    if secs < 60.0 {
        format!("{secs:.1}")
    } else {
        format!("{}:{:02}", (secs / 60.0) as u64, (secs % 60.0) as u64)
    }
}

fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Greedy word wrap.
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
