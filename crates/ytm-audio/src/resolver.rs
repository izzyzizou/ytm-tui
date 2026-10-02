//! Stream resolution: video id → direct audio URL, via `yt-dlp`.
//!
//! yt-dlp keeps up with YouTube's signature / n-parameter / PO-token changes far faster than
//! an in-house extractor could, so we shell out to it and never re-implement it.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamInfo {
    pub video_id: String,
    pub url: String,
    pub codec: String,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub format_id: Option<String>,
    pub duration_s: Option<f64>,
    /// Unix seconds after which the URL stops working (from its `expire=` parameter).
    pub expires_at: Option<u64>,
}

impl StreamInfo {
    /// True if the URL expires within `margin` (or already has).
    pub fn expires_within(&self, margin: Duration) -> bool {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.expires_at.is_some_and(|e| e <= now + margin.as_secs())
    }

    /// e.g. `opus 160 kb/s · 48 kHz`
    pub fn describe(&self) -> String {
        let mut s = self.codec.clone();
        if let Some(b) = self.bitrate_kbps {
            s.push_str(&format!(" {b} kb/s"));
        }
        if let Some(r) = self.sample_rate {
            s.push_str(&format!(" · {} kHz", r / 1000));
        }
        s
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ResolveError {
    #[error("yt-dlp not found — install it (and keep it updated)")]
    NotInstalled,
    #[error("YouTube wants a signed-in session · run `ytm-tui auth import` and `yt-dlp -U`")]
    BotCheck,
    #[error("video unavailable")]
    Unavailable,
    #[error("yt-dlp timed out")]
    Timeout,
    #[error("yt-dlp: {0}")]
    Other(String),
}

#[async_trait::async_trait]
pub trait StreamResolver: Send + Sync {
    async fn resolve(&self, video_id: &str) -> Result<StreamInfo, ResolveError>;
}

pub struct YtDlp {
    pub binary: String,
    pub format: String,
    pub cookies: Option<PathBuf>,
    pub timeout: Duration,
}

impl Default for YtDlp {
    fn default() -> Self {
        Self {
            binary: "yt-dlp".into(),
            format: "bestaudio[acodec=opus]/bestaudio/best".into(),
            cookies: None,
            timeout: Duration::from_secs(25),
        }
    }
}

#[async_trait::async_trait]
impl StreamResolver for YtDlp {
    async fn resolve(&self, video_id: &str) -> Result<StreamInfo, ResolveError> {
        if !is_video_id(video_id) {
            return Err(ResolveError::Other(format!("not a YouTube video id: {video_id:?}")));
        }
        let mut cmd = Command::new(&self.binary);
        cmd.args(["-J", "--no-playlist", "--no-warnings", "--socket-timeout", "10", "-f", &self.format]);
        if let Some(c) = &self.cookies {
            cmd.arg("--cookies").arg(c);
        }
        cmd.arg(format!("https://music.youtube.com/watch?v={video_id}")).kill_on_drop(true);
        let out = match tokio::time::timeout(self.timeout, cmd.output()).await {
            Err(_) => return Err(ResolveError::Timeout),
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => return Err(ResolveError::NotInstalled),
            Ok(Err(e)) => return Err(ResolveError::Other(e.to_string())),
            Ok(Ok(o)) => o,
        };
        if !out.status.success() {
            return Err(classify_stderr(&String::from_utf8_lossy(&out.stderr)));
        }
        let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| ResolveError::Other(e.to_string()))?;
        parse_info(video_id, &v).ok_or_else(|| ResolveError::Other("no audio URL in yt-dlp output".into()))
    }
}

/// YouTube video ids are 11 chars of `[A-Za-z0-9_-]`. Checked before the id is put on yt-dlp's
/// command line, since ids can arrive over IPC from any client.
pub fn is_video_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn classify_stderr(stderr: &str) -> ResolveError {
    let s = stderr.to_ascii_lowercase();
    if s.contains("sign in to confirm") || s.contains("not a bot") {
        ResolveError::BotCheck
    } else if s.contains("video unavailable") || s.contains("private video") || s.contains("has been removed") {
        ResolveError::Unavailable
    } else {
        let last = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed");
        ResolveError::Other(last.trim().to_string())
    }
}

/// Pick the selected audio format out of `yt-dlp -J` output.
pub fn parse_info(video_id: &str, v: &Value) -> Option<StreamInfo> {
    // With a single selected format its fields are at top level; with merges, in requested_formats.
    let fmt = v
        .get("requested_formats")
        .and_then(Value::as_array)
        .and_then(|fs| fs.iter().find(|f| f.get("acodec").and_then(Value::as_str).is_some_and(|a| a != "none")))
        .unwrap_or(v);
    let url = fmt.get("url")?.as_str()?.to_string();
    let codec = fmt.get("acodec").and_then(Value::as_str).unwrap_or("unknown");
    let codec = codec.split('.').next().unwrap_or(codec).replace("mp4a", "aac");
    Some(StreamInfo {
        video_id: video_id.to_string(),
        expires_at: expire_param(&url),
        bitrate_kbps: fmt.get("abr").and_then(Value::as_f64).map(|b| b.round() as u32),
        sample_rate: fmt.get("asr").and_then(Value::as_u64).map(|r| r as u32),
        format_id: fmt.get("format_id").and_then(Value::as_str).map(str::to_owned),
        duration_s: v.get("duration").and_then(Value::as_f64),
        codec,
        url,
    })
}

fn expire_param(url: &str) -> Option<u64> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|kv| kv.strip_prefix("expire=")?.parse().ok())
}

/// A resolver for the null backend and tests: never touches the network.
pub struct FakeResolver;

#[async_trait::async_trait]
impl StreamResolver for FakeResolver {
    async fn resolve(&self, video_id: &str) -> Result<StreamInfo, ResolveError> {
        Ok(StreamInfo {
            video_id: video_id.into(),
            url: format!("null://{video_id}"),
            codec: "null".into(),
            bitrate_kbps: None,
            sample_rate: None,
            format_id: None,
            duration_s: None,
            expires_at: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_single_format() {
        let v = json!({"id":"abc","duration":318.0,"format_id":"251","acodec":"opus","abr":160.123,"asr":48000,
            "url":"https://rr1---sn.googlevideo.com/videoplayback?expire=1790000000&itag=251&x=1"});
        let s = parse_info("abc", &v).unwrap();
        assert_eq!(s.codec, "opus");
        assert_eq!(s.bitrate_kbps, Some(160));
        assert_eq!(s.expires_at, Some(1_790_000_000));
        assert_eq!(s.describe(), "opus 160 kb/s · 48 kHz");
    }

    #[test]
    fn parses_requested_formats() {
        let v = json!({"requested_formats":[{"acodec":"none","url":"v"},{"acodec":"mp4a.40.2","abr":128.0,"url":"https://x/a?expire=5"}]});
        let s = parse_info("abc", &v).unwrap();
        assert_eq!(s.codec, "aac");
        assert_eq!(s.url, "https://x/a?expire=5");
        assert!(s.expires_within(Duration::from_secs(60)));
    }

    #[test]
    fn validates_video_ids() {
        assert!(is_video_id("dQw4w9WgXcQ"));
        assert!(is_video_id("-_aZ09-_aZ0"));
        assert!(!is_video_id("--exec=evil"));
        assert!(!is_video_id("abc&list=xyz"));
        assert!(!is_video_id("short"));
    }

    #[test]
    fn classifies_errors() {
        assert_eq!(classify_stderr("ERROR: [youtube] x: Sign in to confirm you’re not a bot."), ResolveError::BotCheck);
        assert_eq!(classify_stderr("ERROR: [youtube] x: Video unavailable"), ResolveError::Unavailable);
        assert_eq!(classify_stderr("WARNING: a\nERROR: boom\n"), ResolveError::Other("ERROR: boom".into()));
    }
}
