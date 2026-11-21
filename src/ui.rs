use std::path::Path;
use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, LineGauge, List, ListItem, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Focus, Mode, Prompt};
use crate::login::{LABELS, Login};
use crate::remote::{Entry, Remote};
use crate::track::Track;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [main, status] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(5)]).areas(frame.area());
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(main);

    match app.mode {
        Mode::Local => draw_browser(frame, app, left),
        Mode::Remote => draw_remote(frame, app, left),
    }
    draw_queue(frame, app, right);
    draw_status(frame, app, status);
    if let Some(login) = &app.login {
        draw_login(frame, login);
    }
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

fn prompt_line(label: &str, text: &str, active: bool) -> Line<'static> {
    let cursor = if active { "█" } else { "" };
    Line::from(vec![
        Span::raw(format!(" {label}")).fg(Color::Yellow),
        Span::raw(format!("{text}{cursor} ")),
    ])
}

fn draw_browser(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Browser;
    let playing = app.queue().current().and_then(Track::local_path);
    let inner_width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .browser
        .visible()
        .map(|e| {
            if e.is_dir {
                ListItem::new(format!("{}/", e.name)).style(Style::new().fg(Color::Blue).bold())
            } else {
                let style = if Some(e.path.as_path()) == playing {
                    Style::new().fg(Color::Green)
                } else {
                    Style::new()
                };
                let track = Track::Local(e.path.clone());
                ListItem::new(track_row(app, &track, "", inner_width)).style(style)
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
    let filtering = app.prompt == Some(Prompt::Filter);
    if filtering || !app.browser.filter.is_empty() {
        block = block.title_bottom(prompt_line("/", &app.browser.filter, filtering));
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

fn draw_remote(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Browser;
    let Some(remote) = &app.remote else {
        let block = pane_block(Line::from(" ⛁ OpenSubsonic ").bold(), focused);
        let hint = Paragraph::new(" Not connected. Press L to log in.").block(block);
        frame.render_widget(hint, area);
        return;
    };
    let width = area.width.saturating_sub(2) as usize;
    let title = format!(" ⛁ {} › {} ", remote.client.server_name(), remote.path());
    let mut block = pane_block(
        // Cut from the left so the current location stays visible.
        Line::from(truncate_left(&title, area.width.saturating_sub(4) as usize)).bold(),
        focused,
    );
    if app.prompt == Some(Prompt::Search) {
        block = block.title_bottom(prompt_line("search: ", &app.search_input, true));
    } else if remote.loading.is_some() {
        block = block.title_bottom(Line::from(" loading… ").fg(Color::Yellow));
    }

    let Some(view) = remote.view() else {
        let text = if remote.loading.is_some() {
            " Loading…"
        } else {
            ""
        };
        frame.render_widget(Paragraph::new(text).block(block), area);
        return;
    };
    if view.entries.is_empty() {
        frame.render_widget(Paragraph::new(" Nothing found.").block(block), area);
        return;
    }
    let items = remote_items(app, remote, width);
    let cursor = view.cursor;
    let list = List::new(items)
        .block(block)
        .highlight_style(highlight(focused));
    app.browser_state.select(Some(cursor));
    frame.render_stateful_widget(list, area, &mut app.browser_state);
}

fn remote_items(app: &App, remote: &Remote, width: usize) -> Vec<ListItem<'static>> {
    let playing = match app.queue().current() {
        Some(Track::Remote(song)) => Some(song.id.as_str()),
        _ => None,
    };
    let Some(view) = remote.view() else {
        return Vec::new();
    };
    view.entries
        .iter()
        .map(|entry| match entry {
            Entry::Header(name) => ListItem::new(format!("── {name} ──"))
                .style(Style::new().fg(Color::DarkGray).bold()),
            Entry::Artist(a) => {
                let count = a.album_count.map(|n| n.to_string()).unwrap_or_default();
                ListItem::new(row(&format!("{}/", a.name), &count, width))
                    .style(Style::new().fg(Color::Blue).bold())
            }
            Entry::Album(a) => {
                let mut label = format!("{}/", a.name);
                if let Some(artist) = &a.artist
                    && view.title != *artist
                {
                    label = format!("{}/ – {artist}", a.name);
                }
                let year = a.year.map(|y| y.to_string()).unwrap_or_default();
                ListItem::new(row(&label, &year, width)).style(Style::new().fg(Color::Blue))
            }
            Entry::Song(song) => {
                let style = if Some(song.id.as_str()) == playing {
                    Style::new().fg(Color::Green)
                } else {
                    Style::new()
                };
                let track = Track::Remote(song.clone());
                ListItem::new(track_row(app, &track, "", width)).style(style)
            }
        })
        .collect()
}

fn draw_queue(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Queue;
    let queue = app.queue();
    let current = queue.current_index();
    let inner_width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = queue
        .tracks()
        .iter()
        .enumerate()
        .map(|(i, track)| {
            if Some(i) == current {
                ListItem::new(track_row(app, track, "▶ ", inner_width))
                    .style(Style::new().fg(Color::Green).bold())
            } else {
                ListItem::new(track_row(app, track, "  ", inner_width))
            }
        })
        .collect();

    let title = match current {
        Some(i) => format!(" Queue ({}/{}) ", i + 1, queue.len()),
        None => format!(" Queue ({}) ", queue.len()),
    };
    let selected = (!queue.is_empty()).then_some(app.session().cursor);
    let list = List::new(items)
        .block(pane_block(Line::from(title).bold(), focused))
        .highlight_style(highlight(focused));
    app.queue_state.select(selected);
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
    let current = app.queue().current().filter(|_| app.player.is_loaded());
    let (icon, title, album) = match current {
        Some(track) => {
            let icon = if app.player.is_paused() { "⏸" } else { "▶" };
            let album = track
                .album(&app.cache)
                .map(|a| format!("  ·  {a}"))
                .unwrap_or_default();
            (icon, app.label(track), album)
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
    let duration = app
        .player
        .duration()
        .or_else(|| current.and_then(|t| app.track_duration(t)));
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

    // Line 3: mode, volume, shuffle/repeat, messages.
    let on_off = |b: bool| if b { "on" } else { "off" };
    let mode = match app.mode {
        Mode::Local => " LOCAL ",
        Mode::Remote => " OSS ",
    };
    let mut info = vec![
        Span::raw(" "),
        Span::raw(mode)
            .fg(Color::Cyan)
            .add_modifier(Modifier::REVERSED),
        Span::raw(format!(
            "  vol {:>3}%",
            (app.player.volume() * 100.0).round() as u32
        )),
        Span::raw("  shuffle ").fg(Color::DarkGray),
        Span::raw(on_off(app.queue().shuffle())),
        Span::raw("  repeat ").fg(Color::DarkGray),
        Span::raw(app.queue().repeat.label()),
    ];
    if let Some(msg) = &app.message {
        info.push(Span::raw(format!("  {msg}")).fg(Color::Red));
    }
    frame.render_widget(Line::from(info), info_area);
}

fn draw_login(frame: &mut Frame, login: &Login) {
    let [area] = Layout::horizontal([Constraint::Length(64)])
        .flex(Flex::Center)
        .areas(frame.area());
    let [area] = Layout::vertical([Constraint::Length(10)])
        .flex(Flex::Center)
        .areas(area);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Cyan))
        .title(Line::from(" ⛁ OpenSubsonic login ").bold());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let field_width = inner.width.saturating_sub(15) as usize;
    let mut lines = vec![Line::raw("")];
    for (i, label) in LABELS.iter().enumerate() {
        let value = if i == 2 {
            "•".repeat(login.fields[i].chars().count())
        } else {
            login.fields[i].clone()
        };
        let focused = i == login.focus;
        let cursor = if focused { "█" } else { "" };
        let shown = truncate_left(&format!("{value}{cursor}"), field_width);
        let value_style = if focused {
            Style::new().add_modifier(Modifier::UNDERLINED)
        } else {
            Style::new()
        };
        lines.push(Line::from(vec![
            Span::raw(format!(" {label:<12} ")).fg(if focused {
                Color::Cyan
            } else {
                Color::DarkGray
            }),
            Span::styled(shown, value_style),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(match (&login.error, login.is_checking()) {
        (_, true) => Line::from(" Connecting…").fg(Color::Yellow),
        (Some(err), false) => Line::from(format!(" {err}")).fg(Color::Red),
        (None, false) => Line::raw(""),
    });
    lines.push(
        Line::from(" Enter: next / log in   Tab: switch field   Esc: cancel").fg(Color::DarkGray),
    );
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Keep the end of `s` (where the cursor is) within `max` columns.
fn truncate_left(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let mut out: Vec<char> = Vec::new();
    let mut used = 1; // room for "…"
    for c in s.chars().rev() {
        let w = c.width().unwrap_or(0);
        if used + w > max {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out.into_iter().rev().collect()
}

/// "prefix + label" with the track duration right-aligned in `width` columns.
fn track_row(app: &App, track: &Track, prefix: &str, width: usize) -> String {
    let label = format!("{prefix}{}", app.label(track));
    let time = app.track_duration(track).map(fmt_time).unwrap_or_default();
    row(&label, &time, width)
}

/// `left` and `right` in `width` columns; `left` is shortened so `right` stays visible.
fn row(left: &str, right: &str, width: usize) -> String {
    if right.is_empty() {
        return truncate(left, width);
    }
    // Keep at least one space between both parts.
    let Some(max_left) = width.checked_sub(right.width() + 1) else {
        return truncate(left, width);
    };
    let left = truncate(left, max_left);
    let pad = width - left.width() - right.width();
    format!("{left}{}{right}", " ".repeat(pad))
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
