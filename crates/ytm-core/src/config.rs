//! `config.toml`. Every field has a default, so a missing or partial file is fine.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub audio: AudioConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// "auto" (mpv, falling back to null), "mpv" or "null".
    pub backend: String,
    pub volume: u8,
    /// yt-dlp format selector.
    pub format: String,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self { backend: "auto".into(), volume: 72, format: "bestaudio[acodec=opus]/bestaudio/best".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// "dark" or "light".
    pub theme: String,
    pub mouse: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { theme: "dark".into(), mouse: true }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = crate::paths::config_dir().join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("ignoring invalid {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }
}
