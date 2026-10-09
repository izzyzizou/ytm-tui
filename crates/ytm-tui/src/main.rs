//! ytm-tui — no subcommand opens the TUI; subcommands are the headless CLI.

mod app;
mod cli;
mod theme;
mod ui;

use std::sync::Arc;

use clap::{Parser, Subcommand};
use ytm_core::client::Client;
use ytm_core::config::Config;
use ytm_core::paths;

#[derive(Parser)]
#[command(name = "ytm-tui", version, about = "A keyboard-first YouTube Music client for the terminal")]
pub struct Cli {
    /// Audio backend for a daemon started by this command: auto | mpv | null
    #[arg(long, global = true)]
    backend: Option<String>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Run the playback daemon in the foreground
    Daemon,
    /// Search and play the best match (no query: resume)
    Play {
        query: Vec<String>,
        /// Add to the end of the queue instead of playing now
        #[arg(long)]
        enqueue: bool,
        /// Insert after the current track
        #[arg(long)]
        next: bool,
    },
    Pause,
    Resume,
    Toggle,
    Stop,
    Next,
    Prev,
    /// Seek: 90, 1:30, +30, -10
    Seek {
        #[arg(allow_hyphen_values = true)]
        position: String,
    },
    /// Volume: 60, +5, -5
    Volume {
        #[arg(allow_hyphen_values = true)]
        level: String,
    },
    /// Shuffle: on | off | toggle
    Shuffle {
        mode: Option<String>,
    },
    /// Cycle repeat: off → all → one
    Repeat,
    /// Start radio from the current track (replaces the queue's autoplay section)
    Radio,
    /// Autoplay radio when the queue runs out: on | off | toggle
    Autoplay {
        mode: Option<String>,
    },
    /// Show or edit the queue
    Queue {
        #[command(subcommand)]
        action: Option<QueueCmd>,
    },
    /// Print player status
    Status {
        #[arg(long)]
        json: bool,
        /// e.g. '{artist} - {title} [{elapsed}/{duration}]'
        #[arg(long)]
        format: Option<String>,
    },
    /// Search YouTube Music
    Search {
        query: Vec<String>,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Print lyrics of the current track
    Lyrics {
        /// Print LRC timestamps
        #[arg(long)]
        synced: bool,
    },
    /// Manage the browser-session login
    Auth {
        #[command(subcommand)]
        action: AuthCmd,
    },
    /// Check mpv, yt-dlp, deno, auth and the terminal
    Doctor,
    /// Stop the daemon
    Quit,
}

#[derive(Subcommand, Debug)]
pub enum QueueCmd {
    /// List the queue
    Ls {
        #[arg(long)]
        json: bool,
    },
    /// Search and add the best match
    Add {
        query: Vec<String>,
        #[arg(long)]
        next: bool,
    },
    /// Remove a track by its 1-based position
    Rm { position: usize },
    /// Remove everything except the current track
    Clear,
}

#[derive(Subcommand, Debug)]
pub enum AuthCmd {
    /// Import headers: a "Copy as cURL" of a music.youtube.com request, raw request headers,
    /// or a ytmusicapi headers_auth.json. Reads stdin unless --file is given.
    Import {
        #[arg(long)]
        file: Option<std::path::PathBuf>,
    },
    /// Show whether a login is stored
    Status,
    /// Delete the stored login
    Logout,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    rt.block_on(async move {
        match cli.cmd {
            None => match run_tui(cli.backend).await {
                Ok(()) => std::process::ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("ytm-tui: {e:?}");
                    std::process::ExitCode::FAILURE
                }
            },
            Some(Cmd::Daemon) => run_daemon(cli.backend).await,
            Some(cmd) => cli::run(cmd, cli.backend).await,
        }
    })
}

fn init_file_logging(name: &str) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = paths::log_file_dir();
    ytm_api::auth::create_private_dir(&dir).ok()?;
    let appender = tracing_appender::rolling::daily(dir, format!("{name}.log"));
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = tracing_subscriber::EnvFilter::try_from_env("YTM_TUI_LOG").unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(writer).with_ansi(false).try_init().ok()?;
    Some(guard)
}

async fn run_daemon(backend: Option<String>) -> std::process::ExitCode {
    let _guard = init_file_logging("daemon");
    let cfg = Config::load();
    let opts = ytm_core::daemon::DaemonOptions {
        backend: backend.unwrap_or(cfg.audio.backend),
        socket: paths::socket_path(),
        volume: cfg.audio.volume,
        ytdlp_format: cfg.audio.format,
        autoplay: cfg.player.autoplay,
    };
    match ytm_core::daemon::run(opts).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ytm-tui daemon: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run_tui(backend: Option<String>) -> color_eyre::Result<()> {
    color_eyre::install()?;
    let _guard = init_file_logging("tui");
    let cfg = Config::load();
    let extra: Vec<String> = backend.map(|b| vec!["--backend".into(), b]).unwrap_or_default();
    let mut client = Client::connect_or_spawn(&paths::socket_path(), &extra)
        .await
        .map_err(|e| color_eyre::eyre::eyre!("could not start or reach the daemon: {e} (try `ytm-tui daemon` to see why)"))?;
    let events = client.take_events().expect("fresh client");
    client.subscribe().await?;
    let client = Arc::new(client);

    let mut terminal = ratatui::init(); // raw mode + alt screen + panic hook that restores the terminal
    if cfg.ui.mouse {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
    }
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste);
    terminal.clear()?;
    let app = app::App::new(theme::Theme::from_name(&cfg.ui.theme), client.clone());
    let result = app.run(terminal, events).await;
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture, crossterm::event::DisableBracketedPaste);
    ratatui::restore();
    let stop_daemon = result?;
    if stop_daemon {
        let _ = client.call(ytm_core::protocol::method::SHUTDOWN, serde_json::json!({})).await;
    }
    Ok(())
}
