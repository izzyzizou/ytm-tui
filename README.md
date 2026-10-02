# ytm-tui

[![CI](https://github.com/izzyzizou/ytm-tui/actions/workflows/ci.yml/badge.svg)](https://github.com/izzyzizou/ytm-tui/actions/workflows/ci.yml)

A keyboard-first YouTube Music client for the terminal. Vim-style keys, a calm two-theme
palette that degrades from TrueColor to 16 colors, synced lyrics, and a background
daemon you can control from scripts, status bars or another terminal.

```text
╭─ ytm-tui ──────────────────────────────────────────────────────────────╮
│ ▶ PLAYING   ● daemon   │   ⌕ daft punk          ⇄ off  ↻ all  │ ◉ anon │
├────────────────┬────────────────────────────────────┬──────────────────┤
│ ⌂ Home       1 │   # TITLE              ARTIST TIME │ [Lyrics] Spectrum│
│ » Queue · 3  7 │ ▶ 1 One More Time      Daft…  5:22 │ › current line   │
│                │ › 2 Around the World   Daft…  4:02 │   next line      │
╰────────────────┴────────────────────────────────────┴──────────────────╯
```

> Unofficial, for personal use. It talks to the private endpoints the YouTube Music web app
> uses and relies on yt-dlp for streams; both can change without notice.

## Install

Requirements: Rust (stable), [`mpv`](https://mpv.io), [`yt-dlp`](https://github.com/yt-dlp/yt-dlp)
(keep it updated) and [`deno`](https://deno.com) (yt-dlp uses it for YouTube).

```sh
brew install mpv yt-dlp deno            # macOS  (Arch: pacman -S mpv yt-dlp deno)
git clone https://github.com/izzyzizou/ytm-tui && cd ytm-tui
cargo install --path crates/ytm-tui
ytm-tui doctor                          # checks mpv, yt-dlp, deno, auth, terminal colors
ytm-tui                                 # press / to search, Enter to play, ? for help
```

### Sign in (optional, recommended)

Anonymous mode can search and play. Signing in unlocks better search results, fewer
"confirm you're not a bot" errors, and (soon) your library:

1. Open <https://music.youtube.com> signed in, open DevTools → Network, filter `browse`.
2. Right-click any `browse` request → **Copy → Copy as cURL**.
3. `pbpaste | ytm-tui auth import` (or `ytm-tui auth import --file headers_auth.json` for an
   existing ytmusicapi file), then `ytm-tui quit && ytm-tui`.

The login is stored in your config dir with mode 0600.

## CLI

```sh
ytm-tui play "daft punk one more time"   # search + play best match
ytm-tui queue add "around the world" [--next]
ytm-tui toggle | next | prev | stop
ytm-tui seek +30 | seek 1:30 | volume -10
ytm-tui status --format '{artist} - {title} [{elapsed}/{duration}]'
ytm-tui status --json
ytm-tui lyrics --synced
ytm-tui quit                             # stop the background daemon
```

Exit codes: 0 ok · 1 error · 2 usage · 3 daemon not running · 4 not found · 5 auth required.

## Keys

`/` search · `Enter` play · `Space` pause · `n`/`p` next/prev · `[ ]` seek ±5s · `{ }` ±30s ·
`+`/`-` volume · `a`/`A` queue / play next · `7` queue view · `J`/`K` move · `d` remove ·
`s` shuffle · `r` repeat · `y` copy link · `V` side panel · `?` all keys · `q` quit · `Q`
quit but keep playing. Full map: `docs/blueprint/3-keybindings.md`.

## Status

Search, playback through mpv, queue, shuffle/repeat, synced lyrics (LRCLIB), the daemon +
CLI and the TUI work. Library views, likes, radio, album art, media keys and the
visualizer are next — see `docs/ROADMAP.md`.

## Contributing

Issues and pull requests are welcome — start with [`CONTRIBUTING.md`](CONTRIBUTING.md). To
report a security problem (for example, credential leakage), see [`SECURITY.md`](SECURITY.md).

## Disclaimer

ytm-tui is not affiliated with, endorsed by, or sponsored by Google or YouTube. "YouTube"
and "YouTube Music" are trademarks of Google LLC. Use it in line with YouTube's Terms of
Service. Don't use it to download or redistribute content you don't have rights to.

## License

[MIT](LICENSE)
