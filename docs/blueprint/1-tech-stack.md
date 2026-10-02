# 1 · Tech stack

## Summary

`ytm-tui` is a Rust workspace: a **playback daemon** that owns the queue, the audio backend and all network access, and two thin clients on top of it — the full-screen **TUI** (ratatui) and the scriptable **CLI** (clap). The two clients speak the same JSON-RPC protocol over a local socket, so `ytm-tui next` from a shell, a media key, and `n` in the TUI all go through one code path.

**Recommendation: Rust + ratatui + crossterm + tokio**, with **mpv** as the default audio engine, **InnerTube** (the YouTube Music web client's private API) for metadata, and **yt-dlp** for stream URLs.

## Why Rust over Go or Python

| | Rust · ratatui | Go · bubbletea | Python · textual |
|---|---|---|---|
| Render model | Immediate mode, diffed buffer; 30 fps visualizer at ~1–2% CPU | Elm-style, string-rendered views; fine for lists, heavier for per-frame spectra | Retained widget tree + CSS; very productive, highest CPU/RAM |
| Startup | ~10 ms, single static binary | ~10 ms, single binary | 300–800 ms, needs a Python env |
| Audio / DSP | `cpal`, `rustfft`, lock-free ring buffers, no GC pauses on the audio thread | Good via cgo/oto; GC is rarely a problem but cgo complicates builds | Needs native extensions; GIL contention with the UI |
| YTM API | Port parsers from ytmusicapi (or run it as a sidecar) | Same as Rust | `ytmusicapi` directly — the biggest pull toward Python |
| Image protocols | `ratatui-image` (Kitty, Sixel, iTerm2, half-block fallback) | Partial, DIY | `textual-image` |

Python wins on time-to-first-prototype because `ytmusicapi` is native there. Rust wins on everything a long-running, always-on player needs: memory (~25 MB RSS), a predictable audio thread, one binary to ship. The blueprint keeps a door open: phase 0 can run `ytmusicapi` as a sidecar process behind the same `MusicApi` trait while the Rust parsers are written (see "API strategy").

## Core crates

| Concern | Crate | Notes |
|---|---|---|
| TUI | `ratatui` 0.29+, `crossterm` 0.28 (`event-stream`) | Use `ratatui::init()` / `restore()` — installs a panic hook that restores the terminal. |
| Async runtime | `tokio` (multi-thread) | Network, IPC, timers. Never touches the audio callback. |
| CLI | `clap` 4 (derive) | Subcommands + `--json` output. |
| HTTP | `reqwest` (rustls, gzip, brotli, cookies off — we set headers ourselves) | One shared client, HTTP/2. |
| Serialization | `serde`, `serde_json`, `toml` | |
| Errors / logs | `color-eyre`, `thiserror`, `tracing`, `tracing-appender` | Logs to file; stdout belongs to the TUI. |
| Paths | `directories` | XDG on Linux, `~/Library/...` on macOS, `%APPDATA%` on Windows. |
| Cache index | `rusqlite` (bundled) | Metadata TTLs, audio/thumbnail LRU, lyric cache. |
| Secrets | `keyring` | Cookies/tokens in Keychain / Secret Service / Credential Manager; 0600 file fallback. |
| DSP | `rustfft`, `rtrb` (lock-free SPSC ring) | Spectrum from a PCM tap. |
| Audio out (native backend) | `cpal` | Also used for loopback capture for the visualizer. |
| Album art | `ratatui-image`, `image` (webp, jpeg) | Picks Kitty / Sixel / iTerm2 protocol at runtime; half-blocks `▀` elsewhere. |
| Media keys / OS integration | `souvlaki` | MPRIS (Linux), Now Playing (macOS), SMTC (Windows). |
| Clipboard | `arboard` + OSC 52 | OSC 52 makes `y` work over SSH. |
| IPC | `tokio::net::UnixListener` / named pipes (`interprocess` on Windows) | Newline-delimited JSON-RPC 2.0. |

## Audio backends

Everything implements one trait (section 5), so backends are swappable at runtime with `--backend` or `audio.backend` in config.

| Backend | How | Strengths | Weaknesses | Role |
|---|---|---|---|---|
| **mpv** (default) | Spawn `mpv --idle --no-video --input-ipc-server=<sock>` and drive it over its JSON IPC | Handles HTTPS streaming, Opus/AAC, demuxer cache, seeking in remote streams, gapless (`--prefetch-playlist`), ReplayGain. Process isolation: an mpv crash doesn't take the UI down. | mpv does not expose decoded PCM, so the visualizer needs loopback capture. Requires mpv installed. | Default |
| **native** | yt-dlp URL → `ffmpeg` decodes to `f32le` 48 kHz stereo on stdout → ring buffer → `cpal` output, with a tap for FFT | Exact PCM for the visualizer, sample-accurate position, no mpv dependency. | Seeking restarts ffmpeg with `-ss`; gapless and HTTP retry logic are ours to write. | Opt-in, for best visualizer |
| rodio + symphonia | Pure-Rust decode | No external binaries | Symphonia has no Opus decoder, and YouTube's best audio format is Opus/WebM — would force the 128 kb/s AAC stream | Not recommended |
| GStreamer | `gstreamer-rs` pipeline with `appsink` tap | Everything in one framework | Heavy runtime dependency, painful on macOS/Windows | Not recommended |

**Visualizer source by backend:** native → direct PCM tap. mpv → `cpal` loopback capture of the output device where the OS allows it (PipeWire/PulseAudio monitor source on Linux, WASAPI loopback on Windows; on macOS a virtual device such as BlackHole is needed). If no source is available the Spectrum tab says so and offers `--backend native`.

## API strategy

Three independent pipelines, each behind a trait so it can be replaced when YouTube changes something:

1. **Metadata — InnerTube, called directly from Rust.** `POST https://music.youtube.com/youtubei/v1/{search,browse,next,player,like/like,like/removelike,browse/edit_playlist,...}` with the `WEB_REMIX` client context. Response parsing is ported from `ytmusicapi` (MIT-licensed), which also serves as a test oracle: record its outputs on fixtures and assert the Rust parser agrees. **Phase-0 alternative:** run `ytmusicapi` in a small Python sidecar speaking the same JSON-RPC shape, and swap it out endpoint by endpoint.
2. **Streams — yt-dlp as a subprocess.** `yt-dlp -J --no-playlist -f "bestaudio[acodec=opus]/bestaudio" https://music.youtube.com/watch?v=<id>` returns the media URL, codec, bitrate, size and expiry. yt-dlp keeps up with signature/n-parameter changes and proof-of-origin tokens far faster than any in-house extractor could. Recent yt-dlp releases expect an external JavaScript runtime (Deno) for full YouTube support; `ytm-tui doctor` checks for it. Never re-implement the extractor.
3. **Lyrics — LRCLIB first, YouTube Music second.** LRCLIB (`https://lrclib.net/api/get`) returns time-synced LRC by artist/title/album/duration; YouTube Music's own lyrics tab is the unsynced fallback.

## Runtime dependencies

| Tool | Needed for | Check |
|---|---|---|
| `mpv` ≥ 0.35 | Default backend | `mpv --version` |
| `yt-dlp` (current release) | Stream resolution | `yt-dlp --version`; `ytm-tui doctor` warns if older than ~60 days |
| `deno` | yt-dlp's YouTube JS challenges | `deno --version` |
| `ffmpeg` | Native backend, audio cache remux | `ffmpeg -version` |
| A TrueColor terminal | Full theme (256/16-color fallbacks exist) | `echo $COLORTERM` |
