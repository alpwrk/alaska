use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

/// File extensions the player can decode.
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "opus", "m4a", "mp4", "aac", "wav",
];

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// A directory listing with a cursor and an optional filter.
pub struct Browser {
    pub dir: PathBuf,
    entries: Vec<Entry>,
    /// Indices into `entries` that match the current filter.
    visible: Vec<usize>,
    pub cursor: usize,
    pub filter: String,
    pub error: Option<String>,
}

impl Browser {
    pub fn new(dir: PathBuf) -> Self {
        let mut browser = Browser {
            dir,
            entries: Vec::new(),
            visible: Vec::new(),
            cursor: 0,
            filter: String::new(),
            error: None,
        };
        browser.reload();
        browser
    }

    /// Re-read the current directory. Shows directories and audio files only.
    pub fn reload(&mut self) {
        self.entries.clear();
        self.error = None;
        match fs::read_dir(&self.dir) {
            Ok(rd) => {
                for entry in rd.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    let path = entry.path();
                    // `metadata` follows symlinks, so linked dirs behave like dirs.
                    let is_dir = fs::metadata(&path).is_ok_and(|m| m.is_dir());
                    if is_dir || is_audio(&path) {
                        self.entries.push(Entry { path, name, is_dir });
                    }
                }
            }
            Err(e) => self.error = Some(format!("{}: {e}", self.dir.display())),
        }
        self.entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| natural_cmp(&a.name, &b.name))
        });
        self.filter.clear();
        self.apply_filter();
    }

    pub fn cd(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.cursor = 0;
        self.reload();
    }

    /// Go to the parent directory and put the cursor on the directory we came from.
    pub fn parent(&mut self) {
        let Some(parent) = self.dir.parent().map(Path::to_path_buf) else {
            return;
        };
        let previous = std::mem::replace(&mut self.dir, parent);
        self.reload();
        self.select_path(&previous);
    }

    pub fn select_path(&mut self, path: &Path) {
        if let Some(pos) = self
            .visible
            .iter()
            .position(|&i| self.entries[i].path == path)
        {
            self.cursor = pos;
        }
    }

    pub fn visible(&self) -> impl Iterator<Item = &Entry> {
        self.visible.iter().map(|&i| &self.entries[i])
    }

    pub fn len(&self) -> usize {
        self.visible.len()
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.visible.get(self.cursor).map(|&i| &self.entries[i])
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.visible.is_empty() {
            self.cursor = 0;
            return;
        }
        let max = self.visible.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, max) as usize;
    }

    /// All audio files of the current directory in display order, ignoring the filter.
    pub fn audio_files(&self) -> Vec<PathBuf> {
        self.entries
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.path.clone())
            .collect()
    }

    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.apply_filter();
    }

    fn apply_filter(&mut self) {
        let selected = self.selected().map(|e| e.path.clone());
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| fuzzy_match(&self.filter, &e.name))
            .map(|(i, _)| i)
            .collect();
        self.cursor = 0;
        if let Some(path) = selected {
            self.select_path(&path);
        }
    }
}

/// Case-insensitive subsequence match: every char of `pattern` appears in `text` in order.
pub fn fuzzy_match(pattern: &str, text: &str) -> bool {
    let mut chars = text.chars().flat_map(char::to_lowercase);
    pattern
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| !c.is_whitespace())
        .all(|p| chars.any(|c| c == p))
}

/// Compare strings case-insensitively, treating runs of digits as numbers ("2" < "10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut a = a.chars().peekable();
    let mut b = b.chars().peekable();
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = take_number(&mut a);
                let nb = take_number(&mut b);
                // Compare by length first (after stripping leading zeros) to avoid overflow.
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

fn take_number(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        s.push(c);
        it.next();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_sort_orders_numbers() {
        let mut v = vec!["track 10", "Track 2", "track 1", "a"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a", "track 1", "Track 2", "track 10"]);
    }

    #[test]
    fn natural_sort_handles_leading_zeros() {
        assert_eq!(natural_cmp("01 x", "1 x"), Ordering::Equal);
        assert_eq!(natural_cmp("09", "10"), Ordering::Less);
    }

    #[test]
    fn fuzzy_matches_subsequence() {
        assert!(fuzzy_match("", "anything"));
        assert!(fuzzy_match("dsm", "Dark Side of the Moon"));
        assert!(fuzzy_match("DARK moon", "dark side of the moon"));
        assert!(!fuzzy_match("xyz", "dark side"));
        assert!(!fuzzy_match("nm", "moon"));
    }

    #[test]
    fn audio_extension_detection() {
        assert!(is_audio(Path::new("a/b.FLAC")));
        assert!(is_audio(Path::new("x.opus")));
        assert!(!is_audio(Path::new("cover.jpg")));
        assert!(!is_audio(Path::new("noext")));
    }
}
