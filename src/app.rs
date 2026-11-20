use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::widgets::ListState;

use crate::action::{Action, map_key};
use crate::browser::{Browser, is_audio};
use crate::meta::{self, TagCache, TrackMeta};
use crate::player::Player;
use crate::queue::Queue;
use crate::state::State;
use crate::ui;

const TICK: Duration = Duration::from_millis(100);
const SEEK_STEP: i64 = 5;
const VOLUME_STEP: f32 = 0.05;
const PAGE: isize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Browser,
    Queue,
}

pub struct App {
    pub browser: Browser,
    pub queue: Queue,
    pub player: Player,
    pub cache: TagCache,
    /// Directory whose tags were last loaded into the cache.
    meta_dir: Option<PathBuf>,
    pub focus: Focus,
    pub queue_cursor: usize,
    pub filter_mode: bool,
    pub message: Option<String>,
    pub browser_state: ListState,
    pub queue_state: ListState,
    quit: bool,
}

impl App {
    pub fn new(start_dir: PathBuf) -> Result<Self> {
        let mut app = App {
            browser: Browser::new(start_dir),
            queue: Queue::default(),
            player: Player::new()?,
            cache: TagCache::load(),
            meta_dir: None,
            focus: Focus::Browser,
            queue_cursor: 0,
            filter_mode: false,
            message: None,
            browser_state: ListState::default(),
            queue_state: ListState::default(),
            quit: false,
        };
        app.restore(State::load());
        Ok(app)
    }

    /// Restore volume, modes and queue; the current track is loaded paused.
    fn restore(&mut self, state: State) {
        self.player.set_volume(state.volume);
        self.queue.repeat = state.repeat;
        self.queue.set_shuffle(state.shuffle);
        self.cache.ensure(&state.queue);
        self.queue.set(state.queue, state.current);
        if let Some(i) = self.queue.current_index() {
            self.queue_cursor = i;
            let start = Duration::try_from_secs_f64(state.position_secs).unwrap_or_default();
            self.load_current(start, true);
        }
    }

    fn snapshot(&self) -> State {
        State {
            volume: self.player.volume(),
            queue: self.queue.tracks().to_vec(),
            current: self.queue.current_index(),
            position_secs: self.player.position().as_secs_f64(),
            shuffle: self.queue.shuffle(),
            repeat: self.queue.repeat,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            self.refresh_meta();
            terminal.draw(|frame| ui::draw(frame, self))?;
            if event::poll(TICK)? {
                // Drain all pending input before redrawing to stay responsive.
                loop {
                    if let Event::Key(key) = event::read()?
                        && key.kind != KeyEventKind::Release
                    {
                        self.handle_key(key);
                    }
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }
            self.tick();
        }
        self.snapshot().save()?;
        self.cache.save()
    }

    /// Load tags for the files of a newly entered directory.
    fn refresh_meta(&mut self) {
        if self.meta_dir.as_ref() != Some(&self.browser.dir) {
            self.cache.ensure(&self.browser.audio_files());
            self.meta_dir = Some(self.browser.dir.clone());
        }
    }

    pub fn meta(&self, path: &Path) -> Option<&TrackMeta> {
        self.cache.get(path)
    }

    /// Display name for a track.
    pub fn track_name(&self, path: &Path) -> String {
        meta::label(path, self.meta(path))
    }

    fn tick(&mut self) {
        if self.player.is_finished() {
            match self.queue.advance(true) {
                Some(_) => self.play_current(),
                None => self.player.stop(),
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.filter_mode {
            self.handle_filter_key(key);
        } else if let Some(action) = map_key(key) {
            self.message = None;
            self.dispatch(action);
        }
    }

    fn handle_filter_key(&mut self, key: KeyEvent) {
        let mut filter = self.browser.filter.clone();
        match key.code {
            KeyCode::Esc => {
                self.filter_mode = false;
                filter.clear();
            }
            KeyCode::Enter => {
                self.filter_mode = false;
                return;
            }
            KeyCode::Backspace => {
                filter.pop();
            }
            KeyCode::Down => return self.browser.move_by(1),
            KeyCode::Up => return self.browser.move_by(-1),
            KeyCode::Char(c) => filter.push(c),
            _ => return,
        }
        self.browser.set_filter(filter);
    }

    pub fn dispatch(&mut self, action: Action) {
        use Action::*;
        match action {
            Up => self.move_cursor(-1),
            Down => self.move_cursor(1),
            PageUp => self.move_cursor(-PAGE),
            PageDown => self.move_cursor(PAGE),
            Top => self.move_cursor(isize::MIN / 2),
            Bottom => self.move_cursor(isize::MAX / 2),
            Open => self.open(),
            Parent => {
                if self.focus == Focus::Browser {
                    self.browser.parent();
                }
            }
            Home => {
                if let Some(home) = dirs::home_dir() {
                    self.focus = Focus::Browser;
                    self.browser.cd(home);
                }
            }
            ToggleFocus => {
                self.focus = match self.focus {
                    Focus::Browser => Focus::Queue,
                    Focus::Queue => Focus::Browser,
                };
                if let Some(i) = self.queue.current_index()
                    && self.focus == Focus::Queue
                {
                    self.queue_cursor = i;
                }
            }
            PlayPause => {
                if self.player.is_loaded() {
                    self.player.toggle_pause();
                } else if self.queue.current().is_some() {
                    self.play_current();
                }
            }
            Next => {
                if self.queue.advance(false).is_some() {
                    self.play_current();
                }
            }
            Prev => {
                // Like most players: restart the track unless we are near its start.
                if self.player.position() > Duration::from_secs(3) {
                    self.player.seek_to(Duration::ZERO);
                } else if self.queue.back().is_some() {
                    self.play_current();
                }
            }
            VolumeUp => self.player.set_volume(self.player.volume() + VOLUME_STEP),
            VolumeDown => self.player.set_volume(self.player.volume() - VOLUME_STEP),
            SeekForward => self.player.seek_by(SEEK_STEP),
            SeekBackward => self.player.seek_by(-SEEK_STEP),
            ToggleShuffle => self.queue.toggle_shuffle(),
            CycleRepeat => self.queue.repeat = self.queue.repeat.cycle(),
            StartFilter => {
                self.focus = Focus::Browser;
                self.filter_mode = true;
            }
            ClearFilter => self.browser.set_filter(String::new()),
            Remove => self.remove_from_queue(),
            MoveDown => self.move_in_queue(1),
            MoveUp => self.move_in_queue(-1),
            Quit => self.quit = true,
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        match self.focus {
            Focus::Browser => self.browser.move_by(delta),
            Focus::Queue => {
                let max = self.queue.len().saturating_sub(1) as isize;
                self.queue_cursor = (self.queue_cursor as isize)
                    .saturating_add(delta)
                    .clamp(0, max) as usize;
            }
        }
    }

    fn open(&mut self) {
        match self.focus {
            Focus::Browser => {
                let Some(entry) = self.browser.selected().cloned() else {
                    return;
                };
                if entry.is_dir {
                    self.browser.cd(entry.path);
                } else if is_audio(&entry.path) {
                    // The whole folder becomes the queue, starting at the selected track.
                    let tracks = self.browser.audio_files();
                    let start = tracks.iter().position(|p| *p == entry.path);
                    self.queue.set(tracks, start);
                    self.queue_cursor = start.unwrap_or(0);
                    self.play_current();
                }
            }
            Focus::Queue => {
                if self.queue_cursor < self.queue.len() {
                    self.queue.jump(self.queue_cursor);
                    self.play_current();
                }
            }
        }
    }

    fn remove_from_queue(&mut self) {
        if self.focus != Focus::Queue || self.queue.is_empty() {
            return;
        }
        if self.queue.remove(self.queue_cursor) {
            if self.queue.current().is_some() {
                self.play_current();
            } else {
                self.player.stop();
            }
        }
        self.queue_cursor = self.queue_cursor.min(self.queue.len().saturating_sub(1));
    }

    fn move_in_queue(&mut self, delta: isize) {
        if self.focus != Focus::Queue {
            return;
        }
        let target = self.queue_cursor as isize + delta;
        if target < 0 || target as usize >= self.queue.len() {
            return;
        }
        self.queue.swap(self.queue_cursor, target as usize);
        self.queue_cursor = target as usize;
    }

    fn play_current(&mut self) {
        self.load_current(Duration::ZERO, false);
    }

    fn load_current(&mut self, start: Duration, paused: bool) {
        let Some(path) = self.queue.current().cloned() else {
            return;
        };
        if let Err(e) = self.player.load(&path, start, paused) {
            self.message = Some(format!("{e:#}"));
            self.player.stop();
        }
    }
}
