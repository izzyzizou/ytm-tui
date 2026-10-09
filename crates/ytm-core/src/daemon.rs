//! The playback daemon: owns the player state, the audio backend and all network access.
//!
//! ```text
//! socket ─► connection tasks ─► CoreMsg ─┐
//! backend ─────────► PlayerEvent ────────┼─► core loop: reduce() → effects → backend / resolver
//! resolver tasks ──► Input::Resolved ────┘            └─► broadcast Event to subscribers
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc, oneshot};
use ytm_api::auth::Credentials;
use ytm_api::{InnerTube, MusicApi, SearchKind};
use ytm_audio::mpv::MpvBackend;
use ytm_audio::null::NullBackend;
use ytm_audio::{AudioBackend, FakeResolver, PlayerEvent, StreamInfo, StreamResolver, YtDlp};

use crate::lyrics::LyricsFetcher;
use crate::paths;
use crate::protocol::{method, Event, LyricsUpdate, Request, Response, RpcError};
use crate::reducer::{reduce, Command, Effect, Input, Level, Toast};
use crate::state::PlayerState;

#[derive(Debug, Clone)]
pub struct DaemonOptions {
    /// "auto", "mpv" or "null".
    pub backend: String,
    pub socket: PathBuf,
    pub volume: u8,
    pub ytdlp_format: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("a daemon is already running on {0}")]
    AlreadyRunning(PathBuf),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

enum CoreMsg {
    Input(Input),
    Command(Command, oneshot::Sender<PlayerState>),
    Snapshot(oneshot::Sender<PlayerState>),
    /// A prefetched stream: cache it, don't feed it to the reducer.
    Prefetched(StreamInfo),
    Shutdown,
}

type LyricsSlot = Arc<Mutex<Option<LyricsUpdate>>>;

#[derive(Clone)]
struct Shared {
    core: mpsc::UnboundedSender<CoreMsg>,
    events: broadcast::Sender<Event>,
    api: Arc<dyn MusicApi>,
    lyrics: LyricsSlot,
}

#[cfg(unix)]
pub async fn run(opts: DaemonOptions) -> Result<(), DaemonError> {
    use tokio::net::{UnixListener, UnixStream};

    if UnixStream::connect(&opts.socket).await.is_ok() {
        return Err(DaemonError::AlreadyRunning(opts.socket));
    }
    let _ = std::fs::remove_file(&opts.socket);
    if let Some(dir) = opts.socket.parent() {
        ytm_api::auth::create_private_dir(dir)?;
    }
    let listener = UnixListener::bind(&opts.socket)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&opts.socket, std::fs::Permissions::from_mode(0o600))?;
    }
    tracing::info!("daemon listening on {}", opts.socket.display());

    let creds = Credentials::load(&paths::auth_file()).unwrap_or_else(|e| {
        tracing::warn!("could not read auth file: {e}");
        None
    });
    let cookies = match &creds {
        Some(c) => {
            let p = paths::cookies_txt();
            if let Some(d) = p.parent() {
                let _ = ytm_api::auth::create_private_dir(d);
            }
            ytm_api::auth::write_private(&p, c.to_netscape_cookies().as_bytes()).ok().map(|_| p)
        }
        None => None,
    };
    let authenticated = creds.is_some();
    let api: Arc<dyn MusicApi> = Arc::new(InnerTube::new(creds).map_err(|e| DaemonError::Other(e.to_string()))?);

    let (core_tx, core_rx) = mpsc::unbounded_channel();
    let (events_tx, _) = broadcast::channel(256);
    let (player_tx, player_rx) = mpsc::unbounded_channel();

    let (backend, warn) = make_backend(&opts.backend, player_tx.clone(), opts.volume).await;
    let resolver: Arc<dyn StreamResolver> = if backend.name() == "null" {
        Arc::new(FakeResolver)
    } else {
        Arc::new(YtDlp { cookies, format: opts.ytdlp_format.clone(), ..YtDlp::default() })
    };

    let shared = Shared { core: core_tx.clone(), events: events_tx.clone(), api, lyrics: Arc::default() };

    let accept_shared = shared.clone();
    let accept = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tokio::spawn(handle_conn(stream, accept_shared.clone()));
                }
                Err(e) => {
                    tracing::error!("accept failed: {e}");
                    break;
                }
            }
        }
    });

    let mut state = PlayerState { volume: opts.volume, backend: backend.name().into(), authenticated, ..PlayerState::default() };
    let mut core = Core {
        backend,
        backend_kind: opts.backend.clone(),
        player_tx,
        resolver,
        stream_cache: HashMap::new(),
        core_tx,
        shared,
        last_broadcast: Instant::now(),
        lyrics_for: None,
        fetcher: Arc::new(LyricsFetcher::default()),
    };
    if let Some(w) = warn {
        core.toast(Level::Warn, w);
    }
    core.run(&mut state, core_rx, player_rx).await;

    accept.abort();
    let _ = std::fs::remove_file(&opts.socket);
    tracing::info!("daemon stopped");
    Ok(())
}

#[cfg(not(unix))]
pub async fn run(_opts: DaemonOptions) -> Result<(), DaemonError> {
    Err(DaemonError::Other("the daemon needs a named-pipe transport on Windows (TODO)".into()))
}

async fn make_backend(kind: &str, tx: mpsc::UnboundedSender<PlayerEvent>, volume: u8) -> (Box<dyn AudioBackend>, Option<String>) {
    match kind {
        "null" => (Box::new(NullBackend::new(tx)), None),
        "mpv" => match MpvBackend::spawn(tx.clone(), volume).await {
            Ok(b) => (Box::new(b), None),
            Err(e) => (Box::new(NullBackend::new(tx)), Some(format!("{e} · using silent backend"))),
        },
        _ => match MpvBackend::spawn(tx.clone(), volume).await {
            Ok(b) => (Box::new(b), None),
            Err(e) => {
                tracing::warn!("mpv unavailable: {e}");
                (Box::new(NullBackend::new(tx)), Some("mpv not found · playing silently (install mpv)".into()))
            }
        },
    }
}

struct Core {
    backend: Box<dyn AudioBackend>,
    backend_kind: String,
    player_tx: mpsc::UnboundedSender<PlayerEvent>,
    resolver: Arc<dyn StreamResolver>,
    stream_cache: HashMap<String, StreamInfo>,
    core_tx: mpsc::UnboundedSender<CoreMsg>,
    shared: Shared,
    last_broadcast: Instant,
    lyrics_for: Option<String>,
    fetcher: Arc<LyricsFetcher>,
}

impl Core {
    async fn run(
        &mut self,
        state: &mut PlayerState,
        mut core_rx: mpsc::UnboundedReceiver<CoreMsg>,
        mut player_rx: mpsc::UnboundedReceiver<PlayerEvent>,
    ) {
        loop {
            let input = tokio::select! {
                msg = core_rx.recv() => match msg {
                    None | Some(CoreMsg::Shutdown) => break,
                    Some(CoreMsg::Input(i)) => i,
                    Some(CoreMsg::Snapshot(reply)) => { let _ = reply.send(state.clone()); continue; }
                    Some(CoreMsg::Prefetched(info)) => { self.stream_cache.insert(info.video_id.clone(), info); continue; }
                    Some(CoreMsg::Command(c, reply)) => {
                        self.apply(state, Input::Cmd(c)).await;
                        let _ = reply.send(state.clone());
                        continue;
                    }
                },
                Some(ev) = player_rx.recv() => Input::Player(ev),
                _ = tokio::signal::ctrl_c() => break,
            };
            self.apply(state, input).await;
        }
        let _ = self.backend.stop().await;
    }

    async fn apply(&mut self, state: &mut PlayerState, input: Input) {
        let position_only = matches!(input, Input::Player(PlayerEvent::Position(_)));
        if let Input::Resolved { result: Ok(info), .. } = &input {
            self.stream_cache.insert(info.video_id.clone(), info.clone());
        }
        let effects = reduce(state, input);
        for fx in effects {
            self.execute(state, fx).await;
        }
        self.maybe_fetch_lyrics(state);
        if !position_only || self.last_broadcast.elapsed() >= Duration::from_millis(250) {
            self.last_broadcast = Instant::now();
            let _ = self.shared.events.send(Event::State(Box::new(state.clone())));
        }
    }

    async fn execute(&mut self, state: &mut PlayerState, fx: Effect) {
        let result = match fx {
            Effect::Resolve { seq, video_id, fresh } => {
                if !fresh {
                    if let Some(info) = self.cached(&video_id) {
                        let _ = self.core_tx.send(CoreMsg::Input(Input::Resolved { seq, result: Ok(info.clone()) }));
                        return;
                    }
                }
                let (resolver, tx) = (self.resolver.clone(), self.core_tx.clone());
                tokio::spawn(async move {
                    let result = resolver.resolve(&video_id).await.map_err(|e| e.to_string());
                    let _ = tx.send(CoreMsg::Input(Input::Resolved { seq, result }));
                });
                Ok(())
            }
            Effect::Prefetch { video_id } => {
                if self.cached(&video_id).is_some() {
                    return;
                }
                let (resolver, tx) = (self.resolver.clone(), self.core_tx.clone());
                tokio::spawn(async move {
                    match resolver.resolve(&video_id).await {
                        Ok(info) => {
                            tracing::debug!("prefetched {video_id}");
                            let _ = tx.send(CoreMsg::Prefetched(info));
                        }
                        Err(e) => tracing::debug!("prefetch {video_id}: {e}"),
                    }
                });
                Ok(())
            }
            Effect::Load { media, start } => self.backend.load(media, start).await,
            Effect::SetPaused(p) => self.backend.set_paused(p).await,
            Effect::Seek(to) => self.backend.seek(to).await,
            Effect::SetVolume(v) => self.backend.set_volume(v).await,
            Effect::Stop => self.backend.stop().await,
            Effect::RestartBackend => {
                tracing::warn!("{} backend exited; restarting", self.backend.name());
                let (b, warn) = make_backend(&self.backend_kind, self.player_tx.clone(), state.volume).await;
                self.backend = b;
                state.backend = self.backend.name().into();
                if let Some(w) = warn {
                    self.toast(Level::Warn, w);
                }
                Ok(())
            }
            Effect::Toast(t) => {
                let _ = self.shared.events.send(Event::Toast(t));
                Ok(())
            }
        };
        if let Err(e) = result {
            tracing::warn!("backend: {e}");
            let _ = self.player_tx.send(PlayerEvent::Error(e.to_string()));
        }
    }

    /// A cached stream that stays valid for at least ten more minutes.
    fn cached(&self, video_id: &str) -> Option<&StreamInfo> {
        self.stream_cache.get(video_id).filter(|i| !i.expires_within(Duration::from_secs(600)))
    }

    fn toast(&self, level: Level, text: String) {
        let _ = self.shared.events.send(Event::Toast(Toast { level, text }));
    }

    /// Start a lyrics lookup whenever the current track changes.
    fn maybe_fetch_lyrics(&mut self, state: &PlayerState) {
        let Some(track) = state.current_track() else { return };
        if self.lyrics_for.as_deref() == Some(&track.video_id) {
            return;
        }
        self.lyrics_for = Some(track.video_id.clone());
        *self.shared.lyrics.lock().expect("lyrics lock") = None;
        let (track, fetcher, slot, events) = (track.clone(), self.fetcher.clone(), self.shared.lyrics.clone(), self.shared.events.clone());
        tokio::spawn(async move {
            let lyrics = fetcher.fetch(&track, &paths::lyrics_dir()).await;
            let update = LyricsUpdate { video_id: track.video_id.clone(), lyrics };
            *slot.lock().expect("lyrics lock") = Some(update.clone());
            let _ = events.send(Event::Lyrics(update));
        });
    }
}

// ───────────────────────────── connections ─────────────────────────────

#[cfg(unix)]
async fn handle_conn(stream: tokio::net::UnixStream, shared: Shared) {
    let (rd, mut wr) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(line) = out_rx.recv().await {
            if wr.write_all(line.as_bytes()).await.is_err() || wr.write_all(b"\n").await.is_err() {
                break;
            }
        }
    });
    let mut lines = BufReader::new(rd).lines();
    let mut forwarder: Option<tokio::task::JoinHandle<()>> = None;
    while let Ok(Some(line)) = lines.next_line().await {
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response { id: 0, result: None, error: Some(RpcError::new("bad_request", e.to_string())) };
                let _ = out_tx.send(serde_json::to_string(&resp).expect("json"));
                continue;
            }
        };
        if req.method == method::SUBSCRIBE && forwarder.is_none() {
            forwarder = Some(subscribe(&shared, out_tx.clone()).await);
        }
        let (shared, out) = (shared.clone(), out_tx.clone());
        tokio::spawn(async move {
            let id = req.id;
            let resp = match dispatch(&shared, req).await {
                Ok(v) => Response { id, result: Some(v), error: None },
                Err(e) => Response { id, result: None, error: Some(e) },
            };
            let _ = out.send(serde_json::to_string(&resp).expect("json"));
        });
    }
    if let Some(f) = forwarder {
        f.abort();
    }
    writer.abort();
}

/// Push the current state + lyrics now, then every broadcast event.
async fn subscribe(shared: &Shared, out: mpsc::UnboundedSender<String>) -> tokio::task::JoinHandle<()> {
    let mut rx = shared.events.subscribe();
    if let Ok(s) = snapshot(shared).await {
        let _ = out.send(serde_json::to_string(&Event::State(Box::new(s))).expect("json"));
    }
    if let Some(l) = shared.lyrics.lock().expect("lyrics lock").clone() {
        let _ = out.send(serde_json::to_string(&Event::Lyrics(l)).expect("json"));
    }
    let shared = shared.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if out.send(serde_json::to_string(&ev).expect("json")).is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Slow client: skip ahead with a fresh snapshot instead of blocking the core.
                    if let Ok(s) = snapshot(&shared).await {
                        let _ = out.send(serde_json::to_string(&Event::State(Box::new(s))).expect("json"));
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

async fn snapshot(shared: &Shared) -> Result<PlayerState, RpcError> {
    let (tx, rx) = oneshot::channel();
    shared.core.send(CoreMsg::Snapshot(tx)).map_err(|_| RpcError::new("shutting_down", "daemon is stopping"))?;
    rx.await.map_err(|_| RpcError::new("shutting_down", "daemon is stopping"))
}

async fn command(shared: &Shared, c: Command) -> Result<PlayerState, RpcError> {
    let (tx, rx) = oneshot::channel();
    shared.core.send(CoreMsg::Command(c, tx)).map_err(|_| RpcError::new("shutting_down", "daemon is stopping"))?;
    rx.await.map_err(|_| RpcError::new("shutting_down", "daemon is stopping"))
}

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("serializable")
}

#[derive(Deserialize)]
struct SearchParams {
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    kind: SearchKind,
}

fn default_limit() -> usize {
    20
}

#[derive(Deserialize)]
struct PlayQueryParams {
    query: String,
    #[serde(default)]
    enqueue: bool,
    #[serde(default)]
    next: bool,
}

async fn dispatch(shared: &Shared, req: Request) -> Result<Value, RpcError> {
    let bad = |e: serde_json::Error| RpcError::new("bad_request", e.to_string());
    let api_err = |e: ytm_api::ApiError| match e {
        ytm_api::ApiError::SignedOut => RpcError::new("auth_required", e.to_string()),
        ytm_api::ApiError::RateLimited { .. } => RpcError::new("rate_limited", e.to_string()),
        _ => RpcError::new("api_error", e.to_string()),
    };
    match req.method.as_str() {
        method::PING => Ok(json!({ "version": env!("CARGO_PKG_VERSION"), "pid": std::process::id() })),
        method::SUBSCRIBE => Ok(json!({ "subscribed": true })),
        method::STATUS => snapshot(shared).await.map(|s| to_value(&s)),
        method::COMMAND => {
            let c: Command = serde_json::from_value(req.params).map_err(bad)?;
            command(shared, c).await.map(|s| to_value(&s))
        }
        method::SEARCH => {
            let p: SearchParams = serde_json::from_value(req.params).map_err(bad)?;
            let tracks = shared.api.search(&p.query, p.kind, p.limit).await.map_err(api_err)?;
            Ok(to_value(&tracks))
        }
        method::PLAY_QUERY => {
            let p: PlayQueryParams = serde_json::from_value(req.params).map_err(bad)?;
            let tracks = shared.api.search(&p.query, SearchKind::Songs, 5).await.map_err(api_err)?;
            let Some(best) = tracks.into_iter().next() else {
                return Err(RpcError::new("not_found", format!("no results for \"{}\"", p.query)));
            };
            let c = if p.enqueue || p.next {
                Command::Enqueue { tracks: vec![best.clone()], next: p.next }
            } else {
                Command::PlayTracks { tracks: vec![best.clone()], start: 0 }
            };
            command(shared, c).await?;
            Ok(json!({ "track": best }))
        }
        method::LYRICS => Ok(to_value(&shared.lyrics.lock().expect("lyrics lock").clone())),
        method::SHUTDOWN => {
            let _ = shared.core.send(CoreMsg::Shutdown);
            Ok(json!({ "stopping": true }))
        }
        other => Err(RpcError::new("unknown_method", format!("unknown method {other}"))),
    }
}
