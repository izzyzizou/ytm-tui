# Roadmap

Milestones from `docs/blueprint/6-repository.md`, with what exists today.

| # | Milestone | Status |
|---|---|---|
| 0 | Workspace, theme tokens, event loop, layout | ✅ done |
| 1 | mpv backend + yt-dlp resolver | ✅ done, verified with real audio (macOS, mpv 0.41, yt-dlp 2026.08) |
| 2 | InnerTube search + `auth import` | ✅ search (anonymous fixture-tested), header/cURL/JSON import + SAPISIDHASH |
| 3 | Daemon + IPC; TUI and CLI share playback; `Q` detaches | ✅ done, end-to-end tested with the null backend |
| 4 | Library views (Home, Explore, Library, Playlists, Liked, History), likes, radio | ⬜ next |
| 5 | Lyrics ✅ (LRCLIB + local .lrc) · thumbnails ⬜ · media keys / MPRIS ⬜ | 🟨 partial |
| 6 | Native backend (ffmpeg → cpal) + spectrum visualizer | ⬜ |
| 7 | Audio cache, offline mode, metadata cache (SQLite) | ⬜ |

## Next tasks, in order

1. **Gapless**: next-track prefetch is done (`Effect::Prefetch` at 75 % or 30 s left warms
   the stream cache; a track change drops from ~1.8 s to ~0.15 s). Next, use mpv
   `loadfile … append` with the prefetched URL so tracks join without a gap.
2. **Signed-in search**: record a fixture from a signed-in session (Songs shelf with album
   + fixed-column duration) and tighten `search::build` against it.
3. **Milestone 4 endpoints** in `ytm-api`: `browse` for `FEmusic_home`,
   `FEmusic_liked_videos`, `FEmusic_history`, `FEmusic_liked_playlists`, playlist pages;
   `next` for radio (`R`) and autoplay; `like/like` + `like/removelike` for `L`.
   Each: endpoint fn + parser + fixture test + a `View` in the TUI.
4. **Keymap file** (`keymap.toml`) and the command palette (`:`), per blueprint §3.
5. **MPRIS / Now Playing** via `souvlaki` inside the daemon.
6. **Thumbnails** with `ratatui-image` (Kitty/Sixel/iTerm2), disk cache in `state_dir/thumbs`.
7. **Lyrics from YouTube Music** (unsynced fallback) via `next` → lyrics `browseId`.
8. Windows named-pipe transport for the daemon socket and mpv IPC.
