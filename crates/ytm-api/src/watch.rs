//! `next` (watch queue) parsing: radio pages and their continuations.
//!
//! Queue items are `playlistPanelVideoRenderer`s (sometimes wrapped in a
//! `playlistPanelVideoWrapperRenderer` that pairs a song with its music video). Their byline is
//! `longBylineText` ("Artist • Album • 2001" for songs, "Artist • 620M views • …" for videos)
//! and the duration is `lengthText`.

use serde_json::Value;

use crate::models::{ItemKind, Track, WatchPlaylist};
use crate::parse::{continuation_token, continued, find_key, last_thumbnail, runs_text};
use crate::search::build;

/// Body of a `next` request that starts a fresh radio from `video_id`.
pub fn radio_body(video_id: &str) -> Value {
    serde_json::json!({
        "videoId": video_id,
        "playlistId": format!("RDAMVM{video_id}"),
        "params": "wAEB",
        "isAudioOnly": true,
        "enablePersistentPlaylistPanel": true,
        "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
    })
}

/// Tracks and next-page token of a `next` response or of a continuation of one.
/// Unplayable and unparseable items are skipped.
pub fn parse_watch(resp: &Value) -> WatchPlaylist {
    if let Some(panel) = find_key(resp, "playlistPanelRenderer") {
        let items = panel.get("contents").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
        return WatchPlaylist { tracks: tracks(items), continuation: continuation_token(panel) };
    }
    match continued(resp) {
        Some(page) => WatchPlaylist { tracks: tracks(page.items), continuation: page.next },
        None => {
            tracing::debug!("next: no playlist panel in response");
            WatchPlaylist::default()
        }
    }
}

fn tracks(items: Vec<&Value>) -> Vec<Track> {
    items.into_iter().filter_map(panel_item).collect()
}

fn panel_item(item: &Value) -> Option<Track> {
    let r = item
        .get("playlistPanelVideoRenderer")
        .or_else(|| item.pointer("/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer"))?;
    if r.get("unplayableText").is_some() {
        return None;
    }
    let video_id = r.get("videoId").or_else(|| r.pointer("/navigationEndpoint/watchEndpoint/videoId"))?.as_str()?.to_string();
    let mut meta: Vec<&Value> = r.pointer("/longBylineText/runs").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    meta.extend(r.pointer("/lengthText/runs").and_then(Value::as_array).into_iter().flatten());
    let thumbnail = last_thumbnail(r.pointer("/thumbnail/thumbnails"));
    let mut track = build(video_id, runs_text(r.get("title")?), &meta, thumbnail)?;
    let video_type =
        r.pointer("/navigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType");
    match video_type.and_then(Value::as_str) {
        Some("MUSIC_VIDEO_TYPE_ATV") => track.kind = ItemKind::Song,
        Some("MUSIC_VIDEO_TYPE_PODCAST_EPISODE") => track.kind = ItemKind::Episode,
        Some(_) => track.kind = ItemKind::Video,
        None => {}
    }
    Some(track)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(name: &str) -> Value {
        let text = match name {
            "radio" => include_str!("../tests/fixtures/next_radio.json"),
            _ => include_str!("../tests/fixtures/next_radio_continuation.json"),
        };
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn fixture_radio_first_page() {
        // Anonymous radio for "One More Time" (FGBhQbmPwH8), trimmed to 8 items, recorded 2026-10-09.
        let page = parse_watch(&fixture("radio"));
        assert_eq!(page.tracks.len(), 8, "{:#?}", page.tracks);
        let seed = &page.tracks[0];
        assert_eq!(seed.video_id, "FGBhQbmPwH8");
        assert_eq!(seed.title, "One More Time");
        assert_eq!(seed.artists, vec!["Daft Punk"]);
        assert_eq!(seed.duration_s, Some(322));
        assert_eq!(seed.kind, ItemKind::Video, "official music video");
        assert!(page.tracks.iter().all(|t| t.video_id.len() == 11 && !t.title.is_empty() && !t.artists.is_empty()));
        assert!(page.tracks.iter().all(|t| t.duration_s.is_some() && t.thumbnail.is_some()));
        assert!(page.continuation.as_deref().is_some_and(|c| !c.is_empty()));
    }

    #[test]
    fn fixture_radio_continuation() {
        let page = parse_watch(&fixture("continuation"));
        assert_eq!(page.tracks.len(), 4, "{:#?}", page.tracks);
        assert!(page.tracks.iter().all(|t| t.video_id.len() == 11 && !t.artists.is_empty()));
        assert!(page.continuation.is_some());
    }

    #[test]
    fn song_byline_wrapper_and_unplayable() {
        let song = |id: &str| {
            json!({"playlistPanelVideoRenderer": {
                "videoId": id,
                "title": {"runs": [{"text": "Glasswater Signals"}]},
                "longBylineText": {"runs": [
                    {"text": "Lowtide Assembly", "navigationEndpoint": {"browseEndpoint": {"browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}}}},
                    {"text": " • "},
                    {"text": "Halflight Sessions", "navigationEndpoint": {"browseEndpoint": {"browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ALBUM"}}}}},
                    {"text": " • "},
                    {"text": "2019"}
                ]},
                "lengthText": {"runs": [{"text": "5:18"}]},
                "navigationEndpoint": {"watchEndpoint": {"videoId": id, "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig": {"musicVideoType": "MUSIC_VIDEO_TYPE_ATV"}}}}
            }})
        };
        let mut gone = song("CCCCCCCCCCC");
        gone["playlistPanelVideoRenderer"]["unplayableText"] = json!({"runs": [{"text": "Video unavailable"}]});
        let resp = json!({"playlistPanelRenderer": {"contents": [
            song("AAAAAAAAAAA"),
            {"playlistPanelVideoWrapperRenderer": {"primaryRenderer": song("BBBBBBBBBBB"), "counterpart": []}},
            gone,
            {"automixPreviewVideoRenderer": {}}
        ]}});
        let page = parse_watch(&resp);
        let ids: Vec<&str> = page.tracks.iter().map(|t| t.video_id.as_str()).collect();
        assert_eq!(ids, ["AAAAAAAAAAA", "BBBBBBBBBBB"]);
        let t = &page.tracks[0];
        assert_eq!(t.artists, vec!["Lowtide Assembly"]);
        assert_eq!(t.album.as_deref(), Some("Halflight Sessions"));
        assert_eq!(t.duration_s, Some(318));
        assert_eq!(t.kind, ItemKind::Song);
        assert_eq!(page.continuation, None);
    }

    #[test]
    fn unknown_shape_is_empty() {
        assert_eq!(parse_watch(&json!({"contents": {}})), WatchPlaylist::default());
    }
}
