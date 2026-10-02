//! Browser-session authentication.
//!
//! The user copies one signed-in `POST /youtubei/v1/browse` request from their browser's
//! devtools (as cURL or as raw request headers), or points us at an existing ytmusicapi
//! `headers_auth.json` / `browser.json`. We keep only what InnerTube needs and sign each
//! request with `SAPISIDHASH`.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

pub const ORIGIN: &str = "https://music.youtube.com";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    pub cookie: String,
    #[serde(default)]
    pub user_agent: Option<String>,
    /// `X-Goog-AuthUser` — which signed-in Google account in that browser (usually "0").
    #[serde(default)]
    pub auth_user: Option<String>,
    /// `X-Goog-PageId` — set for brand accounts.
    #[serde(default)]
    pub page_id: Option<String>,
}

/// Redacts the cookie so credentials can never end up in a log line by accident.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("cookie", &format_args!("<redacted, {} bytes>", self.cookie.len()))
            .field("user_agent", &self.user_agent)
            .field("auth_user", &self.auth_user)
            .field("page_id", &self.page_id)
            .finish()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("no Cookie header found — copy a request made while signed in to music.youtube.com")]
    NoCookie,
    #[error("the cookie has no SAPISID / __Secure-3PAPISID — are you signed in?")]
    NoSapisid,
    #[error("could not parse headers JSON: {0}")]
    Json(String),
}

impl Credentials {
    /// Auto-detect the format: JSON object, a cURL command, or raw `Name: value` lines.
    pub fn parse(input: &str) -> Result<Self, AuthError> {
        let trimmed = input.trim();
        let creds = if trimmed.starts_with('{') {
            Self::from_json(trimmed)?
        } else if trimmed.starts_with("curl ") || trimmed.contains("\ncurl ") {
            Self::from_curl(trimmed)?
        } else {
            Self::from_raw_headers(trimmed)?
        };
        if creds.sapisid().is_none() {
            return Err(AuthError::NoSapisid);
        }
        Ok(creds)
    }

    /// ytmusicapi `headers_auth.json` / `browser.json`: a flat object of header names.
    pub fn from_json(s: &str) -> Result<Self, AuthError> {
        let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(s).map_err(|e| AuthError::Json(e.to_string()))?;
        let get = |name: &str| map.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).and_then(|(_, v)| v.as_str().map(str::to_owned));
        Self::from_pairs(get("cookie"), get("user-agent"), get("x-goog-authuser"), get("x-goog-pageid"))
    }

    /// `curl 'https://music.youtube.com/youtubei/v1/browse?...' -H 'cookie: ...' -b '...'`
    pub fn from_curl(s: &str) -> Result<Self, AuthError> {
        let joined = s.replace("\\\n", " ").replace("^\n", " ");
        let args = shell_words(&joined);
        let mut headers = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "-H" | "--header" => {
                    if let Some(h) = it.next() {
                        if let Some((k, v)) = h.split_once(':') {
                            headers.push((k.trim().to_string(), v.trim().to_string()));
                        }
                    }
                }
                "-b" | "--cookie" => {
                    if let Some(c) = it.next() {
                        headers.push(("cookie".into(), c.trim().to_string()));
                    }
                }
                _ => {}
            }
        }
        Self::from_header_list(&headers)
    }

    /// Raw request headers as copied from devtools: one `Name: value` per line.
    pub fn from_raw_headers(s: &str) -> Result<Self, AuthError> {
        let headers: Vec<(String, String)> = s
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                // HTTP/2 pseudo-headers (":authority: ...") are skipped.
                let l = l.strip_prefix(':').map_or(Some(l), |_| None)?;
                let (k, v) = l.split_once(':')?;
                Some((k.trim().to_string(), v.trim().to_string()))
            })
            .collect();
        Self::from_header_list(&headers)
    }

    fn from_header_list(headers: &[(String, String)]) -> Result<Self, AuthError> {
        let get = |name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone());
        Self::from_pairs(get("cookie"), get("user-agent"), get("x-goog-authuser"), get("x-goog-pageid"))
    }

    fn from_pairs(
        cookie: Option<String>,
        user_agent: Option<String>,
        auth_user: Option<String>,
        page_id: Option<String>,
    ) -> Result<Self, AuthError> {
        let cookie = cookie.filter(|c| !c.trim().is_empty()).ok_or(AuthError::NoCookie)?;
        Ok(Self { cookie: cookie.trim().to_string(), user_agent, auth_user, page_id })
    }

    /// Value of `SAPISID` (or `__Secure-3PAPISID`) from the cookie string.
    pub fn sapisid(&self) -> Option<&str> {
        let find = |name: &str| {
            self.cookie.split(';').find_map(|kv| {
                let (k, v) = kv.trim().split_once('=')?;
                (k == name).then_some(v)
            })
        };
        find("SAPISID").or_else(|| find("__Secure-3PAPISID"))
    }

    /// `Authorization` header value for a request made now.
    pub fn authorization(&self) -> Option<String> {
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        Some(sapisidhash(self.sapisid()?, ts, ORIGIN))
    }

    /// Cookies in Netscape `cookies.txt` format, for `yt-dlp --cookies`.
    pub fn to_netscape_cookies(&self) -> String {
        let mut out = String::from("# Netscape HTTP Cookie File\n# written by ytm-tui; do not share\n");
        for kv in self.cookie.split(';') {
            if let Some((k, v)) = kv.trim().split_once('=') {
                let secure = if k.starts_with("__Secure-") || k.starts_with("__Host-") { "TRUE" } else { "FALSE" };
                out.push_str(&format!(".youtube.com\tTRUE\t/\t{secure}\t2147483647\t{k}\t{v}\n"));
            }
        }
        out
    }

    pub fn load(path: &Path) -> std::io::Result<Option<Self>> {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).map(Some).map_err(std::io::Error::other),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Writes with mode 0600 on Unix. (OS keyring storage is a planned upgrade.)
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            create_private_dir(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        write_private(path, json.as_bytes())
    }
}

/// `SAPISIDHASH <ts>_<sha1("<ts> <SAPISID> <origin>")>`
pub fn sapisidhash(sapisid: &str, ts: u64, origin: &str) -> String {
    let digest = Sha1::digest(format!("{ts} {sapisid} {origin}").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("SAPISIDHASH {ts}_{hex}")
}

/// Writes `bytes` readable by the owner only. Tightens an existing file's mode too (`mode()`
/// only applies on creation) and refuses to follow a symlink at `path`.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(std::io::Error::other(format!("refusing to write credentials through a symlink: {}", path.display())));
        }
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(path)?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        f.set_len(0)?;
        f.write_all(bytes)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

/// `create_dir_all`, but directories it creates are 0700 on Unix (existing ones are left alone).
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

/// Minimal POSIX-ish word splitting for pasted cURL commands (single/double quotes, `$'...'`).
fn shell_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    cur.push(c);
                }
            }
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '\'' => break,
                        '\\' => {
                            if let Some(n) = chars.next() {
                                cur.push(n);
                            }
                        }
                        _ => cur.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => {
                            if let Some(n) = chars.next() {
                                cur.push(n);
                            }
                        }
                        _ => cur.push(c),
                    }
                }
            }
            '\\' => {
                if let Some(n) = chars.next() {
                    in_word = true;
                    cur.push(n);
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    const COOKIE: &str = "VISITOR_INFO1_LIVE=abc; SAPISID=sap123/xyz; __Secure-3PAPISID=sap123/xyz; SID=s";

    #[test]
    fn hash_matches_reference() {
        // sha1("1700000000 sap123/xyz https://music.youtube.com")
        let h = sapisidhash("sap123/xyz", 1_700_000_000, ORIGIN);
        let expected = {
            let d = Sha1::digest(b"1700000000 sap123/xyz https://music.youtube.com");
            d.iter().map(|b| format!("{b:02x}")).collect::<String>()
        };
        assert_eq!(h, format!("SAPISIDHASH 1700000000_{expected}"));
    }

    #[test]
    fn parses_curl() {
        let curl = format!(
            "curl 'https://music.youtube.com/youtubei/v1/browse?prettyPrint=false' \\\n  -H 'accept: */*' \\\n  -H 'x-goog-authuser: 1' \\\n  -b '{COOKIE}' \\\n  -H 'user-agent: Mozilla/5.0 Test' \\\n  --data-raw '{{}}'"
        );
        let c = Credentials::parse(&curl).unwrap();
        assert_eq!(c.cookie, COOKIE);
        assert_eq!(c.auth_user.as_deref(), Some("1"));
        assert_eq!(c.user_agent.as_deref(), Some("Mozilla/5.0 Test"));
        assert_eq!(c.sapisid(), Some("sap123/xyz"));
    }

    #[test]
    fn parses_raw_headers() {
        let raw = format!(":authority: music.youtube.com\n:method: POST\naccept: */*\ncookie: {COOKIE}\nx-goog-authuser: 0\n");
        let c = Credentials::parse(&raw).unwrap();
        assert_eq!(c.cookie, COOKIE);
        assert_eq!(c.auth_user.as_deref(), Some("0"));
    }

    #[test]
    fn parses_ytmusicapi_json() {
        let json = format!(r#"{{"User-Agent":"UA","Cookie":"{COOKIE}","X-Goog-AuthUser":"0","x-origin":"https://music.youtube.com"}}"#);
        let c = Credentials::parse(&json).unwrap();
        assert_eq!(c.user_agent.as_deref(), Some("UA"));
        assert!(c.authorization().unwrap().starts_with("SAPISIDHASH "));
    }

    #[test]
    fn rejects_signed_out() {
        assert_eq!(Credentials::parse("cookie: VISITOR_INFO1_LIVE=abc"), Err(AuthError::NoSapisid));
        assert_eq!(Credentials::parse("accept: */*"), Err(AuthError::NoCookie));
    }

    #[test]
    fn netscape_export() {
        let c = Credentials::parse(&format!("cookie: {COOKIE}")).unwrap();
        let txt = c.to_netscape_cookies();
        assert!(txt.contains(".youtube.com\tTRUE\t/\tTRUE\t2147483647\t__Secure-3PAPISID\tsap123/xyz"));
        assert!(txt.contains("\tFALSE\t2147483647\tSID\ts"));
    }

    #[test]
    fn debug_redacts_cookie() {
        let c = Credentials::parse(&format!("cookie: {COOKIE}")).unwrap();
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("sap123"), "{dbg}");
        assert!(dbg.contains("redacted"));
    }

    #[cfg(unix)]
    #[test]
    fn write_private_tightens_existing_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ytm-auth-test-{}", std::process::id()));
        create_private_dir(&dir).unwrap();
        let path = dir.join("auth.json");
        std::fs::write(&path, "a much longer previous content").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, b"new").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        let link = dir.join("link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(write_private(&link, b"x").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
