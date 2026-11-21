use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::widgets::ListState;
use souvlaki::{MediaControlEvent, SeekDirection};

use crate::action::{Action, map_key};
use crate::browser::{Browser, is_audio};
use crate::config::Config;
use crate::login::{self, Login};
use crate::meta::TagCache;
use crate::mpris::{Mpris, Playback};
use crate::player::Player;
use crate::queue::Queue;
use crate::remote::Remote;
use crate::state::{RemoteState, State};
use crate::subsonic::Client;
use crate::track::Track;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Local,
    Remote,
}

/// Queue and playback position of one mode.
#[derive(Default)]
pub struct Session {
    pub queue: Queue,
    pub cursor: usize,
    /// Where to continue when switching back to this mode.
    resume: Duration,
}

/// What the text prompt at the bottom of the browser is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// Live filter of the local directory.
    Filter,
    /// Server search, run on Enter.
    Search,
}

pub struct App {
    pub mode: Mode,
    pub browser: Browser,
    pub remote: Option<Remote>,
    pub login: Option<Login>,
    config: Config,
    local: Session,
    remote_session: Session,
    pub player: Player,
    pub cache: TagCache,
    /// Directory whose tags were last loaded into the cache.
    meta_dir: Option<PathBuf>,
    mpris: Option<Mpris>,
    pub focus: Focus,
    pub prompt: Option<Prompt>,
    pub search_input: String,
    pub message: Option<String>,
    pub browser_state: ListState,
    pub queue_state: ListState,
    quit: bool,
}

impl App {
    pub fn new(start_dir: PathBuf) -> Result<Self> {
        let (config, config_error) = match Config::load() {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(format!("{e:#}"))),
        };
        let mut app = App {
            mode: Mode::Local,
            browser: Browser::new(start_dir),
            remote: None,
            login: None,
            config,
            local: Session::default(),
            remote_session: Session::default(),
            player: Player::new()?,
            cache: TagCache::load(),
            meta_dir: None,
            mpris: Mpris::new(),
            focus: Focus::Browser,
            prompt: None,
            search_input: String::new(),
            message: config_error,
            browser_state: ListState::default(),
            queue_state: ListState::default(),
            quit: false,
        };
        app.restore(State::load());
        Ok(app)
    }

    pub fn session(&self) -> &Session {
        match self.mode {
            Mode::Local => &self.local,
            Mode::Remote => &self.remote_session,
        }
    }

    fn session_mut(&mut self) -> &mut Session {
        match self.mode {
            Mode::Local => &mut self.local,
            Mode::Remote => &mut self.remote_session,
        }
    }

    pub fn queue(&self) -> &Queue {
        &self.session().queue
    }

    fn queue_mut(&mut self) -> &mut Queue {
        &mut self.session_mut().queue
    }

    fn client(&self) -> Option<&Client> {
        self.remote.as_ref().map(|r| &r.client)
    }

    /// Restore volume and both sessions. Only the local track is loaded (paused);
    /// the remote one is loaded when switching to OSS mode.
    fn restore(&mut self, state: State) {
        self.player.set_volume(state.volume);
        let position = |secs: f64| Duration::try_from_secs_f64(secs).unwrap_or_default();

        self.cache.ensure(&state.queue);
        let local = &mut self.local;
        local.queue.repeat = state.repeat;
        local.queue.set_shuffle(state.shuffle);
        local.resume = position(state.position_secs);
        local.queue.set(
            state.queue.into_iter().map(Track::Local).collect(),
            state.current,
        );
        local.cursor = local.queue.current_index().unwrap_or(0);

        let r = state.remote;
        let remote = &mut self.remote_session;
        remote.queue.repeat = r.repeat;
        remote.queue.set_shuffle(r.shuffle);
        remote.resume = position(r.position_secs);
        remote
            .queue
            .set(r.queue.into_iter().map(Track::Remote).collect(), r.current);
        remote.cursor = remote.queue.current_index().unwrap_or(0);

        self.resume_session();
    }

    fn snapshot(&mut self) -> State {
        self.save_position();
        let local = &self.local;
        let remote = &self.remote_session;
        State {
            volume: self.player.volume(),
            queue: local
                .queue
                .tracks()
                .iter()
                .filter_map(|t| t.local_path().map(PathBuf::from))
                .collect(),
            current: local.queue.current_index(),
            position_secs: local.resume.as_secs_f64(),
            shuffle: local.queue.shuffle(),
            repeat: local.queue.repeat,
            remote: RemoteState {
                queue: remote
                    .queue
                    .tracks()
                    .iter()
                    .filter_map(|t| match t {
                        Track::Remote(s) => Some(s.clone()),
                        Track::Local(_) => None,
                    })
                    .collect(),
                current: remote.queue.current_index(),
                position_secs: remote.resume.as_secs_f64(),
                shuffle: remote.queue.shuffle(),
                repeat: remote.queue.repeat,
            },
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
            self.sync_mpris();
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

    pub fn label(&self, track: &Track) -> String {
        track.label(&self.cache)
    }

    pub fn track_duration(&self, track: &Track) -> Option<Duration> {
        track.duration(&self.cache)
    }

    fn tick(&mut self) {
        if let Some(remote) = &mut self.remote
            && let Some(err) = remote.poll()
        {
            self.message = Some(err);
        }
        if let Some(login) = &mut self.login
            && let login::Outcome::LoggedIn(client) = login.poll()
        {
            self.logged_in(client);
        }
        if self.player.is_finished() {
            match self.queue_mut().advance(true) {
                Some(_) => self.play_current(),
                None => self.player.stop(),
            }
        }
    }

    fn sync_mpris(&mut self) {
        let Some(mpris) = &self.mpris else {
            return;
        };
        let events: Vec<_> = mpris.events().collect();
        for event in events {
            self.handle_media_event(event);
        }
        let track = self
            .queue()
            .current()
            .filter(|_| self.player.is_loaded())
            .cloned();
        let playback = Playback {
            track: track.as_ref(),
            cache: &self.cache,
            paused: self.player.is_paused(),
            position: self.player.position(),
            volume: self.player.volume(),
        };
        if let Some(mpris) = &mut self.mpris {
            mpris.update(playback);
        }
    }

    fn handle_media_event(&mut self, event: MediaControlEvent) {
        let signed = |dir: SeekDirection, secs: i64| match dir {
            SeekDirection::Forward => secs,
            SeekDirection::Backward => -secs,
        };
        match event {
            MediaControlEvent::Play => {
                if self.player.is_loaded() {
                    self.player.play();
                } else {
                    self.play_current();
                }
            }
            MediaControlEvent::Pause | MediaControlEvent::Stop => self.player.pause(),
            MediaControlEvent::Toggle => self.dispatch(Action::PlayPause),
            MediaControlEvent::Next => self.dispatch(Action::Next),
            MediaControlEvent::Previous => self.dispatch(Action::Prev),
            MediaControlEvent::Seek(dir) => self.player.seek_by(signed(dir, SEEK_STEP)),
            MediaControlEvent::SeekBy(dir, d) => {
                self.player.seek_by(signed(dir, d.as_secs() as i64));
            }
            MediaControlEvent::SetPosition(pos) => self.player.seek_to(pos.0),
            MediaControlEvent::SetVolume(v) => self.player.set_volume(v as f32),
            MediaControlEvent::SetShuffle(on) => self.queue_mut().set_shuffle(on),
            MediaControlEvent::Quit => self.quit = true,
            _ => {}
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if let Some(login) = &mut self.login {
            match login.handle_key(key) {
                login::Outcome::Cancelled => self.login = None,
                login::Outcome::LoggedIn(client) => self.logged_in(client),
                login::Outcome::Pending => {}
            }
        } else if let Some(prompt) = self.prompt {
            self.handle_prompt_key(prompt, key);
        } else if let Some(action) = map_key(key) {
            self.message = None;
            self.dispatch(action);
        }
    }

    fn handle_prompt_key(&mut self, prompt: Prompt, key: KeyEvent) {
        match prompt {
            Prompt::Filter => {
                let mut filter = self.browser.filter.clone();
                match key.code {
                    KeyCode::Esc => {
                        self.prompt = None;
                        filter.clear();
                    }
                    KeyCode::Enter => {
                        self.prompt = None;
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
            Prompt::Search => match key.code {
                KeyCode::Esc => self.prompt = None,
                KeyCode::Enter => {
                    self.prompt = None;
                    let query = self.search_input.trim().to_string();
                    if let Some(remote) = &mut self.remote
                        && !query.is_empty()
                    {
                        remote.search(query);
                    }
                }
                KeyCode::Backspace => {
                    self.search_input.pop();
                }
                KeyCode::Char(c) => self.search_input.push(c),
                _ => {}
            },
        }
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
                    match (self.mode, &mut self.remote) {
                        (Mode::Remote, Some(remote)) => remote.back(),
                        _ => self.browser.parent(),
                    }
                }
            }
            Home => {
                self.focus = Focus::Browser;
                match (self.mode, &mut self.remote) {
                    (Mode::Remote, Some(remote)) => remote.home(),
                    _ => {
                        if let Some(home) = dirs::home_dir() {
                            self.browser.cd(home);
                        }
                    }
                }
            }
            ToggleFocus => {
                self.focus = match self.focus {
                    Focus::Browser => Focus::Queue,
                    Focus::Queue => Focus::Browser,
                };
                if let Some(i) = self.queue().current_index()
                    && self.focus == Focus::Queue
                {
                    self.session_mut().cursor = i;
                }
            }
            PlayPause => {
                if self.player.is_loaded() {
                    self.player.toggle_pause();
                } else if self.queue().current().is_some() {
                    self.play_current();
                }
            }
            Next => {
                if self.queue_mut().advance(false).is_some() {
                    self.play_current();
                }
            }
            Prev => {
                // Like most players: restart the track unless we are near its start.
                if self.player.position() > Duration::from_secs(3) {
                    self.player.seek_to(Duration::ZERO);
                } else if self.queue_mut().back().is_some() {
                    self.play_current();
                }
            }
            VolumeUp => self.player.set_volume(self.player.volume() + VOLUME_STEP),
            VolumeDown => self.player.set_volume(self.player.volume() - VOLUME_STEP),
            SeekForward => self.player.seek_by(SEEK_STEP),
            SeekBackward => self.player.seek_by(-SEEK_STEP),
            ToggleShuffle => self.queue_mut().toggle_shuffle(),
            CycleRepeat => {
                let q = self.queue_mut();
                q.repeat = q.repeat.cycle();
            }
            StartFilter => {
                self.focus = Focus::Browser;
                match self.mode {
                    Mode::Local => self.prompt = Some(Prompt::Filter),
                    Mode::Remote if self.remote.is_some() => {
                        self.search_input.clear();
                        self.prompt = Some(Prompt::Search);
                    }
                    Mode::Remote => {}
                }
            }
            ClearFilter => {
                if self.mode == Mode::Local {
                    self.browser.set_filter(String::new());
                }
            }
            Remove => self.remove_from_queue(),
            MoveDown => self.move_in_queue(1),
            MoveUp => self.move_in_queue(-1),
            ToggleMode => self.toggle_mode(),
            Action::Login => self.login = Some(login::Login::new(self.config.subsonic.as_ref())),
            Quit => self.quit = true,
        }
    }

    fn toggle_mode(&mut self) {
        match self.mode {
            Mode::Remote => self.switch_mode(Mode::Local),
            Mode::Local => {
                if self.remote.is_none() {
                    let Some(server) = self.config.subsonic.clone() else {
                        self.login = Some(Login::new(None));
                        return;
                    };
                    self.remote = Some(Remote::new(Client::new(server)));
                }
                self.switch_mode(Mode::Remote);
            }
        }
    }

    fn logged_in(&mut self, client: Client) {
        self.login = None;
        let changed = self.config.subsonic.as_ref() != Some(client.config());
        self.config.subsonic = Some(client.config().clone());
        if let Err(e) = self.config.save() {
            self.message = Some(format!("cannot save config: {e:#}"));
        }
        if changed {
            // Another server or user: the old remote queue no longer applies.
            if self.mode == Mode::Remote {
                self.player.stop();
            }
            self.remote_session = Session::default();
        }
        self.remote = Some(Remote::new(client));
        if self.mode == Mode::Remote {
            self.resume_session();
        } else {
            self.switch_mode(Mode::Remote);
        }
    }

    /// Pause the active mode, remember its position, and resume `mode` paused.
    fn switch_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        self.save_position();
        self.player.stop();
        self.mode = mode;
        self.prompt = None;
        self.resume_session();
    }

    fn save_position(&mut self) {
        if self.player.is_loaded() {
            let pos = self.player.position();
            self.session_mut().resume = pos;
        }
    }

    /// Load the active session's current track paused at its saved position.
    fn resume_session(&mut self) {
        if self.queue().current().is_some() && !self.player.is_loaded() {
            let resume = self.session().resume;
            self.load_current(resume, true);
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        match (self.focus, self.mode) {
            (Focus::Browser, Mode::Local) => self.browser.move_by(delta),
            (Focus::Browser, Mode::Remote) => {
                if let Some(remote) = &mut self.remote {
                    remote.move_by(delta);
                }
            }
            (Focus::Queue, _) => {
                let max = self.queue().len().saturating_sub(1) as isize;
                let session = self.session_mut();
                session.cursor = (session.cursor as isize)
                    .saturating_add(delta)
                    .clamp(0, max) as usize;
            }
        }
    }

    fn open(&mut self) {
        match (self.focus, self.mode) {
            (Focus::Browser, Mode::Local) => {
                let Some(entry) = self.browser.selected().cloned() else {
                    return;
                };
                if entry.is_dir {
                    self.browser.cd(entry.path);
                } else if is_audio(&entry.path) {
                    // The whole folder becomes the queue, starting at the selected track.
                    let tracks = self.browser.audio_files();
                    let start = tracks.iter().position(|p| *p == entry.path);
                    self.start_queue(tracks.into_iter().map(Track::Local).collect(), start);
                }
            }
            (Focus::Browser, Mode::Remote) => {
                let Some(remote) = &mut self.remote else {
                    return;
                };
                if let Some(song) = remote.open() {
                    // All songs of the current view become the queue.
                    let songs = remote.view().map(|v| v.songs()).unwrap_or_default();
                    let start = songs.iter().position(|s| s.id == song.id);
                    self.start_queue(songs.into_iter().map(Track::Remote).collect(), start);
                }
            }
            (Focus::Queue, _) => {
                let cursor = self.session().cursor;
                if cursor < self.queue().len() {
                    self.queue_mut().jump(cursor);
                    self.play_current();
                }
            }
        }
    }

    fn start_queue(&mut self, tracks: Vec<Track>, start: Option<usize>) {
        let session = self.session_mut();
        session.queue.set(tracks, start);
        session.cursor = start.unwrap_or(0);
        self.play_current();
    }

    fn remove_from_queue(&mut self) {
        if self.focus != Focus::Queue || self.queue().is_empty() {
            return;
        }
        let cursor = self.session().cursor;
        if self.queue_mut().remove(cursor) {
            if self.queue().current().is_some() {
                self.play_current();
            } else {
                self.player.stop();
            }
        }
        let max = self.queue().len().saturating_sub(1);
        let session = self.session_mut();
        session.cursor = session.cursor.min(max);
    }

    fn move_in_queue(&mut self, delta: isize) {
        if self.focus != Focus::Queue {
            return;
        }
        let cursor = self.session().cursor;
        let target = cursor as isize + delta;
        if target < 0 || target as usize >= self.queue().len() {
            return;
        }
        let session = self.session_mut();
        session.queue.swap(cursor, target as usize);
        session.cursor = target as usize;
    }

    fn play_current(&mut self) {
        self.load_current(Duration::ZERO, false);
    }

    fn load_current(&mut self, start: Duration, paused: bool) {
        let Some(track) = self.queue().current().cloned() else {
            return;
        };
        let client = self.client().cloned();
        if let Err(e) = self.player.load(&track, client.as_ref(), start, paused) {
            self.message = Some(format!("{e:#}"));
            self.player.stop();
        }
    }
}
