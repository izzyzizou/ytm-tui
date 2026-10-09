//! Player state: the queue, transport status, volume, modes. Owned by the daemon's core task
//! and sent to clients as a snapshot (it is `Serialize`).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use ytm_api::Track;
use ytm_audio::StreamInfo;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Stopped,
    /// Resolving the stream URL / opening media.
    Loading,
    Playing,
    Paused,
    Buffering,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    pub fn cycle(self) -> Self {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

/// Tracks in play order. `current` indexes into `items`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Queue {
    pub items: Vec<Track>,
    pub current: Option<usize>,
    /// Video ids of the upcoming tracks in their order before shuffling, so turning
    /// shuffle off restores it.
    #[serde(skip)]
    pub unshuffled: Option<Vec<String>>,
}

impl Queue {
    pub fn current_track(&self) -> Option<&Track> {
        self.items.get(self.current?)
    }

    pub fn has_next(&self) -> bool {
        self.current.is_some_and(|c| c + 1 < self.items.len())
    }

    /// Shuffle everything after the current track.
    pub fn shuffle_upcoming(&mut self) {
        let start = self.current.map_or(0, |c| c + 1);
        if start >= self.items.len() {
            self.unshuffled = Some(Vec::new());
            return;
        }
        self.unshuffled = Some(self.items[start..].iter().map(|t| t.video_id.clone()).collect());
        fastrand::shuffle(&mut self.items[start..]);
    }

    /// Put upcoming tracks back in their pre-shuffle order (tracks added since go last).
    pub fn unshuffle_upcoming(&mut self) {
        let Some(order) = self.unshuffled.take() else { return };
        let start = self.current.map_or(0, |c| c + 1);
        if start >= self.items.len() {
            return;
        }
        self.items[start..].sort_by_key(|t| order.iter().position(|id| *id == t.video_id).unwrap_or(usize::MAX));
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerState {
    pub status: Status,
    pub queue: Queue,
    #[serde(with = "secs")]
    pub position: Duration,
    #[serde(with = "secs_opt")]
    pub duration: Option<Duration>,
    pub volume: u8,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub stream: Option<StreamInfo>,
    pub backend: String,
    pub authenticated: bool,

    /// Generation counter; resolver results for an older generation are ignored.
    #[serde(skip)]
    pub load_seq: u64,
    /// Where the pending load should start (resume after re-resolve).
    #[serde(skip)]
    pub pending_start: Duration,
    /// Recovery attempts for the current track (expired URL, backend crash).
    #[serde(skip)]
    pub attempts: u8,
    /// Video id of the upcoming track whose stream was last prefetched.
    #[serde(skip)]
    pub prefetched: Option<String>,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            status: Status::Stopped,
            queue: Queue::default(),
            position: Duration::ZERO,
            duration: None,
            volume: 72,
            shuffle: false,
            repeat: Repeat::Off,
            stream: None,
            backend: String::new(),
            authenticated: false,
            load_seq: 0,
            pending_start: Duration::ZERO,
            attempts: 0,
            prefetched: None,
        }
    }
}

impl PlayerState {
    pub fn current_track(&self) -> Option<&Track> {
        self.queue.current_track()
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status, Status::Playing | Status::Buffering | Status::Loading)
    }
}

mod secs {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64((d.as_secs_f64() * 10.0).round() / 10.0)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Duration::try_from_secs_f64(f64::deserialize(d)?.max(0.0)).map_err(serde::de::Error::custom)
    }
}

mod secs_opt {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_some(&((d.as_secs_f64() * 10.0).round() / 10.0)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        Option::<f64>::deserialize(d)?.map(|s| Duration::try_from_secs_f64(s.max(0.0))).transpose().map_err(serde::de::Error::custom)
    }
}
