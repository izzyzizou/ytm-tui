//! Rendering: `(&App) -> Frame`. No state changes here. Layout and colors follow
//! docs/design-system.md (AppShell + PlayerBar).

use std::time::Duration;

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, LineGauge, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use ytm_api::models::ItemKind;
use ytm_api::Track;
use ytm_core::reducer::Level;
use ytm_core::state::{Repeat, Status};

use crate::app::{App, Mode, Pane, SideTab, View};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < 40 || area.height < 8 {
        f.render_widget(Paragraph::new("terminal too small").fg(app.theme.ink_muted), area);
        return;
    }
    // top rule + track line + progress line (+ key hints on tall terminals)
    let footer_h = if area.height >= 24 { 4 } else { 3 };
    let [header, body, footer] = Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(footer_h)]).areas(area);

    let show_side = app.side_visible && area.width >= 110;
    let show_sidebar = area.width >= 80;
    let [side, main, panel] = Layout::horizontal([
        Constraint::Length(if show_sidebar { 20 } else { 0 }),
        Constraint::Min(30),
        Constraint::Length(if show_side { 26 } else { 0 }),
    ])
    .areas(body);

    draw_header(f, app, header);
    if show_sidebar {
        draw_sidebar(f, app, side);
    }
    draw_main(f, app, main);
    if show_side {
        draw_side_panel(f, app, panel);
    }
    draw_footer(f, app, footer);
    draw_toasts(f, app, main);
    if app.show_help {
        draw_help(f, app, area);
    }
}

/// Unfocused: plain border in `border`. Focused: rounded border + bold title in `lagoon`.
fn pane<'a>(app: &App, which: Pane, title: impl Into<String>) -> Block<'a> {
    let t = &app.theme;
    let focused = app.focus == which;
    Block::new()
        .borders(Borders::ALL)
        .border_type(if focused { BorderType::Rounded } else { BorderType::Plain })
        .border_style(Style::new().fg(if focused { t.lagoon } else { t.border }))
        .title(Span::styled(
            format!(" {} ", title.into()),
            Style::new().fg(if focused { t.lagoon } else { t.ink_muted }).add_modifier(Modifier::BOLD),
        ))
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let state = match app.player.status {
        Status::Playing => Span::styled(" ▶ PLAYING", Style::new().fg(t.ember).bold()),
        Status::Paused => Span::styled(" ‖ PAUSED", Style::new().fg(t.ink).bold()),
        Status::Loading => Span::styled(" ◌ LOADING", Style::new().fg(t.amber).bold()),
        Status::Buffering => Span::styled(" ◌ BUFFERING", Style::new().fg(t.amber).bold()),
        Status::Stopped => Span::styled(" ■ STOPPED", Style::new().fg(t.ink_muted).bold()),
    };
    let link = if app.connected {
        Span::styled("   ● daemon", Style::new().fg(t.moss))
    } else {
        Span::styled("   ✗ daemon lost", Style::new().fg(t.error))
    };
    let mut left = vec![state, link, Span::styled("   │   ", Style::new().fg(t.border)), Span::styled("⌕ ", Style::new().fg(t.lagoon))];
    if app.query.is_empty() && app.mode != Mode::Search {
        left.push(Span::styled("press / to search", Style::new().fg(t.ink_faint)));
    } else {
        left.push(Span::styled(app.query.clone(), Style::new().fg(t.ink)));
    }
    if app.mode == Mode::Search {
        left.push(Span::styled("▏", Style::new().fg(t.lagoon)));
    }
    if app.searching {
        left.push(Span::styled("  searching…", Style::new().fg(t.amber)));
    }
    let repeat = match app.player.repeat {
        Repeat::Off => "↻ off",
        Repeat::All => "↻ all",
        Repeat::One => "↻ one",
    };
    let shuffle_style = Style::new().fg(if app.player.shuffle { t.lagoon } else { t.ink_muted });
    let account = if app.player.authenticated { "signed in" } else { "anonymous" };
    let right = vec![
        Span::styled(if app.player.shuffle { "⇄ shuffle" } else { "⇄ off" }, shuffle_style),
        Span::raw("  "),
        Span::styled(repeat, Style::new().fg(if app.player.repeat == Repeat::Off { t.ink_muted } else { t.lagoon })),
        Span::styled("   │   ", Style::new().fg(t.border)),
        Span::styled("◉ ", Style::new().fg(if app.player.authenticated { t.lagoon } else { t.ink_muted })),
        Span::styled(format!("{account} "), Style::new().fg(t.ink_muted)),
    ];
    let bg = Block::new().style(Style::new().bg(t.bg_raised));
    f.render_widget(bg, area);
    f.render_widget(Paragraph::new(Line::from(left)), area);
    f.render_widget(Paragraph::new(Line::from(right)).alignment(Alignment::Right), area);
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let block = pane(app, Pane::Sidebar, "NAVIGATE");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines = Vec::new();
    for (i, v) in View::NAV.iter().enumerate() {
        let active = app.view == *v;
        let cursor = app.focus == Pane::Sidebar && app.sidebar_cursor == i;
        let marker = if active { Span::styled("▌", Style::new().fg(t.lagoon)) } else { Span::raw(" ") };
        let glyph_style = if *v == View::Liked { Style::new().fg(t.ember) } else { Style::new().fg(t.ink) };
        let label = if *v == View::Queue && !app.player.queue.items.is_empty() {
            format!("{} · {}", v.label(), app.player.queue.items.len())
        } else {
            v.label().to_string()
        };
        let label_style = Style::new().fg(if active { t.lagoon } else { t.ink });
        let width = inner.width as usize;
        let text = truncate(&label, width.saturating_sub(6));
        let pad = width.saturating_sub(4 + text.width() + 1);
        let mut line = Line::from(vec![
            marker,
            Span::styled(format!("{} ", v.glyph()), glyph_style),
            Span::styled(text, label_style),
            Span::raw(" ".repeat(pad)),
            Span::styled(format!("{}", i + 1), Style::new().fg(t.ink_faint)),
        ]);
        if cursor {
            line = line.style(select_style(app));
        }
        lines.push(line);
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn select_style(app: &App) -> Style {
    if app.theme.select_uses_reverse() {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new().bg(app.theme.bg_select)
    }
}

fn draw_main(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    match app.view {
        View::Search => {
            let title = if app.query.is_empty() { "SEARCH".to_string() } else { format!("SEARCH › {}", app.query) };
            let block = pane(app, Pane::Main, title);
            let inner = block.inner(area);
            f.render_widget(block, area);
            if app.results.is_empty() {
                let hint = if app.searching { "searching…" } else { "press / and type an artist, song or album, then Enter" };
                f.render_widget(Paragraph::new(format!(" {hint}")).fg(t.ink_muted), inner);
                return;
            }
            draw_track_table(f, app, inner, &app.results, app.result_cursor, None);
        }
        View::Queue => {
            let q = &app.player.queue;
            let total: u64 = q.items.iter().filter_map(|t| t.duration_s).sum();
            let block = pane(app, Pane::Main, format!("QUEUE · {} tracks · {} min", q.items.len(), total / 60));
            let inner = block.inner(area);
            f.render_widget(block, area);
            if q.items.is_empty() {
                f.render_widget(Paragraph::new(" queue is empty · search with / and press a to add").fg(t.ink_muted), inner);
                return;
            }
            draw_track_table(f, app, inner, &q.items, app.queue_cursor, q.current);
        }
        v => {
            let block = pane(app, Pane::Main, v.label().to_uppercase());
            let inner = block.inner(area);
            f.render_widget(block, area);
            let msg = if app.player.authenticated {
                format!(" {} is on the roadmap (docs/ROADMAP.md, milestone 4).", v.label())
            } else {
                format!(
                    " {} needs a signed-in session: run `ytm-tui auth import`.\n {} itself is on the roadmap (milestone 4).",
                    v.label(),
                    v.label()
                )
            };
            f.render_widget(Paragraph::new(msg).fg(t.ink_muted).wrap(Wrap { trim: false }), inner);
        }
    }
}

/// `▶ 1 Title  ♥ Artist  5:18` rows with the cursor row highlighted and the list scrolled to keep it visible.
fn draw_track_table(f: &mut Frame, app: &App, area: Rect, tracks: &[Track], cursor: usize, playing_index: Option<usize>) {
    let t = &app.theme;
    if area.height < 2 {
        return;
    }
    let w = area.width as usize;
    let num_w = tracks.len().to_string().len().max(2);
    let time_w = 5;
    let fixed = 2 + num_w + 1 + 1 + time_w + 1;
    let flexible = w.saturating_sub(fixed);
    let artist_w = if w >= 70 { (flexible * 2 / 5).min(28) } else { 0 };
    let title_w = flexible.saturating_sub(artist_w);

    let header = format!(
        "  {:>num_w$} {}{}{:>time_w$}",
        "#",
        pad_to("TITLE", title_w),
        if artist_w > 0 { pad_to("ARTIST", artist_w) } else { String::new() },
        "TIME"
    );
    f.render_widget(Paragraph::new(Span::styled(header, Style::new().fg(t.ink_muted))), Rect { height: 1, ..area });

    let rows_area = Rect { y: area.y + 1, height: area.height - 1, ..area };
    let visible = rows_area.height as usize;
    let offset = if cursor >= visible { cursor + 1 - visible } else { 0 };
    let current_id = app.player.current_track().map(|t| t.video_id.as_str());
    let mut lines = Vec::new();
    for (i, tr) in tracks.iter().enumerate().skip(offset).take(visible) {
        let is_playing = match playing_index {
            Some(p) => p == i,
            None => Some(tr.video_id.as_str()) == current_id,
        };
        let is_cursor = i == cursor && app.focus == Pane::Main;
        let marker = if is_playing {
            Span::styled("▶ ", Style::new().fg(t.ember))
        } else if is_cursor {
            Span::styled("› ", Style::new().fg(t.lagoon))
        } else {
            Span::raw("  ")
        };
        let mut title_style = Style::new().fg(if is_playing { t.ember } else { t.ink });
        if is_playing {
            title_style = title_style.add_modifier(Modifier::BOLD);
        }
        let kind_tag = match tr.kind {
            ItemKind::Song => "",
            ItemKind::Video => " (video)",
            ItemKind::Episode => " (episode)",
        };
        let title = format!("{}{}", tr.title, kind_tag);
        let mut spans = vec![
            marker,
            Span::styled(format!("{:>num_w$} ", i + 1), Style::new().fg(t.ink_muted)),
            Span::styled(pad_to(&title, title_w), title_style),
        ];
        if artist_w > 0 {
            spans.push(Span::styled(pad_to(&tr.artist_line(), artist_w), Style::new().fg(t.ink_muted)));
        }
        spans.push(Span::styled(format!("{:>time_w$}", tr.duration_s.map(mmss_secs).unwrap_or_default()), Style::new().fg(t.ink_muted)));
        let mut line = Line::from(spans);
        if is_cursor {
            line = line.style(select_style(app));
        }
        lines.push(line);
    }
    f.render_widget(Paragraph::new(lines), rows_area);
}

fn draw_side_panel(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let title = match app.side_tab {
        SideTab::Lyrics => "LYRICS",
        SideTab::Spectrum => "SPECTRUM",
    };
    let block = pane(app, Pane::Side, title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 {
        return;
    }
    let tabs = Line::from(match app.side_tab {
        SideTab::Lyrics => {
            vec![Span::styled("[Lyrics]", Style::new().fg(t.lagoon)), Span::styled(" Spectrum", Style::new().fg(t.ink_muted))]
        }
        SideTab::Spectrum => {
            vec![Span::styled("Lyrics ", Style::new().fg(t.ink_muted)), Span::styled("[Spectrum]", Style::new().fg(t.lagoon))]
        }
    });
    f.render_widget(Paragraph::new(tabs), Rect { height: 1, ..inner });
    let body = Rect { y: inner.y + 2, height: inner.height.saturating_sub(3), ..inner };
    let foot = Rect { y: inner.y + inner.height - 1, height: 1, ..inner };

    match app.side_tab {
        SideTab::Spectrum => {
            f.render_widget(
                Paragraph::new("The visualizer needs the native audio backend (roadmap milestone 6).")
                    .fg(t.ink_muted)
                    .wrap(Wrap { trim: true }),
                body,
            );
        }
        SideTab::Lyrics => {
            let current_id = app.player.current_track().map(|t| t.video_id.clone());
            let lyr = app.lyrics.as_ref().filter(|l| Some(&l.video_id) == current_id.as_ref());
            match (current_id, lyr) {
                (None, _) => f.render_widget(Paragraph::new("nothing playing").fg(t.ink_muted), body),
                (Some(_), None) => f.render_widget(Paragraph::new("looking for lyrics…").fg(t.ink_muted), body),
                (Some(_), Some(u)) => match &u.lyrics {
                    None => f.render_widget(Paragraph::new("no lyrics found").fg(t.ink_muted), body),
                    Some(l) => {
                        let cur = l.current(app.display_position(), app.lyrics_offset_ms);
                        let h = body.height as usize;
                        let anchor = h / 3;
                        let start = if l.synced {
                            cur.unwrap_or(0).saturating_sub(anchor)
                        } else {
                            // Unsynced: scroll proportionally to playback position.
                            let frac = match app.player.duration {
                                Some(d) if !d.is_zero() => app.display_position().as_secs_f64() / d.as_secs_f64(),
                                _ => 0.0,
                            };
                            ((l.lines.len().saturating_sub(h)) as f64 * frac) as usize
                        };
                        let width = body.width as usize;
                        let lines: Vec<Line> = l
                            .lines
                            .iter()
                            .enumerate()
                            .skip(start)
                            .take(h)
                            .map(|(i, line)| {
                                let text = truncate(&line.text, width.saturating_sub(2));
                                match cur {
                                    Some(c) if c == i => Line::from(vec![
                                        Span::styled("› ", Style::new().fg(t.ember)),
                                        Span::styled(text, Style::new().fg(t.ember).bold()),
                                    ]),
                                    Some(c) if i == c + 1 || i + 1 == c => Line::styled(format!("  {text}"), Style::new().fg(t.ink)),
                                    _ => Line::styled(format!("  {text}"), Style::new().fg(t.ink_muted)),
                                }
                            })
                            .collect();
                        f.render_widget(Paragraph::new(lines), body);
                        let label = format!(
                            "{} · {}  {:+.1}s",
                            if l.synced { "synced" } else { "unsynced" },
                            l.source,
                            app.lyrics_offset_ms as f64 / 1000.0
                        );
                        f.render_widget(Paragraph::new(Span::styled(label, Style::new().fg(t.ink_faint))), foot);
                    }
                },
            }
        }
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let block = Block::new().style(Style::new().bg(t.bg_raised)).borders(Borders::TOP).border_style(Style::new().fg(t.border));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Length(1); 3]).split(inner);

    // Row 1: track + stream info
    let track = app.player.current_track();
    let left = match (track, app.player.status) {
        (Some(tr), status) => {
            let (glyph, color) = match status {
                Status::Playing => ("▶", t.ember),
                Status::Paused => ("‖", t.ink),
                Status::Loading | Status::Buffering => ("◌", t.amber),
                Status::Stopped => ("■", t.ink_muted),
            };
            let title_color =
                if status == Status::Playing || status == Status::Loading || status == Status::Buffering { t.ember } else { t.ink };
            let mut v = vec![
                Span::styled(format!(" {glyph} "), Style::new().fg(color)),
                Span::styled(tr.title.clone(), Style::new().fg(title_color).bold()),
                Span::styled(" — ", Style::new().fg(t.ink_faint)),
                Span::styled(tr.artist_line(), Style::new().fg(t.ink)),
            ];
            if let Some(a) = &tr.album {
                v.push(Span::styled(format!(" · {a}"), Style::new().fg(t.ink_muted)));
            }
            Line::from(v)
        }
        (None, _) => Line::styled(" nothing playing · / to search, Enter to play", Style::new().fg(t.ink_muted)),
    };
    let right = match (&app.player.stream, app.player.status) {
        (_, Status::Loading) => Line::styled("resolving stream… ", Style::new().fg(t.amber)),
        (_, Status::Buffering) => Line::styled("buffering… ", Style::new().fg(t.amber)),
        (Some(s), _) if s.codec != "null" => Line::styled(format!("{} ", s.describe()), Style::new().fg(t.ink_muted)),
        _ if app.player.backend == "null" => Line::styled("silent backend ", Style::new().fg(t.amber)),
        _ => Line::raw(""),
    };
    f.render_widget(Paragraph::new(left), rows[0]);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), rows[0]);

    // Row 2: progress + volume
    if rows.len() > 1 && rows[1].height > 0 {
        let pos = app.display_position();
        let total = app.player.duration.unwrap_or(Duration::ZERO);
        let ratio = if total.is_zero() { 0.0 } else { (pos.as_secs_f64() / total.as_secs_f64()).clamp(0.0, 1.0) };
        let vol_blocks = (app.player.volume as usize + 5) / 10;
        let vol = Line::from(vec![
            Span::styled("vol ", Style::new().fg(t.ink_muted)),
            Span::styled("▮".repeat(vol_blocks), Style::new().fg(t.ink)),
            Span::styled("▯".repeat(10 - vol_blocks), Style::new().fg(t.ink_faint)),
            Span::styled(format!(" {:>3}% ", app.player.volume), Style::new().fg(t.ink_muted)),
        ]);
        let [gauge_area, vol_area] = Layout::horizontal([Constraint::Min(20), Constraint::Length(20)]).areas(rows[1]);
        let gauge = LineGauge::default()
            .ratio(ratio)
            .label(Span::styled(format!(" {} / {}", mmss(pos), mmss(total)), Style::new().fg(t.ink)))
            .line_set(symbols::line::THICK)
            .filled_style(Style::new().fg(t.ember))
            .unfilled_style(Style::new().fg(t.border));
        f.render_widget(gauge, gauge_area);
        f.render_widget(Paragraph::new(vol).alignment(Alignment::Right), vol_area);
    }

    // Row 3: key hints
    if rows.len() > 2 && rows[2].height > 0 {
        let keys: &[(&str, &str)] = match app.mode {
            Mode::Search => &[("Enter", "search"), ("Esc", "cancel"), ("^U", "clear")],
            Mode::Normal => &[
                ("/", "search"),
                ("⏎", "play"),
                ("␣", "pause"),
                ("n/p", "next/prev"),
                ("[ ]", "seek"),
                ("+/-", "vol"),
                ("a", "queue"),
                ("7", "queue view"),
                ("?", "help"),
                ("q", "quit"),
            ],
        };
        let spans: Vec<Span> = keys
            .iter()
            .flat_map(|(k, a)| {
                [Span::styled(format!(" {k}"), Style::new().fg(t.lagoon)), Span::styled(format!(" {a} "), Style::new().fg(t.ink_muted))]
            })
            .collect();
        f.render_widget(Paragraph::new(Line::from(spans)), rows[2]);
    }
}

fn draw_toasts(f: &mut Frame, app: &App, body: Rect) {
    let t = &app.theme;
    for (i, (toast, _)) in app.toasts.iter().rev().take(3).enumerate() {
        let (glyph, color) = match toast.level {
            Level::Info => ("●", t.lagoon),
            Level::Warn => ("●", t.amber),
            Level::Error => ("✗", t.error),
        };
        let text = format!(" {glyph} {} ", toast.text);
        let w = (text.width() as u16 + 1).min(body.width.saturating_sub(2));
        let y = body.y + body.height.saturating_sub(2 + i as u16);
        if y <= body.y {
            break;
        }
        let r = Rect { x: body.x + body.width.saturating_sub(w + 1), y, width: w, height: 1 };
        f.render_widget(Clear, r);
        f.render_widget(Paragraph::new(Span::styled(truncate(&text, w as usize), Style::new().fg(color).bg(t.bg_raised))), r);
    }
}

fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let rows: &[(&str, &str)] = &[
        ("/", "search (Enter runs it)"),
        ("j k  gg G  ^d ^u", "move"),
        ("Tab  ^h ^l", "switch pane"),
        ("1–7  gh gq …", "jump to a view"),
        ("Enter", "play (from search: queues the results)"),
        ("Space", "play / pause"),
        ("n  p", "next / previous"),
        ("[ ]  { }", "seek ±5s / ±30s"),
        ("+ -  M", "volume / mute"),
        ("s  r", "shuffle / repeat"),
        ("a  A", "add to queue / play next"),
        ("d  J K  c", "queue: remove / move / clear"),
        (".", "show now playing in queue"),
        ("y  Y", "copy URL / video id"),
        ("V  t", "side panel / lyrics ↔ spectrum"),
        ("< >", "lyrics offset ∓100ms"),
        ("q  Q", "quit / quit but keep playing"),
    ];
    let w = 64.min(area.width.saturating_sub(4));
    let h = (rows.len() as u16 + 4).min(area.height.saturating_sub(2));
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.lagoon))
        .title(Span::styled(" HELP · Esc closes ", Style::new().fg(t.lagoon).bold()))
        .style(Style::new().bg(t.bg_raised));
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| {
            Line::from(vec![Span::styled(format!(" {k:<18}"), Style::new().fg(t.lagoon)), Span::styled(*v, Style::new().fg(t.ink))])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(block), r);
}

// ───────────────────────────── helpers ─────────────────────────────

fn mmss(d: Duration) -> String {
    mmss_secs(d.as_secs())
}

fn mmss_secs(s: u64) -> String {
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Truncate to `max` display cells with `…`, never splitting a wide character.
pub fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw > max - 1 {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// Truncate/pad to exactly `width` cells, leaving one trailing space as a column gap.
fn pad_to(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let t = truncate(s, width.saturating_sub(1));
    let pad = width - t.width();
    format!("{t}{}", " ".repeat(pad))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_respects_display_width() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(truncate("日本語の曲", 5), "日本…");
        assert_eq!(pad_to("abc", 6).width(), 6);
        assert_eq!(pad_to("日本語の曲名", 7).width(), 7);
    }

    #[test]
    fn time_format() {
        assert_eq!(mmss_secs(318), "5:18");
        assert_eq!(mmss_secs(3723), "1:02:03");
    }
}
