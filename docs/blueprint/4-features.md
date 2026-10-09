# 4 · Feature matrix

| Feature | Approach | Fallback | Phase |
|---|---|---|---|
| Authentication | Browser header/cookie import + SAPISIDHASH | OAuth device flow (experimental), anonymous mode | MVP |
| Streaming | yt-dlp resolve → mpv (or native) with read-ahead and next-track prefetch | Lower-bitrate AAC format, re-resolve on expiry | MVP |
| Thumbnail cache | `.webp` on disk, LRU, rendered with ratatui-image | Half-block art, then text-only | v0.2 |
| Audio cache / offline | Opt-in tee-to-disk while streaming, SQLite index, pinning | Streaming only | v0.3 |
| Synced lyrics | Local `.lrc` → LRCLIB → YouTube Music lyrics | Unsynced scroll, then "no lyrics" | v0.2 |
| CLI mode | JSON-RPC client to the daemon | Starts a headless daemon on demand | MVP |
| OS media integration | MPRIS / Now Playing / SMTC via souvlaki | — | v0.2 |

---

## 1 · Authentication

### Browser header import (primary)

The reliable path, and the same one `ytmusicapi` uses.

1. User opens `music.youtube.com` signed in, opens devtools → Network, filters for `browse`, and copies any `POST /youtubei/v1/browse` request **as cURL** (or as raw request headers).
2. `ytm-tui auth import` reads it from the clipboard or stdin (`ytm-tui auth import < request.txt`), extracts `Cookie`, `User-Agent`, `X-Goog-AuthUser` and (for brand accounts) `X-Goog-PageId`.
3. It also accepts an existing **`headers_auth.json` / `browser.json`** from ytmusicapi, so users of other tools can reuse their file: `ytm-tui auth import --file headers_auth.json`.
4. Optional shortcut: `ytm-tui auth import --from-browser firefox` asks `yt-dlp --cookies-from-browser firefox` to export the cookie jar, then keeps only the `.youtube.com` cookies.
5. Credentials are stored in the OS keyring (`keyring` crate) under service `ytm-tui`; on systems without a keyring, in `~/.config/ytm-tui/auth.json` with mode `0600`. Never logged; `--debug` logs redact `Cookie` and `Authorization`.

**Per-request signing.** Each InnerTube call sends:

```text
Authorization: SAPISIDHASH <unix_ts>_<sha1("<unix_ts> <SAPISID> https://music.youtube.com")>
X-Origin: https://music.youtube.com
X-Goog-AuthUser: <n>
Cookie: <imported cookie string>
```

`SAPISID` (or `__Secure-3PAPISID`) comes from the cookie string. The same cookies are written to a Netscape `cookies.txt` (0600, in the runtime dir) and passed to yt-dlp with `--cookies`, so premium-only formats and age-restricted tracks resolve.

**Validation and expiry.** On import and at daemon start, call `browse` for the account menu; a response without account info, or HTTP 401/403, means signed out. The daemon then keeps playing what doesn't need auth, marks the header `◉ signed out` in `amber`, and offers `ytm-tui auth import` in a toast. Cookies typically last months but are invalidated when the user signs out of that browser session.

### OAuth2 device flow (experimental)

`ytm-tui auth oauth` runs the TV-style device-code flow (show a code, user approves at `google.com/device`), stores the refresh token, and refreshes access tokens 5 minutes before expiry. Caveats to state in the docs: Google now requires users to supply their own OAuth client ID/secret from a Google Cloud project for this flow, and OAuth tokens are no longer accepted by yt-dlp for streaming — so OAuth covers library/metadata only, and cookies are still needed for some streams. Keep it behind `--experimental`.

### Anonymous mode

No auth: search, browse, radio and playback of public tracks work; library, likes, history and playlists are hidden from the sidebar.

---

## 2 · Streaming & caching

### Resolution

```text
videoId ──► StreamResolver (yt-dlp -J) ──► StreamInfo {
              url, expires_at (from the `expire=` query param),
              codec: opus | aac, bitrate_kbps, sample_rate,
              content_length, itag }
```

- Format preference: Opus/WebM (itag 251, ~160 kb/s) → AAC/M4A (itag 140, 128 kb/s) → anything `bestaudio`. Configurable (`audio.prefer = "aac"` for devices without Opus).
- Resolved URLs are memoized per `videoId` until `expires_at − 10 min` (URLs typically live ~6 h).
- yt-dlp runs with `--no-warnings --no-playlist --socket-timeout 10`; at most 2 concurrent resolutions; each has a 20 s timeout.

### Buffering and prefetch

- **mpv backend:** `--cache=yes --demuxer-max-bytes=64MiB --demuxer-readahead-secs=60 --prefetch-playlist=yes`. The next track is resolved when the current one reaches 75 % or 30 s remaining, whichever is first, and appended with `loadfile <url> append` — gapless transitions.
- **native backend:** HTTP range requests in 1 MiB chunks (large single ranges get throttled) feed ffmpeg's stdin; decoded PCM goes into a 2-second `rtrb` ring buffer read by the `cpal` callback. Below 250 ms buffered the header shows `◌ BUFFERING` in `amber`; output pauses instead of stuttering.
- Network hiccups shorter than the buffer are invisible to the user.

### Thumbnail cache

- YouTube Music art URLs (`lh3.googleusercontent.com/...=w120-h120`) are rewritten to the size needed for the current cell grid (e.g. `=w226-h226-l90-rj`), fetched as WebP, stored at `cache/thumbs/<sha256(url)>.webp`.
- Disk LRU capped by `cache.thumbs_max = "200MiB"`; decoded images kept in an in-memory LRU of 64.
- Fetch is lazy: only cards/rows on screen plus one page ahead; scrolling cancels off-screen fetches.
- Rendering: `ratatui-image` auto-detects Kitty graphics protocol, Sixel, or iTerm2 inline images (Kitty, WezTerm, iTerm2, foot, recent Konsole); half-block `▀` downsample elsewhere (Alacritty), text-only in 16-color mode.

### Audio cache (opt-in, off by default)

- `cache.audio = true` enables **write-through tee**: bytes streamed for playback are also written to `cache/audio/<videoId>.<itag>.part`; on reaching `content_length` the file is fsync'd and renamed to `.webm`/`.m4a`. Partial files from skips are deleted.
- Index in SQLite: `(video_id, itag, path, bytes, last_played, play_count, pinned)`.
- LRU eviction by size (`cache.audio_max = "5GiB"`); pinned entries never evicted. `w` or `ytm-tui cache pin <playlist-url>` downloads explicitly (background, 2 at a time).
- Playback checks the cache first; a hit plays from disk with zero network. Footer shows `● cached` in `amber`.
- **Offline mode** (no network or `--offline`): only cached tracks are playable; others render in `ink-faint` with `✗` and are skipped by the queue.

### Metadata cache

SQLite table `(key, json, fetched_at, ttl)` with stale-while-revalidate: show the cached response immediately, refresh in the background, re-render if changed. TTLs: home/explore 15 min, library and playlists 5 min, albums/artists 7 days, search 10 min.

---

## 3 · Synchronized lyrics

### Sources (first hit wins, cached per `videoId`)

1. **Local override**: `~/.config/ytm-tui/lyrics/<videoId>.lrc` or `<artist> - <title>.lrc`.
2. **LRCLIB**: `GET https://lrclib.net/api/get?artist_name=…&track_name=…&album_name=…&duration=<s>` — exact match within ±2 s of duration; on 404, `GET /api/search?q=<artist> <title>` and take the closest duration. Prefer `syncedLyrics`, else `plainLyrics`. Send a descriptive `User-Agent` (`ytm-tui/0.1 (+repo url)`), as LRCLIB asks.
3. **YouTube Music**: the `next` endpoint's lyrics tab → `browse` with its `browseId` returns lyrics text and source credit (usually unsynced).
4. Nothing found → "no lyrics" with the attempted sources, negatively cached for 7 days.

Title normalisation before lookup: strip `(Official Video)`, `[Remastered 2011]`, `feat. …`, and use the first listed artist.

### Parsing

- Lines `[mm:ss.xx]text`, multiple timestamps per line (`[00:12.00][01:40.50]chorus`), metadata tags `[ar:]`, `[ti:]`, `[offset:+250]`.
- Optional enhanced LRC word timing `<mm:ss.xx>` enables word-level highlight.
- Result: `Vec<LyricLine { at: Duration, text: String, words: Option<Vec<(Duration, String)>> }>` sorted by `at`.

### Display and auto-scroll

- Current line = `lines.partition_point(|l| l.at <= pos + offset) - 1` (binary search each frame — cheap).
- Position comes from the backend every ~250 ms and is **interpolated** with `Instant` between reports, so highlighting is smooth at 30 fps.
- The current line is held at one-third of the panel height; text wraps at the panel width with a 2-col hanging indent.
- Current line `ember` bold, ±1 lines `ink`, the rest `ink-muted`; instrumental gaps longer than 8 s show `♪` with a slim progress dot row.
- Manual scrolling (`j`/`k` with the lyrics pane focused) suspends auto-follow; `.` or 5 s of inactivity resumes it.
- `<` / `>` adjust offset by 100 ms; saved per `videoId` in SQLite.
- Unsynced lyrics scroll proportionally to playback position (position/duration), labelled `unsynced`.

---

## 4 · CLI mode

All commands talk to the daemon over `$XDG_RUNTIME_DIR/ytm-tui.sock` (macOS: `$TMPDIR`; Linux without a runtime dir: the state dir, never a shared `/tmp`; Windows: `\\.\pipe\ytm-tui`). If no daemon is running, playback commands start one headless (`ytm-tui daemon --detach`); query commands report `not running` with exit code 3.

```sh
ytm-tui                                  # open the TUI (starts/attaches to the daemon)
ytm-tui play "lowtide assembly - glasswater signals"   # search, play best song match
ytm-tui play --album "halflight sessions"
ytm-tui play https://music.youtube.com/playlist?list=…  # URLs and IDs work too
ytm-tui queue add "coral static"         # append without interrupting
ytm-tui queue ls --json
ytm-tui pause | resume | toggle | stop
ytm-tui next | prev
ytm-tui seek +30 | seek -10 | seek 1:30
ytm-tui volume 60 | volume +5
ytm-tui like | unlike                    # current track
ytm-tui radio                            # radio from current track
ytm-tui autoplay on | off | toggle       # radio when the queue runs out
ytm-tui status                           # one-line human status
ytm-tui status --json                    # machine-readable (schema below)
ytm-tui status --format '{artist} - {title} [{elapsed}/{duration}]'   # for status bars
ytm-tui watch --json                     # newline-delimited events until Ctrl-C
ytm-tui lyrics --synced                  # print LRC of the current track
ytm-tui search "query" --type songs --limit 5 --json
ytm-tui do toggle_like                   # any named action
ytm-tui auth import | auth status | auth logout
ytm-tui cache stats | cache pin <url> | cache clear --thumbs
ytm-tui doctor                           # check mpv, yt-dlp, deno, ffmpeg, auth, terminal
ytm-tui daemon [--detach]
```

`status --json`:

```json
{
  "state": "playing",
  "track": {
    "video_id": "abc123XYZ00",
    "title": "Glasswater Signals",
    "artists": ["Lowtide Assembly"],
    "album": "Halflight Sessions",
    "duration_s": 318,
    "liked": true,
    "url": "https://music.youtube.com/watch?v=abc123XYZ00"
  },
  "position_s": 161.4,
  "volume": 72,
  "shuffle": true,
  "repeat": "all",
  "queue": { "index": 0, "length": 12, "autoplay": 9 },
  "autoplay": true,
  "radio": { "seed": { "video_id": "abc123XYZ00", "title": "Glasswater Signals" }, "loading": false },
  "stream": { "codec": "opus", "bitrate_kbps": 160, "sample_rate": 48000, "cached": true },
  "network": "online"
}
```

Exit codes: `0` ok · `1` error · `2` usage · `3` daemon not running · `4` not found (search had no match) · `5` auth required. `--json` errors are `{"error": {"code": "...", "message": "..."}}` on stdout so scripts can always parse.

Status-bar integration falls out of `watch --json` / `status --format` (tmux, waybar, polybar, sketchybar). Media keys and desktop widgets work through MPRIS/Now Playing without any CLI call.
