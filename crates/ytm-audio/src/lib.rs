//! Audio playback. Everything above this crate talks to an [`AudioBackend`] and listens
//! for [`PlayerEvent`]s; it never knows whether mpv or the simulated backend is underneath.

pub mod mpv;
pub mod null;
pub mod resolver;

use std::time::Duration;

pub use resolver::{FakeResolver, ResolveError, StreamInfo, StreamResolver, YtDlp};

/// Something the backend can open: an HTTPS stream URL or a local file path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    pub location: String,
    /// Known duration, used by backends that can't probe (the null backend).
    pub duration_hint: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    /// Media opened and audio is about to start.
    Started {
        duration: Option<Duration>,
    },
    Position(Duration),
    Paused(bool),
    /// True while the backend is starved for data.
    Buffering(bool),
    /// Reached the end of the current media.
    Ended,
    /// The current media failed (e.g. HTTP 403 on an expired stream URL).
    Error(String),
    /// The backend process died; the supervisor may restart it.
    Exited,
}

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("backend not available: {0}")]
    Unavailable(String),
    #[error("backend I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("backend protocol: {0}")]
    Protocol(String),
}

#[async_trait::async_trait]
pub trait AudioBackend: Send {
    fn name(&self) -> &'static str;
    async fn load(&mut self, media: Media, start: Duration) -> Result<(), PlayerError>;
    async fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError>;
    async fn seek(&mut self, to: Duration) -> Result<(), PlayerError>;
    async fn set_volume(&mut self, percent: u8) -> Result<(), PlayerError>;
    async fn stop(&mut self) -> Result<(), PlayerError>;
}
