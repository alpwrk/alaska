//! Ogg Opus decoding via libopus (symphonia has no Opus decoder).

use std::io::{Read, Seek};
use std::num::NonZero;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use ogg::PacketReader;
use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};

/// Opus always decodes at 48 kHz; granule positions count samples at this rate.
const RATE: u32 = 48_000;
/// Largest Opus frame: 120 ms at 48 kHz.
const MAX_FRAME: usize = 5760;

pub struct OpusSource<R: Read + Seek> {
    reader: PacketReader<R>,
    decoder: opus::Decoder,
    serial: u32,
    channels: u16,
    pre_skip: u64,
    duration: Option<Duration>,
    buf: Vec<f32>,
    buf_pos: usize,
    /// Samples (per channel) still to drop, for pre-skip after start or seek.
    skip: usize,
    done: bool,
}

impl<R: Read + Seek> OpusSource<R> {
    pub fn new(input: R, duration: Option<Duration>) -> Result<Self> {
        let mut reader = PacketReader::new(input);

        let head = reader.read_packet()?.context("empty ogg stream")?;
        let data = &head.data;
        if data.len() < 19 || &data[..8] != b"OpusHead" {
            bail!("not an Ogg Opus file");
        }
        let channels = data[9] as u16;
        let pre_skip = u16::from_le_bytes([data[10], data[11]]) as u64;
        let mapping_family = data[18];
        if mapping_family != 0 || !(1..=2).contains(&channels) {
            bail!("unsupported Opus channel layout ({channels} channels)");
        }
        // The second packet is OpusTags; skip it.
        reader.read_packet()?.context("missing OpusTags")?;

        let ch = if channels == 1 {
            opus::Channels::Mono
        } else {
            opus::Channels::Stereo
        };
        Ok(OpusSource {
            reader,
            decoder: opus::Decoder::new(RATE, ch)?,
            serial: head.stream_serial(),
            channels,
            pre_skip,
            duration,
            buf: Vec::with_capacity(MAX_FRAME * channels as usize),
            buf_pos: 0,
            skip: pre_skip as usize,
            done: false,
        })
    }

    /// Decode the next packet into `buf`. Returns false at end of stream.
    fn fill(&mut self) -> bool {
        loop {
            let packet = match self.reader.read_packet() {
                Ok(Some(p)) => p,
                _ => return false,
            };
            if packet.stream_serial() != self.serial {
                continue;
            }
            let ch = self.channels as usize;
            self.buf.resize(MAX_FRAME * ch, 0.0);
            let Ok(frames) = self
                .decoder
                .decode_float(&packet.data, &mut self.buf, false)
            else {
                // Skip corrupt packets instead of ending playback.
                continue;
            };
            self.buf.truncate(frames * ch);
            let drop = self.skip.min(frames);
            self.skip -= drop;
            self.buf_pos = drop * ch;
            if self.buf_pos < self.buf.len() {
                return true;
            }
        }
    }
}

impl<R: Read + Seek> Iterator for OpusSource<R> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.done {
            return None;
        }
        if self.buf_pos >= self.buf.len() && !self.fill() {
            self.done = true;
            return None;
        }
        let sample = self.buf[self.buf_pos];
        self.buf_pos += 1;
        Some(sample)
    }
}

impl<R: Read + Seek> Source for OpusSource<R> {
    fn current_span_len(&self) -> Option<usize> {
        if self.done { Some(0) } else { None }
    }

    fn channels(&self) -> ChannelCount {
        NonZero::new(self.channels).expect("channels checked in open")
    }

    fn sample_rate(&self) -> SampleRate {
        NonZero::new(RATE).unwrap()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.duration
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let target = (pos.as_secs_f64() * RATE as f64) as u64 + self.pre_skip;
        let found = self
            .reader
            .seek_absgp(Some(self.serial), target)
            .map_err(|e| SeekError::Other(Arc::new(e)))?;
        if !found {
            // Past the end: finish the track.
            self.done = true;
            return Ok(());
        }
        // Page-accurate seek is good enough for a music player.
        self.decoder
            .reset_state()
            .map_err(|e| SeekError::Other(Arc::new(e)))?;
        self.buf.clear();
        self.buf_pos = 0;
        self.skip = 0;
        self.done = false;
        Ok(())
    }
}
