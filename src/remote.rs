//! Browser for an OpenSubsonic library: Artists → Albums → Songs, plus search.
//! Network requests run on worker threads so the UI never blocks.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use anyhow::Result;

use crate::subsonic::{Album, Artist, Client, SearchResult, Song};

#[derive(Debug, Clone)]
pub enum Entry {
    /// Non-selectable section title (used in search results).
    Header(String),
    Artist(Artist),
    Album(Album),
    Song(Song),
}

#[derive(Debug, Clone)]
pub struct View {
    /// Breadcrumb label: "Artists", the artist name, the album name, "Search: …".
    pub title: String,
    pub entries: Vec<Entry>,
    pub cursor: usize,
}

impl View {
    fn new(title: String, entries: Vec<Entry>) -> Self {
        let mut view = View {
            title,
            entries,
            cursor: 0,
        };
        view.skip_headers(1);
        view
    }

    /// Move off a header in direction `dir` (±1), if possible.
    fn skip_headers(&mut self, dir: isize) {
        let n = self.entries.len() as isize;
        let mut i = self.cursor as isize;
        while (0..n).contains(&i) && matches!(self.entries[i as usize], Entry::Header(_)) {
            i += dir;
        }
        if (0..n).contains(&i) {
            self.cursor = i as usize;
        }
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    /// All songs of this view in display order.
    pub fn songs(&self) -> Vec<Song> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                Entry::Song(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }
}

struct Response {
    request: u64,
    result: Result<View>,
}

pub struct Remote {
    pub client: Client,
    /// Navigation stack; the last view is shown.
    pub stack: Vec<View>,
    /// Title of the view being loaded, if any.
    pub loading: Option<String>,
    request: u64,
    tx: Sender<Response>,
    rx: Receiver<Response>,
}

impl Remote {
    pub fn new(client: Client) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut remote = Remote {
            client,
            stack: Vec::new(),
            loading: None,
            request: 0,
            tx,
            rx,
        };
        remote.load_artists();
        remote
    }

    pub fn view(&self) -> Option<&View> {
        self.stack.last()
    }

    fn view_mut(&mut self) -> Option<&mut View> {
        self.stack.last_mut()
    }

    /// Run `fetch` on a worker thread; its view is pushed when it arrives.
    /// A newer request makes older ones stale.
    fn request(
        &mut self,
        title: String,
        fetch: impl FnOnce(&Client) -> Result<View> + Send + 'static,
    ) {
        self.request += 1;
        let request = self.request;
        let client = self.client.clone();
        let tx = self.tx.clone();
        self.loading = Some(title);
        thread::spawn(move || {
            let _ = tx.send(Response {
                request,
                result: fetch(&client),
            });
        });
    }

    /// Apply finished requests. Returns an error message to show, if any.
    pub fn poll(&mut self) -> Option<String> {
        let mut error = None;
        while let Ok(resp) = self.rx.try_recv() {
            if resp.request != self.request {
                continue;
            }
            self.loading = None;
            match resp.result {
                Ok(view) => self.stack.push(view),
                Err(e) => error = Some(format!("{e:#}")),
            }
        }
        error
    }

    pub fn load_artists(&mut self) {
        self.stack.clear();
        self.request("Artists".into(), |c| {
            let entries = c.artists()?.into_iter().map(Entry::Artist).collect();
            Ok(View::new("Artists".into(), entries))
        });
    }

    pub fn search(&mut self, query: String) {
        let title = format!("Search: {query}");
        self.request(title.clone(), move |c| {
            Ok(View::new(title, search_entries(c.search(&query)?)))
        });
    }

    /// Open the selected artist or album. Returns the selected song instead
    /// if the cursor is on one, so the caller can start playback.
    pub fn open(&mut self) -> Option<Song> {
        match self.view()?.selected()?.clone() {
            Entry::Header(_) => None,
            Entry::Artist(artist) => {
                self.request(artist.name.clone(), move |c| {
                    let entries = c
                        .artist_albums(&artist.id)?
                        .into_iter()
                        .map(Entry::Album)
                        .collect();
                    Ok(View::new(artist.name, entries))
                });
                None
            }
            Entry::Album(album) => {
                self.request(album.name.clone(), move |c| {
                    let entries = c
                        .album_songs(&album.id)?
                        .into_iter()
                        .map(Entry::Song)
                        .collect();
                    Ok(View::new(album.name, entries))
                });
                None
            }
            Entry::Song(song) => Some(song),
        }
    }

    /// Go up one level; the previous view keeps its cursor.
    pub fn back(&mut self) {
        if self.loading.take().is_some() {
            // Cancel the pending request instead of leaving the view.
            self.request += 1;
            return;
        }
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    /// Back to the artist list.
    pub fn home(&mut self) {
        self.request += 1;
        self.loading = None;
        self.stack.truncate(1);
        if self.stack.is_empty() {
            self.load_artists();
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        let Some(view) = self.view_mut() else {
            return;
        };
        if view.entries.is_empty() {
            return;
        }
        let max = view.entries.len() as isize - 1;
        view.cursor = (view.cursor as isize).saturating_add(delta).clamp(0, max) as usize;
        view.skip_headers(if delta < 0 { -1 } else { 1 });
        view.skip_headers(if delta < 0 { 1 } else { -1 });
    }

    /// Breadcrumb of the current location.
    pub fn path(&self) -> String {
        let mut parts: Vec<&str> = self.stack.iter().map(|v| v.title.as_str()).collect();
        if let Some(loading) = &self.loading {
            parts.push(loading);
        }
        parts.join(" › ")
    }
}

fn search_entries(r: SearchResult) -> Vec<Entry> {
    let mut entries = Vec::new();
    if !r.artists.is_empty() {
        entries.push(Entry::Header("Artists".into()));
        entries.extend(r.artists.into_iter().map(Entry::Artist));
    }
    if !r.albums.is_empty() {
        entries.push(Entry::Header("Albums".into()));
        entries.extend(r.albums.into_iter().map(Entry::Album));
    }
    if !r.songs.is_empty() {
        entries.push(Entry::Header("Songs".into()));
        entries.extend(r.songs.into_iter().map(Entry::Song));
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str) -> Song {
        Song {
            id: id.into(),
            title: id.into(),
            artist: None,
            album: None,
            track: None,
            duration: None,
            suffix: None,
        }
    }

    #[test]
    fn search_view_skips_headers() {
        let entries = search_entries(SearchResult {
            artists: vec![],
            albums: vec![],
            songs: vec![song("a"), song("b")],
        });
        let view = View::new("Search".into(), entries);
        assert_eq!(view.cursor, 1);
        assert_eq!(view.songs().len(), 2);
    }

    #[test]
    fn empty_search_has_no_entries() {
        assert!(search_entries(SearchResult::default()).is_empty());
    }
}
