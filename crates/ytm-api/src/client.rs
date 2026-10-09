//! InnerTube HTTP client (the private API the music.youtube.com web app uses).

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::{header, StatusCode};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::auth::{Credentials, ORIGIN};
use crate::models::{SearchKind, Track, WatchPlaylist};
use crate::{search, watch};

pub const BASE: &str = "https://music.youtube.com/youtubei/v1";
/// WEB_REMIX client version for a given time: `1.YYYYMMDD.01.00` with today's UTC date, as the
/// web app (and ytmusicapi) sends it. Deriving it means it never goes stale.
pub fn client_version(now: SystemTime) -> String {
    let days = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("1.{y:04}{m:02}{d:02}.01.00")
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian (H. Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
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
    /// `X-Goog-Visitor-Id`: adopted from the first response that carries one and sent from then
    /// on, so anonymous requests look like one returning visitor rather than a new one each time.
    visitor: std::sync::Mutex<Option<String>>,
    visitor_file: Option<PathBuf>,
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
            visitor: std::sync::Mutex::new(None),
            visitor_file: None,
        })
    }

    /// Load the visitor id from `path` if present, and save it there once one is assigned.
    pub fn with_visitor_file(mut self, path: PathBuf) -> Self {
        let saved = std::fs::read_to_string(&path).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        *self.visitor.get_mut().unwrap_or_else(|e| e.into_inner()) = saved;
        self.visitor_file = Some(path);
        self
    }

    pub fn visitor_data(&self) -> Option<String> {
        self.visitor.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    async fn adopt_visitor(&self, resp: &Value) {
        let Some(v) = resp.pointer("/responseContext/visitorData").and_then(Value::as_str) else { return };
        {
            let mut cur = self.visitor.lock().unwrap_or_else(|e| e.into_inner());
            if cur.is_some() {
                return;
            }
            *cur = Some(v.to_owned());
        }
        if let Some(path) = &self.visitor_file {
            if let Some(dir) = path.parent() {
                let _ = tokio::fs::create_dir_all(dir).await;
            }
            if let Err(e) = tokio::fs::write(path, v).await {
                tracing::debug!(path = %path.display(), "could not save visitor id: {e}");
            }
        }
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
            "client": { "clientName": "WEB_REMIX", "clientVersion": client_version(SystemTime::now()), "hl": self.hl, "gl": self.gl },
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
        if let Some(v) = self.visitor_data() {
            req = req.header("X-Goog-Visitor-Id", v);
        }
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
            s if s.is_success() => {
                let v: Value = resp.json().await.map_err(|e| ApiError::Parse(e.to_string()))?;
                self.adopt_visitor(&v).await;
                Ok(v)
            }
            StatusCode::TOO_MANY_REQUESTS => {
                let secs = resp.headers().get(header::RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(10);
                Err(ApiError::RateLimited { retry_after: Duration::from_secs(secs) })
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN if self.creds.is_some() => Err(ApiError::SignedOut),
            s => Err(ApiError::Http(s)),
        }
    }

    /// Next page of a list from `endpoint` (`browse` or `next`); `token` comes from
    /// [`parse::continuation_token`](crate::parse::continuation_token). Parse the result with
    /// [`parse::continued`](crate::parse::continued).
    pub async fn continuation(&self, endpoint: &str, token: &str) -> Result<Value, ApiError> {
        self.post(endpoint, json!({ "continuation": token })).await
    }

    pub async fn radio(&self, seed: &str, continuation: Option<&str>) -> Result<WatchPlaylist, ApiError> {
        let resp = match continuation {
            Some(token) => self.continuation("next", token).await?,
            None => self.post("next", watch::radio_body(seed)).await?,
        };
        Ok(watch::parse_watch(&resp))
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

    async fn radio(&self, seed: &str, continuation: Option<&str>) -> Result<WatchPlaylist, ApiError> {
        InnerTube::radio(self, seed, continuation).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_version_uses_utc_date() {
        let at = |secs: u64| client_version(UNIX_EPOCH + Duration::from_secs(secs));
        assert_eq!(at(0), "1.19700101.01.00");
        assert_eq!(at(951_782_400), "1.20000229.01.00"); // leap day
        assert_eq!(at(1_791_590_399), "1.20261009.01.00"); // 2026-10-09 23:59:59 UTC
        assert_eq!(at(1_791_590_400), "1.20261010.01.00");
    }

    #[tokio::test]
    async fn visitor_is_adopted_once_and_saved() {
        let dir = std::env::temp_dir().join(format!("ytm-api-visitor-{}", std::process::id()));
        let file = dir.join("visitor_data");
        let _ = std::fs::remove_dir_all(&dir);
        let api = InnerTube::new(None).unwrap().with_visitor_file(file.clone());
        assert_eq!(api.visitor_data(), None);
        api.adopt_visitor(&json!({"responseContext": {"visitorData": "first"}})).await;
        api.adopt_visitor(&json!({"responseContext": {"visitorData": "second"}})).await;
        assert_eq!(api.visitor_data().as_deref(), Some("first"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "first");
        let reloaded = InnerTube::new(None).unwrap().with_visitor_file(file);
        assert_eq!(reloaded.visitor_data().as_deref(), Some("first"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
