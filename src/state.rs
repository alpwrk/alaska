use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::meta::write_atomic;
use crate::queue::Repeat;

/// Player state persisted between runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub volume: f32,
    pub queue: Vec<PathBuf>,
    pub current: Option<usize>,
    pub position_secs: f64,
    pub shuffle: bool,
    pub repeat: Repeat,
}

impl Default for State {
    fn default() -> Self {
        State {
            volume: 0.8,
            queue: Vec::new(),
            current: None,
            position_secs: 0.0,
            shuffle: false,
            repeat: Repeat::Off,
        }
    }
}

impl State {
    fn file() -> Option<PathBuf> {
        dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .map(|d| d.join("alaska").join("state.json"))
    }

    pub fn load() -> State {
        let mut state: State = Self::file()
            .and_then(|f| fs::read(f).ok())
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        state.drop_missing_tracks();
        state
    }

    pub fn save(&self) -> Result<()> {
        if let Some(file) = Self::file() {
            write_atomic(&file, &serde_json::to_vec_pretty(self)?)?;
        }
        Ok(())
    }

    /// Remove tracks that no longer exist, keeping `current` pointing at the same file.
    fn drop_missing_tracks(&mut self) {
        let current = self.current.and_then(|i| self.queue.get(i)).cloned();
        self.queue.retain(|p| p.is_file());
        self.current = current.and_then(|c| self.queue.iter().position(|p| *p == c));
        if self.current.is_none() {
            self.position_secs = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults_for_missing_fields() {
        let state = State {
            volume: 0.5,
            queue: vec![PathBuf::from("/x.mp3")],
            current: Some(0),
            position_secs: 12.5,
            shuffle: true,
            repeat: Repeat::All,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<State>(&json).unwrap(), state);

        let partial: State = serde_json::from_str(r#"{"volume":0.3}"#).unwrap();
        assert_eq!(partial.volume, 0.3);
        assert!(partial.queue.is_empty());
    }

    #[test]
    fn missing_tracks_are_dropped() {
        let existing = std::env::current_exe().unwrap();
        let mut state = State {
            queue: vec![PathBuf::from("/does/not/exist.mp3"), existing.clone()],
            current: Some(1),
            position_secs: 3.0,
            ..State::default()
        };
        state.drop_missing_tracks();
        assert_eq!(state.queue, [existing]);
        assert_eq!(state.current, Some(0));
        assert_eq!(state.position_secs, 3.0);

        state.queue = vec![PathBuf::from("/gone.mp3")];
        state.current = Some(0);
        state.drop_missing_tracks();
        assert_eq!(state.current, None);
        assert_eq!(state.position_secs, 0.0);
    }
}
