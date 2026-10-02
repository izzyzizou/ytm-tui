# Contributing to ytm-tui

Thanks for your interest! Bug reports, small fixes, and features from `docs/ROADMAP.md` are
all welcome. For anything large, open an issue first so we can agree on the approach.

## Development

```sh
cargo build                              # debug build → target/debug/ytm-tui
cargo test --workspace                   # all unit tests (no network, no audio)
cargo clippy --workspace --all-targets   # must stay warning-free (CI uses -D warnings)
cargo fmt --all                          # rustfmt.toml: max_width = 140
```

Run against an isolated, silent daemon (no mpv, no audio, real search):

```sh
export YTM_TUI_SOCKET=/tmp/ytm-dev.sock YTM_TUI_CONFIG_DIR=/tmp/ytm-dev-cfg YTM_TUI_STATE_DIR=/tmp/ytm-dev-state
cargo run -- --backend null              # TUI (auto-starts the daemon)
cargo run -- --backend null search daft punk
cargo run -- status --json
cargo run -- quit                        # stop the daemon
```

Logs go to `$YTM_TUI_STATE_DIR/{daemon,tui}.log.<date>`. Set the level with `YTM_TUI_LOG=debug`.

## Architecture

`ytm-tui daemon` owns everything stateful: the queue, the audio backend, and network access.
The TUI and the CLI are thin clients. They talk newline-delimited JSON over a Unix socket
(`crates/ytm-core/src/protocol.rs`). Inside the daemon, one core task owns `PlayerState`.
Every input goes through the pure `reducer::reduce(&mut state, input) -> Vec<Effect>`, and
the daemon executes the returned effects.

```
crates/
  ytm-api/    InnerTube client: auth import + SAPISIDHASH, search + resilient parsing
  ytm-audio/  AudioBackend trait; mpv (JSON IPC), null (simulated clock); yt-dlp resolver
  ytm-core/   state, reducer, LRC parser, LRCLIB lyrics, daemon, IPC protocol + client
  ytm-tui/    the binary: clap CLI, ratatui TUI (app.rs = state/input, ui/ = rendering)
```

Dependencies point one way: `ytm-tui → ytm-core → (ytm-api, ytm-audio)`.

The design lives in `docs/blueprint/`. The visual language (colors, glyphs, layout) is in
`docs/design-system.md`.

## Ground rules

- **Player logic goes in `reducer.rs`**, with a test. The daemon only executes effects. The
  TUI only sends `Command`s and renders `PlayerState`.
- **Never block the core loop or the UI loop.** Spawn network and subprocess work, and have
  it report back through a channel. Drop stale results by sequence number.
- **InnerTube parsing must not panic** on unknown shapes: skip and log. Add a fixture under
  `crates/ytm-api/tests/fixtures/` for any new endpoint.
- **yt-dlp is the only stream extractor.** Don't re-implement signature or PO-token logic.
- **Colors come from `theme.rs` tokens** (mirrors `docs/design-tokens.json`). Never use color
  without a glyph or word next to it.
- `unsafe` is forbidden workspace-wide.

## Never commit credentials

Fixtures and bug reports must not contain cookies, `Authorization` headers, `SAPISID`
values, `visitorData`, or account names. Scrub captured InnerTube responses before adding
them. Don't commit a "Copy as cURL" dump or a `headers_auth.json`. Code must never log the
`Cookie` or `Authorization` headers.

## Pull requests

- Keep each PR focused, and describe what changed and why.
- Make sure `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  and `cargo test --workspace` pass.
- By contributing, you agree that your work is licensed under the MIT license.
