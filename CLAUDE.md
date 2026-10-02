# CLAUDE.md — working on ytm-tui

A keyboard-first YouTube Music client for the terminal (Rust · ratatui · tokio). Read this
first, then `docs/ROADMAP.md` for what's next. The full design is in `docs/blueprint/` and the
visual language (colors, glyphs, layout rules) in `docs/design-system.md`.

## Commands

```sh
cargo build                              # debug build → target/debug/ytm-tui
cargo test --workspace                   # all unit tests (no network, no audio)
cargo clippy --workspace --all-targets   # must stay warning-free (CI uses -D warnings)
cargo fmt --all                          # rustfmt.toml: max_width = 140

# Run against an isolated, silent daemon (no mpv needed, no audio, real search):
export YTM_TUI_SOCKET=/tmp/ytm-dev.sock YTM_TUI_CONFIG_DIR=/tmp/ytm-dev-cfg YTM_TUI_STATE_DIR=/tmp/ytm-dev-state
cargo run -- --backend null              # TUI (auto-starts the daemon)
cargo run -- --backend null search daft punk
cargo run -- status --json
cargo run -- quit                        # stop the daemon
```

Logs: `$YTM_TUI_STATE_DIR/{daemon,tui}.log.<date>`; level via `YTM_TUI_LOG=debug`.

## Architecture (one paragraph)

`ytm-tui daemon` owns everything stateful: the queue, the audio backend, network access.
The TUI and the CLI are thin clients talking newline-delimited JSON over a Unix socket
(`ytm-core/src/protocol.rs`). Inside the daemon one core task owns `PlayerState`; every
input (client command, backend event, resolver result) goes through the **pure**
`reducer::reduce(&mut state, input) -> Vec<Effect>`, and the daemon executes the effects.
All player logic lives in the reducer and is unit-tested there.

```
crates/
  ytm-api/    InnerTube client: auth import + SAPISIDHASH, search + resilient parsing
  ytm-audio/  AudioBackend trait; mpv (JSON IPC), null (simulated clock); yt-dlp resolver
  ytm-core/   state, reducer, LRC parser, LRCLIB lyrics, daemon, IPC protocol + client
  ytm-tui/    the binary: clap CLI, ratatui TUI (app.rs = state/input, ui/ = rendering)
```

Dependency direction: `ytm-tui → ytm-core → (ytm-api, ytm-audio)`. `ytm-api` and
`ytm-audio` never depend on each other or on UI code.

## Rules of the road

- **Player logic goes in `reducer.rs`**, with a test. The daemon only executes effects;
  the TUI only sends `Command`s and renders `PlayerState`.
- **Never block the core loop or the UI loop.** Network and subprocess work is spawned and
  reports back through a channel (`Input::Resolved`, `Msg::SearchResults`). Stale results
  are dropped by sequence number (`load_seq`, `search_seq`).
- **InnerTube parsing must not panic** on unknown shapes: skip and log. Add a fixture under
  `crates/ytm-api/tests/fixtures/` for any new endpoint (scrub cookies/personal data).
- **yt-dlp is the only stream extractor.** Don't re-implement signature/PO-token logic.
- **Colors come from `theme.rs` tokens** (mirrors `docs/design-tokens.json`). One meaning
  each: ember = now playing, lagoon = focus, amber = waiting, moss = healthy, error = broken.
  Focused pane = rounded border + lagoon. Never color without a glyph/word too.
- Secrets: auth file and cookies.txt are written 0600; never log `Cookie`/`Authorization`.
- `unsafe` is forbidden workspace-wide.
- Commits end with the session's attribution trailer if your environment provides one.

## Known constraints

- From datacenter IPs YouTube answers yt-dlp with "Sign in to confirm you're not a bot";
  on a home connection it usually works anonymously, otherwise `ytm-tui auth import`.
  The resolver maps that error to `ResolveError::BotCheck` with a helpful message.
- Anonymous search returns no "Songs" filter chip, so results mix videos/episodes and many
  lack durations. Signed-in sessions get the Songs shelf (`search::filter_params`).
- `InnerTube::CLIENT_VERSION` should be bumped occasionally.
- Windows: socket/mpv transports are Unix-only for now (named pipes are a TODO).
