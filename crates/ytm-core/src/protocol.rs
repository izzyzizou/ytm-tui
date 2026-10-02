//! Wire protocol between the daemon and its clients (TUI, CLI, scripts).
//!
//! Newline-delimited JSON over a Unix socket. Requests carry an `id`; responses echo it.
//! After `subscribe`, the server also pushes events (`{"event": "...", "data": ...}`).
//!
//! ```text
//! → {"id":1,"method":"player.command","params":{"cmd":"next"}}
//! ← {"id":1,"result":{...PlayerState...}}
//! ← {"event":"toast","data":{"level":"info","text":"added 1 track"}}
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::lrc::Lyrics;
use crate::reducer::Toast;
use crate::state::PlayerState;

pub mod method {
    pub const PING: &str = "daemon.ping";
    pub const SHUTDOWN: &str = "daemon.shutdown";
    pub const SUBSCRIBE: &str = "subscribe";
    pub const STATUS: &str = "player.status";
    /// params: a `reducer::Command`
    pub const COMMAND: &str = "player.command";
    /// params: `{"query": "...", "enqueue": false, "next": false}`
    pub const PLAY_QUERY: &str = "player.play_query";
    /// params: `{"query": "...", "limit": 20, "kind": "songs"|"all"}`
    pub const SEARCH: &str = "api.search";
    pub const LYRICS: &str = "lyrics.get";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

impl RpcError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricsUpdate {
    pub video_id: String,
    pub lyrics: Option<Lyrics>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum Event {
    State(Box<PlayerState>),
    Toast(Toast),
    Lyrics(LyricsUpdate),
}

/// Anything the server can send.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ServerMessage {
    Response(Response),
    Event(Event),
}
