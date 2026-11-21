use std::path::Path;
use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, LineGauge, List, ListItem, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Focus};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [main, status] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(5)]).areas(frame.area());
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(main);

    draw_browser(frame, app, left);
    draw_queue(frame, app, right);
    draw_status(frame, app, status);
}

fn pane_block(title: Line<'static>, focused: bool) -> Block<'static> {
    let border = if focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(title)
}

fn highlight(focused: bool) -> Style {
    if focused {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    }
}

fn draw_browser(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Browser;
    let playing = app.queue.current();
    let inner_width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .browser
        .visible()
        .map(|e| {
            if e.is_dir {
                ListItem::new(format!("{}/", e.name)).style(Style::new().fg(Color::Blue).bold())
            } else {
                let style = if Some(&e.path) == playing {
                    Style::new().fg(Color::Green)
                } else {
                    Style::new()
                };
                ListItem::new(track_row(app, &e.path, "", inner_width)).style(style)
            }
        })
        .collect();

    let mut block = pane_block(
        Line::from(format!(
            " {} ",
            short_path(&app.browser.dir, area.width.saturating_sub(4) as usize)
        ))
        .bold(),
        focused,
    );
    if app.filter_mode || !app.browser.filter.is_empty() {
        let cursor = if app.filter_mode { "█" } else { "" };
        block = block.title_bottom(Line::from(vec![
            Span::raw(" /").fg(Color::Yellow),
            Span::raw(format!("{}{cursor} ", app.browser.filter)),
        ]));
    }
    if let Some(err) = &app.browser.error {
        frame.render_widget(Paragraph::new(err.as_str().red()).block(block), area);
        return;
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(highlight(focused));
    app.browser_state
        .select((app.browser.len() > 0).then_some(app.browser.cursor));
    frame.render_stateful_widget(list, area, &mut app.browser_state);
}

fn draw_queue(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Queue;
    let current = app.queue.current_index();
    let inner_width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .queue
        .tracks()
        .iter()
        .enumerate()
        .map(|(i, path)| {
            if Some(i) == current {
                ListItem::new(track_row(app, path, "▶ ", inner_width))
                    .style(Style::new().fg(Color::Green).bold())
            } else {
                ListItem::new(track_row(app, path, "  ", inner_width))
            }
        })
        .collect();

    let mut title = format!(" Queue ({}) ", app.queue.len());
    if let Some(i) = current {
        title = format!(" Queue ({}/{}) ", i + 1, app.queue.len());
    }
    let list = List::new(items)
        .block(pane_block(Line::from(title).bold(), focused))
        .highlight_style(highlight(focused));
    app.queue_state
        .select((!app.queue.is_empty()).then_some(app.queue_cursor));
    frame.render_stateful_widget(list, area, &mut app.queue_state);
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [title_area, gauge_area, info_area] =
        Layout::vertical([Constraint::Length(1); 3]).areas(inner);

    // Line 1: what is playing.
    let (icon, title, album) = match app.queue.current().filter(|_| app.player.is_loaded()) {
        Some(path) => {
            let icon = if app.player.is_paused() { "⏸" } else { "▶" };
            let album = app
                .meta(path)
                .and_then(|m| m.album.clone())
                .map(|a| format!("  ·  {a}"))
                .unwrap_or_default();
            (icon, app.track_name(path), album)
        }
        None => ("■", "Nothing playing".to_string(), String::new()),
    };
    frame.render_widget(
        Line::from(vec![
            Span::raw(format!(" {icon} ")).fg(Color::Green).bold(),
            Span::raw(title).bold(),
            Span::raw(album).fg(Color::DarkGray),
        ]),
        title_area,
    );

    // Line 2: progress.
    let pos = app.player.position();
    let duration = app.player.duration();
    let ratio = match duration {
        Some(d) if !d.is_zero() => (pos.as_secs_f64() / d.as_secs_f64()).clamp(0.0, 1.0),
        _ => 0.0,
    };
    let time = match duration {
        Some(d) => format!(" {} / {} ", fmt_time(pos), fmt_time(d)),
        None => format!(" {} ", fmt_time(pos)),
    };
    frame.render_widget(
        LineGauge::default()
            .ratio(ratio)
            .label(time)
            .filled_style(Style::new().fg(Color::Green))
            .unfilled_style(Style::new().fg(Color::DarkGray)),
        gauge_area,
    );

    // Line 3: volume, modes, messages.
    let on_off = |b: bool| if b { "on" } else { "off" };
    let mut info = vec![
        Span::raw(format!(
            " vol {:>3}%",
            (app.player.volume() * 100.0).round() as u32
        )),
        Span::raw("  shuffle ").fg(Color::DarkGray),
        Span::raw(on_off(app.queue.shuffle())),
        Span::raw("  repeat ").fg(Color::DarkGray),
        Span::raw(app.queue.repeat.label()),
    ];
    if let Some(msg) = &app.message {
        info.push(Span::raw(format!("  {msg}")).fg(Color::Red));
    }
    frame.render_widget(Line::from(info), info_area);
}

/// "prefix + label" with the track duration right-aligned in `width` columns.
fn track_row(app: &App, path: &Path, prefix: &str, width: usize) -> String {
    let label = format!("{prefix}{}", app.track_name(path));
    let Some(duration) = app.meta(path).and_then(|m| m.duration()) else {
        return label;
    };
    let time = fmt_time(duration);
    // Keep at least one space between label and time; shorten the label, not the time.
    let Some(max_label) = width.checked_sub(time.len() + 1) else {
        return label;
    };
    let label = truncate(&label, max_label);
    let pad = width - label.width() - time.len();
    format!("{label}{}{time}", " ".repeat(pad))
}

/// Cut `s` to at most `max` display columns, ending in "…" if shortened.
fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > max {
            break;
        }
        out.push(c);
        used += w;
    }
    if max > 0 {
        out.push('…');
    }
    out
}

/// Path with `~` for the home dir, cut from the left to fit `max` chars.
fn short_path(path: &Path, max: usize) -> String {
    let mut s = path.display().to_string();
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        s = Path::new("~").join(rest).display().to_string();
    }
    let len = s.chars().count();
    if len > max && max > 1 {
        let tail: String = s.chars().skip(len - (max - 1)).collect();
        s = format!("…{tail}");
    }
    s
}

pub fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_display_width() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a very long title", 8), "a very …");
        assert_eq!(truncate("a very long title", 8).width(), 8);
        // Wide (CJK) chars take two columns each.
        assert_eq!(truncate("日本語の曲", 5), "日本…");
        assert_eq!(truncate("abc", 0), "");
    }
}
