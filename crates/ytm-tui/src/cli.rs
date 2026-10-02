//! Headless CLI. Exit codes: 0 ok · 1 error · 2 usage · 3 daemon not running · 4 not found · 5 auth required.

use std::io::Read;
use std::process::ExitCode;

use serde_json::{json, Value};
use ytm_api::auth::Credentials;
use ytm_api::Track;
use ytm_core::client::{Client, ClientError};
use ytm_core::paths;
use ytm_core::protocol::{method, LyricsUpdate};
use ytm_core::reducer::Command;
use ytm_core::state::PlayerState;

use crate::{AuthCmd, Cmd, QueueCmd};

/// `println!` that exits quietly when stdout is closed (e.g. `ytm-tui lyrics | head`).
macro_rules! say {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if let Err(e) = writeln!(std::io::stdout().lock(), $($arg)*) {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                std::process::exit(0);
            }
        }
    }};
}

const EXIT_ERR: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_NOT_RUNNING: u8 = 3;
const EXIT_NOT_FOUND: u8 = 4;
const EXIT_AUTH: u8 = 5;

struct Fail(u8, String);

impl From<ClientError> for Fail {
    fn from(e: ClientError) -> Self {
        match &e {
            ClientError::NotRunning => Fail(EXIT_NOT_RUNNING, "daemon not running · start it with `ytm-tui` or `ytm-tui daemon`".into()),
            ClientError::Remote(r) if r.code == "not_found" => Fail(EXIT_NOT_FOUND, r.message.clone()),
            ClientError::Remote(r) if r.code == "auth_required" => Fail(EXIT_AUTH, r.message.clone()),
            _ => Fail(EXIT_ERR, e.to_string()),
        }
    }
}

pub async fn run(cmd: Cmd, backend: Option<String>) -> ExitCode {
    let json_mode = matches!(
        &cmd,
        Cmd::Status { json: true, .. } | Cmd::Search { json: true, .. } | Cmd::Queue { action: Some(QueueCmd::Ls { json: true }) }
    );
    match exec(cmd, backend).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(Fail(code, msg)) => {
            if json_mode {
                say!("{}", json!({ "error": { "code": code, "message": msg } }));
            } else {
                eprintln!("ytm-tui: {msg}");
            }
            ExitCode::from(code)
        }
    }
}

async fn connect() -> Result<Client, Fail> {
    Ok(Client::connect(&paths::socket_path()).await?)
}

async fn connect_or_spawn(backend: &Option<String>) -> Result<Client, Fail> {
    let extra: Vec<String> = backend.as_ref().map(|b| vec!["--backend".into(), b.clone()]).unwrap_or_default();
    Ok(Client::connect_or_spawn(&paths::socket_path(), &extra).await?)
}

async fn simple(c: Command) -> Result<(), Fail> {
    connect().await?.command(c).await?;
    Ok(())
}

async fn exec(cmd: Cmd, backend: Option<String>) -> Result<(), Fail> {
    match cmd {
        Cmd::Daemon => unreachable!("handled in main"),
        Cmd::Play { query, enqueue, next } => {
            if query.is_empty() {
                return simple(Command::Play).await;
            }
            let client = connect_or_spawn(&backend).await?;
            let v = client.call(method::PLAY_QUERY, json!({ "query": query.join(" "), "enqueue": enqueue, "next": next })).await?;
            let t: Track = serde_json::from_value(v["track"].clone()).map_err(|e| Fail(EXIT_ERR, e.to_string()))?;
            let verb = if enqueue || next { "queued" } else { "playing" };
            say!("{verb}: {} — {}", t.title, t.artist_line());
            Ok(())
        }
        Cmd::Pause => simple(Command::Pause).await,
        Cmd::Resume => simple(Command::Play).await,
        Cmd::Toggle => simple(Command::Toggle).await,
        Cmd::Stop => simple(Command::Stop).await,
        Cmd::Next => simple(Command::Next).await,
        Cmd::Prev => simple(Command::Prev).await,
        Cmd::Seek { position } => {
            let (seconds, relative) = parse_seek(&position).ok_or_else(|| Fail(EXIT_USAGE, format!("bad seek position: {position}")))?;
            simple(Command::Seek { seconds, relative }).await
        }
        Cmd::Volume { level } => {
            let relative = level.starts_with('+') || level.starts_with('-');
            let value: i16 = level.trim_start_matches('+').parse().map_err(|_| Fail(EXIT_USAGE, format!("bad volume: {level}")))?;
            simple(Command::Volume { value, relative }).await
        }
        Cmd::Shuffle { mode } => {
            let on = match mode.as_deref() {
                None | Some("toggle") => None,
                Some("on") => Some(true),
                Some("off") => Some(false),
                Some(m) => return Err(Fail(EXIT_USAGE, format!("shuffle takes on | off | toggle, not {m}"))),
            };
            let s = connect().await?.command(Command::SetShuffle { on }).await?;
            say!("shuffle {}", if s.shuffle { "on" } else { "off" });
            Ok(())
        }
        Cmd::Repeat => {
            let s = connect().await?.command(Command::CycleRepeat).await?;
            say!("repeat {}", serde_json::to_value(s.repeat).unwrap_or_default().as_str().unwrap_or("?"));
            Ok(())
        }
        Cmd::Queue { action } => match action.unwrap_or(QueueCmd::Ls { json: false }) {
            QueueCmd::Ls { json } => {
                let s = connect().await?.status().await?;
                if json {
                    say!("{}", serde_json::to_string_pretty(&s.queue).unwrap_or_default());
                } else if s.queue.items.is_empty() {
                    say!("queue is empty");
                } else {
                    for (i, t) in s.queue.items.iter().enumerate() {
                        let mark = if Some(i) == s.queue.current { "▶" } else { " " };
                        say!("{mark} {:>3}  {} — {}  {}", i + 1, t.title, t.artist_line(), t.duration_s.map(fmt_secs).unwrap_or_default());
                    }
                }
                Ok(())
            }
            QueueCmd::Add { query, next } => {
                if query.is_empty() {
                    return Err(Fail(EXIT_USAGE, "queue add needs a search query".into()));
                }
                let client = connect_or_spawn(&backend).await?;
                let v = client.call(method::PLAY_QUERY, json!({ "query": query.join(" "), "enqueue": true, "next": next })).await?;
                say!("queued: {}", v["track"]["title"].as_str().unwrap_or("?"));
                Ok(())
            }
            QueueCmd::Rm { position } => {
                if position == 0 {
                    return Err(Fail(EXIT_USAGE, "positions start at 1".into()));
                }
                simple(Command::Remove { index: position - 1 }).await
            }
            QueueCmd::Clear => simple(Command::Clear).await,
        },
        Cmd::Status { json, format } => {
            let s = connect().await?.status().await?;
            if json {
                say!("{}", serde_json::to_string_pretty(&status_json(&s)).unwrap_or_default());
            } else if let Some(fmt) = format {
                say!("{}", format_status(&fmt, &s));
            } else {
                match s.current_track() {
                    Some(t) => say!(
                        "{} — {} [{}/{}] ({})",
                        t.title,
                        t.artist_line(),
                        fmt_secs(s.position.as_secs()),
                        s.duration.map(|d| fmt_secs(d.as_secs())).unwrap_or_else(|| "?".into()),
                        serde_json::to_value(s.status).unwrap_or_default().as_str().unwrap_or("?")
                    ),
                    None => say!("stopped · nothing in the queue"),
                }
            }
            Ok(())
        }
        Cmd::Search { query, limit, json } => {
            let client = connect_or_spawn(&backend).await?;
            let v = client.call(method::SEARCH, json!({ "query": query.join(" "), "limit": limit, "kind": "songs" })).await?;
            let tracks: Vec<Track> = serde_json::from_value(v).map_err(|e| Fail(EXIT_ERR, e.to_string()))?;
            if json {
                say!("{}", serde_json::to_string_pretty(&tracks).unwrap_or_default());
            } else if tracks.is_empty() {
                return Err(Fail(EXIT_NOT_FOUND, "no results".into()));
            } else {
                for t in tracks {
                    say!("{}  {} — {}  {}", t.video_id, t.title, t.artist_line(), t.duration_s.map(fmt_secs).unwrap_or_default());
                }
            }
            Ok(())
        }
        Cmd::Lyrics { synced } => {
            let v = connect().await?.call(method::LYRICS, Value::Null).await?;
            let u: Option<LyricsUpdate> = serde_json::from_value(v).map_err(|e| Fail(EXIT_ERR, e.to_string()))?;
            let Some(l) = u.and_then(|u| u.lyrics) else {
                return Err(Fail(EXIT_NOT_FOUND, "no lyrics for the current track".into()));
            };
            for line in &l.lines {
                if synced && l.synced {
                    say!("[{:02}:{:02}.{:02}]{}", line.at_ms / 60_000, (line.at_ms / 1000) % 60, (line.at_ms % 1000) / 10, line.text);
                } else {
                    say!("{}", line.text);
                }
            }
            Ok(())
        }
        Cmd::Auth { action } => auth(action),
        Cmd::Doctor => {
            doctor().await;
            Ok(())
        }
        Cmd::Quit => {
            connect().await?.call(method::SHUTDOWN, json!({})).await?;
            say!("daemon stopped");
            Ok(())
        }
    }
}

fn auth(action: AuthCmd) -> Result<(), Fail> {
    let path = paths::auth_file();
    match action {
        AuthCmd::Import { file } => {
            let input = match file {
                Some(f) => std::fs::read_to_string(&f).map_err(|e| Fail(EXIT_ERR, format!("{}: {e}", f.display())))?,
                None => {
                    eprintln!("Paste a signed-in music.youtube.com request (Copy as cURL, or raw request headers), then press Ctrl-D:");
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s).map_err(|e| Fail(EXIT_ERR, e.to_string()))?;
                    s
                }
            };
            let creds = Credentials::parse(&input).map_err(|e| Fail(EXIT_AUTH, e.to_string()))?;
            creds.save(&path).map_err(|e| Fail(EXIT_ERR, e.to_string()))?;
            say!("saved login to {} (mode 0600)", path.display());
            say!("restart the daemon to use it: ytm-tui quit && ytm-tui");
            Ok(())
        }
        AuthCmd::Status => {
            match Credentials::load(&path) {
                Ok(Some(c)) => say!("signed in (account index {}) · {}", c.auth_user.as_deref().unwrap_or("0"), path.display()),
                Ok(None) => say!("anonymous · run `ytm-tui auth import` to sign in"),
                Err(e) => return Err(Fail(EXIT_ERR, format!("{}: {e}", path.display()))),
            }
            Ok(())
        }
        AuthCmd::Logout => {
            match std::fs::remove_file(&path) {
                Ok(()) => say!("removed {}", path.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => say!("no stored login"),
                Err(e) => return Err(Fail(EXIT_ERR, e.to_string())),
            }
            let _ = std::fs::remove_file(paths::cookies_txt());
            Ok(())
        }
    }
}

async fn doctor() {
    async fn version(bin: &str, arg: &str) -> Option<String> {
        let out = tokio::process::Command::new(bin).arg(arg).output().await.ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        s.lines().next().map(|l| l.trim().to_string())
    }
    let check = |ok: bool| if ok { "✓" } else { "✗" };
    for (bin, arg, why) in [
        ("mpv", "--version", "audio playback (default backend)"),
        ("yt-dlp", "--version", "stream URLs"),
        ("deno", "--version", "yt-dlp's YouTube JS challenges"),
        ("ffmpeg", "-version", "native backend (roadmap)"),
    ] {
        let v = version(bin, arg).await;
        say!("{} {bin:<7} {}  — {why}", check(v.is_some()), v.unwrap_or_else(|| "not found".into()));
    }
    let auth = Credentials::load(&paths::auth_file()).ok().flatten().is_some();
    say!("{} auth    {}", check(auth), if auth { "signed in" } else { "anonymous (ytm-tui auth import)" });
    let daemon = Client::connect(&paths::socket_path()).await.is_ok();
    say!("{} daemon  {} ({})", check(daemon), if daemon { "running" } else { "not running" }, paths::socket_path().display());
    let ct = std::env::var("COLORTERM").unwrap_or_default();
    let term = std::env::var("TERM").unwrap_or_default();
    let tier = if matches!(ct.as_str(), "truecolor" | "24bit") {
        "truecolor"
    } else if term.contains("256color") {
        "256 colors"
    } else {
        "16 colors"
    };
    say!("✓ terminal {tier} (TERM={term}, COLORTERM={ct})");
    say!("  config  {}", paths::config_dir().display());
    say!("  logs    {}", paths::log_file_dir().display());
}

fn parse_seek(s: &str) -> Option<(f64, bool)> {
    let (sign, rest) = match s.chars().next()? {
        '+' => (1.0, &s[1..]),
        '-' => (-1.0, &s[1..]),
        _ => (0.0, s),
    };
    let secs = if rest.contains(':') { ytm_api::models::parse_duration(rest)? as f64 } else { rest.parse::<f64>().ok()? };
    Some(if sign == 0.0 { (secs, false) } else { (sign * secs, true) })
}

fn fmt_secs(s: u64) -> String {
    format!("{}:{:02}", s / 60, s % 60)
}

fn status_json(s: &PlayerState) -> Value {
    let t = s.current_track();
    json!({
        "state": s.status,
        "track": t.map(|t| json!({
            "video_id": t.video_id, "title": t.title, "artists": t.artists, "album": t.album,
            "duration_s": t.duration_s, "url": t.url(),
        })),
        "position_s": (s.position.as_secs_f64() * 10.0).round() / 10.0,
        "volume": s.volume,
        "shuffle": s.shuffle,
        "repeat": s.repeat,
        "queue": { "index": s.queue.current, "length": s.queue.items.len() },
        "stream": s.stream.as_ref().map(|st| json!({ "codec": st.codec, "bitrate_kbps": st.bitrate_kbps, "sample_rate": st.sample_rate })),
        "backend": s.backend,
        "authenticated": s.authenticated,
    })
}

fn format_status(fmt: &str, s: &PlayerState) -> String {
    let t = s.current_track();
    fmt.replace("{title}", t.map(|t| t.title.as_str()).unwrap_or(""))
        .replace("{artist}", &t.map(|t| t.artist_line()).unwrap_or_default())
        .replace("{album}", t.and_then(|t| t.album.as_deref()).unwrap_or(""))
        .replace("{elapsed}", &fmt_secs(s.position.as_secs()))
        .replace("{duration}", &s.duration.map(|d| fmt_secs(d.as_secs())).unwrap_or_default())
        .replace("{state}", serde_json::to_value(s.status).unwrap_or_default().as_str().unwrap_or(""))
        .replace("{volume}", &s.volume.to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_seek;

    #[test]
    fn seek_specs() {
        assert_eq!(parse_seek("90"), Some((90.0, false)));
        assert_eq!(parse_seek("1:30"), Some((90.0, false)));
        assert_eq!(parse_seek("+30"), Some((30.0, true)));
        assert_eq!(parse_seek("-10"), Some((-10.0, true)));
        assert_eq!(parse_seek("x"), None);
    }
}
