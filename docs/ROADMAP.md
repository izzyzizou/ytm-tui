# Roadmap

Milestones from `docs/blueprint/6-repository.md`, with what exists today.

| # | Milestone | Status |
|---|---|---|
| 0 | Workspace, theme tokens, event loop, layout | ✅ done |
| 1 | mpv backend + yt-dlp resolver | ✅ implemented, unit-tested; **needs a real-audio check on a machine with mpv + yt-dlp** |
| 2 | InnerTube search + `auth import` | ✅ search (anonymous fixture-tested), header/cURL/JSON import + SAPISIDHASH |
| 3 | Daemon + IPC; TUI and CLI share playback; `Q` detaches | ✅ done, end-to-end tested with the null backend |
| 4 | Library views (Home, Explore, Library, Playlists, Liked, History), likes, radio | ⬜ next |
| 5 | Lyrics ✅ (LRCLIB + local .lrc) · thumbnails ⬜ · media keys / MPRIS ⬜ | 🟨 partial |
| 6 | Native backend (ffmpeg → cpal) + spectrum visualizer | ⬜ |
| 7 | Audio cache, offline mode, metadata cache (SQLite) | ⬜ |

## Next tasks, in order

1. **Verify real playback** on macOS/Linux with `mpv` and `yt-dlp` installed:
   `cargo run -- play "daft punk one more time"`. Check `daemon.log` if silent. Expected
   rough edges: mpv `start` property handling, `paused-for-cache` flapping.
2. **Next-track prefetch**: when position passes 75 % (or 30 s left), emit a new
   `Effect::Prefetch { video_id }` so the resolver cache is warm; later use mpv
   `loadfile … append` for gapless.
3. **Signed-in search**: record a fixture from a signed-in session (Songs shelf with album
   + fixed-column duration) and tighten `search::build` against it.
4. **Milestone 4 endpoints** in `ytm-api`: `browse` for `FEmusic_home`,
   `FEmusic_liked_videos`, `FEmusic_history`, `FEmusic_liked_playlists`, playlist pages;
   `next` for radio (`R`) and autoplay; `like/like` + `like/removelike` for `L`.
   Each: endpoint fn + parser + fixture test + a `View` in the TUI.
5. **Keymap file** (`keymap.toml`) and the command palette (`:`), per blueprint §3.
6. **MPRIS / Now Playing** via `souvlaki` inside the daemon.
7. **Thumbnails** with `ratatui-image` (Kitty/Sixel/iTerm2), disk cache in `state_dir/thumbs`.
8. **Lyrics from YouTube Music** (unsynced fallback) via `next` → lyrics `browseId`.
9. Windows named-pipe transport for the daemon socket and mpv IPC.
