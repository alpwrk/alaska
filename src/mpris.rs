//! MPRIS integration: media keys, playerctl and desktop widgets.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};

use crate::meta::TagCache;
use crate::track::Track;

/// How often the reported position is refreshed while playing.
const POSITION_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Status {
    Stopped,
    Paused,
    Playing,
}

pub struct Mpris {
    controls: MediaControls,
    events: Receiver<MediaControlEvent>,
    track: Option<Track>,
    status: Status,
    volume: f32,
    last_position: Duration,
    last_position_update: Instant,
}

/// Snapshot of what the player is doing, pushed to MPRIS each tick.
pub struct Playback<'a> {
    pub track: Option<&'a Track>,
    pub cache: &'a TagCache,
    pub paused: bool,
    pub position: Duration,
    pub volume: f32,
}

impl Mpris {
    /// Connect to the session bus. Returns `None` if that is not possible
    /// (no D-Bus session, or another instance already owns the name).
    pub fn new() -> Option<Self> {
        let mut controls = MediaControls::new(PlatformConfig {
            display_name: "alaska",
            dbus_name: "alaska",
            hwnd: None,
        })
        .ok()?;
        let (tx, events) = mpsc::channel();
        controls
            .attach(move |event| {
                let _ = tx.send(event);
            })
            .ok()?;
        Some(Mpris {
            controls,
            events,
            track: None,
            status: Status::Stopped,
            volume: -1.0,
            last_position: Duration::ZERO,
            last_position_update: Instant::now(),
        })
    }

    pub fn events(&self) -> impl Iterator<Item = MediaControlEvent> + '_ {
        self.events.try_iter()
    }

    /// Push changes to the bus. Cheap when nothing changed.
    pub fn update(&mut self, p: Playback) {
        let track_changed = self.track.as_ref() != p.track;
        if track_changed {
            self.track = p.track.cloned();
            let (title, artist, album, duration) = match p.track {
                Some(t) => (
                    t.title(p.cache),
                    t.artist(p.cache),
                    t.album(p.cache),
                    t.duration(p.cache),
                ),
                None => (None, None, None, None),
            };
            let _ = self.controls.set_metadata(MediaMetadata {
                title: title.as_deref(),
                artist: artist.as_deref(),
                album: album.as_deref(),
                duration,
                cover_url: None,
            });
        }

        let status = match (p.track, p.paused) {
            (None, _) => Status::Stopped,
            (Some(_), true) => Status::Paused,
            (Some(_), false) => Status::Playing,
        };
        let elapsed = self.last_position_update.elapsed();
        let position_due = status == Status::Playing && elapsed >= POSITION_INTERVAL;
        // A position far from where playback should be means a seek happened.
        let expected = match status {
            Status::Playing => self.last_position + elapsed,
            _ => self.last_position,
        };
        let seeked = p.position.abs_diff(expected) > Duration::from_millis(1500);
        if track_changed || status != self.status || position_due || seeked {
            self.status = status;
            self.last_position = p.position;
            self.last_position_update = Instant::now();
            let progress = Some(MediaPosition(p.position));
            let _ = self.controls.set_playback(match status {
                Status::Stopped => MediaPlayback::Stopped,
                Status::Paused => MediaPlayback::Paused { progress },
                Status::Playing => MediaPlayback::Playing { progress },
            });
        }

        if (self.volume - p.volume).abs() > f32::EPSILON {
            self.volume = p.volume;
            let _ = self.controls.set_volume(p.volume as f64);
        }
    }
}
