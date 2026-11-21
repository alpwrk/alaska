use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::meta::{self, TagCache};
use crate::subsonic::Song;

/// A playable item: a local file or a song on an OpenSubsonic server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Track {
    Local(PathBuf),
    Remote(Song),
}

impl Track {
    pub fn local_path(&self) -> Option<&Path> {
        match self {
            Track::Local(p) => Some(p),
            Track::Remote(_) => None,
        }
    }

    /// Display label, "NN Title – Artist" when tags are known.
    pub fn label(&self, cache: &TagCache) -> String {
        match self {
            Track::Local(p) => meta::label(p, cache.get(p)),
            Track::Remote(s) => {
                let mut label = String::new();
                if let Some(n) = s.track {
                    label.push_str(&format!("{n:02} "));
                }
                label.push_str(&s.title);
                if let Some(artist) = &s.artist {
                    label.push_str(" – ");
                    label.push_str(artist);
                }
                label
            }
        }
    }

    pub fn title(&self, cache: &TagCache) -> Option<String> {
        match self {
            Track::Local(p) => cache
                .get(p)
                .and_then(|m| m.title.clone())
                .or_else(|| p.file_stem().map(|s| s.to_string_lossy().into_owned())),
            Track::Remote(s) => Some(s.title.clone()),
        }
    }

    pub fn artist(&self, cache: &TagCache) -> Option<String> {
        match self {
            Track::Local(p) => cache.get(p).and_then(|m| m.artist.clone()),
            Track::Remote(s) => s.artist.clone(),
        }
    }

    pub fn album(&self, cache: &TagCache) -> Option<String> {
        match self {
            Track::Local(p) => cache.get(p).and_then(|m| m.album.clone()),
            Track::Remote(s) => s.album.clone(),
        }
    }

    pub fn duration(&self, cache: &TagCache) -> Option<Duration> {
        match self {
            Track::Local(p) => cache.get(p).and_then(|m| m.duration()),
            Track::Remote(s) => s.duration(),
        }
    }
}
