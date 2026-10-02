//! Lyrics lookup: local `.lrc` override → LRCLIB (synced) → none.
//! (YouTube Music's own unsynced lyrics are a TODO; see docs/ROADMAP.md.)

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use ytm_api::Track;

use crate::lrc::Lyrics;

const LRCLIB: &str = "https://lrclib.net/api";
const UA: &str = concat!("ytm-tui/", env!("CARGO_PKG_VERSION"), " (https://github.com/izzyzizou/ytm-tui)");

#[derive(Deserialize)]
struct LrclibRecord {
    #[serde(rename = "syncedLyrics")]
    synced: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain: Option<String>,
    duration: Option<f64>,
    instrumental: Option<bool>,
}

pub struct LyricsFetcher {
    http: reqwest::Client,
}

impl Default for LyricsFetcher {
    fn default() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(UA)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client");
        Self { http }
    }
}

impl LyricsFetcher {
    pub async fn fetch(&self, track: &Track, local_dir: &Path) -> Option<Lyrics> {
        if let Some(l) = local(track, local_dir) {
            return Some(l);
        }
        self.lrclib(track).await
    }

    async fn lrclib(&self, track: &Track) -> Option<Lyrics> {
        let title = normalize_title(&track.title);
        let artist = track.artists.first().cloned().unwrap_or_default();
        let mut q: Vec<(&str, String)> = vec![("artist_name", artist.clone()), ("track_name", title.clone())];
        if let Some(a) = &track.album {
            q.push(("album_name", a.clone()));
        }
        if let Some(d) = track.duration_s {
            q.push(("duration", d.to_string()));
        }
        if let Ok(r) = self.http.get(format!("{LRCLIB}/get")).query(&q).send().await {
            if r.status().is_success() {
                if let Ok(rec) = r.json::<LrclibRecord>().await {
                    if let Some(l) = to_lyrics(rec) {
                        return Some(l);
                    }
                }
            }
        }
        // Fuzzy search, closest duration wins.
        let recs: Vec<LrclibRecord> =
            self.http.get(format!("{LRCLIB}/search")).query(&[("q", format!("{artist} {title}"))]).send().await.ok()?.json().await.ok()?;
        let target = track.duration_s.map(|d| d as f64);
        let best = recs
            .into_iter()
            .filter(|r| r.synced.is_some() || r.plain.is_some())
            .filter(|r| match (target, r.duration) {
                (Some(t), Some(d)) => (t - d).abs() <= 5.0,
                _ => true,
            })
            .min_by(|a, b| {
                let da = (target.unwrap_or(0.0) - a.duration.unwrap_or(0.0)).abs();
                let db = (target.unwrap_or(0.0) - b.duration.unwrap_or(0.0)).abs();
                da.total_cmp(&db).then_with(|| b.synced.is_some().cmp(&a.synced.is_some()))
            })?;
        to_lyrics(best)
    }
}

fn to_lyrics(rec: LrclibRecord) -> Option<Lyrics> {
    if rec.instrumental == Some(true) {
        return Some(Lyrics::plain("♪ (instrumental)", "lrclib"));
    }
    match (rec.synced, rec.plain) {
        (Some(s), _) if !s.trim().is_empty() => Some(Lyrics::parse_lrc(&s, "lrclib")),
        (_, Some(p)) if !p.trim().is_empty() => Some(Lyrics::plain(&p, "lrclib")),
        _ => None,
    }
}

fn local(track: &Track, dir: &Path) -> Option<Lyrics> {
    let candidates = [format!("{}.lrc", track.video_id), format!("{} - {}.lrc", track.artist_line(), track.title)];
    candidates.iter().find_map(|name| {
        let text = std::fs::read_to_string(dir.join(sanitize(name))).ok()?;
        Some(Lyrics::parse_lrc(&text, "local"))
    })
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '_' } else { c }).collect()
}

/// Strip "(Official Video)", "[Remastered 2011]", "feat. X" etc. before lookup.
pub fn normalize_title(title: &str) -> String {
    let mut t = title.to_string();
    for (open, close) in [('(', ')'), ('[', ']')] {
        while let (Some(a), Some(b)) = (t.find(open), t.find(close)) {
            if b <= a {
                break;
            }
            let inner = t[a + 1..b].to_ascii_lowercase();
            let noise = ["official", "video", "audio", "lyric", "remaster", "feat", "ft.", "visualizer", "live", "hd", "4k", "mv"];
            if noise.iter().any(|n| inner.contains(n)) {
                t.replace_range(a..=b, "");
            } else {
                break;
            }
        }
    }
    for sep in [" feat. ", " ft. ", " featuring "] {
        if let Some(i) = t.to_ascii_lowercase().find(sep) {
            t.truncate(i);
        }
    }
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_titles() {
        assert_eq!(normalize_title("One More Time (Official Video)"), "One More Time");
        assert_eq!(normalize_title("Song [Remastered 2011] (Lyric Video)"), "Song");
        assert_eq!(normalize_title("Song feat. Someone"), "Song");
        assert_eq!(normalize_title("Song (Part 2)"), "Song (Part 2)");
    }
}
