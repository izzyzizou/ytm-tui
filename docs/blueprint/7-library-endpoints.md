# 7 · Milestone 4: library views, likes, radio

Implementation reference for Milestone 4: the InnerTube endpoints behind Home, Explore, Library,
Playlists, Liked, History, likes and radio, and what each crate needs to grow to use them.

Primary reference: ytmusicapi `main` (MIT), `ytmusicapi/{mixins,parsers}/*.py`,
`navigation.py`, `continuations.py`. Cross-checked against YouTube.js `src/core/clients/Music.ts`
(MIT). InnerTune / Metrolist are GPL-3.0: read for behaviour, don't copy code.
**[?]** marks shapes not yet confirmed; capture a fixture before relying on them.

Abbreviations used below:

| Short | Full path / name |
|---|---|
| `SCT` | `contents.singleColumnBrowseResultsRenderer.tabs[0].tabRenderer.content` |
| `TCB` | `contents.twoColumnBrowseResultsRenderer` |
| `MRLIR` | `musicResponsiveListItemRenderer` (track row) |
| `MTRIR` | `musicTwoRowItemRenderer` (card) |

## Corrections to the roadmap

1. `FEmusic_liked_videos` is the library's **songs**, not liked songs. Liked Songs is the
   playlist `VLLM`, fetched like any other playlist page.
2. Two continuation formats coexist (see [Continuations](#continuations)); support both.
3. Playlist and album pages use `TCB`: header in the first column, tracks in
   `secondaryContents`. A single recursive walk won't separate them.
4. `likeEndpoint.status` inside `next` menus is the **action** the toggle would perform, i.e.
   inverted. Playlist rows carry the real state in `likeButtonRenderer.likeStatus`.
5. Pick the lyrics / related tabs of `next` by `pageType`, not index: a Comments tab can appear.
6. ytmusicapi builds `clientVersion` from the current UTC date (`1.YYYYMMDD.01.00`) each run.
   Done: `client::client_version`, replacing the manual `CLIENT_VERSION` bump.

## Request basics

- `POST https://music.youtube.com/youtubei/v1/<endpoint>?prettyPrint=false`. Already done by
  `InnerTube::post` (`crates/ytm-api/src/client.rs`), including SAPISIDHASH and `X-Goog-AuthUser`.
- `X-Goog-Visitor-Id` on every request. Done: `InnerTube` adopts `responseContext.visitorData`
  from the first response (anonymous responses carry one; without the header each response mints
  a new one) and persists it in `state_dir/visitor_data`.
- Some write calls answer HTTP 200 with only `actions[0].showEngagementPanelEndpoint`: the
  account was gated and nothing happened. Treat that as an error.
- No documented rate limits. Keep the existing 200 ms spacing, back off on 429/5xx.
  PO tokens / bot checks only affect `player` (yt-dlp's job), not browse/next/like.

## `browse`

Body: `{"browseId": "...", "params"?: "...", "formData"?: {...}}`.

| browseId | Auth | Contents path | Items |
|---|---|---|---|
| `FEmusic_home` | no | `SCT.sectionListRenderer.contents` | `musicCarouselShelfRenderer` (title `header.musicCarouselShelfBasicHeaderRenderer.title.runs[0].text`, sometimes only `strapline`), `musicImmersiveCarouselShelfRenderer`, `musicDescriptionShelfRenderer`; items `MTRIR` / `MRLIR` / `musicMultiRowListItemRenderer`. Chips at `SCT.sectionListRenderer.header.chipCloudRenderer.chips`. Skip cards with neither browse nor watch endpoint (deleted uploads). |
| `FEmusic_explore` | no | `SCT.sectionListRenderer.contents` | carousels keyed by `header…title.runs[0].navigationEndpoint.browseEndpoint.browseId`: `FEmusic_new_releases_albums`, `FEmusic_moods_and_genres`, `FEmusic_new_releases_videos`, `FEmusic_top_non_music_audio_episodes`, `VLPL…` (top songs), `VLOLA…` (trending) |
| `FEmusic_moods_and_genres` | no | `SCT.sectionListRenderer.contents[].gridRenderer` | `musicNavigationButtonRenderer`: `buttonText.runs[0].text`, `clickCommand.browseEndpoint.params` → browse `FEmusic_moods_and_genres_category` with those `params` |
| `FEmusic_charts` | no | `SCT.sectionListRenderer.contents` | country via `formData: {"selectedValues": ["US"]}`; carousels unlabeled, classify by item type (`MTRIR` playlists vs `MRLIR` artists) |
| `VLLM` (Liked) | yes | playlist page | see below |
| `FEmusic_history` | yes | `SCT.sectionListRenderer.contents[]` | one `musicShelfRenderer` per period (`title.runs[0].text`), items `MRLIR`. A `musicNotifierShelfRenderer` means history is off / error. |
| `FEmusic_liked_playlists` | yes | library contents → `gridRenderer.items` | `MTRIR`; `items[0]` is the "New playlist" tile, skip it. Continuation `gridContinuation`. |
| `FEmusic_liked_videos` (library songs) | yes | library contents → `musicShelfRenderer.contents` | `MRLIR`; may start with a shuffle entry. Continuation `musicShelfContinuation`. |
| `FEmusic_liked_albums`, `FEmusic_library_corpus_track_artists`, `FEmusic_library_corpus_artists` | yes | library contents | grid (albums) / shelf (artists). Sort `params`: A→Z `ggMGKgQIARAA`, Z→A `ggMGKgQIARAB`, recently added `ggMGKgQIABAB` |
| `FEmusic_library_landing` | yes | **[?]** | unused by ytmusicapi; prefer the specific IDs above |
| `VL<playlistId>` | public: no | `TCB` | see below |
| `MPREb_…` (album) | no | `TCB` | see below |
| `UC…` (artist) | no | `SCT.sectionListRenderer.contents` **or** `TCB.tabs[0].tabRenderer.content.sectionListRenderer.contents` | see below |

**Library contents** (ytmusicapi `get_library_contents`): find the `itemSectionRenderer` in
`SCT.sectionListRenderer.contents` and take `.contents[0].<renderer>`; else
`SCT.sectionListRenderer.contents[0].<renderer>`. An empty library has no section list.

**Classifying an `MTRIR` card**:
1. `title.runs[0].navigationEndpoint.browseEndpoint.browseEndpointContextSupportedConfigs.browseEndpointContextMusicConfig.pageType`
   (`MUSIC_PAGE_TYPE_ALBUM | _ARTIST | _USER_CHANNEL | _PLAYLIST | _AUDIOBOOK | _PODCAST_SHOW_DETAIL_PAGE`).
2. Else `navigationEndpoint.watchPlaylistEndpoint.playlistId` → a watch playlist.
3. Else `navigationEndpoint.watchEndpoint.videoId` → song/video;
   `watchEndpointMusicSupportedConfigs.watchEndpointMusicConfig.musicVideoType` is
   `MUSIC_VIDEO_TYPE_ATV` (song) or `_OMV` (video).

### Playlist pages (`VL…`)

- Header: `TCB.tabs[0].tabRenderer.content.sectionListRenderer.contents[0]`, one of
  - `musicResponsiveHeaderRenderer` (not owned): playlist id from the `musicPlayButtonRenderer`
    in `buttons[]` (find by key, not index) → `playNavigationEndpoint.watchEndpoint.playlistId`;
  - `musicEditablePlaylistDetailHeaderRenderer` (owned): `.playlistId`,
    `.header.musicResponsiveHeaderRenderer`, privacy at
    `.editHeader.musicPlaylistEditHeaderRenderer.privacy`.
  - Fields: `title.runs[0].text`, `description.musicDescriptionShelfRenderer.description.runs`,
    `subtitle.runs`, `secondSubtitle.runs` (may be absent).
- Tracks: `TCB.secondaryContents.sectionListRenderer.contents[0].musicPlaylistShelfRenderer.contents[]` (`MRLIR`).
  - `videoId`: `overlay.musicItemThumbnailOverlayRenderer.content.musicPlayButtonRenderer.playNavigationEndpoint.watchEndpoint.videoId`
    (`playlistItemData.videoId` **[?]**).
  - `setVideoId` (needed to remove from a playlist): `menu.menuRenderer.items[].menuServiceItemRenderer.serviceEndpoint.playlistEditEndpoint.actions[0].setVideoId`.
  - Unavailable: `musicItemRendererDisplayPolicy == "MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT"`.
  - Like state: `menu.menuRenderer.topLevelButtons[0].likeButtonRenderer.likeStatus` (`LIKE | DISLIKE | INDIFFERENT`).
- `OLA…` audio playlists may have no header; id at `musicPlaylistShelfRenderer.targetId`.
- Track continuation: new format (below).

### Album pages (`MPREb_…`)

- Header: `TCB.tabs[0].tabRenderer.content.sectionListRenderer.contents[0].musicResponsiveHeaderRenderer`:
  `title`, `subtitle.runs` (type, year from `runs[2:]`), `straplineTextOne.runs` (artists),
  `secondSubtitle.runs` (count, duration), `thumbnail.musicThumbnailRenderer.thumbnail.thumbnails`.
- Audio playlist id (`OLAK5uy_…`, used for like/queue-all): `buttons[]` →
  `musicPlayButtonRenderer.playNavigationEndpoint.watchPlaylistEndpoint.playlistId`, or
  `…watchEndpoint.playlistId` (A/B test).
- Tracks: `TCB.secondaryContents.sectionListRenderer.contents[0].musicShelfRenderer.contents[]`.
- `contents[1:]`: carousels; `itemSize` `…_SMALL` = related, `…_MEDIUM` = other versions.

### Artist pages (`UC…`)

- Header `header.musicImmersiveHeaderRenderer`: `title`,
  `subscriptionButton.subscribeButtonRenderer.{channelId, subscribed, subscriberCountText}`,
  `playButton.buttonRenderer.navigationEndpoint.watchEndpoint.playlistId` (shuffle),
  `startRadioButton.buttonRenderer.navigationEndpoint.{watchEndpoint|watchPlaylistEndpoint}.playlistId` (radio).
- Body: `contents[0].musicShelfRenderer` = top songs ("see all" at
  `title.runs[0].navigationEndpoint.browseEndpoint.browseId`, usually `VL…`), then carousels
  (albums, singles, videos; "more" = `browseId` + `params` → a `gridRenderer`), then a
  `musicDescriptionShelfRenderer` bio.

### Continuations

- **Old format** (home, library grids/shelves, artist grids, radio queue): token at
  `<renderer>.continuations[0].nextContinuationData.continuation` (radio:
  `nextRadioContinuationData`). Re-POST with `{"continuation": token}` in the body (Metrolist)
  or the original body plus `&ctoken=<t>&continuation=<t>` (ytmusicapi). Result at
  `continuationContents.<type>.{contents|items, continuations}`, type one of
  `sectionListContinuation`, `gridContinuation`, `musicShelfContinuation`,
  `musicPlaylistShelfContinuation`, `playlistPanelContinuation`.
- **New format** (playlist tracks): the last list item is a `continuationItemRenderer`; token at
  `continuationEndpoint.continuationCommand.token`, or inside
  `continuationEndpoint.commandExecutorCommand.commands[]` (the one with
  `continuationCommand.request == "CONTINUATION_REQUEST_TYPE_BROWSE"`). POST `browse` with
  `{"continuation": token}`; items at
  `onResponseReceivedActions[0].appendContinuationItemsAction.continuationItems` (the last may be
  the next `continuationItemRenderer`).
- Done: `parse::continuation_token(renderer)` handles both (trailing `continuationItemRenderer`
  first), `InnerTube::continuation(endpoint, token)` fetches, `parse::continued(resp)` parses
  either response shape. Verified live 2026-10-09 (anonymous): a body of only
  `{"continuation": token}` works for home (`sectionListContinuation`), playlist tracks
  (`appendContinuationItemsAction`) and the radio queue (`playlistPanelContinuation`); the
  `ctoken` query form isn't needed.

## `next` (radio, autoplay)

```json
{"videoId": "<id>", "playlistId": "RDAMVM<id>", "params": "wAEB",
 "isAudioOnly": true, "enablePersistentPlaylistPanel": true,
 "tunerSettingValue": "AUTOMIX_SETTING_NORMAL"}
```

- `params`: `wAEB` = fresh radio; `wAEB8gECKAE%3D` = shuffle a playlist (with `playlistId`).
  For plain "play this video in context", drop `params` and add
  `watchEndpointMusicSupportedConfigs.watchEndpointMusicConfig = {hasPersistentPlaylistPanel: true, musicVideoType: "MUSIC_VIDEO_TYPE_ATV"}`.
- No auth needed.
- Root: `contents.singleColumnMusicWatchNextResultsRenderer.tabbedRenderer.watchNextTabbedResultsRenderer`.
- Queue: `.tabs[0].tabRenderer.content.musicQueueRenderer.content.playlistPanelRenderer`
  (`contents[]`, `playlistId`, `continuations`). Items:
  - `playlistPanelVideoRenderer`: `videoId`, `title.runs[0].text`, `lengthText.runs[0].text`,
    `longBylineText.runs` (artist • album • year, with `pageType` on artist/album runs),
    `thumbnail.thumbnails`, `selected`. Skip if `unplayableText` is present.
  - `playlistPanelVideoWrapperRenderer`: song/video pair at `primaryRenderer.playlistPanelVideoRenderer`
    and `counterpart[0].counterpartRenderer.playlistPanelVideoRenderer`.
  - `automixPreviewVideoRenderer` (usually last):
    `content.automixPlaylistVideoRenderer.navigationEndpoint.watchPlaylistEndpoint.{playlistId, params}`
    → call `next` again with those to get the automix queue.
  - Like state: `menu.menuRenderer.items[].toggleMenuServiceItemRenderer.defaultServiceEndpoint.likeEndpoint.status`,
    **inverted** (`LIKE` = currently not liked); can't distinguish dislike from indifferent.
- More: `playlistPanelRenderer.continuations[0].nextRadioContinuationData.continuation` →
  `continuationContents.playlistPanelContinuation.contents`.
- Lyrics / related: over `watchNextTabbedResultsRenderer.tabs[]` (skip `tabRenderer.unselectable`),
  read `tabRenderer.endpoint.browseEndpoint.browseId` and match `…browseEndpointContextMusicConfig.pageType`:
  `MUSIC_PAGE_TYPE_TRACK_LYRICS` → `MPLYt…`, `MUSIC_PAGE_TYPE_TRACK_RELATED` → `MPTRt…`.
  - `browse MPLYt…` → `contents.sectionListRenderer.contents[0].musicDescriptionShelfRenderer`:
    text `description.runs[0].text`, source `footer.runs[0].text`. Plain text only; synced
    lyrics need the `ANDROID_MUSIC` client (out of scope here).

## `like/*`

- `like/like`, `like/dislike`, `like/removelike` (back to indifferent). Auth required.
- Body `{"target": {"videoId": "<id>"}}`, or `{"target": {"playlistId": "<id>"}}` for playlists
  and albums (album = its `OLAK5uy_` id).
- Response not parsed by ytmusicapi **[?]**: treat 2xx as success, minus the
  `showEngagementPanelEndpoint` gate.

## Anonymous vs signed in

- Anonymous: home, explore, moods, charts, playlists, albums, artists, `next`, lyrics, related.
- Signed in only: `VLLM`, `FEmusic_history`, `FEmusic_liked_*`, `FEmusic_library_*`, `like/*`.
  Matches §4 Auth: hide Library / Playlists / Liked / History from the sidebar when anonymous.
- Signed in also changes shapes: personalised home with more continuations, and the album
  playlist-id location. Record fixtures for both modes where they differ.

## What each crate needs

### `ytm-api`

- `InnerTube::post` serves every endpoint as is. Add `X-Goog-Visitor-Id` and the date-based
  `clientVersion`.
- Move the private helpers in `search.rs` (`runs_text`, `visit`, `find_key`, `last_thumbnail`)
  into a shared `parse` module.
- `parse_tracks` likely works unchanged for history, library songs and playlist rows. New parsers:
  carousels / `MTRIR` cards, `playlistPanelVideoRenderer` (byline in `longBylineText`), playlist
  and album headers, continuation tokens.
- `Track` gains `liked: Option<Rating>`, `set_video_id`, `album_id`, `artist_ids` (for `gA`/`gr`).
  New models: `BrowsePage { title, sections: Vec<Section> }`, `Section { title, items: Cards | Tracks, more }`,
  `PlaylistSummary`, `WatchPlaylist { tracks, continuation }`, `Rating`.
- `MusicApi` gains `browse`, `browse_continuation`, `rate`, `library_playlists` (§5 sketches
  the trait). Done: `radio(seed, continuation) -> WatchPlaylist` (`watch.rs`; fixtures
  `next_radio.json`, `next_radio_continuation.json`).
- Fixtures under `crates/ytm-api/tests/fixtures/`: `home_anon`, `explore`, `playlist_public`,
  `album`, `artist` (both layouts if found), `next_radio`, and signed-in `liked_vllm`, `history`,
  `liked_playlists`. Scrub names, emails, avatars, `visitorData`.

### `ytm-core`

- Browse and like follow the `api.search` pattern in `daemon.rs` `dispatch()`: new
  `api.browse`, `api.browse_more`, `api.like`, `library.playlists` methods, outside the reducer.
- Radio and autoplay go through the reducer. Done:
  - `Queue.autoplay` counts the trailing suggestions; they always follow the current track, so
    played ones become history (`Queue::settle`). Appends land above them; a suggestion moved
    above the divider is adopted; shuffle leaves them alone. `x` = `Command::ClearAutoplay`.
  - `Command::Radio { track }` (`R`, default the current track) replaces the section; a seed
    that isn't queued leads it, and plays at once if the player was idle.
  - `Effect::FetchRadio { seq, seed, continuation }` → `Input::Radio { seq, result }`; stale
    pages dropped by `radio_seq`; continuation tokens never leave the daemon.
  - Autoplay asks when the last track *starts* (or a running radio has ≤ 3 left), once per
    track, so suggestions are queued and prefetched before the end. If the queue still runs
    out, `advance()` stops, fetches, and plays the first suggestion when it lands.
  - Setting: `[player] autoplay = true`, `Command::SetAutoplay` (`X` in the TUI), CLI `ytm-tui autoplay`.
    Repeat all/one disables autoplay.
- `status --json` track gains `"liked"` (§4 CLI).

### `ytm-tui`

- Per-view state: data, cursor, loading flag, sequence number (search's `search_seq` pattern),
  plus a back/forward stack (`Backspace`/`Ctrl-o`, `Ctrl-i`) and `Ctrl-r` refresh (§3).
- Reuse `draw_track_table` for every track list; draw `♥` in `ember` (ASCII `*`) for liked rows.
- New: card grid (12×6 cards, 2-col gutter, `floor((width-2)/14)` columns, `h/j/k/l`, §2),
  user playlists under the sidebar's `PLAYLISTS` header, signed-in items hidden when anonymous,
  footer hints `a queue  R radio  L like`.
- `L` toggles optimistically and rolls back on failure; `D` dislikes. Done: `R` starts radio,
  `x` clears suggestions, the queue draws the `─ autoplay · radio from X ── ↻` divider with
  suggestions in `ink-muted` (`↻ finding more…` in amber while loading), header shows `∞ autoplay`.
- CLI: `like`, `unlike` (§4). Done: `radio`, `autoplay on|off|toggle`, `queue ls` divider,
  `status --json` `autoplay` / `radio` / `queue.autoplay`.

## Order of work

1. ✅ Groundwork: shared `parse` module, continuation helper, visitor id, date-based client version.
2. ✅ Radio + autoplay: anonymous, testable end to end on the null backend; most of the reducer work.
3. Playlist page + Liked (`VLLM`): one parser covers playlists, liked songs and albums.
4. History and library playlists.
5. Likes: `L` / `D`, CLI `like` / `unlike`.
6. Home and Explore: needs the card grid.
7. Alongside 3–6: view history and `Ctrl-r`.

## Open questions

- Like response body; whether playlist rows always carry `playlistItemData.videoId`; what
  `FEmusic_new_releases_albums` and `FEmusic_library_landing` return when browsed directly.
- Whether Google will require the newer `SAPISID1PHASH` / `SAPISID3PHASH` variants the web
  client now sends. ytmusicapi still sends plain `SAPISIDHASH` and it works.
- Caching is Milestone 7 (§4: SQLite, stale-while-revalidate). Until then every view fetches
  live; an in-memory per-view cache in the TUI may be enough.
- These layouts changed several times in 2026. Watch ytmusicapi's changelog, and keep the
  "skip unknown shapes and log" rule.
