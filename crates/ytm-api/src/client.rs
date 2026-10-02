//! InnerTube HTTP client (the private API the music.youtube.com web app uses).

use std::time::{Duration, Instant};

use reqwest::{header, StatusCode};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::auth::{Credentials, ORIGIN};
use crate::models::{SearchKind, Track};
use crate::search;

pub const BASE: &str = "https://music.youtube.com/youtubei/v1";
/// WEB_REMIX client version. YouTube accepts fairly old versions, but bump this
/// occasionally (copy it from any music.youtube.com request in devtools).
pub const CLIENT_VERSION: &str = "1.20260928.01.00";
const DEFAULT_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("rate limited · retry in {}s", .retry_after.as_secs())]
    RateLimited { retry_after: Duration },
    #[error("signed out · run `ytm-tui auth import`")]
    SignedOut,
    #[error("HTTP {0}")]
    Http(StatusCode),
    #[error("network: {0}")]
    Network(String),
    #[error("unexpected response: {0}")]
    Parse(String),
}

/// A header value hyper/reqwest will never print in debug or trace logs.
fn sensitive(v: &str) -> Result<header::HeaderValue, ApiError> {
    let mut v = header::HeaderValue::from_str(v).map_err(|e| ApiError::Network(format!("invalid credential header: {e}")))?;
    v.set_sensitive(true);
    Ok(v)
}

impl From<reqwest::Error> for ApiError {
    fn from(e: reqwest::Error) -> Self {
        ApiError::Network(e.to_string())
    }
}

pub struct InnerTube {
    http: reqwest::Client,
    creds: Option<Credentials>,
    /// Earliest instant the next request may start (simple 5 req/s limiter).
    next_slot: Mutex<Instant>,
    min_interval: Duration,
    hl: String,
    gl: String,
}

impl InnerTube {
    pub fn new(creds: Option<Credentials>) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(15)).build()?;
        Ok(Self {
            http,
            creds,
            next_slot: Mutex::new(Instant::now()),
            min_interval: Duration::from_millis(200),
            hl: "en".into(),
            gl: "US".into(),
        })
    }

    pub fn is_authenticated(&self) -> bool {
        self.creds.is_some()
    }

    async fn throttle(&self) {
        let wait = {
            let mut slot = self.next_slot.lock().await;
            let now = Instant::now();
            let start = (*slot).max(now);
            *slot = start + self.min_interval;
            start - now
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }

    /// POST `{BASE}/{endpoint}` with the client context merged into `body`.
    pub async fn post(&self, endpoint: &str, mut body: Value) -> Result<Value, ApiError> {
        self.throttle().await;
        body["context"] = json!({
            "client": { "clientName": "WEB_REMIX", "clientVersion": CLIENT_VERSION, "hl": self.hl, "gl": self.gl },
            "user": {}
        });
        let mut req = self
            .http
            .post(format!("{BASE}/{endpoint}?prettyPrint=false"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, ORIGIN)
            .header("X-Origin", ORIGIN)
            .header(header::REFERER, format!("{ORIGIN}/"));
        let ua = self.creds.as_ref().and_then(|c| c.user_agent.clone()).unwrap_or_else(|| DEFAULT_UA.into());
        req = req.header(header::USER_AGENT, ua);
        if let Some(c) = &self.creds {
            req = req.header(header::COOKIE, sensitive(&c.cookie)?).header("X-Goog-AuthUser", c.auth_user.as_deref().unwrap_or("0"));
            if let Some(auth) = c.authorization() {
                req = req.header(header::AUTHORIZATION, sensitive(&auth)?);
            }
            if let Some(p) = &c.page_id {
                req = req.header("X-Goog-PageId", p);
            }
        }
        let resp = req.json(&body).send().await?;
        match resp.status() {
            s if s.is_success() => resp.json().await.map_err(|e| ApiError::Parse(e.to_string())),
            StatusCode::TOO_MANY_REQUESTS => {
                let secs = resp.headers().get(header::RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(10);
                Err(ApiError::RateLimited { retry_after: Duration::from_secs(secs) })
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN if self.creds.is_some() => Err(ApiError::SignedOut),
            s => Err(ApiError::Http(s)),
        }
    }

    pub async fn search(&self, query: &str, kind: SearchKind, limit: usize) -> Result<Vec<Track>, ApiError> {
        let first = self.post("search", json!({ "query": query })).await?;
        let page = if kind == SearchKind::Songs {
            match search::filter_params(&first, "Songs") {
                Some(params) => self.post("search", json!({ "query": query, "params": params })).await?,
                None => first, // anonymous / some regions: no Songs chip — fall back to all playable
            }
        } else {
            first
        };
        let mut tracks = search::parse_tracks(&page);
        if kind == SearchKind::Songs {
            // Songs first, then videos, then episodes; stable within each group.
            tracks.sort_by_key(|t| t.kind as u8);
        }
        tracks.truncate(limit);
        Ok(tracks)
    }
}

#[async_trait::async_trait]
impl crate::MusicApi for InnerTube {
    async fn search(&self, query: &str, kind: SearchKind, limit: usize) -> Result<Vec<Track>, ApiError> {
        InnerTube::search(self, query, kind, limit).await
    }
}
