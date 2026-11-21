use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::time::Duration;

use anyhow::{Context, Result};
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Source};

use crate::meta::TrackMeta;
use crate::opus::OpusSource;
use crate::prefetch::Prefetch;
use crate::subsonic::Client;
use crate::subsonic::stream::HttpStream;
use crate::track::Track;

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
    pub fn load(
        &mut self,
        track: &Track,
        client: Option<&Client>,
        start: Duration,
        paused: bool,
    ) -> Result<()> {
        self.stop();
        let source = open_source(track, client)?;
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
        // Round to whole percent so repeated steps do not accumulate float drift.
        self.volume = (volume.clamp(0.0, 1.0) * 100.0).round() / 100.0;
        if let Some(t) = &self.track {
            t.set_volume(self.volume);
        }
    }
}

fn open_source(track: &Track, client: Option<&Client>) -> Result<Box<dyn Source + Send>> {
    match track {
        Track::Local(path) => {
            let file =
                File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
            let len = file.metadata()?.len();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();
            let duration = || TrackMeta::read(path).duration();
            open_input(BufReader::new(file), Some(len), &ext, duration)
                .with_context(|| format!("cannot play {}", path.display()))
        }
        Track::Remote(song) => {
            let client = client.context("not connected to a server")?;
            let stream = HttpStream::open(client.agent(), &client.stream_url(&song.id))?;
            let len = stream.len();
            let ext = song
                .suffix
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let source = open_input(stream, len, &ext, || song.duration())
                .with_context(|| format!("cannot play {}", song.title))?;
            // Network reads may stall; keep them off the audio thread.
            Ok(Box::new(Prefetch::new(source)))
        }
    }
}

/// Pick a decoder for `input`: libopus for Opus, symphonia for everything else.
fn open_input<R>(
    mut input: R,
    byte_len: Option<u64>,
    ext: &str,
    duration: impl FnOnce() -> Option<Duration>,
) -> Result<Box<dyn Source + Send>>
where
    R: Read + Seek + Send + Sync + 'static,
{
    // Ogg files may contain Opus, which symphonia cannot decode.
    if ext == "opus" || (matches!(ext, "ogg" | "oga") && is_ogg_opus(&mut input)?) {
        return Ok(Box::new(OpusSource::new(input, duration())?));
    }
    let mut builder = Decoder::builder()
        .with_data(input)
        .with_seekable(true)
        .with_gapless(true);
    if let Some(len) = byte_len {
        builder = builder.with_byte_len(len);
    }
    if !ext.is_empty() {
        builder = builder.with_hint(ext);
    }
    Ok(Box::new(builder.build()?))
}

/// Check whether an Ogg stream starts with an Opus header, then rewind.
fn is_ogg_opus<R: Read + Seek>(input: &mut R) -> Result<bool> {
    let mut head = [0u8; 64];
    let mut n = 0;
    while n < head.len() {
        match input.read(&mut head[n..])? {
            0 => break,
            k => n += k,
        }
    }
    input.seek(SeekFrom::Start(0))?;
    Ok(head[..n].windows(8).any(|w| w == b"OpusHead"))
}
