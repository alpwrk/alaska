use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::Result;
use lofty::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track: Option<u32>,
    pub duration_ms: Option<u64>,
}

impl TrackMeta {
    pub fn read(path: &Path) -> TrackMeta {
        let Ok(file) = lofty::read_from_path(path) else {
            return TrackMeta::default();
        };
        let duration = file.properties().duration();
        let mut meta = TrackMeta {
            duration_ms: (!duration.is_zero()).then_some(duration.as_millis() as u64),
            ..TrackMeta::default()
        };
        if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
            let clean = |s: Option<std::borrow::Cow<str>>| {
                s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
            };
            meta.title = clean(tag.title());
            meta.artist = clean(tag.artist());
            meta.album = clean(tag.album());
            meta.track = tag.track();
        }
        meta
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration_ms.map(Duration::from_millis)
    }
}

/// Display label for a track: "NN Title – Artist", falling back to the file name.
pub fn label(path: &Path, meta: Option<&TrackMeta>) -> String {
    let file_name = || {
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let Some(meta) = meta else {
        return file_name();
    };
    let Some(title) = &meta.title else {
        return file_name();
    };
    let mut s = String::new();
    if let Some(n) = meta.track {
        s.push_str(&format!("{n:02} "));
    }
    s.push_str(title);
    if let Some(artist) = &meta.artist {
        s.push_str(" – ");
        s.push_str(artist);
    }
    s
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    mtime_ms: u64,
    meta: TrackMeta,
}

/// Tag cache keyed by path; entries are invalidated by file modification time.
#[derive(Debug, Default)]
pub struct TagCache {
    entries: HashMap<PathBuf, CacheEntry>,
    dirty: bool,
}

impl TagCache {
    fn file() -> Option<PathBuf> {
        dirs::cache_dir().map(|d| d.join("alaska").join("tags.json"))
    }

    /// Load the cache from disk; a missing or corrupt cache yields an empty one.
    pub fn load() -> TagCache {
        let entries = Self::file()
            .and_then(|f| fs::read(f).ok())
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        TagCache {
            entries,
            dirty: false,
        }
    }

    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        // Drop entries for files that no longer exist.
        self.entries.retain(|p, _| p.exists());
        if let Some(file) = Self::file() {
            write_atomic(&file, &serde_json::to_vec(&self.entries)?)?;
        }
        self.dirty = false;
        Ok(())
    }

    pub fn get(&self, path: &Path) -> Option<&TrackMeta> {
        self.entries.get(path).map(|e| &e.meta)
    }

    /// Make sure all `paths` have up-to-date metadata.
    pub fn ensure<'a>(&mut self, paths: impl IntoIterator<Item = &'a PathBuf>) {
        for path in paths {
            let Some(mtime_ms) = mtime_ms(path) else {
                continue;
            };
            if self
                .entries
                .get(path)
                .is_some_and(|e| e.mtime_ms == mtime_ms)
            {
                continue;
            }
            let meta = TrackMeta::read(path);
            self.entries
                .insert(path.clone(), CacheEntry { mtime_ms, meta });
            self.dirty = true;
        }
    }
}

fn mtime_ms(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// Write via a temp file and rename so a crash never leaves a truncated file.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_uses_tags_and_falls_back_to_file_name() {
        let path = Path::new("/music/03 - song.flac");
        assert_eq!(label(path, None), "03 - song");
        assert_eq!(label(path, Some(&TrackMeta::default())), "03 - song");
        let meta = TrackMeta {
            title: Some("Song".into()),
            artist: Some("Band".into()),
            track: Some(3),
            ..TrackMeta::default()
        };
        assert_eq!(label(path, Some(&meta)), "03 Song – Band");
    }

    #[test]
    fn cache_entries_round_trip() {
        let mut map = HashMap::new();
        map.insert(
            PathBuf::from("/a.mp3"),
            CacheEntry {
                mtime_ms: 42,
                meta: TrackMeta {
                    title: Some("A".into()),
                    duration_ms: Some(1000),
                    ..TrackMeta::default()
                },
            },
        );
        let json = serde_json::to_vec(&map).unwrap();
        let back: HashMap<PathBuf, CacheEntry> = serde_json::from_slice(&json).unwrap();
        assert_eq!(back[Path::new("/a.mp3")].mtime_ms, 42);
        assert_eq!(back[Path::new("/a.mp3")].meta.title.as_deref(), Some("A"));
    }
}
