//! mpv backend: spawn `mpv --idle` and drive it over its JSON IPC socket.
//!
//! mpv handles HTTPS streaming, Opus/AAC decoding, demuxer read-ahead and seeking in
//! remote streams. Running it as a separate process means an mpv crash can't take the UI
//! down: we report [`PlayerEvent::Exited`] and the daemon restarts it.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::{AudioBackend, Media, PlayerError, PlayerEvent};

const OBS_TIME: u64 = 1;
const OBS_PAUSE: u64 = 2;
const OBS_CACHE: u64 = 3;
const OBS_DURATION: u64 = 4;

pub struct MpvBackend {
    child: Child,
    tx: mpsc::UnboundedSender<String>,
    socket: PathBuf,
    request_id: Arc<AtomicU64>,
}

impl MpvBackend {
    /// Spawn mpv and connect to its IPC socket. Fails with `Unavailable` if mpv isn't installed.
    #[cfg(unix)]
    pub async fn spawn(events: mpsc::UnboundedSender<PlayerEvent>, volume: u8) -> Result<Self, PlayerError> {
        let socket = private_socket_dir()?.join("mpv.sock");
        let child = Command::new("mpv")
            .args([
                "--idle=yes",
                "--no-video",
                "--no-terminal",
                "--audio-display=no",
                "--keep-open=no",
                "--cache=yes",
                "--demuxer-max-bytes=64MiB",
                "--demuxer-readahead-secs=60",
                "--prefetch-playlist=yes",
                &format!("--volume={volume}"),
                &format!("--input-ipc-server={}", socket.display()),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| PlayerError::Unavailable(format!("could not start mpv ({e}); install it or use --backend null")))?;

        // mpv creates the socket shortly after start.
        let mut stream = None;
        for _ in 0..50 {
            if let Ok(s) = tokio::net::UnixStream::connect(&socket).await {
                stream = Some(s);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let stream = stream.ok_or_else(|| PlayerError::Unavailable("mpv IPC socket never appeared".into()))?;
        let (rd, mut wr) = stream.into_split();

        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                if wr.write_all(line.as_bytes()).await.is_err() || wr.write_all(b"\n").await.is_err() {
                    break;
                }
            }
        });

        tokio::spawn(async move {
            let mut lines = BufReader::new(rd).lines();
            let mut buffering = false;
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                for ev in translate(&msg, &mut buffering) {
                    if events.send(ev).is_err() {
                        return;
                    }
                }
            }
            let _ = events.send(PlayerEvent::Exited);
        });

        let me = Self { child, tx, socket, request_id: Arc::new(AtomicU64::new(100)) };
        for (id, prop) in [(OBS_TIME, "time-pos"), (OBS_PAUSE, "pause"), (OBS_CACHE, "paused-for-cache"), (OBS_DURATION, "duration")] {
            me.send(json!(["observe_property", id, prop]))?;
        }
        Ok(me)
    }

    #[cfg(not(unix))]
    pub async fn spawn(_events: mpsc::UnboundedSender<PlayerEvent>, _volume: u8) -> Result<Self, PlayerError> {
        Err(PlayerError::Unavailable("mpv backend needs a named-pipe transport on Windows (TODO)".into()))
    }

    fn send(&self, command: Value) -> Result<(), PlayerError> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);
        let line = json!({ "command": command, "request_id": id }).to_string();
        self.tx.send(line).map_err(|_| PlayerError::Protocol("mpv connection closed".into()))
    }

    fn set(&self, prop: &str, value: Value) -> Result<(), PlayerError> {
        self.send(json!(["set_property", prop, value]))
    }
}

impl Drop for MpvBackend {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        let _ = std::fs::remove_file(&self.socket);
        if let Some(dir) = self.socket.parent() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

/// A fresh 0700 directory for mpv's IPC socket. `create` (not `create_dir_all`) fails if the
/// path already exists, so another local user can't pre-create it in a shared `/tmp` and
/// hijack the socket — mpv's IPC can run arbitrary commands.
#[cfg(unix)]
fn private_socket_dir() -> Result<PathBuf, PlayerError> {
    use std::os::unix::fs::DirBuilderExt;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    let dir = std::env::temp_dir().join(format!("ytm-tui-mpv-{}-{nanos:x}", std::process::id()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|e| PlayerError::Unavailable(format!("could not create mpv socket dir {} ({e})", dir.display())))?;
    Ok(dir)
}

#[async_trait::async_trait]
impl AudioBackend for MpvBackend {
    fn name(&self) -> &'static str {
        "mpv"
    }

    async fn load(&mut self, media: Media, start: Duration) -> Result<(), PlayerError> {
        // `start` as a property works on every mpv version (loadfile's option argument moved in 0.38).
        let start = if start.is_zero() { "none".to_string() } else { format!("{:.3}", start.as_secs_f64()) };
        self.set("start", json!(start))?;
        self.send(json!(["loadfile", media.location, "replace"]))?;
        self.set("pause", json!(false))
    }

    async fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError> {
        self.set("pause", json!(paused))
    }

    async fn seek(&mut self, to: Duration) -> Result<(), PlayerError> {
        self.send(json!(["seek", to.as_secs_f64(), "absolute"]))
    }

    async fn set_volume(&mut self, percent: u8) -> Result<(), PlayerError> {
        self.set("volume", json!(percent))
    }

    async fn stop(&mut self) -> Result<(), PlayerError> {
        self.send(json!(["stop"]))
    }
}

/// Map one mpv IPC message to zero or more player events.
fn translate(msg: &Value, buffering: &mut bool) -> Vec<PlayerEvent> {
    let secs = |v: &Value| v.as_f64().filter(|s| s.is_finite() && *s >= 0.0).map(Duration::from_secs_f64);
    match msg.get("event").and_then(Value::as_str) {
        Some("property-change") => {
            let data = msg.get("data").unwrap_or(&Value::Null);
            match msg.get("id").and_then(Value::as_u64) {
                Some(OBS_TIME) => secs(data).map(PlayerEvent::Position).into_iter().collect(),
                Some(OBS_PAUSE) => data.as_bool().map(PlayerEvent::Paused).into_iter().collect(),
                Some(OBS_CACHE) => match data.as_bool() {
                    Some(b) if b != *buffering => {
                        *buffering = b;
                        vec![PlayerEvent::Buffering(b)]
                    }
                    _ => vec![],
                },
                Some(OBS_DURATION) => secs(data).map(|d| PlayerEvent::Started { duration: Some(d) }).into_iter().collect(),
                _ => vec![],
            }
        }
        Some("file-loaded") => vec![PlayerEvent::Started { duration: None }],
        Some("end-file") => match msg.get("reason").and_then(Value::as_str) {
            Some("eof") => vec![PlayerEvent::Ended],
            Some("error") => {
                let why = msg.get("file_error").and_then(Value::as_str).unwrap_or("playback failed");
                vec![PlayerEvent::Error(why.to_string())]
            }
            _ => vec![], // "stop" (replaced by the next loadfile), "quit", "redirect"
        },
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_ipc_messages() {
        let mut buf = false;
        let t = |s: &str, b: &mut bool| translate(&serde_json::from_str(s).unwrap(), b);
        assert_eq!(
            t(r#"{"event":"property-change","id":1,"name":"time-pos","data":12.5}"#, &mut buf),
            vec![PlayerEvent::Position(Duration::from_millis(12500))]
        );
        assert_eq!(t(r#"{"event":"property-change","id":1,"name":"time-pos","data":null}"#, &mut buf), vec![]);
        assert_eq!(t(r#"{"event":"property-change","id":3,"data":true}"#, &mut buf), vec![PlayerEvent::Buffering(true)]);
        assert_eq!(t(r#"{"event":"property-change","id":3,"data":true}"#, &mut buf), vec![], "deduplicated");
        assert_eq!(t(r#"{"event":"end-file","reason":"eof"}"#, &mut buf), vec![PlayerEvent::Ended]);
        assert_eq!(t(r#"{"event":"end-file","reason":"stop"}"#, &mut buf), vec![]);
        assert_eq!(
            t(r#"{"event":"end-file","reason":"error","file_error":"loading failed"}"#, &mut buf),
            vec![PlayerEvent::Error("loading failed".into())]
        );
        assert_eq!(t(r#"{"request_id":5,"error":"success"}"#, &mut buf), vec![]);
    }
}
