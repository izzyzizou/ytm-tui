//! Client side of the daemon socket, shared by the TUI and the CLI.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::protocol::{method, Event, Request, Response, RpcError, ServerMessage};
use crate::reducer::Command;
use crate::state::PlayerState;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("daemon not running")]
    NotRunning,
    #[error("connection to daemon closed")]
    Closed,
    #[error("{}", .0.message)]
    Remote(RpcError),
    #[error("bad response: {0}")]
    Decode(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>>;

pub struct Client {
    tx: mpsc::UnboundedSender<String>,
    pending: Pending,
    next_id: AtomicU64,
    events: Option<mpsc::UnboundedReceiver<Event>>,
}

impl Client {
    #[cfg(unix)]
    pub async fn connect(path: &Path) -> Result<Self, ClientError> {
        let stream = tokio::net::UnixStream::connect(path).await.map_err(|_| ClientError::NotRunning)?;
        let (rd, mut wr) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                if wr.write_all(line.as_bytes()).await.is_err() || wr.write_all(b"\n").await.is_err() {
                    break;
                }
            }
        });
        let pending: Pending = Arc::default();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let p = pending.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(rd).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match serde_json::from_str::<ServerMessage>(&line) {
                    Ok(ServerMessage::Response(r)) => {
                        if let Some(waiter) = p.lock().expect("pending lock").remove(&r.id) {
                            let _ = waiter.send(r);
                        }
                    }
                    Ok(ServerMessage::Event(e)) => {
                        let _ = ev_tx.send(e);
                    }
                    Err(e) => tracing::warn!("undecodable daemon message: {e}"),
                }
            }
            p.lock().expect("pending lock").clear(); // drops waiters → Closed
        });
        Ok(Self { tx, pending, next_id: AtomicU64::new(1), events: Some(ev_rx) })
    }

    #[cfg(not(unix))]
    pub async fn connect(_path: &Path) -> Result<Self, ClientError> {
        Err(ClientError::NotRunning) // TODO: named pipes on Windows
    }

    /// Connect, starting a detached `ytm-tui daemon` first if none is listening.
    pub async fn connect_or_spawn(path: &Path, extra_args: &[String]) -> Result<Self, ClientError> {
        if let Ok(c) = Self::connect(path).await {
            return Ok(c);
        }
        spawn_daemon(extra_args)?;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(c) = Self::connect(path).await {
                return Ok(c);
            }
        }
        Err(ClientError::NotRunning)
    }

    /// Server-pushed events (after [`Client::subscribe`]). Can be taken once.
    pub fn take_events(&mut self) -> Option<mpsc::UnboundedReceiver<Event>> {
        self.events.take()
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(id, tx);
        let req = Request { id, method: method.into(), params };
        self.tx.send(serde_json::to_string(&req).expect("request json")).map_err(|_| ClientError::Closed)?;
        let resp = rx.await.map_err(|_| ClientError::Closed)?;
        match (resp.result, resp.error) {
            (_, Some(e)) => Err(ClientError::Remote(e)),
            (Some(v), None) => Ok(v),
            (None, None) => Ok(Value::Null),
        }
    }

    pub async fn command(&self, cmd: Command) -> Result<PlayerState, ClientError> {
        let v = self.call(method::COMMAND, serde_json::to_value(cmd).expect("command json")).await?;
        serde_json::from_value(v).map_err(|e| ClientError::Decode(e.to_string()))
    }

    pub async fn status(&self) -> Result<PlayerState, ClientError> {
        let v = self.call(method::STATUS, Value::Null).await?;
        serde_json::from_value(v).map_err(|e| ClientError::Decode(e.to_string()))
    }

    pub async fn subscribe(&self) -> Result<(), ClientError> {
        self.call(method::SUBSCRIBE, json!({})).await.map(|_| ())
    }
}

fn spawn_daemon(extra_args: &[String]) -> Result<(), ClientError> {
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon")
        .args(extra_args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0); // survive the terminal closing
    }
    cmd.spawn()?;
    Ok(())
}
