//! Progressive HTTP download exposed as `Read + Seek` for the decoders.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use anyhow::{Context, Result};

const CHUNK: usize = 64 * 1024;

#[derive(Default)]
struct Buffer {
    data: Vec<u8>,
    done: bool,
    error: Option<String>,
}

struct Shared {
    buffer: Mutex<Buffer>,
    ready: Condvar,
    cancel: AtomicBool,
}

/// A file being downloaded in the background. Reads block until the requested
/// bytes have arrived; seeking is free and reads after it wait as needed.
pub struct HttpStream {
    shared: Arc<Shared>,
    pos: u64,
    len: Option<u64>,
}

impl HttpStream {
    /// Start downloading `url`. Returns once the response headers are in.
    pub fn open(agent: &ureq::Agent, url: &str) -> Result<Self> {
        let response = agent.get(url).call().context("stream request failed")?;
        let body = response.into_body();
        let len = body.content_length();
        let mut reader = body.into_reader();

        let shared = Arc::new(Shared {
            buffer: Mutex::new(Buffer {
                data: Vec::with_capacity(len.unwrap_or(0) as usize),
                ..Buffer::default()
            }),
            ready: Condvar::new(),
            cancel: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        thread::spawn(move || {
            let mut chunk = vec![0u8; CHUNK];
            loop {
                if worker.cancel.load(Ordering::Relaxed) {
                    break;
                }
                let result = reader.read(&mut chunk);
                let mut buf = worker.buffer.lock().unwrap();
                match result {
                    Ok(0) => {
                        buf.done = true;
                    }
                    Ok(n) => buf.data.extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        buf.error = Some(e.to_string());
                        buf.done = true;
                    }
                }
                let done = buf.done;
                drop(buf);
                worker.ready.notify_all();
                if done {
                    break;
                }
            }
        });

        Ok(HttpStream {
            shared,
            pos: 0,
            len,
        })
    }

    /// Total length if known (from `Content-Length`).
    pub fn len(&self) -> Option<u64> {
        self.len
    }

    /// Block until the download has finished and return the final length.
    fn wait_for_end(&self) -> u64 {
        let mut buf = self.shared.buffer.lock().unwrap();
        while !buf.done {
            buf = self.shared.ready.wait(buf).unwrap();
        }
        buf.data.len() as u64
    }
}

impl Read for HttpStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let mut buf = self.shared.buffer.lock().unwrap();
        while (buf.data.len() as u64) <= self.pos && !buf.done {
            buf = self.shared.ready.wait(buf).unwrap();
        }
        let available = buf.data.len() as u64;
        if self.pos >= available {
            return match &buf.error {
                Some(e) => Err(io::Error::other(e.clone())),
                None => Ok(0),
            };
        }
        let start = self.pos as usize;
        let n = out.len().min(available as usize - start);
        out[..n].copy_from_slice(&buf.data[start..start + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpStream {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
            SeekFrom::End(d) => {
                let len = self.len.unwrap_or_else(|| self.wait_for_end());
                len as i128 + d as i128
            }
        };
        if target < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before start",
            ));
        }
        self.pos = target as u64;
        Ok(self.pos)
    }
}

impl Drop for HttpStream {
    fn drop(&mut self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    /// Serve `body` once over plain HTTP, sending it in slow chunks.
    fn serve(body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut req = [0u8; 1024];
            let _ = sock.read(&mut req);
            write!(
                sock,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            for part in body.chunks(1000) {
                sock.write_all(part).unwrap();
                thread::sleep(std::time::Duration::from_millis(2));
            }
        });
        format!("http://{addr}/stream")
    }

    #[test]
    fn reads_and_seeks_while_downloading() {
        let body: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        let url = serve(body.clone());
        let mut s = HttpStream::open(&ureq::Agent::new_with_defaults(), &url).unwrap();
        assert_eq!(s.len(), Some(body.len() as u64));

        // Seek ahead of the download; the read waits for the data.
        s.seek(SeekFrom::Start(15_000)).unwrap();
        let mut part = [0u8; 100];
        s.read_exact(&mut part).unwrap();
        assert_eq!(&part[..], &body[15_000..15_100]);

        s.seek(SeekFrom::End(-10)).unwrap();
        let mut rest = Vec::new();
        s.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, &body[body.len() - 10..]);

        s.seek(SeekFrom::Start(0)).unwrap();
        let mut all = Vec::new();
        s.read_to_end(&mut all).unwrap();
        assert_eq!(all, body);
        assert!(s.seek(SeekFrom::Current(-(body.len() as i64) - 1)).is_err());
    }
}
