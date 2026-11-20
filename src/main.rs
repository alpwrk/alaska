mod action;
mod app;
mod browser;
mod meta;
mod player;
mod queue;
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
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
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
