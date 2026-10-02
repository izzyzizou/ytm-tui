//! A silent backend with a simulated clock. Used in tests, in CI, on machines without mpv,
//! and with `--backend null` to develop the UI without audio.

use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{AudioBackend, Media, PlayerError, PlayerEvent};

const DEFAULT_LENGTH: Duration = Duration::from_secs(180);

pub struct NullBackend {
    events: mpsc::UnboundedSender<PlayerEvent>,
    clock: Option<Clock>,
    ticker: Option<JoinHandle<()>>,
    /// How often positions are reported; tests shrink this.
    pub tick: Duration,
    /// Simulated playback speed (1.0 = real time); tests raise it.
    pub speed: f64,
}

#[derive(Clone, Copy)]
struct Clock {
    base: Duration,
    since: Option<Instant>, // None = paused
    length: Duration,
}

impl Clock {
    fn now(&self, speed: f64) -> Duration {
        self.base + self.since.map(|s| s.elapsed().mul_f64(speed)).unwrap_or_default()
    }
}

impl NullBackend {
    pub fn new(events: mpsc::UnboundedSender<PlayerEvent>) -> Self {
        Self { events, clock: None, ticker: None, tick: Duration::from_millis(250), speed: 1.0 }
    }

    fn restart_ticker(&mut self) {
        if let Some(t) = self.ticker.take() {
            t.abort();
        }
        let Some(clock) = self.clock else { return };
        if clock.since.is_none() {
            return;
        }
        let (tx, tick, speed) = (self.events.clone(), self.tick, self.speed);
        self.ticker = Some(tokio::spawn(async move {
            let mut iv = tokio::time::interval(tick);
            loop {
                iv.tick().await;
                let pos = clock.now(speed);
                if pos >= clock.length {
                    let _ = tx.send(PlayerEvent::Position(clock.length));
                    let _ = tx.send(PlayerEvent::Ended);
                    break;
                }
                if tx.send(PlayerEvent::Position(pos)).is_err() {
                    break;
                }
            }
        }));
    }
}

#[async_trait::async_trait]
impl AudioBackend for NullBackend {
    fn name(&self) -> &'static str {
        "null"
    }

    async fn load(&mut self, media: Media, start: Duration) -> Result<(), PlayerError> {
        let length = media.duration_hint.unwrap_or(DEFAULT_LENGTH);
        self.clock = Some(Clock { base: start.min(length), since: Some(Instant::now()), length });
        let _ = self.events.send(PlayerEvent::Started { duration: Some(length) });
        self.restart_ticker();
        Ok(())
    }

    async fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError> {
        let speed = self.speed;
        if let Some(c) = &mut self.clock {
            match (paused, c.since) {
                (true, Some(_)) => {
                    c.base = c.now(speed);
                    c.since = None;
                }
                (false, None) => c.since = Some(Instant::now()),
                _ => {}
            }
            let _ = self.events.send(PlayerEvent::Paused(paused));
        }
        self.restart_ticker();
        Ok(())
    }

    async fn seek(&mut self, to: Duration) -> Result<(), PlayerError> {
        if let Some(c) = &mut self.clock {
            c.base = to.min(c.length);
            if c.since.is_some() {
                c.since = Some(Instant::now());
            }
            let _ = self.events.send(PlayerEvent::Position(c.base));
        }
        self.restart_ticker();
        Ok(())
    }

    async fn set_volume(&mut self, _percent: u8) -> Result<(), PlayerError> {
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), PlayerError> {
        self.clock = None;
        if let Some(t) = self.ticker.take() {
            t.abort();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn plays_to_the_end() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut b = NullBackend::new(tx);
        b.tick = Duration::from_millis(5);
        b.speed = 200.0;
        b.load(Media { location: "x".into(), duration_hint: Some(Duration::from_secs(2)) }, Duration::ZERO).await.unwrap();
        let mut saw_end = false;
        while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
            if ev == PlayerEvent::Ended {
                saw_end = true;
                break;
            }
        }
        assert!(saw_end);
    }

    #[tokio::test]
    async fn pause_freezes_clock() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut b = NullBackend::new(tx);
        b.load(Media { location: "x".into(), duration_hint: None }, Duration::from_secs(10)).await.unwrap();
        b.set_paused(true).await.unwrap();
        let t1 = b.clock.unwrap().now(1.0);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(b.clock.unwrap().now(1.0), t1);
    }
}
