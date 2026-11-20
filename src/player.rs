use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Source};

/// Thin wrapper around a rodio output device and a per-track `rodio::Player`.
pub struct Player {
    device: MixerDeviceSink,
    track: Option<rodio::Player>,
    duration: Option<Duration>,
    volume: f32,
}

impl Player {
    pub fn new() -> Result<Self> {
        let mut device =
            DeviceSinkBuilder::open_default_sink().context("failed to open audio output")?;
        device.log_on_drop(false);
        Ok(Player {
            device,
            track: None,
            duration: None,
            volume: 0.8,
        })
    }

    /// Replace the current track with `path`, starting at `start`.
    pub fn load(&mut self, path: &Path, start: Duration, paused: bool) -> Result<()> {
        self.stop();
        let source = open_source(path)?;
        self.duration = source.total_duration();

        let track = rodio::Player::connect_new(self.device.mixer());
        track.set_volume(self.volume);
        if paused {
            track.pause();
        }
        track.append(source);
        if !start.is_zero() {
            // Not all sources can seek; starting from the beginning is an acceptable fallback.
            let _ = track.try_seek(start);
        }
        self.track = Some(track);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.track = None;
        self.duration = None;
    }

    pub fn is_loaded(&self) -> bool {
        self.track.is_some()
    }

    /// True if a track was loaded and has played to its end.
    pub fn is_finished(&self) -> bool {
        self.track.as_ref().is_some_and(rodio::Player::empty)
    }

    pub fn is_paused(&self) -> bool {
        self.track.as_ref().is_none_or(rodio::Player::is_paused)
    }

    pub fn toggle_pause(&self) {
        if let Some(t) = &self.track {
            if t.is_paused() { t.play() } else { t.pause() }
        }
    }

    pub fn play(&self) {
        if let Some(t) = &self.track {
            t.play();
        }
    }

    pub fn pause(&self) {
        if let Some(t) = &self.track {
            t.pause();
        }
    }

    pub fn position(&self) -> Duration {
        self.track
            .as_ref()
            .map_or(Duration::ZERO, rodio::Player::get_pos)
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    pub fn seek_to(&self, pos: Duration) {
        let pos = match self.duration {
            Some(d) => pos.min(d.saturating_sub(Duration::from_millis(500))),
            None => pos,
        };
        if let Some(t) = &self.track {
            let _ = t.try_seek(pos);
        }
    }

    pub fn seek_by(&self, secs: i64) {
        let pos = self.position();
        let delta = Duration::from_secs(secs.unsigned_abs());
        let target = if secs < 0 {
            pos.saturating_sub(delta)
        } else {
            pos + delta
        };
        self.seek_to(target);
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(t) = &self.track {
            t.set_volume(self.volume);
        }
    }
}

fn open_source(path: &Path) -> Result<Box<dyn Source + Send>> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let len = file.metadata()?.len();
    let mut builder = Decoder::builder()
        .with_data(BufReader::new(file))
        .with_byte_len(len)
        .with_seekable(true)
        .with_gapless(true);
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        builder = builder.with_hint(ext);
    }
    let decoder = builder
        .build()
        .with_context(|| format!("cannot decode {}", path.display()))?;
    Ok(Box::new(decoder))
}
