//! Where things live on disk.

use std::path::PathBuf;

use directories::ProjectDirs;

fn dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "ytm-tui")
}

/// `~/.config/ytm-tui` (Linux), `~/Library/Application Support/ytm-tui` (macOS).
pub fn config_dir() -> PathBuf {
    std::env::var_os("YTM_TUI_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs().map(|d| d.config_dir().to_path_buf()))
        .unwrap_or_else(|| PathBuf::from(".ytm-tui"))
}

pub fn state_dir() -> PathBuf {
    std::env::var_os("YTM_TUI_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs().map(|d| d.state_dir().unwrap_or(d.data_local_dir()).to_path_buf()))
        .unwrap_or_else(|| std::env::temp_dir().join("ytm-tui"))
}

pub fn auth_file() -> PathBuf {
    config_dir().join("auth.json")
}

pub fn lyrics_dir() -> PathBuf {
    config_dir().join("lyrics")
}

pub fn cookies_txt() -> PathBuf {
    state_dir().join("cookies.txt")
}

/// InnerTube visitor id, kept so the daemon presents as the same visitor across restarts.
pub fn visitor_file() -> PathBuf {
    state_dir().join("visitor_data")
}

pub fn log_file_dir() -> PathBuf {
    state_dir()
}

/// The daemon's control socket. Always in a directory only this user can write to, so another
/// local user can't squat the path: `$XDG_RUNTIME_DIR`, the per-user `$TMPDIR` on macOS, or the
/// state dir (never a shared `/tmp`).
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("YTM_TUI_SOCKET") {
        return PathBuf::from(p);
    }
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("ytm-tui.sock");
    }
    if cfg!(target_os = "macos") {
        let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        return std::env::temp_dir().join(format!("ytm-tui-{user}.sock"));
    }
    state_dir().join("ytm-tui.sock")
}
