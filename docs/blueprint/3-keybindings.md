# 3 · Keybindings

Vim-flavoured, modal, and fully remappable in `keymap.toml`. Lowercase keys act on what's under the cursor or on navigation; Shift-modified keys are "bigger" versions or actions on a track. Counts work like vim: `5j` moves five rows, `3n` skips three tracks.

## Modes

| Mode | Entered with | Indicator | Left with |
|---|---|---|---|
| **Normal** | default | — | — |
| **Search** (insert) | `/` | caret `▏` in header search field | `Enter` (run) · `Esc` (cancel) |
| **Filter** | `f` in a list | `filter: …` in pane title | `Esc` |
| **Move** | `m` on a queue/playlist row | row marked `≡`, pane title `MOVE` | `Enter` (drop) · `Esc` (cancel) |
| **Visual** | `v` | selected rows in `bg-select` | `Esc`, or any action applies to the selection |
| **Command** | `:` | palette popup | `Enter` · `Esc` |

## Global navigation

| Key | Action |
|---|---|
| `h` `j` `k` `l` / arrows | Left · down · up · right (in grids `h`/`l` move between cards; in lists `l`/`Enter` opens, `h` goes back) |
| `gg` / `G` | First / last item |
| `Ctrl-d` / `Ctrl-u` | Half page down / up |
| `Ctrl-f` / `Ctrl-b` | Full page down / up |
| `Tab` / `Shift-Tab` | Focus next / previous pane (sidebar → main → side panel) |
| `Ctrl-h` / `Ctrl-l` | Focus pane left / right |
| `1`–`7` | Jump to Home · Explore · Library · Playlists · Liked Songs · History · Queue |
| `gh` `ge` `gl` `gp` `gL` `gH` `gq` | Same jumps, mnemonic form |
| `Enter` | Open item / play track |
| `Backspace` / `Ctrl-o` | Back in view history |
| `Ctrl-i` | Forward in view history |
| `/` | Search (results: `Tab` cycles Songs · Albums · Artists · Playlists) |
| `f` | Filter the current list in place |
| `:` | Command palette |
| `?` | Help for the focused pane |
| `V` | Toggle side panel |
| `t` | In the side panel: cycle Lyrics / Spectrum / Art |
| `Z` | Zoom focused pane to full body |
| `Ctrl-r` | Refresh current view (bypass cache) |
| `q` | Quit (stops playback unless `daemon.persist = true`) |
| `Q` | Detach: close the UI, keep playing in the daemon |
| `Esc` | Cancel / close popup / clear selection |

## Playback

| Key | Action |
|---|---|
| `Space` | Play / pause |
| `n` / `p` | Next / previous track (`p` within the first 3 s goes to the previous track, otherwise restarts) |
| `+` (`=`) / `-` | Volume ±5 % |
| `M` | Mute toggle |
| `[` / `]` | Seek −5 s / +5 s |
| `{` / `}` | Seek −30 s / +30 s |
| `:seek 1:30` | Seek to an absolute time |
| `s` | Shuffle toggle |
| `r` | Repeat cycle: off → all → one |
| `.` | Jump the cursor to the now-playing track |
| `<` / `>` | Lyrics offset −100 ms / +100 ms (saved per track) |

## Queue management

| Key | Action |
|---|---|
| `a` | Add to end of queue (works on tracks, albums, playlists, visual selections) |
| `A` | Play next (insert after the current track) |
| `d` | Remove from queue (in the Queue view) / remove from playlist (own playlists, with confirm) |
| `u` | Undo last queue edit (20-step history) |
| `m` | Enter move mode; then `j`/`k` or `J`/`K` to move, `Enter` to drop |
| `J` / `K` | Move item down / up directly (no mode) |
| `c` | Clear queue (keeps the current track; confirm) |
| `x` | Clear the autoplay section only |
| `S` | Save queue as a playlist |

## Track operations

| Key | Action |
|---|---|
| `L` | Like / unlike (`♥` toggles optimistically, rolled back on API failure) |
| `D` | Dislike (hides from radio; confirm) |
| `y` | Copy `https://music.youtube.com/watch?v=<id>` (system clipboard, OSC 52 over SSH) |
| `Y` | Copy the bare video ID |
| `R` | Start radio from this track (replaces the autoplay section) |
| `P` | Add to playlist… (picker popup) |
| `i` | Track info popup: album, year, codec, bitrate, loudness, cache status |
| `gA` / `gr` | Go to album / go to artist |
| `o` | Open in browser |
| `w` | Download to the offline cache (if caching is enabled) |

## Mouse (optional)

Click to focus a pane or select a row, double-click to play, wheel to scroll, click on the progress bar to seek, wheel over the volume to change it. Disable with `ui.mouse = false`.

## Remapping

```toml
# ~/.config/ytm-tui/keymap.toml
[normal]
"ctrl-n" = "next"
"ctrl-p" = "previous"
"F"      = "toggle_like"   # overrides L

[queue]
"x" = "remove"             # make x remove, like a file manager
```

Every action has a stable name (`next`, `toggle_like`, `seek_relative:+5`, `focus_pane:main`) shared with the command palette and the CLI, so `ytm-tui do toggle_like` works from a shell.

## Conflicts avoided on purpose

- `h`/`l` are navigation, so seeking moved to `[ ]` / `{ }` rather than arrow keys.
- `L` is like (not "move right") and `R` is radio (not "redo"); vim users rarely need those in a music player, and `u` still undoes.
- `p` is previous, not paste — there is no paste in this app.
