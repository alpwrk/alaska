//! Decode a source on its own thread so slow input (network streams) can
//! never block the audio callback.

use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::thread;
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};

/// Samples per message.
const CHUNK: usize = 4096;
/// Messages buffered ahead (about 3 s of 44.1 kHz stereo).
const AHEAD: usize = 64;

enum Msg {
    Samples(u64, Vec<Sample>),
    End(u64),
}

/// Seek to a position; the `u64` is the new generation.
struct SeekCmd(Duration, u64);

pub struct Prefetch {
    rx: Receiver<Msg>,
    cmd: Sender<SeekCmd>,
    chunk: Vec<Sample>,
    pos: usize,
    /// Messages from older generations (before a seek) are dropped.
    generation: u64,
    channels: ChannelCount,
    sample_rate: SampleRate,
    duration: Option<Duration>,
    done: bool,
}

impl Prefetch {
    pub fn new(mut inner: Box<dyn Source + Send>) -> Self {
        let channels = inner.channels();
        let sample_rate = inner.sample_rate();
        let duration = inner.total_duration();
        let (tx, rx) = mpsc::sync_channel(AHEAD);
        let (cmd_tx, cmd_rx) = mpsc::channel();
        thread::spawn(move || decode_loop(&mut *inner, &tx, &cmd_rx));
        Prefetch {
            rx,
            cmd: cmd_tx,
            chunk: Vec::new(),
            pos: 0,
            generation: 0,
            channels,
            sample_rate,
            duration,
            done: false,
        }
    }
}

fn decode_loop(inner: &mut dyn Source, tx: &SyncSender<Msg>, cmds: &Receiver<SeekCmd>) {
    let mut generation = 0;
    let mut ended = false;
    loop {
        // Apply the newest pending seek, if any.
        let mut seek = None;
        loop {
            match cmds.try_recv() {
                Ok(cmd) => seek = Some(cmd),
                Err(TryRecvError::Empty) => break,
                // The source was dropped.
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if ended && seek.is_none() {
            // Wait for a seek back into the track, or exit when dropped.
            match cmds.recv() {
                Ok(cmd) => seek = Some(cmd),
                Err(_) => return,
            }
        }
        if let Some(SeekCmd(pos, g)) = seek {
            generation = g;
            ended = false;
            let _ = inner.try_seek(pos);
        }

        let mut chunk = Vec::with_capacity(CHUNK);
        while chunk.len() < CHUNK {
            match inner.next() {
                Some(s) => chunk.push(s),
                None => break,
            }
        }
        let msg = if chunk.is_empty() {
            ended = true;
            Msg::End(generation)
        } else {
            Msg::Samples(generation, chunk)
        };
        if tx.send(msg).is_err() {
            return;
        }
    }
}

impl Iterator for Prefetch {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        loop {
            if self.done {
                return None;
            }
            if self.pos < self.chunk.len() {
                let s = self.chunk[self.pos];
                self.pos += 1;
                return Some(s);
            }
            match self.rx.try_recv() {
                Ok(Msg::Samples(g, chunk)) if g == self.generation => {
                    self.chunk = chunk;
                    self.pos = 0;
                }
                Ok(Msg::End(g)) if g == self.generation => self.done = true,
                // Stale data from before a seek.
                Ok(_) => {}
                // Decoder is behind: play silence instead of blocking the audio thread.
                Err(TryRecvError::Empty) => return Some(0.0),
                Err(TryRecvError::Disconnected) => self.done = true,
            }
        }
    }
}

impl Source for Prefetch {
    fn current_span_len(&self) -> Option<usize> {
        if self.done { Some(0) } else { None }
    }

    fn channels(&self) -> ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.duration
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.generation += 1;
        self.chunk.clear();
        self.pos = 0;
        self.done = false;
        let _ = self.cmd.send(SeekCmd(pos, self.generation));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::source::SineWave;
    use std::time::Instant;

    fn source() -> Box<dyn Source + Send> {
        Box::new(SineWave::new(440.0).take_duration(Duration::from_millis(100)))
    }

    /// Pull samples, skipping silence produced while the decoder catches up.
    fn drain(p: &mut Prefetch) -> usize {
        let start = Instant::now();
        let mut n = 0;
        while let Some(s) = p.next() {
            if s != 0.0 {
                n += 1;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "never ended");
        }
        n
    }

    #[test]
    fn plays_everything_then_ends() {
        let mut p = Prefetch::new(source());
        assert!(drain(&mut p) > 4000);
        assert_eq!(p.next(), None);
    }

    #[test]
    fn seek_after_end_restarts() {
        let mut p = Prefetch::new(source());
        let full = drain(&mut p);
        p.try_seek(Duration::from_millis(50)).unwrap();
        let rest = drain(&mut p);
        assert!(rest > 0 && rest < full, "rest {rest}, full {full}");
    }
}
