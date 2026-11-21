mod action;
mod app;
mod browser;
mod config;
mod login;
mod meta;
mod mpris;
mod opus;
mod player;
mod prefetch;
mod queue;
mod remote;
mod state;
mod subsonic;
mod track;
mod ui;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::app::App;

const USAGE: &str = "usage: alaska [DIRECTORY]";

fn main() -> Result<()> {
    let start_dir = parse_args()?;
    // Build the app (opens the audio device) before touching the terminal,
    // so startup errors are printed normally.
    let mut app = App::new(start_dir)?;

    // `ratatui::init` installs a panic hook that restores the terminal.
    let mut terminal = ratatui::init();
    // Libraries like alsa-lib print to stderr, which would garble the UI.
    let stderr = StderrRedirect::to_log();
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        StderrRedirect::restore_raw(stderr.saved);
        previous_hook(info);
    }));
    let result = app.run(&mut terminal);
    StderrRedirect::restore_raw(stderr.saved);
    ratatui::restore();
    result
}

/// Points fd 2 at a log file while the TUI runs.
#[derive(Clone, Copy)]
struct StderrRedirect {
    /// Duplicate of the original stderr, or -1 if nothing was redirected.
    saved: i32,
}

impl StderrRedirect {
    fn to_log() -> Self {
        let Some(path) = dirs::cache_dir().map(|d| d.join("alaska").join("stderr.log")) else {
            return StderrRedirect { saved: -1 };
        };
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let Ok(log) = std::fs::File::create(&path) else {
            return StderrRedirect { saved: -1 };
        };
        use std::os::fd::AsRawFd;
        // SAFETY: plain fd duplication; `log` stays open until dup2 has copied it.
        let saved = unsafe { libc::dup(2) };
        if saved >= 0 {
            unsafe { libc::dup2(log.as_raw_fd(), 2) };
        }
        StderrRedirect { saved }
    }

    fn restore_raw(saved: i32) {
        if saved >= 0 {
            // SAFETY: `saved` is a valid duplicate of the original stderr.
            unsafe { libc::dup2(saved, 2) };
        }
    }
}

fn parse_args() -> Result<PathBuf> {
    let mut args = std::env::args().skip(1);
    let dir = match args.next().as_deref() {
        Some("-h" | "--help") => {
            println!("{USAGE}");
            std::process::exit(0);
        }
        Some(arg) => PathBuf::from(arg),
        None => std::env::current_dir().context("cannot determine current directory")?,
    };
    if args.next().is_some() {
        bail!("{USAGE}");
    }
    let dir = dir
        .canonicalize()
        .with_context(|| format!("cannot open {}", dir.display()))?;
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    Ok(dir)
}
