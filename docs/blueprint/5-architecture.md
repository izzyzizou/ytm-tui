# 5 · System architecture & data flow

## Process model

```text
 ┌──────────────┐   ┌──────────────┐   ┌──────────────┐   ┌───────────────┐
 │  TUI client  │   │  CLI client  │   │ media keys / │   │ status bars   │
 │  (ratatui)   │   │  (clap)      │   │ MPRIS / SMTC │   │ (watch --json)│
 └──────┬───────┘   └──────┬───────┘   └──────┬───────┘   └──────┬────────┘
        │  JSON-RPC 2.0 over Unix socket / named pipe            │
        └──────────────────┴────────┬─────────┴──────────────────┘
                                    ▼
 ┌──────────────────────────── ytm-tui daemon ───────────────────────────────┐
 │                                                                            │
 │   IpcServer ──Command──►  ┌──────────────┐ ──Effect──►  ApiActor  (InnerTube)
 │                           │  Core        │ ──Effect──►  Resolver  (yt-dlp)
 │   MediaControls ─Command─►│  state +     │ ──Effect──►  LyricsActor (LRCLIB/YTM)
 │                           │  reducer     │ ──Effect──►  CacheActor  (SQLite + files)
 │   ◄──Event broadcast──────│  (one task)  │ ──Cmd────►  PlayerActor ──► AudioBackend
 │                           └──────▲───────┘                  │      (mpv | native)
 │                                  └──────── Msg (results, player events) ◄──┘
 └────────────────────────────────────────────────────────────────────────────┘
                                                     audio thread ──PCM tap──► rtrb ring
                                                                    (native / loopback)
```

When the TUI starts and no daemon is listening, it spawns one (`ytm-tui daemon --detach`) and connects. In single-process mode (`--embedded`, useful for debugging) the same actors run inside the TUI process and the "socket" is an in-memory channel — the protocol types are identical.

## State management: one owner, unidirectional

The **Core** task exclusively owns `PlayerState` (queue, current index, position, volume, shuffle/repeat, auth status, network status). Everything else sends it messages; nothing else mutates state.

```text
Command (from a client)  ─┐
Msg (from an actor)      ─┼─► reduce(&mut state, input) -> Vec<Effect>
Tick (1 Hz housekeeping) ─┘          │
                                     ├─► effects dispatched to actors (non-blocking sends)
                                     └─► Event::StateChanged(diff) broadcast to clients
```

- `reduce` is a pure, synchronous function — trivially unit-testable with recorded inputs.
- Effects are data (`Effect::Resolve(video_id)`, `Effect::Fetch(Request)`, `Effect::Player(PlayerCmd::Load{..})`), executed by actors that report back as `Msg`.
- Clients receive an initial snapshot, then diffs over a `tokio::sync::broadcast` channel. A slow client that lags gets a fresh snapshot instead of blocking the core.

**TUI side.** The TUI has its own `UiState` (focused pane, cursors, scroll offsets, open popups, view history, search text) plus a mirror of `PlayerState`. It follows the same pattern: `crossterm` events → `Action` → `update()` → maybe an RPC call. View data (search results, a playlist's tracks) is fetched through the daemon (`api.search`, `api.browse`) so it shares the daemon's cache and rate limiter.

## The TUI event loop

One `tokio::select!` over four sources; rendering happens only when something changed.

| Source | Produces | Notes |
|---|---|---|
| `crossterm::EventStream` | Key, mouse, resize, focus, paste | Key **press** events only (Windows also reports releases) |
| Daemon event stream | `StateChanged`, `Toast`, `Position` | From the socket reader task |
| RPC responses | Search results, browse pages | Matched to requests by id; stale responses (superseded search) are dropped |
| Frame ticker 33 ms | Animation frame | Enabled only while playing **and** something animates (progress, spectrum, lyrics). Paused → no ticks, ~0 % CPU |

`terminal.draw()` is called when `dirty` is set; ratatui diffs the buffer so only changed cells are written. Typical frame: < 1 ms render, ~2–5 KB written.

## Async vs real-time: keeping audio glitch-free

| Thread / task | Runs on | Rules |
|---|---|---|
| Audio callback (native backend) | `cpal`'s real-time thread | No allocation, no locks, no I/O, no logging. Reads PCM from an `rtrb` ring, writes a copy to the visualizer ring with `push` that drops on full. |
| Decoder feeder | Dedicated std thread | Reads ffmpeg stdout, pushes into the PCM ring; blocks when the ring is full (natural backpressure). |
| mpv | Separate process | Controlled by `PlayerActor` over IPC; property observers (`time-pos`, `pause`, `eof-reached`, `demuxer-cache-state`) become `Msg`s. |
| Network / IPC / SQLite | tokio tasks; SQLite via `spawn_blocking` | Every request has a timeout and a `CancellationToken`. |
| Spectrum FFT | UI tick in the TUI process | 2048-sample Hann window, `rustfft` (plan cached), magnitudes → log-spaced bins → dB → smoothing (attack 0.6, decay 0.15) → peak hold. ~0.1 ms per frame. |

For the visualizer in a separate TUI process, the daemon publishes the reduced spectrum (one `u8` per bar, ≤ 200 bytes) at 30 Hz on a dedicated `spectrum` subscription, not raw PCM.

Search-as-you-type: debounce 250 ms, cancel the previous in-flight request via its token, ignore any response whose request id is no longer current.

## Core traits

```rust
#[async_trait::async_trait]
pub trait MusicApi: Send + Sync {
    async fn search(&self, q: &str, filter: SearchFilter) -> Result<SearchPage, ApiError>;
    async fn browse(&self, id: &BrowseId) -> Result<BrowsePage, ApiError>;
    async fn watch_next(&self, video_id: &str, radio: bool) -> Result<WatchPlaylist, ApiError>;
    async fn rate(&self, video_id: &str, rating: Rating) -> Result<(), ApiError>;
    async fn library_playlists(&self) -> Result<Vec<PlaylistSummary>, ApiError>;
}

#[async_trait::async_trait]
pub trait StreamResolver: Send + Sync {
    async fn resolve(&self, video_id: &str) -> Result<StreamInfo, ResolveError>;
}

pub trait AudioBackend: Send {
    fn load(&mut self, src: MediaSource, start: Duration) -> Result<(), PlayerError>;
    fn enqueue_next(&mut self, src: MediaSource) -> Result<(), PlayerError>; // gapless
    fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError>;
    fn seek(&mut self, to: Duration) -> Result<(), PlayerError>;
    fn set_volume(&mut self, percent: u8) -> Result<(), PlayerError>;
    fn events(&mut self) -> tokio::sync::mpsc::Receiver<PlayerEvent>; // Position, Ended, Buffering, Error
    fn pcm_tap(&self) -> Option<rtrb::Consumer<f32>>;                  // None for mpv w/o loopback
}

pub enum MediaSource { Url { url: String, headers: Vec<(String, String)> }, File(std::path::PathBuf) }
```

## Error handling

Errors are typed per layer (`ApiError`, `ResolveError`, `PlayerError` via `thiserror`), mapped by the core to a **user-facing condition** with a recovery policy. Nothing panics on bad network data.

| Condition | Detection | Recovery | What the user sees |
|---|---|---|---|
| **Rate limited** | HTTP 429, or InnerTube error body / consent interstitial | Per-host token bucket (default 5 req/s, burst 10) prevents most; on hit, exponential backoff with full jitter (1 s → 60 s cap), honour `Retry-After`. Background work (prefetch, thumbnails) pauses first; user-initiated requests get priority. | `rate limited · retry in 12s` in `amber`, countdown in header |
| **Stream URL expired** | `expires_at` within 10 min before load, or HTTP 403/410 from the media host mid-stream (mpv: `end-file` with error; native: range request 403) | Re-resolve via yt-dlp, reload at last known position, max 2 attempts per track | Brief `◌ BUFFERING`; on failure `✗ can't play · skipped` toast and auto-skip |
| **yt-dlp failure** | Non-zero exit; stderr patterns like "Sign in to confirm", "Video unavailable", "Requested format is not available" | Bot check → suggest `auth import` and `yt-dlp -U`; unavailable → skip, mark track; format → retry with `bestaudio` | Specific toast with the fix, not the raw stderr (that goes to the log) |
| **Network drop** | reqwest connect/read timeouts (5 s / 15 s), mpv `paused-for-cache`, periodic HEAD probe when errors cluster | Enter **degraded** (amber) after 2 failures, **offline** (red) after probe fails; playback continues from buffer, then from cache; queue skips uncached tracks; probe every 5 s → 60 s; resume automatically | Header `● degraded` / `✗ offline · playing from cache` |
| **Auth expired** | 401/403 on authed endpoints, or account menu missing | Stop authed calls, keep public playback, single toast with `ytm-tui auth import` | `◉ signed out` in `amber` |
| **Schema drift** | Parser can't find an expected renderer / field | Parsers return `Result` with a JSON-path context; unknown shelf/renderer types are skipped and logged, never fatal; the raw response is saved to `state/failed/` for bug reports | Partially filled view + `some items couldn't be shown` once per session |
| **mpv crash / missing** | IPC socket closes, process exit | Supervisor restarts mpv (3 tries/min) and reloads at last position; if missing, suggest `--backend native` | `player restarted` toast |
| **Audio device change** | cpal stream error | Rebuild output stream on the new default device | — (seamless) |
| **Panic** | `ratatui::init` panic hook + `color-eyre` | Terminal restored first, report written to `state/crash-<ts>.log`; daemon panics are caught per-task and the task restarted | Normal shell + path to the crash log |

Logging: `tracing` to `~/.local/state/ytm-tui/ytm-tui.log` (rotated daily, 7 kept), `RUST_LOG`-style filter via `--log-level`. Cookies and `Authorization` are redacted by a custom field formatter.
