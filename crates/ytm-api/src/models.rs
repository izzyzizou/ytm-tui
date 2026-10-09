use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A playable item (song, music video or podcast episode).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub video_id: String,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: Option<String>,
    /// Whole seconds; `None` when the listing didn't say.
    #[serde(default)]
    pub duration_s: Option<u64>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub kind: ItemKind,
}

impl Track {
    pub fn duration(&self) -> Option<Duration> {
        self.duration_s.map(Duration::from_secs)
    }

    pub fn artist_line(&self) -> String {
        if self.artists.is_empty() {
            "Unknown artist".into()
        } else {
            self.artists.join(", ")
        }
    }

    pub fn url(&self) -> String {
        format!("https://music.youtube.com/watch?v={}", self.video_id)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    #[default]
    Song,
    Video,
    Episode,
}

/// A watch queue from the `next` endpoint (radio), one page at a time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchPlaylist {
    pub tracks: Vec<Track>,
    /// Token for the next page; `None` when the queue has ended.
    #[serde(default)]
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchKind {
    /// Songs if the server offers a Songs filter, otherwise every playable result.
    #[default]
    Songs,
    /// Every playable result in server order.
    All,
}

/// Parse `3:20` / `1:02:03` into seconds.
pub fn parse_duration(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if !(2..=3).contains(&parts.len()) || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    parts.iter().try_fold(0u64, |acc, p| acc.checked_mul(60)?.checked_add(p.parse::<u64>().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("3:20"), Some(200));
        assert_eq!(parse_duration("1:02:03"), Some(3723));
        assert_eq!(parse_duration("37M views"), None);
        assert_eq!(parse_duration(":12"), None);
    }
}
