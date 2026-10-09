# 6 · Repository & boilerplate

> **Note:** this is the original plan. The implemented workspace lives in `crates/` and differs in places (no `xtask/`, single `ui/mod.rs`, config in `ytm-core`). See `CONTRIBUTING.md` for the current layout and `docs/ROADMAP.md` for status.

## Workspace layout

```text
ytm-tui/
├── Cargo.toml                    # [workspace] members, shared deps & lints
├── rust-toolchain.toml           # pin stable
├── config/
│   ├── default.toml              # every setting with its default, copied on first run
│   └── keymap.toml               # default key map (section 3)
├── themes/
│   ├── dark.toml                 # tokens from this design system → TrueColor / 256 / 16
│   └── light.toml
├── crates/
│   ├── ytm-api/                  # InnerTube client — no UI, no audio
│   │   └── src/
│   │       ├── lib.rs            # MusicApi trait
│   │       ├── client.rs         # reqwest client, context body, retries, rate limiter
│   │       ├── auth/
│   │       │   ├── headers.rs    # cURL / headers_auth.json import, SAPISIDHASH
│   │       │   ├── oauth.rs      # device-code flow (experimental)
│   │       │   └── store.rs      # keyring + 0600 file fallback
│   │       ├── endpoints/        # search.rs browse.rs next.rs library.rs rate.rs playlist.rs
│   │       ├── parse/            # renderer → model parsers (ported from ytmusicapi)
│   │       └── models.rs         # Track, Album, Artist, Playlist, SearchPage, BrowsePage
│   ├── ytm-audio/                # playback engines
│   │   └── src/
│   │       ├── lib.rs            # AudioBackend trait, PlayerEvent, MediaSource
│   │       ├── mpv.rs            # mpv JSON IPC backend + supervisor
│   │       ├── native.rs         # ffmpeg → rtrb → cpal backend
│   │       ├── loopback.rs       # cpal loopback capture for the visualizer
│   │       ├── resolver.rs       # yt-dlp StreamResolver, URL expiry memo
│   │       └── spectrum.rs       # FFT, log bins, smoothing, peak hold
│   ├── ytm-core/                 # the daemon's brain
│   │   └── src/
│   │       ├── state.rs          # PlayerState, Queue (user + autoplay sections), undo
│   │       ├── reducer.rs        # reduce(state, input) -> Vec<Effect>   (pure, tested)
│   │       ├── effects.rs        # effect executors / actors
│   │       ├── cache/            # sqlite.rs, thumbs.rs, audio.rs (LRU, pinning)
│   │       ├── lyrics/           # lrc.rs (parser), lrclib.rs, ytm.rs, local.rs
│   │       ├── ipc/              # protocol.rs (JSON-RPC types), server.rs, client.rs
│   │       ├── media_controls.rs # souvlaki: MPRIS / Now Playing / SMTC
│   │       ├── net.rs            # connectivity probe, online/degraded/offline
│   │       └── daemon.rs         # wires actors, supervision, shutdown
│   └── ytm-tui/                  # the binary
│       └── src/
│           ├── main.rs           # clap; no subcommand → TUI
│           ├── cli.rs            # CLI subcommands → JSON-RPC calls, --json / --format
│           ├── app.rs            # UiState, Action, event loop
│           ├── keymap.rs         # key sequences (gg, 5j) → Action, from keymap.toml
│           ├── theme.rs          # tokens → ratatui Color per tier
│           └── ui/
│               ├── mod.rs        # top-level layout + breakpoints
│               ├── header.rs  sidebar.rs  footer.rs  popup.rs  toast.rs
│               └── views/        # tracklist.rs grid.rs artist.rs home.rs queue.rs
│                                 # lyrics.rs spectrum.rs art.rs
├── tests/
│   ├── fixtures/innertube/       # recorded JSON responses (scrubbed of personal data)
│   ├── parse_fixtures.rs         # parser vs ytmusicapi oracle outputs
│   └── reducer_scenarios.rs      # queue/shuffle/repeat/expiry scenarios
├── xtask/                        # cargo xtask: record-fixtures, release, man pages
├── docs/                         # user guide, auth guide with screenshots
└── .github/workflows/ci.yml      # fmt, clippy -D warnings, test, cross builds
```

Dependency direction: `ytm-tui → ytm-core → (ytm-api, ytm-audio)`. `ytm-api` and `ytm-audio` never depend on each other or on anything UI.

## Getting started

### 1. Install runtime tools

```sh
# macOS
brew install mpv yt-dlp deno ffmpeg
# Arch
sudo pacman -S mpv yt-dlp deno ffmpeg
# Debian/Ubuntu (yt-dlp from pipx to stay current; deno from deno.land)
sudo apt install mpv ffmpeg && pipx install yt-dlp
curl -fsSL https://deno.land/install.sh | sh
```

### 2. Scaffold the workspace

```sh
cargo new --bin ytm-tui && cd ytm-tui
cargo add ratatui@0.29 crossterm@0.28 --features crossterm/event-stream
cargo add tokio --features full
cargo add futures color-eyre serde_json
cargo add clap --features derive
```

(Start as a single crate with the four files below; split into the workspace above once `ytm-api` gets its first real endpoint.)

### 3. Paste the starter files, run it

```sh
cargo run                       # TUI: press / to load demo results, Enter to "play", q to quit
cargo run -- status --json      # CLI: exits 3 until the daemon exists
```

### 4. Build order (milestones)

| # | Milestone | Done when |
|---|---|---|
| 0 | Starter loop (this page) | Renders the layout, keys work, async demo search |
| 1 | `ytm-audio::mpv` + `resolver` | `ytm-tui play <videoId>` plays audio through mpv |
| 2 | `ytm-api` search + auth import | `/` searches real YouTube Music; `auth import` validates |
| 3 | Daemon + IPC | TUI and CLI control the same playback; `Q` detaches |
| 4 | Queue, radio, likes, library views | Sidebar fully navigable |
| 5 | Lyrics (LRCLIB + YTM), thumbnails, media keys | Lyrics auto-scroll; art in Kitty/WezTerm |
| 6 | Native backend + spectrum | Visualizer at 30 fps < 3 % CPU |
| 7 | Audio cache, offline mode, `doctor` | Plays a pinned playlist with networking off |

## Starter code

A working single-crate starter that compiles against ratatui 0.29 / crossterm 0.28 (checked with Rust 1.97). It already follows the rules above: focus shown by rounded border + `lagoon`, render-on-dirty, a 30 fps ticker only while playing, background tasks reporting back through a channel, and a CLI that speaks JSON-RPC to the daemon socket.

### `Cargo.toml`

```toml
[package]
name = "ytm-tui"
version = "0.1.0"
edition = "2021"

[dependencies]
clap = { version = "4", features = ["derive"] }
color-eyre = "0.6"
crossterm = { version = "0.28", features = ["event-stream"] }
futures = "0.3"
ratatui = "0.29"
serde_json = "1"
tokio = { version = "1", features = ["full"] }
```

### `src/main.rs`

```rust
//! ytm-tui — entry point. No subcommand opens the TUI; anything else is a CLI call.
mod app;
mod cli;
mod theme;
mod ui;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ytm-tui", version, about = "YouTube Music in your terminal")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Search and play the best matching song
    Play { query: String },
    /// Toggle play/pause
    Toggle,
    /// Skip to the next track
    Next,
    /// Go to the previous track
    Prev,
    /// Print player status
    Status {
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    match Cli::parse().cmd {
        None => {
            let terminal = ratatui::init(); // raw mode + alt screen + panic hook
            let result = app::App::new(theme::Theme::detect()).run(terminal).await;
            ratatui::restore();
            result
        }
        Some(cmd) => cli::dispatch(cmd).await,
    }
}
```

### `src/app.rs` — state and event loop

```rust
//! App state + the async event loop. The UI never awaits network work:
//! background tasks report back through `Msg` on an mpsc channel.
use std::time::{Duration, Instant};

use color_eyre::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use tokio::{sync::mpsc, time};

use crate::theme::Theme;

/// What a key press means, independent of which key produced it (remappable).
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit,
    FocusNext,
    FocusPrev,
    Up,
    Down,
    Play,
    TogglePause,
    Next,
    Prev,
    Volume(i8),
    Search,
    Frame,
}

/// Results from background tasks.
#[derive(Debug)]
pub enum Msg {
    SearchResults(Vec<Track>),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub duration: Duration,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pane {
    Sidebar,
    #[default]
    Main,
    Side,
}

impl Pane {
    fn next(self) -> Self {
        match self {
            Pane::Sidebar => Pane::Main,
            Pane::Main => Pane::Side,
            Pane::Side => Pane::Sidebar,
        }
    }
    fn prev(self) -> Self {
        self.next().next()
    }
}

pub struct App {
    pub theme: Theme,
    pub focus: Pane,
    pub tracks: Vec<Track>,
    pub cursor: usize,
    pub now_playing: Option<usize>,
    pub paused: bool,
    pub position: Duration,
    pub volume: u8,
    pub status: Option<String>,
    running: bool,
    dirty: bool,
    last_frame: Instant,
    msg_tx: mpsc::UnboundedSender<Msg>,
}

impl App {
    pub fn new(theme: Theme) -> Self {
        let (msg_tx, _) = mpsc::unbounded_channel();
        Self {
            theme,
            focus: Pane::Main,
            tracks: Vec::new(),
            cursor: 0,
            now_playing: None,
            paused: true,
            position: Duration::ZERO,
            volume: 72,
            status: Some("press / to search".into()),
            running: true,
            dirty: true,
            last_frame: Instant::now(),
            msg_tx,
        }
    }

    fn animating(&self) -> bool {
        self.now_playing.is_some() && !self.paused
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        let (tx, mut msg_rx) = mpsc::unbounded_channel();
        self.msg_tx = tx;
        let mut events = EventStream::new();
        let mut frames = time::interval(Duration::from_millis(33)); // ~30 fps, only while animating
        frames.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

        while self.running {
            if self.dirty {
                terminal.draw(|f| crate::ui::draw(f, &self))?;
                self.dirty = false;
            }
            let animating = self.animating();
            tokio::select! {
                maybe_ev = events.next() => match maybe_ev {
                    Some(Ok(ev)) => self.on_event(ev),
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                },
                Some(msg) = msg_rx.recv() => self.on_msg(msg),
                _ = frames.tick(), if animating => self.update(Action::Frame),
            }
        }
        Ok(())
    }

    fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                if let Some(action) = map_key(k) {
                    self.update(action);
                }
            }
            Event::Resize(..) => self.dirty = true,
            _ => {}
        }
    }

    fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::SearchResults(tracks) => {
                self.status = Some(format!("{} results", tracks.len()));
                self.tracks = tracks;
                self.cursor = 0;
            }
            Msg::Error(e) => self.status = Some(e),
        }
        self.dirty = true;
    }

    pub fn update(&mut self, action: Action) {
        match action {
            Action::Quit => self.running = false,
            Action::FocusNext => self.focus = self.focus.next(),
            Action::FocusPrev => self.focus = self.focus.prev(),
            Action::Down => self.cursor = (self.cursor + 1).min(self.tracks.len().saturating_sub(1)),
            Action::Up => self.cursor = self.cursor.saturating_sub(1),
            Action::Play if !self.tracks.is_empty() => {
                self.now_playing = Some(self.cursor);
                self.position = Duration::ZERO;
                self.paused = false;
                self.last_frame = Instant::now();
            }
            Action::Play => {}
            Action::TogglePause => {
                self.paused = !self.paused;
                self.last_frame = Instant::now();
            }
            Action::Next | Action::Prev => {
                if let Some(i) = self.now_playing {
                    let n = self.tracks.len();
                    let j = if action == Action::Next { (i + 1) % n } else { (i + n - 1) % n };
                    self.now_playing = Some(j);
                    self.position = Duration::ZERO;
                }
            }
            Action::Volume(d) => self.volume = (self.volume as i16 + d as i16).clamp(0, 100) as u8,
            Action::Search => {
                self.status = Some("searching…".into());
                let tx = self.msg_tx.clone();
                tokio::spawn(async move {
                    // Real code: daemon.call("api.search", query).await
                    time::sleep(Duration::from_millis(300)).await;
                    let _ = tx.send(Msg::SearchResults(demo_tracks()));
                });
            }
            Action::Frame => {
                // Stand-in for interpolating the backend's reported position.
                let now = Instant::now();
                self.position += now - self.last_frame;
                self.last_frame = now;
                if let Some(t) = self.now_playing.and_then(|i| self.tracks.get(i)) {
                    if self.position >= t.duration {
                        return self.update(Action::Next);
                    }
                }
            }
        }
        self.dirty = true;
    }
}

fn map_key(k: KeyEvent) -> Option<Action> {
    use KeyCode::*;
    Some(match (k.code, k.modifiers) {
        (Char('c'), KeyModifiers::CONTROL) | (Char('q'), _) => Action::Quit,
        (Tab, _) => Action::FocusNext,
        (BackTab, _) => Action::FocusPrev,
        (Char('j') | Down, _) => Action::Down,
        (Char('k') | Up, _) => Action::Up,
        (Enter, _) => Action::Play,
        (Char(' '), _) => Action::TogglePause,
        (Char('n'), _) => Action::Next,
        (Char('p'), _) => Action::Prev,
        (Char('+') | Char('='), _) => Action::Volume(5),
        (Char('-'), _) => Action::Volume(-5),
        (Char('/'), _) => Action::Search,
        _ => return None,
    })
}

fn demo_tracks() -> Vec<Track> {
    [("Glasswater Signals", 318), ("Paper Satellites", 290), ("Northbound at Dusk", 249)]
        .into_iter()
        .map(|(t, s)| Track {
            title: t.into(),
            artist: "Lowtide Assembly".into(),
            duration: Duration::from_secs(s),
        })
        .collect()
}
```

### `src/ui.rs` — rendering

```rust
//! Pure rendering: (&App) -> Frame. No state changes here.
use std::time::Duration;

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style, Stylize},
    symbols,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, LineGauge, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::{App, Pane};

pub fn draw(f: &mut Frame, app: &App) {
    let t = &app.theme;
    let [header, body, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(5), Constraint::Length(3)]).areas(f.area());
    let [side, main, panel] =
        Layout::horizontal([Constraint::Length(20), Constraint::Min(30), Constraint::Length(24)]).areas(body);

    // Header
    let state = match (app.now_playing, app.paused) {
        (None, _) => Span::styled(" ■ STOPPED", Style::new().fg(t.ink_muted)),
        (Some(_), true) => Span::styled(" ‖ PAUSED", Style::new().fg(t.ink)),
        (Some(_), false) => Span::styled(" ▶ PLAYING", Style::new().fg(t.ember).bold()),
    };
    let status = Span::styled(format!("   {}", app.status.clone().unwrap_or_default()), Style::new().fg(t.ink_muted));
    f.render_widget(Paragraph::new(Line::from(vec![state, status])).style(Style::new().bg(t.bg_raised)), header);

    // Sidebar
    let nav = ["⌂ Home", "◇ Explore", "▤ Library", "≡ Playlists", "♥ Liked Songs", "◷ History", "» Queue"];
    let items: Vec<ListItem> = nav.iter().map(|s| ListItem::new(format!(" {s}")).fg(t.ink)).collect();
    f.render_widget(List::new(items).block(pane(app, Pane::Sidebar, "NAVIGATE")), side);

    // Main: track list
    let rows: Vec<ListItem> = app
        .tracks
        .iter()
        .enumerate()
        .map(|(i, tr)| {
            let playing = app.now_playing == Some(i);
            let title = Style::new().fg(if playing { t.ember } else { t.ink });
            ListItem::new(Line::from(vec![
                Span::styled(if playing { "▶ " } else { "  " }, Style::new().fg(t.ember)),
                Span::styled(format!("{:<24}", tr.title), if playing { title.bold() } else { title }),
                Span::styled(format!("{:<18}", tr.artist), Style::new().fg(t.ink_muted)),
                Span::styled(mmss(tr.duration), Style::new().fg(t.ink_muted)),
            ]))
        })
        .collect();
    let list = List::new(rows)
        .block(pane(app, Pane::Main, "TRACKS"))
        .highlight_style(Style::new().bg(t.bg_select))
        .highlight_symbol("›");
    let mut state = ListState::default().with_selected((!app.tracks.is_empty()).then_some(app.cursor));
    f.render_stateful_widget(list, main, &mut state);

    // Side panel (lyrics / spectrum go here)
    f.render_widget(
        Paragraph::new(" no lyrics yet").fg(t.ink_muted).block(pane(app, Pane::Side, "LYRICS")),
        panel,
    );

    draw_footer(f, app, footer);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let block = Block::new().style(Style::new().bg(t.bg_raised));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [info, progress, hints] = Layout::vertical([Constraint::Length(1); 3]).areas(inner);

    let track = app.now_playing.and_then(|i| app.tracks.get(i));
    let line = match track {
        Some(tr) => Line::from(vec![
            Span::styled(" ▶ ", Style::new().fg(t.ember)),
            Span::styled(tr.title.clone(), Style::new().fg(t.ember).add_modifier(Modifier::BOLD)),
            Span::styled(" — ", Style::new().fg(t.ink_faint)),
            Span::styled(tr.artist.clone(), Style::new().fg(t.ink)),
        ]),
        None => Line::styled(" nothing playing", Style::new().fg(t.ink_muted)),
    };
    f.render_widget(Paragraph::new(line), info);

    let total = track.map(|t| t.duration).unwrap_or(Duration::ZERO);
    let ratio = if total.is_zero() { 0.0 } else { (app.position.as_secs_f64() / total.as_secs_f64()).min(1.0) };
    let gauge = LineGauge::default()
        .ratio(ratio)
        .label(Span::styled(format!(" {} / {}  vol {}% ", mmss(app.position), mmss(total), app.volume), Style::new().fg(t.ink_muted)))
        .line_set(symbols::line::THICK)
        .filled_style(Style::new().fg(t.ember))
        .unfilled_style(Style::new().fg(t.border));
    f.render_widget(gauge, progress);

    let keys = [("/", "search"), ("⏎", "play"), ("␣", "pause"), ("n/p", "next/prev"), ("+/-", "vol"), ("q", "quit")];
    let spans: Vec<Span> = keys
        .iter()
        .flat_map(|(k, a)| [Span::styled(format!(" {k}"), Style::new().fg(t.lagoon)), Span::styled(format!(" {a} "), Style::new().fg(t.ink_muted))])
        .collect();
    f.render_widget(Paragraph::new(Line::from(spans)), hints);
}

/// Unfocused: plain border in `border`. Focused: rounded border + bold title in `lagoon`.
fn pane<'a>(app: &App, which: Pane, title: &'a str) -> Block<'a> {
    let t = &app.theme;
    let focused = app.focus == which;
    let color = if focused { t.lagoon } else { t.border };
    Block::new()
        .borders(Borders::ALL)
        .border_type(if focused { BorderType::Rounded } else { BorderType::Plain })
        .border_style(Style::new().fg(color))
        .title(Span::styled(
            format!(" {title} "),
            Style::new().fg(if focused { t.lagoon } else { t.ink_muted }).add_modifier(Modifier::BOLD),
        ))
}

fn mmss(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}
```

### `src/theme.rs` — tokens per color tier

```rust
//! Design tokens → ratatui colors, resolved once for the terminal's color tier.
use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    TrueColor,
    Ansi256,
    Ansi16,
    Mono,
}

impl Tier {
    pub fn detect() -> Self {
        let env = |k| std::env::var(k).unwrap_or_default();
        if std::env::var_os("NO_COLOR").is_some() {
            Tier::Mono
        } else if matches!(env("COLORTERM").as_str(), "truecolor" | "24bit") {
            Tier::TrueColor
        } else if env("TERM").contains("256color") {
            Tier::Ansi256
        } else {
            Tier::Ansi16
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub tier: Tier,
    pub bg_raised: Color,
    pub bg_select: Color,
    pub border: Color,
    pub ink: Color,
    pub ink_muted: Color,
    pub ink_faint: Color,
    pub ember: Color,  // now playing, progress, liked
    pub lagoon: Color, // focus
    pub amber: Color,  // buffering, rate limit, cached
    pub moss: Color,   // online
    pub error: Color,
}

impl Theme {
    pub fn detect() -> Self {
        Self::dark(Tier::detect())
    }

    /// Dark theme. Each token: (TrueColor hex, ANSI-256 index, 16-color fallback).
    pub fn dark(tier: Tier) -> Self {
        let c = |rgb: u32, idx: u8, ansi16: Color| match tier {
            Tier::TrueColor => Color::from_u32(rgb),
            Tier::Ansi256 => Color::Indexed(idx),
            Tier::Ansi16 => ansi16,
            Tier::Mono => Color::Reset,
        };
        Self {
            tier,
            bg_raised: c(0x1c1f26, 234, Color::Reset),
            bg_select: c(0x2a2f3a, 236, Color::Reset), // 16/mono: reverse video instead
            border: c(0x3d4452, 238, Color::DarkGray),
            ink: c(0xe6e1d6, 253, Color::Reset),
            ink_muted: c(0xa39d90, 247, Color::Gray),
            ink_faint: c(0x6b675f, 241, Color::DarkGray),
            ember: c(0xf4845f, 209, Color::LightRed),
            lagoon: c(0x5fc4b8, 79, Color::Cyan),
            amber: c(0xe8b75c, 179, Color::Yellow),
            moss: c(0x93c47d, 114, Color::Green),
            error: c(0xff6b78, 204, Color::Red),
        }
    }
}
```

### `src/cli.rs` — headless client

```rust
//! Headless CLI: one JSON-RPC request per invocation to the daemon socket.
use color_eyre::{eyre::eyre, Result};
use serde_json::{json, Value};

use crate::Cmd;

pub async fn dispatch(cmd: Cmd) -> Result<()> {
    let (method, params) = match &cmd {
        Cmd::Play { query } => ("player.play_query", json!({ "query": query })),
        Cmd::Toggle => ("player.toggle", json!({})),
        Cmd::Next => ("player.next", json!({})),
        Cmd::Prev => ("player.prev", json!({})),
        Cmd::Status { .. } => ("player.status", json!({})),
    };
    let reply = call(method, params).await?;
    match cmd {
        Cmd::Status { json: true } => println!("{}", serde_json::to_string_pretty(&reply)?),
        Cmd::Status { json: false } => println!(
            "{} — {} [{}]",
            reply["track"]["title"].as_str().unwrap_or("-"),
            reply["track"]["artists"][0].as_str().unwrap_or("-"),
            reply["state"].as_str().unwrap_or("stopped"),
        ),
        _ => {}
    }
    Ok(())
}

#[cfg(unix)]
async fn call(method: &str, params: Value) -> Result<Value> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| std::env::temp_dir().display().to_string());
    let path = std::path::Path::new(&dir).join("ytm-tui.sock");
    let Ok(stream) = tokio::net::UnixStream::connect(&path).await else {
        eprintln!("ytm-tui: daemon not running (start it with `ytm-tui daemon`)");
        std::process::exit(3);
    };
    let (rd, mut wr) = stream.into_split();
    let req = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    wr.write_all(format!("{req}\n").as_bytes()).await?;
    let mut line = String::new();
    BufReader::new(rd).read_line(&mut line).await?;
    let mut resp: Value = serde_json::from_str(&line)?;
    match resp.get("error") {
        Some(err) => Err(eyre!("{}", err["message"].as_str().unwrap_or("daemon error"))),
        None => Ok(resp["result"].take()),
    }
}

#[cfg(not(unix))]
async fn call(_method: &str, _params: Value) -> Result<Value> {
    Err(eyre!("named-pipe transport not implemented in the starter"))
}
```

## Default config

```toml
# ~/.config/ytm-tui/config.toml
[ui]
theme = "auto"              # auto | dark | light
paint_background = false    # keep terminal transparency
icons = "unicode"           # unicode | ascii | nerd
mouse = true
side_panel = "lyrics"       # lyrics | spectrum | art | hidden

[audio]
backend = "mpv"             # mpv | native
prefer = "opus"             # opus | aac
volume = 72
gapless = true

[player]
autoplay = true             # radio when the queue runs out

[cache]
thumbs_max = "200MiB"
audio = false               # opt-in raw audio cache
audio_max = "5GiB"

[network]
rate_limit_rps = 5
timeout_connect_s = 5
timeout_read_s = 15

[daemon]
persist = false             # true: q leaves music playing
socket = "auto"
```
