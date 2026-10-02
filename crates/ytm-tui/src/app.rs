//! TUI state and event loop. The TUI is a client of the daemon: it mirrors `PlayerState`
//! from pushed events and sends commands; it never touches audio or the network directly.

use std::sync::Arc;
use std::time::{Duration, Instant};

use color_eyre::Result;
use crossterm::event::{Event as TermEvent, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use serde_json::json;
use tokio::sync::mpsc;
use ytm_api::Track;
use ytm_core::client::Client;
use ytm_core::protocol::{method, Event, LyricsUpdate};
use ytm_core::reducer::{Command, Level, Toast};
use ytm_core::state::{PlayerState, Status};

use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Sidebar,
    Main,
    Side,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Home,
    Explore,
    Library,
    Playlists,
    Liked,
    History,
    Queue,
    Search,
}

impl View {
    pub const NAV: [View; 7] = [View::Home, View::Explore, View::Library, View::Playlists, View::Liked, View::History, View::Queue];

    pub fn label(self) -> &'static str {
        match self {
            View::Home => "Home",
            View::Explore => "Explore",
            View::Library => "Library",
            View::Playlists => "Playlists",
            View::Liked => "Liked Songs",
            View::History => "History",
            View::Queue => "Queue",
            View::Search => "Search",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            View::Home => "⌂",
            View::Explore => "◇",
            View::Library => "▤",
            View::Playlists => "≡",
            View::Liked => "♥",
            View::History => "◷",
            View::Queue => "»",
            View::Search => "⌕",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideTab {
    Lyrics,
    Spectrum,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Typing in the header search field.
    Search,
}

/// Results from background tasks.
enum Msg {
    SearchResults { seq: u64, result: Result<Vec<Track>, String> },
    State(Box<PlayerState>),
    Toast(Toast),
}

pub struct App {
    pub theme: Theme,
    pub client: Arc<Client>,
    pub player: PlayerState,
    /// When `player.position` was last received, for smooth interpolation.
    pub position_at: Instant,
    pub mode: Mode,
    pub focus: Pane,
    pub view: View,
    pub sidebar_cursor: usize,
    pub query: String,
    pub results: Vec<Track>,
    pub result_cursor: usize,
    pub queue_cursor: usize,
    pub searching: bool,
    search_seq: u64,
    pub side_visible: bool,
    pub side_tab: SideTab,
    pub lyrics: Option<LyricsUpdate>,
    pub lyrics_offset_ms: i64,
    pub toasts: Vec<(Toast, Instant)>,
    pub show_help: bool,
    /// False once the daemon connection drops.
    pub connected: bool,
    pending_g: bool,
    running: bool,
    /// Quit also stops the daemon (q) vs. leaves it playing (Q).
    pub stop_daemon_on_exit: bool,
    dirty: bool,
    msg_tx: mpsc::UnboundedSender<Msg>,
}

impl App {
    pub fn new(theme: Theme, client: Arc<Client>) -> Self {
        let (tx, _) = mpsc::unbounded_channel(); // replaced in run()
        Self {
            theme,
            client,
            player: PlayerState::default(),
            position_at: Instant::now(),
            mode: Mode::Normal,
            focus: Pane::Main,
            view: View::Search,
            sidebar_cursor: 0,
            query: String::new(),
            results: Vec::new(),
            result_cursor: 0,
            queue_cursor: 0,
            searching: false,
            search_seq: 0,
            side_visible: true,
            side_tab: SideTab::Lyrics,
            lyrics: None,
            lyrics_offset_ms: 0,
            toasts: Vec::new(),
            show_help: false,
            connected: true,
            pending_g: false,
            running: true,
            stop_daemon_on_exit: false,
            dirty: true,
            msg_tx: tx,
        }
    }

    /// Position shown in the UI: last reported position plus time since, while playing.
    pub fn display_position(&self) -> Duration {
        let mut p = self.player.position;
        if self.player.status == Status::Playing {
            p += self.position_at.elapsed();
        }
        match self.player.duration {
            Some(d) => p.min(d),
            None => p,
        }
    }

    fn animating(&self) -> bool {
        self.player.status == Status::Playing || !self.toasts.is_empty() || self.searching
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal, mut daemon_events: mpsc::UnboundedReceiver<Event>) -> Result<bool> {
        let (tx, mut msg_rx) = mpsc::unbounded_channel();
        self.msg_tx = tx;
        let mut term_events = EventStream::new();
        let mut frames = tokio::time::interval(Duration::from_millis(33));
        frames.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut daemon_alive = true;

        while self.running {
            if self.dirty {
                terminal.draw(|f| crate::ui::draw(f, &self))?;
                self.dirty = false;
            }
            let animating = self.animating();
            tokio::select! {
                ev = term_events.next() => match ev {
                    Some(Ok(ev)) => self.on_terminal_event(ev),
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                },
                ev = daemon_events.recv(), if daemon_alive => match ev {
                    Some(ev) => self.on_daemon_event(ev),
                    None => {
                        daemon_alive = false;
                        self.connected = false;
                        self.toast(Level::Error, "lost connection to the daemon · restart ytm-tui");
                    }
                },
                Some(msg) = msg_rx.recv() => self.on_msg(msg),
                _ = frames.tick(), if animating => {
                    self.toasts.retain(|(t, at)| t.level == Level::Error || at.elapsed() < Duration::from_secs(4));
                    self.dirty = true;
                }
            }
        }
        Ok(self.stop_daemon_on_exit)
    }

    fn on_daemon_event(&mut self, ev: Event) {
        match ev {
            Event::State(s) => self.set_state(*s),
            Event::Toast(t) => self.toasts.push((t, Instant::now())),
            Event::Lyrics(l) => self.lyrics = Some(l),
        }
        self.dirty = true;
    }

    fn set_state(&mut self, s: PlayerState) {
        self.player = s;
        self.position_at = Instant::now();
        let n = self.player.queue.items.len();
        if self.queue_cursor >= n {
            self.queue_cursor = n.saturating_sub(1);
        }
    }

    fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::SearchResults { seq, result } if seq == self.search_seq => {
                self.searching = false;
                match result {
                    Ok(tracks) => {
                        if tracks.is_empty() {
                            self.toast(Level::Info, format!("no results for \"{}\"", self.query));
                        }
                        self.results = tracks;
                        self.result_cursor = 0;
                    }
                    Err(e) => self.toast(Level::Error, format!("search failed · {e}")),
                }
            }
            Msg::SearchResults { .. } => {} // superseded
            Msg::State(s) => self.set_state(*s),
            Msg::Toast(t) => self.toasts.push((t, Instant::now())),
        }
        self.dirty = true;
    }

    pub fn toast(&mut self, level: Level, text: impl Into<String>) {
        self.toasts.push((Toast { level, text: text.into() }, Instant::now()));
        self.dirty = true;
    }

    // ───────────────────────────── input ─────────────────────────────

    fn on_terminal_event(&mut self, ev: TermEvent) {
        match ev {
            TermEvent::Key(k) if k.kind == KeyEventKind::Press => {
                match self.mode {
                    Mode::Search => self.on_search_key(k),
                    Mode::Normal => self.on_normal_key(k),
                }
                self.dirty = true;
            }
            TermEvent::Mouse(m) => match m.kind {
                MouseEventKind::ScrollDown => self.move_cursor(1),
                MouseEventKind::ScrollUp => self.move_cursor(-1),
                _ => {}
            },
            TermEvent::Resize(..) => self.dirty = true,
            TermEvent::Paste(s) if self.mode == Mode::Search => {
                self.query.push_str(s.trim());
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn on_search_key(&mut self, k: KeyEvent) {
        match (k.code, k.modifiers) {
            (KeyCode::Esc, _) => self.mode = Mode::Normal,
            (KeyCode::Enter, _) => {
                self.mode = Mode::Normal;
                self.run_search();
            }
            (KeyCode::Backspace, _) => {
                self.query.pop();
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => self.query.clear(),
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                let t = self.query.trim_end().rfind(' ').map_or(0, |i| i + 1);
                self.query.truncate(t);
            }
            (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => self.query.push(c),
            _ => {}
        }
    }

    fn on_normal_key(&mut self, k: KeyEvent) {
        use KeyCode::*;
        if self.show_help {
            if matches!(k.code, Esc | Char('?') | Char('q')) {
                self.show_help = false;
            }
            return;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let g = std::mem::take(&mut self.pending_g);
        match k.code {
            // g-prefixed jumps (gg, gh, gq, …)
            Char('g') if g => self.move_cursor(isize::MIN / 2),
            Char('h') if g => self.open_view(View::Home),
            Char('e') if g => self.open_view(View::Explore),
            Char('l') if g => self.open_view(View::Library),
            Char('p') if g => self.open_view(View::Playlists),
            Char('L') if g => self.open_view(View::Liked),
            Char('H') if g => self.open_view(View::History),
            Char('q') if g => self.open_view(View::Queue),
            Char('c') if ctrl => self.quit(true),
            Char('q') => self.quit(true),
            Char('Q') => self.quit(false),
            Esc => {}
            Char('?') => self.show_help = true,
            Tab => self.cycle_focus(1),
            BackTab => self.cycle_focus(-1),
            Char('h') if ctrl => self.cycle_focus(-1),
            Char('l') if ctrl => self.cycle_focus(1),
            Char('d') if ctrl => self.move_cursor(10),
            Char('u') if ctrl => self.move_cursor(-10),
            Char('j') | Down => self.move_cursor(1),
            Char('k') | Up => self.move_cursor(-1),
            Char('g') => self.pending_g = true,
            Char('G') | End => self.move_cursor(isize::MAX / 2),
            Home => self.move_cursor(isize::MIN / 2),
            Char(d @ '1'..='7') => self.open_view(View::NAV[(d as u8 - b'1') as usize]),
            Char('/') => {
                self.mode = Mode::Search;
                self.view = View::Search;
                self.focus = Pane::Main;
            }
            Enter | Char('l') | Right => self.activate(),
            Char('h') | Left | Backspace if self.focus == Pane::Main => self.focus = Pane::Sidebar,
            Char(' ') => self.send(Command::Toggle),
            Char('n') => self.send(Command::Next),
            Char('p') => self.send(Command::Prev),
            Char('+') | Char('=') => self.send(Command::Volume { value: 5, relative: true }),
            Char('-') => self.send(Command::Volume { value: -5, relative: true }),
            Char('M') => {
                let v = if self.player.volume > 0 { 0 } else { 60 };
                self.send(Command::Volume { value: v, relative: false })
            }
            Char('[') => self.seek(-5.0),
            Char(']') => self.seek(5.0),
            Char('{') => self.seek(-30.0),
            Char('}') => self.seek(30.0),
            Char('s') => self.send(Command::SetShuffle { on: None }),
            Char('r') => self.send(Command::CycleRepeat),
            Char('a') => self.enqueue_selected(false),
            Char('A') => self.enqueue_selected(true),
            Char('d') | Delete if self.view == View::Queue => self.send(Command::Remove { index: self.queue_cursor }),
            Char('J') if self.view == View::Queue => self.move_queue_item(1),
            Char('K') if self.view == View::Queue => self.move_queue_item(-1),
            Char('c') if self.view == View::Queue => self.send(Command::Clear),
            Char('.') => self.jump_to_now_playing(),
            Char('y') => self.copy_selected(false),
            Char('Y') => self.copy_selected(true),
            Char('L') | Char('R') | Char('D') | Char('P') => self.toast(Level::Info, "not implemented yet · see docs/ROADMAP.md"),
            Char('V') => self.side_visible = !self.side_visible,
            Char('t') => {
                self.side_tab = match self.side_tab {
                    SideTab::Lyrics => SideTab::Spectrum,
                    SideTab::Spectrum => SideTab::Lyrics,
                }
            }
            Char('<') => self.lyrics_offset_ms -= 100,
            Char('>') => self.lyrics_offset_ms += 100,
            _ => {}
        }
    }

    fn quit(&mut self, stop_daemon: bool) {
        self.stop_daemon_on_exit = stop_daemon;
        self.running = false;
    }

    fn cycle_focus(&mut self, dir: i8) {
        let order = [Pane::Sidebar, Pane::Main, Pane::Side];
        let i = order.iter().position(|p| *p == self.focus).unwrap_or(1) as i8;
        let mut next = order[((i + dir).rem_euclid(3)) as usize];
        if next == Pane::Side && !self.side_visible {
            next = order[((i + 2 * dir).rem_euclid(3)) as usize];
        }
        self.focus = next;
    }

    fn open_view(&mut self, v: View) {
        self.view = v;
        if let Some(i) = View::NAV.iter().position(|x| *x == v) {
            self.sidebar_cursor = i;
        }
        self.focus = Pane::Main;
    }

    fn move_cursor(&mut self, delta: isize) {
        let clamp = |cur: usize, len: usize| -> usize {
            if len == 0 {
                0
            } else {
                (cur as isize).saturating_add(delta).clamp(0, len as isize - 1) as usize
            }
        };
        match self.focus {
            Pane::Sidebar => self.sidebar_cursor = clamp(self.sidebar_cursor, View::NAV.len()),
            Pane::Main => match self.view {
                View::Search => self.result_cursor = clamp(self.result_cursor, self.results.len()),
                View::Queue => self.queue_cursor = clamp(self.queue_cursor, self.player.queue.items.len()),
                _ => {}
            },
            Pane::Side => {}
        }
        self.dirty = true;
    }

    fn activate(&mut self) {
        match self.focus {
            Pane::Sidebar => self.open_view(View::NAV[self.sidebar_cursor]),
            Pane::Main => match self.view {
                View::Search if !self.results.is_empty() => {
                    self.send(Command::PlayTracks { tracks: self.results.clone(), start: self.result_cursor })
                }
                View::Queue if !self.player.queue.items.is_empty() => self.send(Command::JumpTo { index: self.queue_cursor }),
                _ => {}
            },
            Pane::Side => {}
        }
    }

    fn selected_track(&self) -> Option<&Track> {
        match self.view {
            View::Search => self.results.get(self.result_cursor),
            View::Queue => self.player.queue.items.get(self.queue_cursor),
            _ => None,
        }
        .or_else(|| self.player.current_track())
    }

    fn enqueue_selected(&mut self, next: bool) {
        if self.view == View::Search {
            if let Some(t) = self.results.get(self.result_cursor).cloned() {
                self.send(Command::Enqueue { tracks: vec![t], next });
            }
        }
    }

    fn move_queue_item(&mut self, dir: isize) {
        let len = self.player.queue.items.len();
        let to = self.queue_cursor as isize + dir;
        if to < 0 || to as usize >= len {
            return;
        }
        self.send(Command::Move { from: self.queue_cursor, to: to as usize });
        self.queue_cursor = to as usize;
    }

    fn jump_to_now_playing(&mut self) {
        if let Some(c) = self.player.queue.current {
            self.open_view(View::Queue);
            self.queue_cursor = c;
        }
    }

    fn copy_selected(&mut self, id_only: bool) {
        let Some(t) = self.selected_track() else { return };
        let text = if id_only { t.video_id.clone() } else { t.url() };
        // OSC 52: works locally and over SSH in Kitty, WezTerm, iTerm2, Alacritty, foot…
        use std::io::Write;
        let _ = write!(std::io::stdout(), "\x1b]52;c;{}\x07", base64(text.as_bytes()));
        let _ = std::io::stdout().flush();
        self.toast(Level::Info, format!("copied {text}"));
    }

    fn seek(&mut self, secs: f64) {
        // Seek relative to the interpolated position the user sees, not the last report.
        let target = self.display_position().as_secs_f64() + secs;
        self.send(Command::Seek { seconds: target.max(0.0), relative: false });
    }

    fn send(&mut self, cmd: Command) {
        let (client, tx) = (self.client.clone(), self.msg_tx.clone());
        tokio::spawn(async move {
            match client.command(cmd).await {
                Ok(s) => {
                    let _ = tx.send(Msg::State(Box::new(s)));
                }
                Err(e) => {
                    let _ = tx.send(Msg::Toast(Toast { level: Level::Error, text: e.to_string() }));
                }
            }
        });
    }

    fn run_search(&mut self) {
        let q = self.query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.search_seq += 1;
        self.searching = true;
        let (seq, client, tx) = (self.search_seq, self.client.clone(), self.msg_tx.clone());
        tokio::spawn(async move {
            let result = client
                .call(method::SEARCH, json!({ "query": q, "limit": 40, "kind": "songs" }))
                .await
                .map_err(|e| e.to_string())
                .and_then(|v| serde_json::from_value::<Vec<Track>>(v).map_err(|e| e.to_string()));
            let _ = tx.send(Msg::SearchResults { seq, result });
        });
    }
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
