use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

use crate::track::Track;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    pub fn cycle(self) -> Self {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Repeat::Off => "off",
            Repeat::All => "all",
            Repeat::One => "one",
        }
    }
}

/// Play queue. `tracks` keeps the display order; `order` is the play order
/// (a permutation of track indices) and `pos` points into `order`.
#[derive(Debug, Default)]
pub struct Queue {
    tracks: Vec<Track>,
    order: Vec<usize>,
    pos: Option<usize>,
    shuffle: bool,
    pub repeat: Repeat,
}

impl Queue {
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    /// Index (into `tracks`) of the current track.
    pub fn current_index(&self) -> Option<usize> {
        self.pos.map(|p| self.order[p])
    }

    pub fn current(&self) -> Option<&Track> {
        self.current_index().map(|i| &self.tracks[i])
    }

    /// Replace the queue and make `start` the current track.
    pub fn set(&mut self, tracks: Vec<Track>, start: Option<usize>) {
        self.tracks = tracks;
        let start = start.filter(|&s| s < self.tracks.len());
        self.rebuild_order(start);
    }

    fn rebuild_order(&mut self, current: Option<usize>) {
        self.order = (0..self.tracks.len()).collect();
        if self.shuffle {
            self.order.shuffle(&mut rand::rng());
            // The current track goes first so the shuffled rest follows it.
            if let Some(c) = current {
                let p = self.order.iter().position(|&i| i == c).unwrap();
                self.order.swap(0, p);
            }
            self.pos = current.map(|_| 0);
        } else {
            self.pos = current;
        }
    }

    /// Move to the next track. `auto` means the previous track ended on its own,
    /// which is when `Repeat::One` applies. Returns the new current index.
    pub fn advance(&mut self, auto: bool) -> Option<usize> {
        let pos = self.pos?;
        if auto && self.repeat == Repeat::One {
            return self.current_index();
        }
        if pos + 1 < self.order.len() {
            self.pos = Some(pos + 1);
        } else if self.repeat != Repeat::Off {
            if self.shuffle {
                self.rebuild_order(None);
            }
            self.pos = Some(0);
        } else {
            return None;
        }
        self.current_index()
    }

    /// Move to the previous track (wraps around with `Repeat::All`).
    pub fn back(&mut self) -> Option<usize> {
        let pos = self.pos?;
        self.pos = Some(match pos {
            0 if self.repeat == Repeat::All => self.order.len() - 1,
            0 => 0,
            p => p - 1,
        });
        self.current_index()
    }

    /// Make track `index` current.
    pub fn jump(&mut self, index: usize) {
        if index >= self.tracks.len() {
            return;
        }
        if self.shuffle {
            self.rebuild_order(Some(index));
        } else {
            self.pos = Some(index);
        }
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        let current = self.current_index();
        self.rebuild_order(current);
    }

    pub fn set_shuffle(&mut self, shuffle: bool) {
        if self.shuffle != shuffle {
            self.toggle_shuffle();
        }
    }

    /// Remove track `index`. Returns true if it was the current track; the
    /// next track in play order then becomes current (if any).
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.tracks.len() {
            return false;
        }
        let was_current = self.current_index() == Some(index);
        let order_pos = self.order.iter().position(|&i| i == index).unwrap();
        self.tracks.remove(index);
        self.order.remove(order_pos);
        for i in &mut self.order {
            if *i > index {
                *i -= 1;
            }
        }
        if let Some(pos) = self.pos {
            if order_pos < pos {
                self.pos = Some(pos - 1);
            } else if order_pos == pos && pos >= self.order.len() {
                self.pos = None;
            }
        }
        was_current
    }

    /// Swap tracks `a` and `b` in display order; play order follows the tracks.
    pub fn swap(&mut self, a: usize, b: usize) {
        if a >= self.tracks.len() || b >= self.tracks.len() {
            return;
        }
        self.tracks.swap(a, b);
        if self.shuffle {
            for i in &mut self.order {
                if *i == a {
                    *i = b;
                } else if *i == b {
                    *i = a;
                }
            }
        } else if let Some(pos) = self.pos {
            // Unshuffled order is the identity, so only the position moves.
            if pos == a {
                self.pos = Some(b);
            } else if pos == b {
                self.pos = Some(a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn queue(n: usize, start: usize) -> Queue {
        let mut q = Queue::default();
        q.set(
            (0..n)
                .map(|i| Track::Local(PathBuf::from(i.to_string())))
                .collect(),
            Some(start),
        );
        q
    }

    fn name(q: &Queue) -> String {
        q.current()
            .and_then(Track::local_path)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn advance_stops_at_end_without_repeat() {
        let mut q = queue(3, 1);
        assert_eq!(q.advance(true), Some(2));
        assert_eq!(q.advance(true), None);
        assert_eq!(q.current_index(), Some(2));
    }

    #[test]
    fn repeat_all_wraps_and_repeat_one_stays() {
        let mut q = queue(2, 1);
        q.repeat = Repeat::All;
        assert_eq!(q.advance(true), Some(0));
        assert_eq!(q.back(), Some(1));
        q.repeat = Repeat::One;
        assert_eq!(q.advance(true), Some(1));
        // A manual skip ignores repeat-one (and wraps like repeat-all).
        assert_eq!(q.advance(false), Some(0));
    }

    #[test]
    fn back_stays_on_first_track() {
        let mut q = queue(3, 0);
        assert_eq!(q.back(), Some(0));
    }

    #[test]
    fn shuffle_keeps_current_and_visits_all() {
        let mut q = queue(20, 7);
        q.toggle_shuffle();
        assert_eq!(q.current_index(), Some(7));
        let mut seen = vec![7];
        while let Some(i) = q.advance(false) {
            seen.push(i);
        }
        seen.sort();
        assert_eq!(seen, (0..20).collect::<Vec<_>>());
        q.toggle_shuffle();
        assert!(!q.shuffle());
    }

    #[test]
    fn remove_before_current_keeps_current() {
        let mut q = queue(4, 2);
        assert!(!q.remove(0));
        assert_eq!(name(&q), "2");
        assert_eq!(q.current_index(), Some(1));
    }

    #[test]
    fn remove_current_selects_next_or_none() {
        let mut q = queue(3, 1);
        assert!(q.remove(1));
        assert_eq!(name(&q), "2");
        assert!(q.remove(1));
        assert_eq!(q.current(), None);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn remove_in_shuffle_mode_keeps_permutation() {
        let mut q = queue(10, 3);
        q.toggle_shuffle();
        q.remove(5);
        q.remove(0);
        assert_eq!(name(&q), "3");
        let mut order = q.order.clone();
        order.sort();
        assert_eq!(order, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn swap_moves_current_along() {
        let mut q = queue(3, 0);
        q.swap(0, 1);
        assert_eq!(q.current_index(), Some(1));
        assert_eq!(name(&q), "0");
        q.toggle_shuffle();
        q.swap(1, 2);
        assert_eq!(name(&q), "0");
        assert_eq!(q.current_index(), Some(2));
    }

    #[test]
    fn jump_sets_current() {
        let mut q = queue(5, 0);
        q.jump(3);
        assert_eq!(q.current_index(), Some(3));
        q.toggle_shuffle();
        q.jump(1);
        assert_eq!(q.current_index(), Some(1));
    }
}
