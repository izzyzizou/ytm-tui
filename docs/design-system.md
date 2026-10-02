# ytm-tui

A keyboard-first YouTube Music client for the terminal — fast to start, quiet to look at, and fully drivable from a shell script.

This system is two things at once: the **visual language** of the TUI (color, type, borders, glyphs, layout) and the **technical blueprint** for building it. The brand book is this page; the blueprint follows as six sections:

1. Tech stack — Rust + ratatui, the audio backend, API strategy
2. TUI layout — the full-screen mockup, panes and breakpoints
3. Keybindings — the complete vim-style map
4. Feature matrix — auth, streaming & cache, synced lyrics, CLI mode
5. Architecture & data flow — actors, event loop, error handling
6. Repository & boilerplate — workspace layout and a runnable event loop

The live, colored version of the main screen is the **AppShell** component; the player bar on its own is **PlayerBar**.

## Principles

- **Keyboard before everything.** Every action has a key; the mouse is a convenience (click to focus, scroll wheel on lists), never a requirement.
- **Color carries one meaning each.** `ember` = what's playing / what you love. `lagoon` = where your keyboard is. `amber` = wait. `moss` = healthy. `error` = broken. Nothing else is colored.
- **Calm by default.** Text is `ink` and `ink-muted` on the terminal's own background. A screen should read as mostly neutral with one or two warm points: the now-playing row and the progress bar.
- **Degrade, never break.** Every color decision has a 256-color and a 16-color answer, and every state is also signalled by a glyph or a word, so the app is usable in `TERM=xterm` over SSH and with `NO_COLOR=1`.
- **The terminal owns the font.** We pick weight (SGR 1), dim (SGR 2) and color — never size or family.

## Color

Two themes ship: **Dark** (default) and **Light**. Pick at runtime with `theme = "auto" | "dark" | "light"`; `auto` queries the terminal background with OSC 11 and falls back to dark.

| Role | Token | Where it appears |
|---|---|---|
| Ground | `bg-base` | Pane fill. By default we paint `Color::Reset` so the user's terminal background and transparency show through. |
| Bars | `bg-raised` | Header, footer, popups. |
| Cursor row | `bg-select` | The row under the list cursor. |
| Lines | `border` | Unfocused pane borders, table rules. |
| Text | `ink` / `ink-muted` / `ink-faint` | Titles / metadata & hints / disabled & placeholders. |
| Now playing | `ember` (`progress-fill`, `liked`) | ▶ marker, playing title, progress fill, ♥, current lyric. |
| Focus | `lagoon` (`focus-ring`) | Focused pane border + title, `›` cursor, search caret. |
| States | `amber`, `moss` (`net-online`), `error` | Buffering/rate-limit, online/cached, failures. |
| Visualizer | `spectrum-low` → `spectrum-mid` → `spectrum-high` | Bars colored by height band, not by frequency. |

Every text token reaches 4.5:1 on both `bg-base` and `bg-select` in both themes, except `ink-faint`, which is reserved for text nobody has to read (placeholders, disabled rows).

### Color tiers

The renderer detects capability once at startup (`COLORTERM=truecolor|24bit` → TrueColor; `TERM` containing `256color` → 256; otherwise 16; `NO_COLOR` set → monochrome) and resolves each token through a `Palette` struct:

| Token | TrueColor (dark / light) | ANSI-256 | 16-color | Monochrome |
|---|---|---|---|---|
| `ink` | `#e6e1d6` / `#24211c` | 253 / 234 | default fg | default |
| `ink-muted` | `#a39d90` / `#5e584e` | 247 / 240 | SGR 2 dim | SGR 2 dim |
| `ember` | `#f4845f` / `#a33c17` | 209 / 130 | bright red 9 | bold |
| `lagoon` | `#5fc4b8` / `#12645e` | 79 / 23 | cyan 6 | underline on titles |
| `amber` | `#e8b75c` / `#77520a` | 179 / 94 | yellow 3 | glyph only |
| `moss` | `#93c47d` / `#375f20` | 114 / 22 | green 2 | glyph only |
| `error` | `#ff6b78` / `#b3261e` | 204 / 124 | red 1 + bold | bold + ✗ |
| `bg-select` | `#2a2f3a` / `#e4ddcf` | 236 / 253 | reverse video | reverse video |

In 16-color mode the theme defers to the user's terminal palette, so a user's Solarized or Gruvbox scheme is respected.

## Typography

One monospace face — whatever the terminal is set to. The docs and previews use `"JetBrains Mono", "Iosevka Term", ui-monospace` as a stand-in.

- `cell` — all body text.
- `cell-bold` (SGR 1) — the now-playing title, the focused pane title, the current lyric line. At most one bold thing per pane.
- `cell-dim` — metadata. Render it as `ink-muted` in color tiers; use real SGR 2 only in 16-color mode, where some terminals otherwise make dim unreadable.
- `cell-italic` — lyric annotations such as *(instrumental)*. Never load-bearing: many terminal fonts have no italic.
- `pane-title` — UPPERCASE, embedded in the top border: `╭ SEARCH › lowtide ─────╮`.

## Borders, focus and shape

- **Unfocused panes**: plain border set `┌─┐└─┘` in `border` (`radius-square`).
- **Focused pane**: rounded set `╭─╮╰─╯` in `lagoon`, title in `lagoon` bold (`radius-round`). Shape and color change together, so focus is visible in monochrome and to color-blind users.
- **Popups** (help `?`, command palette `:`, confirm dialogs): rounded, `bg-raised` fill, centered, max 80×24, `Esc` closes.
- **Shared edges** between panes use T-junctions `┬ ┴ ├ ┤`, so the screen reads as one frame, not floating boxes.

## Glyphs (iconography)

Only glyphs from Unicode blocks that every mainstream terminal font covers (Box Drawing, Block Elements, Geometric Shapes, Arrows, Misc Technical). No emoji, no Nerd Font requirement — Nerd Font icons are an opt-in `icons = "nerd"` setting.

| Meaning | Glyph | ASCII fallback (`icons = "ascii"`) |
|---|---|---|
| Playing / paused | `▶` / `‖` | `>` / `"` |
| List cursor | `›` | `>` |
| Liked | `♥` | `*` |
| Shuffle / repeat all / repeat one | `⇄` / `↻` / `↻1` | `S` / `R` / `R1` |
| Online / cached / failed | `●` / `●` / `✗` (colored moss / amber / error) | `o` / `c` / `x` |
| Search | `⌕` | `/` |
| Progress | `━━━●────` | `===o----` |
| Volume | `▮▮▮▮▯▯` | `####--` |
| Visualizer | `▁▂▃▄▅▆▇█` | `.:-=+*#%` |
| Sidebar | `⌂ ◇ ▤ ≡ ♥ ◷ »` | none (labels only) |

## Spacing and layout

The layout is measured in cells. `col-1` (1 column) is the inner padding of every pane and the gap between table columns; `col-2` indents nested items. Fixed regions: sidebar `sidebar-w` (20 cols), side panel `side-panel-w` (24 cols), footer `footer-h` (3 rows). The main panel takes the rest. Breakpoints are in section 2.

## Voice

Words in the UI are short, lowercase where they are status, and say what happened plus what to do:

- Good: `stream expired · refreshing…` · `rate limited · retry in 12s` · `offline · playing from cache`
- Avoid: `Error 403: Forbidden` · `Oops! Something went wrong`
- Key hints use the key, then the verb: `a queue  R radio  L like`.
- Numbers are plain: `12 in queue`, `5:18`, `160 kb/s`.

## Scope note

`ytm-tui` is an unofficial personal-use client. It talks to the same private endpoints the YouTube Music web app uses and relies on `yt-dlp` for stream extraction, both of which change without notice and are not covered by any public API terms. Keep raw-audio caching **opt-in and off by default**, and don't redistribute cached media.
