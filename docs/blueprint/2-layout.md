# 2 · TUI layout

## Main screen (100 × 22)

The main panel has focus (rounded `lagoon` border in the real app; the colored version is the **AppShell** component). Track and lyric text is placeholder content.

```text
╭─ ytm-tui ────────────────────────────────────────────────────────────────────────────────────────╮
│ ▶ PLAYING   ● online 38ms   │   ⌕ lowtide▏               ⇄ shuffle  ↻ all   │   ◉ jean · premium │
├────────────────────┬────────────────────────────────────────────────────┬────────────────────────┤
│ NAVIGATE           │ SEARCH › lowtide   [Songs] Albums  Artists  Lists  │ [Lyrics] Spectrum      │
│  ⌂ Home         1  │ ────────────────────────────────────────────────── │ ────────────────────── │
│  ◇ Explore      2  │   # TITLE                    ARTIST           TIME │  harbour goes quiet    │
│  ▤ Library      3  │ ▶ 1 Glasswater Signals     ♥ Lowtide Assembly 5:18 │  one window at a time  │
│  ≡ Playlists    4  │   2 Paper Satellites         Lowtide Assembly 4:50 │ › glass on the water   │
│  ♥ Liked Songs  5  │ › 3 Northbound at Dusk       Lowtide Assembly 4:09 │  keeps what I lost     │
│  ◷ History      6  │   4 Halflight              ♥ Lowtide Assembly 4:15 │  and every signal home │
│  » Queue · 12   7  │   5 Small Hours Engine       Lowtide Assembly 6:27 │  arrives a little late │
│                    │   6 Coral Static             Lowtide Assembly 4:11 │                        │
│ PLAYLISTS          │   7 The Long Echo            Lowtide Assembly 6:24 │  ♪ (instrumental)      │
│   Late Night Drive │   8 Lanterns Over Rail     ♥ Lowtide Assembly 4:59 │                        │
│   Deep Focus       │                                                    │                        │
│   Run 170 BPM      │  8 of 48 · j/k move  a queue  R radio  L like      │ ▁▂▄▆█▇▅▃▂▁▂▄▆▅▃▂▁▁▂▃▄▂ │
│   + new playlist   │                                                    │ synced · lrclib  +0.0s │
│                    │                                                    │                        │
├────────────────────┴────────────────────────────────────────────────────┴────────────────────────┤
│ ▶ Glasswater Signals — Lowtide Assembly · Halflight Sessions   opus 160 kb/s · 48 kHz · ● cached │
│ 2:41 ━━━━━━━━━━━━━━━━━━━━━━━━━●──────────────────────── 5:18                  vol ▮▮▮▮▮▮▮▯▯▯ 72% │
│ p prev  ␣ pause  n next  [ ] seek  +/- vol  L like  V lyrics/spectrum  ? help                    │
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯
```

## Regions

| Region | Size | Contents | Color notes |
|---|---|---|---|
| **Header** | 1 row, full width, `bg-raised` | Play state (`▶ PLAYING` / `‖ PAUSED` / `◌ BUFFERING`), network indicator + latency, inline search field, shuffle/repeat state, account | `▶ PLAYING` in `ember` bold; `● online` `moss`, `● degraded` `amber`, `✗ offline` `error`; search caret `lagoon` |
| **Sidebar** | 20 cols fixed | Navigation tree (Home, Explore, Library, Playlists, Liked Songs, History, Queue) with jump digits, then the user's playlists | Active view: `▌` bar + `lagoon`; digits `ink-faint`; queue count updates live |
| **Main panel** | Remaining width | One of: track list (search, playlist, album, liked, history, queue), album/artist **grid**, artist page, home shelves | Cursor row `bg-select` + `›`; now-playing row `▶` + `ember` bold title; liked `♥` `ember`; metadata `ink-muted` |
| **Side panel** | 24 cols, toggled with `V` | Tabs: **Lyrics** (synced, auto-scrolling) · **Spectrum** (visualizer) · **Art** (album art, Kitty/Sixel/iTerm2) | Current lyric `ember` bold, neighbours `ink`, the rest `ink-muted` |
| **Player footer** | 3 rows, full width, `bg-raised` | Row 1: title — artist · album, codec/bitrate/sample rate, cache state. Row 2: elapsed, progress bar, total, volume. Row 3: context key hints | Progress `━` `ember`, rest `─` `border`; volume `▮` `ink`, `▯` `ink-faint` |

## Side panel: Spectrum tab

One bar per column (22 in the default panel, the full width when zoomed) on a log-frequency scale (40 Hz–16 kHz), eighth-block resolution (`▁▂▃▄▅▆▇█`), each cell colored by its height band — bottom third `spectrum-low`, middle `spectrum-mid`, top `spectrum-high` — so the panel stays calm at low volume and warms up on peaks. Peak-hold caps (`▔`) fall at 12 rows/s.

```text
╭ SPECTRUM ──────────────╮
│ Lyrics [Spectrum] Art  │
│    ▂▅                  │
│   ▄██▇ ▂               │
│  ▄████▇█▂              │
│  ████████▄█▁           │
│ ▆███████████▃▆         │
│ ███████████████▄▁▃     │
│ ██████████████████▆▂   │
│ █████████████████████▅ │
│ 40Hz      1k     16k   │
╰────────────────────────╯
```

Full-screen mode: `Z` zooms the focused pane to the whole body (sidebar and side panel hidden); on the Spectrum tab that gives a full-width visualizer with the footer still visible.

## Album grid view

Albums, artists and home shelves render as a grid of cards. With an image protocol available the art is real; otherwise a half-block (`▀`) downsample, and in 16-color mode just the text.

```text
╭ LIBRARY › ALBUMS ─────────────────────────────────────────╮
│ ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐    │
│ │ ▀▀▀▀▀▀▀▀ │  │ ▀▀▀▀▀▀▀▀ │  │ ▀▀▀▀▀▀▀▀ │  │ ▀▀▀▀▀▀▀▀ │    │
│ │ ▀▀▀▀▀▀▀▀ │  │ ▀▀▀▀▀▀▀▀ │  │ ▀▀▀▀▀▀▀▀ │  │ ▀▀▀▀▀▀▀▀ │    │
│ └──────────┘  └──────────┘  └──────────┘  └──────────┘    │
│ Halflight     Paper Moons   Low Season    Northline       │
│ Sessions      Lowtide A.    Ferro Club    Ana Ruiz        │
│ 2024          2023          2021          2025            │
╰───────────────────────────────────────────────────────────╯
```

Card = 12 cols × 6 rows + 2-col gutter; column count = `floor((width - 2) / 14)`. `h/j/k/l` move between cards; `Enter` opens.

## Queue view

```text
╭ QUEUE · 12 tracks · 52 min ───────────────────────────────╮
│ ▶  Glasswater Signals      Lowtide Assembly        5:18   │
│ ─ up next ─────────────────────────────────────────────── │
│ ›  Paper Satellites        Lowtide Assembly        4:50   │
│ ≡  Northbound at Dusk      Lowtide Assembly        4:09   │  ← in move mode (m), J/K drag
│    Coral Static            Lowtide Assembly        4:11   │
│ ─ autoplay (radio) ──────────────────────────────────── ↻ │
│    Ferrous Bloom           Ferro Club              3:58   │
╰───────────────────────────────────────────────────────────╯
```

User-added tracks sit above an "autoplay" divider; radio continuations (`R`, or autoplay when the queue runs out) are added below it in `ink-muted`, so it is always clear what you chose versus what was suggested.

## Popups

- `?` **Help** — key map for the focused pane, searchable.
- `:` **Command palette** — fuzzy list of every action (`:like`, `:theme light`, `:cache pin`, `:seek 1:30`).
- **Toasts** — bottom-right above the footer, 1 row, auto-dismiss in 4 s (errors stay until `Esc`).

## Responsive breakpoints

| Terminal size | Layout |
|---|---|
| ≥ 110 × 24 | Full layout as above |
| 90–109 cols | Side panel hidden (toggle `V` overlays it on the main panel) |
| 60–89 cols | Sidebar collapses to a 3-col icon rail (`⌂ ◇ ▤ ≡ ♥ ◷ »`); ARTIST column folds under the title |
| < 60 cols or < 16 rows | "Compact player": header + now-playing + progress + queue next-up only |
| < 40 × 8 | "Terminal too small" message with current track still shown |

## Rendering rules

- Draw only when state is dirty; while playing, animate at 30 fps (progress interpolation, spectrum). Paused and idle, zero redraws.
- Respect `NO_COLOR`, and `--no-unicode` / `icons = "ascii"` for limited fonts.
- Never paint the background unless `theme.paint_background = true`; transparency in Kitty/Alacritty/WezTerm should show through.
- Long titles are truncated with `…` at grapheme boundaries (`unicode-width` + `unicode-segmentation`), never mid-character; CJK double-width titles count as 2 cells.
