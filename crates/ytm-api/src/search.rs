//! Search response parsing.
//!
//! InnerTube layouts change often and differ between anonymous and signed-in sessions, so
//! instead of following one fixed path we walk the whole response and parse every list item
//! that has a playable `videoId`. Unknown shapes are skipped, never fatal.

use serde_json::Value;

use crate::models::{parse_duration, ItemKind, Track};

/// The `params` of a search filter chip (e.g. "Songs"), if the server offered it.
pub fn filter_params(resp: &Value, label: &str) -> Option<String> {
    let chips = find_key(resp, "chipCloudRenderer")?.get("chips")?.as_array()?;
    chips.iter().find_map(|c| {
        let c = c.get("chipCloudChipRenderer")?;
        let text = runs_text(c.get("text")?);
        if !text.eq_ignore_ascii_case(label) {
            return None;
        }
        let p = c.pointer("/navigationEndpoint/searchEndpoint/params")?.as_str()?;
        Some(p.replace("%3D", "="))
    })
}

/// Every playable item in the response, de-duplicated, in document order.
pub fn parse_tracks(resp: &Value) -> Vec<Track> {
    let mut out: Vec<Track> = Vec::new();
    let mut push = |t: Option<Track>| {
        if let Some(t) = t {
            if !out.iter().any(|o| o.video_id == t.video_id) {
                out.push(t);
            }
        }
    };
    visit(resp, &mut |key, v| match key {
        "musicCardShelfRenderer" => push(parse_card(v)),
        "musicResponsiveListItemRenderer" => push(parse_list_item(v)),
        _ => {}
    });
    out
}

fn parse_list_item(item: &Value) -> Option<Track> {
    let cols: Vec<&Value> =
        item.get("flexColumns")?.as_array()?.iter().filter_map(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text")).collect();
    let title_runs = cols.first()?;
    let video_id = item
        .pointer("/playlistItemData/videoId")
        .or_else(|| {
            item.pointer(
                "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId",
            )
        })
        .or_else(|| title_runs.pointer("/runs/0/navigationEndpoint/watchEndpoint/videoId"))?
        .as_str()?
        .to_string();
    let mut meta: Vec<&Value> = cols[1..].iter().filter_map(|c| c.get("runs")?.as_array()).flatten().collect();
    if let Some(fixed) = item.get("fixedColumns").and_then(Value::as_array) {
        meta.extend(fixed.iter().filter_map(|c| c.pointer("/musicResponsiveListItemFixedColumnRenderer/text/runs")?.as_array()).flatten());
    }
    let thumbnail = last_thumbnail(item.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails"));
    build(video_id, runs_text(title_runs), &meta, thumbnail)
}

fn parse_card(card: &Value) -> Option<Track> {
    let title = card.get("title")?;
    let video_id = title.pointer("/runs/0/navigationEndpoint/watchEndpoint/videoId")?.as_str()?.to_string();
    let meta: Vec<&Value> = card.pointer("/subtitle/runs").and_then(Value::as_array).map(|r| r.iter().collect()).unwrap_or_default();
    let thumbnail = last_thumbnail(card.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails"));
    build(video_id, runs_text(title), &meta, thumbnail)
}

fn build(video_id: String, title: String, meta: &[&Value], thumbnail: Option<String>) -> Option<Track> {
    let mut kind = ItemKind::Song;
    let mut artists = Vec::new();
    let mut album = None;
    let mut duration_s = None;
    let mut loose = Vec::new();
    for (i, run) in meta.iter().enumerate() {
        let text = run.get("text").and_then(Value::as_str).unwrap_or("").trim();
        if text.is_empty() || text == "•" || text == "&" || text == "," {
            continue;
        }
        if i == 0 {
            match text {
                "Song" => continue,
                "Video" => {
                    kind = ItemKind::Video;
                    continue;
                }
                "Episode" => {
                    kind = ItemKind::Episode;
                    continue;
                }
                "Artist" | "Album" | "Single" | "EP" | "Playlist" | "Podcast" | "Profile" => return None,
                _ => {}
            }
        }
        if let Some(d) = parse_duration(text) {
            duration_s = Some(d);
            continue;
        }
        match run
            .pointer("/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
            .and_then(Value::as_str)
        {
            Some("MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL") => artists.push(text.to_string()),
            Some("MUSIC_PAGE_TYPE_ALBUM") => album = Some(text.to_string()),
            _ => loose.push(text.to_string()),
        }
    }
    // Some layouts put an un-linked artist name first ("Song • Some Artist • 3:20").
    if artists.is_empty() && kind != ItemKind::Episode {
        if let Some(first) = loose.into_iter().find(|t| !looks_like_stat(t)) {
            artists.push(first);
        }
    }
    if title.is_empty() {
        return None;
    }
    Some(Track { video_id, title, artists, album, duration_s, thumbnail, kind })
}

fn looks_like_stat(t: &str) -> bool {
    let l = t.to_ascii_lowercase();
    l.ends_with(" views") || l.ends_with(" plays") || l.ends_with(" audience") || l.chars().next().is_some_and(|c| c.is_ascii_digit())
}

fn runs_text(v: &Value) -> String {
    v.get("runs")
        .and_then(Value::as_array)
        .map(|runs| runs.iter().filter_map(|r| r.get("text")?.as_str()).collect::<String>())
        .or_else(|| v.get("simpleText").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default()
}

fn last_thumbnail(v: Option<&Value>) -> Option<String> {
    v?.as_array()?.last()?.get("url")?.as_str().map(str::to_owned)
}

/// Depth-first visit of every object key (does not descend into matched renderers' children twice).
fn visit<'a>(v: &'a Value, f: &mut impl FnMut(&str, &'a Value)) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                f(k, child);
                if k != "musicResponsiveListItemRenderer" {
                    visit(child, f);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|c| visit(c, f)),
        _ => {}
    }
}

fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(map) => map.get(key).or_else(|| map.values().find_map(|c| find_key(c, key))),
        Value::Array(items) => items.iter().find_map(|c| find_key(c, key)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fixture_anonymous_search() {
        let resp: Value = serde_json::from_str(include_str!("../tests/fixtures/search_all.json")).unwrap();
        let tracks = parse_tracks(&resp);
        assert!(tracks.len() >= 4, "{tracks:#?}");
        let top = &tracks[0];
        assert_eq!(top.title, "One More Time");
        assert_eq!(top.artists, vec!["Daft Punk"]);
        assert_eq!(top.duration_s, Some(322));
        assert_eq!(top.kind, ItemKind::Video);
        assert!(tracks.iter().all(|t| t.video_id.len() == 11));
        assert!(tracks.iter().any(|t| t.kind == ItemKind::Episode));
        // Artists / profiles / podcasts have no videoId and must not appear.
        assert!(!tracks.iter().any(|t| t.title == "Pharrell"));
    }

    #[test]
    fn fixture_has_no_songs_chip_anonymously() {
        let resp: Value = serde_json::from_str(include_str!("../tests/fixtures/search_all.json")).unwrap();
        assert_eq!(filter_params(&resp, "Songs"), None);
        assert_eq!(filter_params(&resp, "Videos").as_deref(), Some("EgWKAQIQAWoKEAQQBRAQEBUQEQ=="));
    }

    #[test]
    fn signed_in_song_layout() {
        // Shape of a signed-in "Songs" shelf row: artist + album links, duration in a fixed column.
        let item = json!({"musicResponsiveListItemRenderer": {
            "playlistItemData": {"videoId": "AAAAAAAAAAA"},
            "flexColumns": [
              {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Glasswater Signals"}]}}},
              {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                {"text": "Lowtide Assembly", "navigationEndpoint": {"browseEndpoint": {"browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}}}},
                {"text": " • "},
                {"text": "Halflight Sessions", "navigationEndpoint": {"browseEndpoint": {"browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ALBUM"}}}}}
              ]}}}
            ],
            "fixedColumns": [{"musicResponsiveListItemFixedColumnRenderer": {"text": {"runs": [{"text": "5:18"}]}}}]
        }});
        let t = parse_tracks(&item);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].artists, vec!["Lowtide Assembly"]);
        assert_eq!(t[0].album.as_deref(), Some("Halflight Sessions"));
        assert_eq!(t[0].duration_s, Some(318));
        assert_eq!(t[0].kind, ItemKind::Song);
    }
}
