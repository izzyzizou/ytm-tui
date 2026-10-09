//! `reduce(&mut state, input) -> Vec<Effect>` — all player logic, pure and synchronous.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use ytm_api::{Track, WatchPlaylist};
use ytm_audio::{Media, PlayerEvent, StreamInfo};

use crate::state::{PlayerState, Radio, Repeat, Status};

/// `p` within this much of the start goes to the previous track; later it restarts.
const PREV_RESTART: Duration = Duration::from_secs(3);
/// Max automatic recoveries (re-resolve / backend restart) per track before skipping.
const MAX_ATTEMPTS: u8 = 2;
/// Seek ceiling when the duration is unknown, so a bogus `seek 1e30` can't overflow `Duration`.
const MAX_SEEK_S: f64 = 24.0 * 3600.0;
/// Prefetch the next stream once this fraction of the track has played…
const PREFETCH_FRACTION: f64 = 0.75;
/// …or once this little is left, whichever comes first.
const PREFETCH_REMAINING: Duration = Duration::from_secs(30);
/// While a radio is active, fetch its next page once this few tracks are left after the current one.
const RADIO_LOW_WATER: usize = 3;

/// What a client asks the player to do. Serialized as `{"cmd": "...", ...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Prev,
    Seek {
        seconds: f64,
        #[serde(default)]
        relative: bool,
    },
    Volume {
        value: i16,
        #[serde(default)]
        relative: bool,
    },
    /// Replace the queue and start at `start`.
    PlayTracks {
        tracks: Vec<Track>,
        #[serde(default)]
        start: usize,
    },
    /// Append (or insert after the current track with `next: true`).
    Enqueue {
        tracks: Vec<Track>,
        #[serde(default)]
        next: bool,
    },
    Remove {
        index: usize,
    },
    Move {
        from: usize,
        to: usize,
    },
    /// Remove everything except the current track.
    Clear,
    JumpTo {
        index: usize,
    },
    SetShuffle {
        on: Option<bool>,
    },
    CycleRepeat,
    /// Start radio from `track` (default: the current track), replacing the autoplay section.
    /// A track that isn't already queued leads the new section.
    Radio {
        #[serde(default)]
        track: Option<Track>,
    },
    /// Drop the autoplay section and the radio feeding it.
    ClearAutoplay,
    SetAutoplay {
        on: Option<bool>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Cmd(Command),
    Player(PlayerEvent),
    Resolved {
        seq: u64,
        result: Result<StreamInfo, String>,
    },
    /// A page of radio for `Effect::FetchRadio { seq, .. }`.
    Radio {
        seq: u64,
        result: Result<WatchPlaylist, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Toast {
    pub level: Level,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Resolve a stream URL; `fresh` bypasses the cache (after an expiry error).
    Resolve {
        seq: u64,
        video_id: String,
        fresh: bool,
    },
    /// Warm the stream cache for the upcoming track; nothing is played.
    Prefetch {
        video_id: String,
    },
    /// Fetch a page of radio from `seed`, or the page after `continuation`.
    FetchRadio {
        seq: u64,
        seed: String,
        continuation: Option<String>,
    },
    Load {
        media: Media,
        start: Duration,
    },
    SetPaused(bool),
    Seek(Duration),
    SetVolume(u8),
    Stop,
    RestartBackend,
    Toast(Toast),
}

fn toast(level: Level, text: impl Into<String>) -> Effect {
    Effect::Toast(Toast { level, text: text.into() })
}

pub fn reduce(s: &mut PlayerState, input: Input) -> Vec<Effect> {
    s.queue.settle();
    let fx = match input {
        Input::Cmd(c) => command(s, c),
        Input::Player(e) => player_event(s, e),
        Input::Resolved { seq, result } => resolved(s, seq, result),
        Input::Radio { seq, result } => radio_page(s, seq, result),
    };
    s.queue.settle();
    fx
}

/// Begin loading the current track at `start`.
fn start_current(s: &mut PlayerState, start: Duration, fresh: bool) -> Vec<Effect> {
    let Some(track) = s.current_track().cloned() else {
        return stop(s);
    };
    s.load_seq += 1;
    s.status = Status::Loading;
    s.position = start;
    s.pending_start = start;
    s.duration = track.duration();
    s.stream = None;
    s.prefetched = None;
    let mut fx = vec![Effect::Resolve { seq: s.load_seq, video_id: track.video_id, fresh }];
    fx.extend(maybe_autoplay(s));
    fx
}

fn play_index(s: &mut PlayerState, index: usize) -> Vec<Effect> {
    if index >= s.queue.items.len() {
        return vec![];
    }
    s.queue.current = Some(index);
    s.queue.settle();
    s.attempts = 0;
    s.radio_resume = false;
    start_current(s, Duration::ZERO, false)
}

fn stop(s: &mut PlayerState) -> Vec<Effect> {
    s.load_seq += 1; // invalidate in-flight resolutions
    s.radio_resume = false;
    s.status = Status::Stopped;
    s.position = Duration::ZERO;
    s.stream = None;
    vec![Effect::Stop]
}

/// Move to the next track. `auto` = reached the end naturally (repeat-one applies).
fn advance(s: &mut PlayerState, auto: bool) -> Vec<Effect> {
    let Some(cur) = s.queue.current else { return vec![] };
    if auto && s.repeat == Repeat::One {
        s.attempts = 0;
        return start_current(s, Duration::ZERO, false);
    }
    if cur + 1 < s.queue.items.len() {
        play_index(s, cur + 1)
    } else if s.repeat != Repeat::Off && !s.queue.items.is_empty() {
        play_index(s, 0)
    } else {
        // Out of tracks. With autoplay, wait for radio (normally it's already queued by now).
        let mut fx = stop(s);
        if s.autoplay {
            fx.extend(fetch_radio(s));
            s.radio_resume = s.radio.as_ref().is_some_and(|r| r.loading);
        }
        fx
    }
}

/// Drop the radio and anything it has in flight.
fn reset_radio(s: &mut PlayerState) {
    s.radio = None;
    s.radio_seq += 1;
    s.radio_resume = false;
}

/// Ask for more radio: the next page of the current radio, or a fresh one seeded from the last
/// track in the queue once that has run out. No-op while a page is loading.
fn fetch_radio(s: &mut PlayerState) -> Vec<Effect> {
    if s.radio.as_ref().is_some_and(|r| r.loading) {
        return vec![];
    }
    let (seed, continuation) = match &s.radio {
        Some(r) if r.continuation.is_some() => (r.seed.clone(), r.continuation.clone()),
        _ => match s.queue.items.last() {
            Some(last) => (last.clone(), None),
            None => return vec![],
        },
    };
    s.radio_seq += 1;
    let effect = Effect::FetchRadio { seq: s.radio_seq, seed: seed.video_id.clone(), continuation: continuation.clone() };
    s.radio = Some(Radio { seed, loading: true, continuation });
    vec![effect]
}

/// Keep the autoplay section topped up when a track starts: with autoplay on and repeat off,
/// fetch radio once the queue is on its last track (or a running radio is nearly used up).
fn maybe_autoplay(s: &mut PlayerState) -> Vec<Effect> {
    if !s.autoplay || s.repeat != Repeat::Off {
        return vec![];
    }
    let Some(cur) = s.queue.current else { return vec![] };
    let left = s.queue.items.len().saturating_sub(cur + 1);
    let low = if s.radio.is_some() { left <= RADIO_LOW_WATER } else { left == 0 };
    let id = &s.queue.items[cur].video_id;
    if !low || s.radio_asked_for.as_ref() == Some(id) {
        return vec![];
    }
    s.radio_asked_for = Some(id.clone());
    fetch_radio(s)
}

fn radio_page(s: &mut PlayerState, seq: u64, result: Result<WatchPlaylist, String>) -> Vec<Effect> {
    if seq != s.radio_seq {
        return vec![]; // stale: radio was restarted or cleared
    }
    let resume = std::mem::take(&mut s.radio_resume);
    let Some(radio) = s.radio.as_mut() else { return vec![] };
    radio.loading = false;
    let page = match result {
        Ok(page) => page,
        Err(e) => return vec![toast(Level::Warn, format!("radio: {e}"))],
    };
    radio.continuation = page.continuation;
    let mut fresh: Vec<Track> = Vec::new();
    for t in page.tracks {
        if !s.queue.items.iter().chain(&fresh).any(|q| q.video_id == t.video_id) {
            fresh.push(t);
        }
    }
    if fresh.is_empty() {
        return if resume { vec![toast(Level::Info, "autoplay · no more suggestions")] } else { vec![] };
    }
    let at = s.queue.items.len();
    s.queue.autoplay += fresh.len();
    s.queue.items.extend(fresh);
    if resume && s.status == Status::Stopped {
        play_index(s, at)
    } else {
        vec![]
    }
}

/// The track `advance(s, true)` would play next, if it differs from the current one.
fn upcoming(s: &PlayerState) -> Option<&Track> {
    let cur = s.queue.current?;
    let next = match s.repeat {
        Repeat::One => return None,
        _ if cur + 1 < s.queue.items.len() => cur + 1,
        Repeat::All => 0,
        Repeat::Off => return None,
    };
    (next != cur).then(|| &s.queue.items[next])
}

/// Prefetch the upcoming track once, when the current one is near its end.
fn maybe_prefetch(s: &mut PlayerState) -> Vec<Effect> {
    let Some(d) = s.duration else { return vec![] };
    let near_end = s.position.as_secs_f64() >= d.as_secs_f64() * PREFETCH_FRACTION || s.position + PREFETCH_REMAINING >= d;
    if !near_end {
        return vec![];
    }
    let Some(id) = upcoming(s).map(|t| t.video_id.clone()) else { return vec![] };
    if s.prefetched.as_ref() == Some(&id) {
        return vec![];
    }
    s.prefetched = Some(id.clone());
    vec![Effect::Prefetch { video_id: id }]
}

fn command(s: &mut PlayerState, c: Command) -> Vec<Effect> {
    match c {
        Command::Play => match s.status {
            Status::Paused => {
                s.status = Status::Playing;
                vec![Effect::SetPaused(false)]
            }
            Status::Stopped => play_index(s, s.queue.current.unwrap_or(0)),
            _ => vec![],
        },
        Command::Pause => match s.status {
            Status::Playing | Status::Buffering => {
                s.status = Status::Paused;
                vec![Effect::SetPaused(true)]
            }
            _ => vec![],
        },
        Command::Toggle => {
            let next = if matches!(s.status, Status::Playing | Status::Buffering) { Command::Pause } else { Command::Play };
            command(s, next)
        }
        Command::Stop => stop(s),
        Command::Next => advance(s, false),
        Command::Prev => match s.queue.current {
            Some(cur) if cur > 0 && s.position < PREV_RESTART => play_index(s, cur - 1),
            Some(_) if s.status != Status::Stopped => command(s, Command::Seek { seconds: 0.0, relative: false }),
            Some(cur) => play_index(s, cur),
            None => vec![],
        },
        Command::Seek { seconds, relative } => {
            if s.current_track().is_none() || s.status == Status::Stopped {
                return vec![];
            }
            let base = if relative { s.position.as_secs_f64() } else { 0.0 };
            let max = s.duration.map_or(MAX_SEEK_S, |d| (d.as_secs_f64() - 0.5).max(0.0));
            let target = (base + seconds).max(0.0).min(max); // max() also maps NaN to 0
            s.position = Duration::from_secs_f64(target);
            if s.status == Status::Loading {
                s.pending_start = s.position;
                return vec![];
            }
            vec![Effect::Seek(s.position)]
        }
        Command::Volume { value, relative } => {
            let v = if relative { s.volume as i16 + value } else { value };
            s.volume = v.clamp(0, 100) as u8;
            vec![Effect::SetVolume(s.volume)]
        }
        Command::PlayTracks { tracks, start } => {
            if tracks.is_empty() {
                return vec![toast(Level::Warn, "nothing to play")];
            }
            let start = start.min(tracks.len() - 1);
            reset_radio(s);
            s.queue.items = tracks;
            s.queue.current = Some(start);
            s.queue.autoplay = 0;
            s.queue.unshuffled = None;
            if s.shuffle {
                s.queue.shuffle_upcoming();
            }
            s.attempts = 0;
            start_current(s, Duration::ZERO, false)
        }
        Command::Enqueue { tracks, next } => {
            let n = tracks.len();
            if n == 0 {
                return vec![];
            }
            let was_idle = s.status == Status::Stopped && !s.queue.has_next();
            // User picks go above the autoplay section.
            let at = match (next, s.queue.current) {
                (true, Some(c)) => c + 1,
                _ => s.queue.autoplay_start(),
            };
            s.queue.items.splice(at..at, tracks);
            let msg = if n == 1 { "added 1 track".to_string() } else { format!("added {n} tracks") };
            let mut fx = vec![toast(Level::Info, msg)];
            if was_idle {
                fx.extend(play_index(s, at));
            }
            fx
        }
        Command::Remove { index } => {
            if index >= s.queue.items.len() {
                return vec![];
            }
            if s.queue.is_autoplay(index) {
                s.queue.autoplay -= 1;
            }
            s.queue.items.remove(index);
            match s.queue.current {
                Some(c) if index < c => s.queue.current = Some(c - 1),
                Some(c) if index == c => {
                    if c < s.queue.items.len() {
                        return if s.status == Status::Stopped { vec![] } else { play_index(s, c) };
                    }
                    s.queue.current = s.queue.items.len().checked_sub(1);
                    return stop(s);
                }
                _ => {}
            }
            vec![]
        }
        Command::Move { from, to } => {
            let len = s.queue.items.len();
            let boundary = s.queue.autoplay_start();
            // A suggestion moved above the divider becomes the user's pick; the user's own
            // tracks stay above it.
            let to = if from < boundary { to.min(boundary.saturating_sub(1)) } else { to };
            if from >= len || to >= len || from == to {
                return vec![];
            }
            if from >= boundary && to < boundary {
                s.queue.autoplay -= 1;
            }
            let item = s.queue.items.remove(from);
            s.queue.items.insert(to, item);
            if let Some(c) = s.queue.current {
                s.queue.current = Some(if c == from {
                    to
                } else if from < c && to >= c {
                    c - 1
                } else if from > c && to <= c {
                    c + 1
                } else {
                    c
                });
            }
            vec![]
        }
        Command::Clear => {
            match s.queue.current {
                Some(c) => {
                    let keep = s.queue.items.swap_remove(c);
                    s.queue.items = vec![keep];
                    s.queue.current = Some(0);
                }
                None => s.queue.items.clear(),
            }
            s.queue.autoplay = 0;
            s.queue.unshuffled = None;
            vec![toast(Level::Info, "queue cleared")]
        }
        Command::JumpTo { index } => play_index(s, index),
        Command::SetShuffle { on } => {
            let on = on.unwrap_or(!s.shuffle);
            if on != s.shuffle {
                s.shuffle = on;
                if on {
                    s.queue.shuffle_upcoming();
                } else {
                    s.queue.unshuffle_upcoming();
                }
            }
            vec![]
        }
        Command::CycleRepeat => {
            s.repeat = s.repeat.cycle();
            vec![]
        }
        Command::Radio { track } => {
            let Some(seed) = track.or_else(|| s.current_track().cloned()) else {
                return vec![toast(Level::Warn, "nothing to start radio from")];
            };
            s.queue.clear_autoplay();
            reset_radio(s);
            let mut fx = vec![toast(Level::Info, format!("radio from {}", seed.title))];
            let queued = s.queue.items.iter().any(|t| t.video_id == seed.video_id);
            let was_idle = s.status == Status::Stopped && !s.queue.has_next();
            if !queued {
                s.queue.items.push(seed.clone());
                s.queue.autoplay += 1;
            }
            s.radio_seq += 1;
            fx.push(Effect::FetchRadio { seq: s.radio_seq, seed: seed.video_id.clone(), continuation: None });
            s.radio = Some(Radio { seed, loading: true, continuation: None });
            if !queued && was_idle {
                fx.extend(play_index(s, s.queue.items.len() - 1));
            }
            fx
        }
        Command::ClearAutoplay => {
            let n = s.queue.clear_autoplay();
            reset_radio(s);
            let msg = if n == 1 { "removed 1 suggestion".to_string() } else { format!("removed {n} suggestions") };
            vec![toast(Level::Info, msg)]
        }
        Command::SetAutoplay { on } => {
            let on = on.unwrap_or(!s.autoplay);
            if on == s.autoplay {
                return vec![];
            }
            s.autoplay = on;
            let mut fx = vec![toast(Level::Info, if on { "autoplay on" } else { "autoplay off" })];
            if on {
                s.radio_asked_for = None;
                fx.extend(maybe_autoplay(s));
            } else if let Some(r) = s.radio.as_mut().filter(|r| r.loading) {
                r.loading = false;
                s.radio_seq += 1; // drop the page in flight
                s.radio_resume = false;
            }
            fx
        }
    }
}

fn resolved(s: &mut PlayerState, seq: u64, result: Result<StreamInfo, String>) -> Vec<Effect> {
    if seq != s.load_seq || s.status != Status::Loading {
        return vec![]; // stale: the user moved on
    }
    match result {
        Ok(info) => {
            let hint = s.duration.or(info.duration_s.and_then(|d| Duration::try_from_secs_f64(d).ok()));
            if s.duration.is_none() {
                s.duration = hint;
            }
            let media = Media { location: info.url.clone(), duration_hint: hint };
            s.stream = Some(info);
            vec![Effect::Load { media, start: s.pending_start }]
        }
        Err(e) => {
            let title = s.current_track().map(|t| t.title.clone()).unwrap_or_default();
            let mut fx = vec![toast(Level::Error, format!("can't play {title}: {e} · skipped"))];
            if s.queue.has_next() || (s.autoplay && s.repeat == Repeat::Off) {
                fx.extend(advance(s, false));
            } else {
                fx.extend(stop(s));
            }
            fx
        }
    }
}

fn player_event(s: &mut PlayerState, e: PlayerEvent) -> Vec<Effect> {
    match e {
        PlayerEvent::Started { duration } => {
            if duration.is_some() {
                s.duration = duration;
            }
            if s.status == Status::Loading {
                s.status = Status::Playing;
            }
            vec![]
        }
        PlayerEvent::Position(p) => {
            if s.status != Status::Loading && s.status != Status::Stopped {
                s.position = p;
                if s.attempts > 0 && p > s.pending_start + Duration::from_secs(5) {
                    s.attempts = 0; // healthy again
                }
                if s.status == Status::Playing {
                    return maybe_prefetch(s);
                }
            }
            vec![]
        }
        PlayerEvent::Paused(p) => {
            match (p, s.status) {
                (true, Status::Playing | Status::Buffering) => s.status = Status::Paused,
                (false, Status::Paused) => s.status = Status::Playing,
                _ => {}
            }
            vec![]
        }
        PlayerEvent::Buffering(b) => {
            match (b, s.status) {
                (true, Status::Playing) => s.status = Status::Buffering,
                (false, Status::Buffering) => s.status = Status::Playing,
                _ => {}
            }
            vec![]
        }
        PlayerEvent::Ended => {
            if matches!(s.status, Status::Loading | Status::Stopped) {
                return vec![];
            }
            advance(s, true)
        }
        PlayerEvent::Error(why) => {
            if s.status == Status::Stopped {
                return vec![];
            }
            if s.attempts < MAX_ATTEMPTS {
                // Most mid-stream failures are expired/blocked URLs: re-resolve and resume.
                s.attempts += 1;
                let at = s.position;
                let mut fx = vec![toast(Level::Warn, "stream expired · refreshing…")];
                fx.extend(start_current(s, at, true));
                fx
            } else {
                let mut fx = vec![toast(Level::Error, format!("can't play: {why} · skipped"))];
                fx.extend(advance(s, false));
                fx
            }
        }
        PlayerEvent::Exited => {
            let mut fx = vec![toast(Level::Warn, "player restarted"), Effect::RestartBackend];
            if s.is_active() && s.attempts < MAX_ATTEMPTS {
                s.attempts += 1;
                let at = s.position;
                fx.extend(start_current(s, at, false));
            } else if s.status != Status::Stopped {
                fx.extend(stop(s));
            }
            fx
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ytm_api::models::ItemKind;

    fn track(id: &str) -> Track {
        Track {
            video_id: id.into(),
            title: format!("T{id}"),
            artists: vec!["A".into()],
            album: None,
            duration_s: Some(200),
            thumbnail: None,
            kind: ItemKind::Song,
        }
    }

    fn info(id: &str) -> StreamInfo {
        StreamInfo {
            video_id: id.into(),
            url: format!("https://x/{id}"),
            codec: "opus".into(),
            bitrate_kbps: Some(160),
            sample_rate: Some(48000),
            format_id: None,
            duration_s: Some(200.0),
            expires_at: None,
        }
    }

    fn cmd(s: &mut PlayerState, c: Command) -> Vec<Effect> {
        reduce(s, Input::Cmd(c))
    }

    /// Feed the resolver result for whatever is loading, then the backend's Started.
    fn finish_load(s: &mut PlayerState) {
        let id = s.current_track().unwrap().video_id.clone();
        let fx = reduce(s, Input::Resolved { seq: s.load_seq, result: Ok(info(&id)) });
        assert!(matches!(fx.as_slice(), [Effect::Load { .. }]), "{fx:?}");
        reduce(s, Input::Player(PlayerEvent::Started { duration: None }));
        assert_eq!(s.status, Status::Playing);
    }

    fn playing(ids: &[&str]) -> PlayerState {
        let mut s = PlayerState::default();
        cmd(&mut s, Command::PlayTracks { tracks: ids.iter().map(|i| track(i)).collect(), start: 0 });
        finish_load(&mut s);
        s
    }

    #[test]
    fn play_resolve_load_flow() {
        let mut s = PlayerState::default();
        let fx = cmd(&mut s, Command::PlayTracks { tracks: vec![track("a"), track("b")], start: 1 });
        assert_eq!(s.status, Status::Loading);
        assert_eq!(fx[0], Effect::Resolve { seq: 1, video_id: "b".into(), fresh: false });
        assert!(
            matches!(&fx[1], Effect::FetchRadio { seed, continuation: None, .. } if seed == "b"),
            "last track: autoplay asks for radio"
        );
        finish_load(&mut s);
        assert_eq!(s.stream.as_ref().unwrap().codec, "opus");
    }

    #[test]
    fn stale_resolution_is_ignored() {
        let mut s = PlayerState::default();
        cmd(&mut s, Command::PlayTracks { tracks: vec![track("a"), track("b")], start: 0 });
        let old = s.load_seq;
        cmd(&mut s, Command::Next);
        assert!(reduce(&mut s, Input::Resolved { seq: old, result: Ok(info("a")) }).is_empty());
        assert_eq!(s.current_track().unwrap().video_id, "b");
    }

    #[test]
    fn ended_advances_and_stops_at_end() {
        let mut s = playing(&["a", "b"]);
        reduce(&mut s, Input::Player(PlayerEvent::Ended));
        assert_eq!(s.queue.current, Some(1));
        finish_load(&mut s);
        let fx = reduce(&mut s, Input::Player(PlayerEvent::Ended));
        assert_eq!(s.status, Status::Stopped);
        assert!(fx.contains(&Effect::Stop));
    }

    #[test]
    fn repeat_modes() {
        let mut s = playing(&["a", "b"]);
        cmd(&mut s, Command::CycleRepeat); // all
        cmd(&mut s, Command::JumpTo { index: 1 });
        finish_load(&mut s);
        reduce(&mut s, Input::Player(PlayerEvent::Ended));
        assert_eq!(s.queue.current, Some(0), "repeat all wraps");
        finish_load(&mut s);
        cmd(&mut s, Command::CycleRepeat); // one
        reduce(&mut s, Input::Player(PlayerEvent::Ended));
        assert_eq!(s.queue.current, Some(0), "repeat one replays");
        assert_eq!(s.status, Status::Loading);
    }

    #[test]
    fn prev_restarts_after_three_seconds() {
        let mut s = playing(&["a", "b"]);
        cmd(&mut s, Command::Next);
        finish_load(&mut s);
        reduce(&mut s, Input::Player(PlayerEvent::Position(Duration::from_secs(30))));
        assert_eq!(cmd(&mut s, Command::Prev), vec![Effect::Seek(Duration::ZERO)]);
        assert_eq!(s.queue.current, Some(1));
        reduce(&mut s, Input::Player(PlayerEvent::Position(Duration::from_secs(1))));
        cmd(&mut s, Command::Prev);
        assert_eq!(s.queue.current, Some(0));
    }

    #[test]
    fn toggle_pause() {
        let mut s = playing(&["a"]);
        assert_eq!(cmd(&mut s, Command::Toggle), vec![Effect::SetPaused(true)]);
        assert_eq!(s.status, Status::Paused);
        assert_eq!(cmd(&mut s, Command::Toggle), vec![Effect::SetPaused(false)]);
        assert_eq!(s.status, Status::Playing);
    }

    #[test]
    fn seek_and_volume_clamp() {
        let mut s = playing(&["a"]);
        cmd(&mut s, Command::Seek { seconds: 500.0, relative: false });
        assert!(s.position < Duration::from_secs(200));
        cmd(&mut s, Command::Seek { seconds: -999.0, relative: true });
        assert_eq!(s.position, Duration::ZERO);
        cmd(&mut s, Command::Volume { value: 50, relative: true });
        assert_eq!(s.volume, 100);
        cmd(&mut s, Command::Volume { value: -5, relative: false });
        assert_eq!(s.volume, 0);
    }

    #[test]
    fn absurd_seek_without_duration_does_not_panic() {
        let mut s = playing(&["a"]);
        s.duration = None;
        for seconds in [1e30, f64::INFINITY, f64::NAN] {
            cmd(&mut s, Command::Seek { seconds, relative: false });
            assert!(s.position <= Duration::from_secs_f64(MAX_SEEK_S));
        }
    }

    #[test]
    fn expired_stream_is_re_resolved_then_skipped() {
        let mut s = playing(&["a", "b"]);
        reduce(&mut s, Input::Player(PlayerEvent::Position(Duration::from_secs(42))));
        let fx = reduce(&mut s, Input::Player(PlayerEvent::Error("HTTP 403".into())));
        assert!(fx.contains(&Effect::Resolve { seq: s.load_seq, video_id: "a".into(), fresh: true }));
        let seq = s.load_seq;
        let id_fx = reduce(&mut s, Input::Resolved { seq, result: Ok(info("a")) });
        assert!(matches!(id_fx.as_slice(), [Effect::Load { start, .. }] if *start == Duration::from_secs(42)));
        reduce(&mut s, Input::Player(PlayerEvent::Started { duration: None }));
        reduce(&mut s, Input::Player(PlayerEvent::Error("x".into())));
        finish_load(&mut s);
        reduce(&mut s, Input::Player(PlayerEvent::Error("x".into())));
        assert_eq!(s.queue.current, Some(1), "gives up after {MAX_ATTEMPTS} attempts");
    }

    #[test]
    fn resolve_failure_skips() {
        let mut s = PlayerState::default();
        cmd(&mut s, Command::PlayTracks { tracks: vec![track("a"), track("b")], start: 0 });
        let seq = s.load_seq;
        let fx = reduce(&mut s, Input::Resolved { seq, result: Err("video unavailable".into()) });
        assert!(matches!(fx[0], Effect::Toast(Toast { level: Level::Error, .. })));
        assert_eq!(s.current_track().unwrap().video_id, "b");
    }

    #[test]
    fn queue_edits_keep_current_track() {
        let mut s = playing(&["a", "b", "c", "d"]);
        cmd(&mut s, Command::JumpTo { index: 2 });
        finish_load(&mut s);
        cmd(&mut s, Command::Move { from: 2, to: 0 });
        assert_eq!(s.current_track().unwrap().video_id, "c");
        cmd(&mut s, Command::Move { from: 3, to: 0 });
        assert_eq!(s.current_track().unwrap().video_id, "c");
        cmd(&mut s, Command::Remove { index: 0 });
        assert_eq!(s.current_track().unwrap().video_id, "c");
        cmd(&mut s, Command::Enqueue { tracks: vec![track("x")], next: true });
        let cur = s.queue.current.unwrap();
        assert_eq!(s.queue.items[cur + 1].video_id, "x");
        cmd(&mut s, Command::Clear);
        assert_eq!(s.queue.items.len(), 1);
        assert_eq!(s.current_track().unwrap().video_id, "c");
    }

    #[test]
    fn removing_current_plays_next() {
        let mut s = playing(&["a", "b"]);
        let fx = cmd(&mut s, Command::Remove { index: 0 });
        assert!(matches!(fx.first(), Some(Effect::Resolve { video_id, .. }) if video_id == "b"));
    }

    #[test]
    fn enqueue_when_idle_starts_playback() {
        let mut s = PlayerState::default();
        let fx = cmd(&mut s, Command::Enqueue { tracks: vec![track("a")], next: false });
        assert!(fx.iter().any(|e| matches!(e, Effect::Resolve { .. })));
        let fx = cmd(&mut s, Command::Enqueue { tracks: vec![track("b")], next: false });
        assert!(!fx.iter().any(|e| matches!(e, Effect::Resolve { .. })), "busy: just append");
    }

    #[test]
    fn shuffle_round_trip_restores_order() {
        let ids: Vec<String> = (0..30).map(|i| format!("{i:02}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let mut s = playing(&refs);
        cmd(&mut s, Command::SetShuffle { on: Some(true) });
        assert_eq!(s.queue.items[0].video_id, "00", "current stays put");
        cmd(&mut s, Command::SetShuffle { on: Some(false) });
        let after: Vec<&str> = s.queue.items.iter().map(|t| t.video_id.as_str()).collect();
        assert_eq!(after, refs);
    }

    #[test]
    fn backend_exit_restarts_and_resumes() {
        let mut s = playing(&["a"]);
        reduce(&mut s, Input::Player(PlayerEvent::Position(Duration::from_secs(10))));
        let fx = reduce(&mut s, Input::Player(PlayerEvent::Exited));
        assert!(fx.contains(&Effect::RestartBackend));
        assert_eq!(s.pending_start, Duration::from_secs(10));
    }

    fn at(s: &mut PlayerState, secs: u64) -> Vec<Effect> {
        reduce(s, Input::Player(PlayerEvent::Position(Duration::from_secs(secs))))
    }

    fn prefetch(id: &str) -> Vec<Effect> {
        vec![Effect::Prefetch { video_id: id.into() }]
    }

    #[test]
    fn prefetches_next_track_once_at_75_percent() {
        let mut s = playing(&["a", "b"]); // 200 s tracks
        assert!(at(&mut s, 149).is_empty());
        assert_eq!(at(&mut s, 150), prefetch("b"));
        assert!(at(&mut s, 151).is_empty(), "only once per track");
    }

    #[test]
    fn prefetches_with_30_seconds_left_on_short_tracks() {
        let mut s = playing(&["a", "b"]);
        s.duration = Some(Duration::from_secs(100)); // 75 % would be 75 s
        assert!(at(&mut s, 69).is_empty());
        assert_eq!(at(&mut s, 70), prefetch("b"));
    }

    #[test]
    fn prefetch_follows_queue_and_repeat() {
        let mut s = playing(&["a"]);
        assert!(at(&mut s, 190).is_empty(), "nothing after the last track");
        cmd(&mut s, Command::Enqueue { tracks: vec![track("b")], next: false });
        assert_eq!(at(&mut s, 191), prefetch("b"), "newly queued track is picked up");
        cmd(&mut s, Command::Enqueue { tracks: vec![track("c")], next: true });
        assert_eq!(at(&mut s, 192), prefetch("c"), "re-targets when the next track changes");

        let mut s = playing(&["a", "b"]);
        cmd(&mut s, Command::JumpTo { index: 1 });
        finish_load(&mut s);
        cmd(&mut s, Command::CycleRepeat); // all
        assert_eq!(at(&mut s, 190), prefetch("a"), "repeat all wraps");
        cmd(&mut s, Command::CycleRepeat); // one
        cmd(&mut s, Command::JumpTo { index: 0 });
        finish_load(&mut s);
        assert!(at(&mut s, 190).is_empty(), "repeat one replays the cached track");
    }

    #[test]
    fn no_prefetch_while_paused_or_without_duration() {
        let mut s = playing(&["a", "b"]);
        cmd(&mut s, Command::Pause);
        assert!(at(&mut s, 190).is_empty());
        cmd(&mut s, Command::Play);
        s.duration = None;
        assert!(at(&mut s, 190).is_empty());
    }

    #[test]
    fn each_track_prefetches_its_successor() {
        let mut s = playing(&["a", "b", "c"]);
        assert_eq!(at(&mut s, 190), prefetch("b"));
        reduce(&mut s, Input::Player(PlayerEvent::Ended));
        finish_load(&mut s);
        assert_eq!(at(&mut s, 190), prefetch("c"));
        cmd(&mut s, Command::JumpTo { index: 0 });
        finish_load(&mut s);
        assert_eq!(at(&mut s, 190), prefetch("b"), "replaying a track prefetches again");
    }

    fn ids(s: &PlayerState) -> Vec<&str> {
        s.queue.items.iter().map(|t| t.video_id.as_str()).collect()
    }

    fn radio_fetch(fx: &[Effect]) -> Option<(u64, String, Option<String>)> {
        fx.iter().find_map(|e| match e {
            Effect::FetchRadio { seq, seed, continuation } => Some((*seq, seed.clone(), continuation.clone())),
            _ => None,
        })
    }

    fn page(ids: &[&str], continuation: Option<&str>) -> Result<WatchPlaylist, String> {
        Ok(WatchPlaylist { tracks: ids.iter().map(|i| track(i)).collect(), continuation: continuation.map(Into::into) })
    }

    fn manual(ids: &[&str]) -> PlayerState {
        let mut s = PlayerState { autoplay: false, ..PlayerState::default() };
        cmd(&mut s, Command::PlayTracks { tracks: ids.iter().map(|i| track(i)).collect(), start: 0 });
        finish_load(&mut s);
        s
    }

    #[test]
    fn autoplay_fills_the_queue_from_the_last_track() {
        let mut s = PlayerState::default();
        let fx = cmd(&mut s, Command::PlayTracks { tracks: vec![track("a"), track("b")], start: 0 });
        assert_eq!(radio_fetch(&fx), None, "not yet: b is still to come");
        finish_load(&mut s);
        let fx = reduce(&mut s, Input::Player(PlayerEvent::Ended));
        let (seq, seed, cont) = radio_fetch(&fx).expect("last track starts → radio");
        assert_eq!((seed.as_str(), cont), ("b", None));
        assert!(s.radio.as_ref().unwrap().loading);
        finish_load(&mut s);

        // The seed comes back first and is already queued; a and b are dropped as duplicates.
        reduce(&mut s, Input::Radio { seq, result: page(&["b", "r1", "a", "r2", "r1"], Some("next-page")) });
        assert_eq!(ids(&s), ["a", "b", "r1", "r2"]);
        assert_eq!(s.queue.autoplay, 2);
        assert!(!s.radio.as_ref().unwrap().loading);
        assert_eq!(at(&mut s, 190), prefetch("r1"), "suggestions are prefetched like any next track");

        reduce(&mut s, Input::Player(PlayerEvent::Ended));
        assert_eq!(s.current_track().unwrap().video_id, "r1");
        assert_eq!(s.queue.autoplay, 1, "a played suggestion becomes history");
    }

    #[test]
    fn running_radio_fetches_its_next_page_when_low() {
        let mut s = playing(&["a"]);
        let seq = s.radio_seq;
        reduce(&mut s, Input::Radio { seq, result: page(&["a", "r1", "r2", "r3", "r4", "r5"], Some("p2")) });
        assert_eq!(s.queue.autoplay, 5);
        let fx = reduce(&mut s, Input::Player(PlayerEvent::Ended)); // r1: 4 left
        assert_eq!(radio_fetch(&fx), None);
        finish_load(&mut s);
        let fx = reduce(&mut s, Input::Player(PlayerEvent::Ended)); // r2: 3 left
        let (seq, seed, cont) = radio_fetch(&fx).expect("low water");
        assert_eq!((seed.as_str(), cont.as_deref()), ("a", Some("p2")));
        reduce(&mut s, Input::Radio { seq, result: page(&["r6"], None) });
        assert_eq!(ids(&s).last(), Some(&"r6"));
        assert_eq!(s.queue.autoplay, 4);
    }

    #[test]
    fn exhausted_radio_reseeds_from_the_last_track() {
        let mut s = playing(&["a"]);
        let seq = s.radio_seq;
        reduce(&mut s, Input::Radio { seq, result: page(&["r1"], None) });
        let fx = reduce(&mut s, Input::Player(PlayerEvent::Ended));
        let (_, seed, cont) = radio_fetch(&fx).expect("last suggestion starts, no more pages → new radio");
        assert_eq!((seed.as_str(), cont), ("r1", None));
    }

    #[test]
    fn queue_running_out_waits_for_radio_then_plays() {
        let mut s = playing(&["a"]);
        let seq = s.radio_seq; // in flight since a started
        let fx = reduce(&mut s, Input::Player(PlayerEvent::Ended));
        assert!(fx.contains(&Effect::Stop));
        assert_eq!(s.status, Status::Stopped);
        assert!(s.radio_resume);
        let fx = reduce(&mut s, Input::Radio { seq, result: page(&["a", "r1"], None) });
        assert!(matches!(fx.first(), Some(Effect::Resolve { video_id, .. }) if video_id == "r1"));
        assert_eq!(s.queue.autoplay, 0);
    }

    #[test]
    fn autoplay_off_or_repeat_never_fetches() {
        let mut s = manual(&["a"]);
        let fx = reduce(&mut s, Input::Player(PlayerEvent::Ended));
        assert_eq!(radio_fetch(&fx), None);
        assert_eq!(s.status, Status::Stopped);

        let mut s = PlayerState::default();
        cmd(&mut s, Command::CycleRepeat);
        let fx = cmd(&mut s, Command::PlayTracks { tracks: vec![track("a")], start: 0 });
        assert_eq!(radio_fetch(&fx), None);
    }

    #[test]
    fn radio_failure_and_stale_pages() {
        let mut s = playing(&["a"]);
        let old = s.radio_seq;
        let fx = reduce(&mut s, Input::Radio { seq: old, result: Err("HTTP 500".into()) });
        assert!(matches!(&fx[..], [Effect::Toast(Toast { level: Level::Warn, .. })]));
        assert!(!s.radio.as_ref().unwrap().loading);

        cmd(&mut s, Command::Radio { track: None });
        assert!(reduce(&mut s, Input::Radio { seq: old, result: page(&["x"], None) }).is_empty(), "stale page");
        cmd(&mut s, Command::PlayTracks { tracks: vec![track("b"), track("c")], start: 0 });
        assert!(s.radio.is_none());
        assert!(reduce(&mut s, Input::Radio { seq: old + 1, result: page(&["x"], None) }).is_empty(), "radio reset by a new queue");
        assert_eq!(ids(&s), ["b", "c"]);
    }

    #[test]
    fn r_replaces_the_autoplay_section() {
        let mut s = manual(&["a", "b"]);
        let fx = cmd(&mut s, Command::Radio { track: None });
        let (seq, seed, _) = radio_fetch(&fx).unwrap();
        assert_eq!(seed, "a", "default seed is the current track");
        assert_eq!(ids(&s), ["a", "b"], "a current seed isn't queued again");
        reduce(&mut s, Input::Radio { seq, result: page(&["a", "r1", "r2"], None) });
        assert_eq!(ids(&s), ["a", "b", "r1", "r2"]);

        // R on a search result: it leads the new section; the user's b stays put.
        let fx = cmd(&mut s, Command::Radio { track: Some(track("z")) });
        assert_eq!(ids(&s), ["a", "b", "z"]);
        assert_eq!(s.queue.autoplay, 1);
        assert!(!fx.iter().any(|e| matches!(e, Effect::Resolve { .. })), "busy: nothing interrupted");
        let (seq, ..) = radio_fetch(&fx).unwrap();
        reduce(&mut s, Input::Radio { seq, result: page(&["z", "z1"], None) });
        assert_eq!(ids(&s), ["a", "b", "z", "z1"]);
        assert_eq!(s.radio.as_ref().unwrap().seed.video_id, "z");
    }

    #[test]
    fn r_when_idle_plays_the_seed() {
        let mut s = PlayerState { autoplay: false, ..PlayerState::default() };
        let fx = cmd(&mut s, Command::Radio { track: Some(track("z")) });
        assert!(fx.iter().any(|e| matches!(e, Effect::Resolve { video_id, .. } if video_id == "z")));
        assert!(radio_fetch(&fx).is_some(), "explicit radio works with autoplay off");
        assert_eq!(s.queue.current, Some(0));
        assert_eq!(s.queue.autoplay, 0, "the playing seed is history, not a suggestion");
        assert!(cmd(&mut PlayerState::default(), Command::Radio { track: None }).iter().any(|e| matches!(e, Effect::Toast(_))));
    }

    #[test]
    fn user_tracks_stay_above_suggestions() {
        let mut s = manual(&["a", "b"]);
        let seq = radio_fetch(&cmd(&mut s, Command::Radio { track: None })).unwrap().0;
        reduce(&mut s, Input::Radio { seq, result: page(&["r1", "r2"], None) });
        cmd(&mut s, Command::Enqueue { tracks: vec![track("u")], next: false });
        assert_eq!(ids(&s), ["a", "b", "u", "r1", "r2"], "append lands before the autoplay section");
        cmd(&mut s, Command::Enqueue { tracks: vec![track("n")], next: true });
        assert_eq!(ids(&s), ["a", "n", "b", "u", "r1", "r2"]);
        assert_eq!(s.queue.autoplay, 2);

        cmd(&mut s, Command::Move { from: 2, to: 5 });
        assert_eq!(ids(&s), ["a", "n", "u", "b", "r1", "r2"], "a user track can't sink below the divider");
        cmd(&mut s, Command::Move { from: 5, to: 1 });
        assert_eq!(ids(&s), ["a", "r2", "n", "u", "b", "r1"], "a suggestion moved up is adopted");
        assert_eq!(s.queue.autoplay, 1);
        cmd(&mut s, Command::Remove { index: 5 });
        assert_eq!(s.queue.autoplay, 0);

        let mut s = manual(&["a"]);
        let seq = radio_fetch(&cmd(&mut s, Command::Radio { track: None })).unwrap().0;
        reduce(&mut s, Input::Radio { seq, result: page(&["r1", "r2", "r3"], None) });
        cmd(&mut s, Command::Enqueue { tracks: vec![track("u1"), track("u2"), track("u3")], next: false });
        cmd(&mut s, Command::SetShuffle { on: Some(true) });
        assert_eq!(&ids(&s)[4..], ["r1", "r2", "r3"], "shuffle leaves suggestions alone");
        cmd(&mut s, Command::SetShuffle { on: Some(false) });
        assert_eq!(ids(&s), ["a", "u1", "u2", "u3", "r1", "r2", "r3"]);
    }

    #[test]
    fn clear_autoplay_and_toggle() {
        let mut s = manual(&["a", "b"]);
        let seq = radio_fetch(&cmd(&mut s, Command::Radio { track: None })).unwrap().0;
        reduce(&mut s, Input::Radio { seq, result: page(&["r1", "r2"], Some("p2")) });
        cmd(&mut s, Command::ClearAutoplay);
        assert_eq!(ids(&s), ["a", "b"]);
        assert!(s.radio.is_none());

        cmd(&mut s, Command::JumpTo { index: 1 });
        finish_load(&mut s);
        let fx = cmd(&mut s, Command::SetAutoplay { on: Some(true) });
        let (seq, seed, _) = radio_fetch(&fx).expect("turning autoplay on at the last track fetches");
        assert_eq!(seed, "b");
        cmd(&mut s, Command::SetAutoplay { on: None });
        assert!(!s.autoplay);
        assert!(reduce(&mut s, Input::Radio { seq, result: page(&["r1"], None) }).is_empty(), "page in flight dropped");
        assert_eq!(ids(&s), ["a", "b"]);
    }

    #[test]
    fn queue_state_json_carries_the_divider() {
        let mut s = manual(&["a"]);
        let seq = radio_fetch(&cmd(&mut s, Command::Radio { track: None })).unwrap().0;
        reduce(&mut s, Input::Radio { seq, result: page(&["r1"], Some("secret-token")) });
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["queue"]["autoplay"], 1);
        assert_eq!(v["radio"]["seed"]["video_id"], "a");
        assert_eq!(v["autoplay"], false);
        assert!(!v.to_string().contains("secret-token"), "continuation tokens stay in the daemon");
        let c: Command = serde_json::from_str(r#"{"cmd":"radio"}"#).unwrap();
        assert_eq!(c, Command::Radio { track: None });
    }

    #[test]
    fn command_json_shape() {
        let c: Command = serde_json::from_str(r#"{"cmd":"seek","seconds":-10}"#).unwrap();
        assert_eq!(c, Command::Seek { seconds: -10.0, relative: false });
        assert_eq!(serde_json::to_string(&Command::Next).unwrap(), r#"{"cmd":"next"}"#);
    }
}
