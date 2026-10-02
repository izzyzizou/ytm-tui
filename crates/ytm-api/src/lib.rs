//! YouTube Music API layer. No UI and no audio code lives here.
//!
//! * [`auth`]   — import browser headers / `headers_auth.json`, SAPISIDHASH signing.
//! * [`client`] — the InnerTube HTTP client (context body, rate limiting, errors).
//! * [`search`] — search + resilient response parsing.
//! * [`models`] — plain data types shared with the rest of the app.

pub mod auth;
pub mod client;
pub mod models;
pub mod search;

pub use client::{ApiError, InnerTube};
pub use models::{SearchKind, Track};

/// The metadata API as the rest of the app sees it. Implemented by [`InnerTube`];
/// tests and offline mode can provide their own.
#[async_trait::async_trait]
pub trait MusicApi: Send + Sync {
    async fn search(&self, query: &str, kind: SearchKind, limit: usize) -> Result<Vec<Track>, ApiError>;
}
