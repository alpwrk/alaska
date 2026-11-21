//! Minimal OpenSubsonic API client (tested against Navidrome).

pub mod stream;

use std::time::Duration;

use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};
use rand::RngExt;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::config::ServerConfig;

const API_VERSION: &str = "1.16.1";
const CLIENT_NAME: &str = "alaska";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub album_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub song_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub track: Option<u32>,
    /// Length in seconds.
    #[serde(default)]
    pub duration: Option<u64>,
    /// File extension of the original file, e.g. "flac".
    #[serde(default)]
    pub suffix: Option<String>,
}

impl Song {
    pub fn duration(&self) -> Option<Duration> {
        self.duration.filter(|&d| d > 0).map(Duration::from_secs)
    }
}

#[derive(Debug, Default)]
pub struct SearchResult {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub songs: Vec<Song>,
}

/// Blocking client; cheap to clone so worker threads can own one.
#[derive(Clone)]
pub struct Client {
    config: ServerConfig,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(config: ServerConfig) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_recv_response(Some(Duration::from_secs(15)))
            // Subsonic reports errors in the body, but proxies may not.
            .http_status_as_error(true)
            .build()
            .into();
        Client { config, agent }
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Server display name: host part of the URL.
    pub fn server_name(&self) -> &str {
        let url = self.config.url.trim_end_matches('/');
        let host = url.split_once("://").map_or(url, |(_, rest)| rest);
        host.split('/').next().unwrap_or(host)
    }

    /// Full URL for `endpoint` including auth parameters. Uses token auth, so
    /// the password itself is never sent.
    pub fn url(&self, endpoint: &str, params: &[(&str, &str)]) -> String {
        let salt: String = {
            let mut rng = rand::rng();
            (0..12)
                .map(|_| {
                    char::from(b"abcdefghijklmnopqrstuvwxyz0123456789"[rng.random_range(0..36)])
                })
                .collect()
        };
        let token = auth_token(&self.config.password, &salt);
        let mut url = format!(
            "{}/rest/{endpoint}?u={}&t={token}&s={salt}&v={API_VERSION}&c={CLIENT_NAME}&f=json",
            self.config.url.trim_end_matches('/'),
            encode(&self.config.user),
        );
        for (k, v) in params {
            url.push_str(&format!("&{k}={}", encode(v)));
        }
        url
    }

    pub fn agent(&self) -> &ureq::Agent {
        &self.agent
    }

    fn get<T: DeserializeOwned>(&self, endpoint: &str, params: &[(&str, &str)]) -> Result<T> {
        let body = self
            .agent
            .get(&self.url(endpoint, params))
            .call()
            .with_context(|| format!("request to {} failed", self.server_name()))?
            .into_body()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_string()?;
        parse_response(&body)
    }

    pub fn ping(&self) -> Result<()> {
        self.get::<serde_json::Value>("ping", &[]).map(|_| ())
    }

    pub fn artists(&self) -> Result<Vec<Artist>> {
        #[derive(Deserialize)]
        struct Resp {
            artists: Indexes,
        }
        #[derive(Deserialize)]
        struct Indexes {
            #[serde(default)]
            index: Vec<Index>,
        }
        #[derive(Deserialize)]
        struct Index {
            #[serde(default)]
            artist: Vec<Artist>,
        }
        let resp: Resp = self.get("getArtists", &[])?;
        Ok(resp
            .artists
            .index
            .into_iter()
            .flat_map(|i| i.artist)
            .collect())
    }

    pub fn artist_albums(&self, id: &str) -> Result<Vec<Album>> {
        #[derive(Deserialize)]
        struct Resp {
            artist: ArtistWithAlbums,
        }
        #[derive(Deserialize)]
        struct ArtistWithAlbums {
            #[serde(default)]
            album: Vec<Album>,
        }
        let resp: Resp = self.get("getArtist", &[("id", id)])?;
        let mut albums = resp.artist.album;
        albums.sort_by_key(|a| a.year.unwrap_or(u32::MAX));
        Ok(albums)
    }

    pub fn album_songs(&self, id: &str) -> Result<Vec<Song>> {
        #[derive(Deserialize)]
        struct Resp {
            album: AlbumWithSongs,
        }
        #[derive(Deserialize)]
        struct AlbumWithSongs {
            #[serde(default)]
            song: Vec<Song>,
        }
        let resp: Resp = self.get("getAlbum", &[("id", id)])?;
        Ok(resp.album.song)
    }

    pub fn search(&self, query: &str) -> Result<SearchResult> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Resp {
            search_result3: Results,
        }
        #[derive(Deserialize)]
        struct Results {
            #[serde(default)]
            artist: Vec<Artist>,
            #[serde(default)]
            album: Vec<Album>,
            #[serde(default)]
            song: Vec<Song>,
        }
        let resp: Resp = self.get(
            "search3",
            &[
                ("query", query),
                ("artistCount", "20"),
                ("albumCount", "30"),
                ("songCount", "100"),
            ],
        )?;
        let r = resp.search_result3;
        Ok(SearchResult {
            artists: r.artist,
            albums: r.album,
            songs: r.song,
        })
    }

    pub fn stream_url(&self, id: &str) -> String {
        // No `format`/`maxBitRate`: always stream the original file.
        self.url("stream", &[("id", id)])
    }

    pub fn scrobble(&self, id: &str, submission: bool) -> Result<()> {
        let submission = if submission { "true" } else { "false" };
        self.get::<serde_json::Value>("scrobble", &[("id", id), ("submission", submission)])
            .map(|_| ())
    }
}

/// Subsonic token: md5(password + salt) as lowercase hex.
pub fn auth_token(password: &str, salt: &str) -> String {
    let digest = Md5::digest(format!("{password}{salt}").as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Unwrap the `subsonic-response` envelope and turn API errors into `Err`.
fn parse_response<T: DeserializeOwned>(body: &str) -> Result<T> {
    #[derive(Deserialize)]
    struct Envelope {
        #[serde(rename = "subsonic-response")]
        response: serde_json::Value,
    }
    let env: Envelope = serde_json::from_str(body).context("invalid server response")?;
    let resp = env.response;
    if resp.get("status").and_then(|s| s.as_str()) != Some("ok") {
        let message = resp
            .pointer("/error/message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        bail!("server error: {message}");
    }
    serde_json::from_value(resp).context("unexpected server response")
}

/// Percent-encode a query parameter value.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> Client {
        Client::new(ServerConfig {
            url: "https://music.example.com/".into(),
            user: "me & you".into(),
            password: "sesame".into(),
        })
    }

    #[test]
    fn token_matches_subsonic_spec_example() {
        // Example from the Subsonic API documentation.
        assert_eq!(
            auth_token("sesame", "c19b2d"),
            "26719a1196d2a940705a59634eb18eab"
        );
    }

    #[test]
    fn url_uses_token_auth_and_encodes_params() {
        let url = client().url("getAlbum", &[("id", "a/b c")]);
        assert!(url.starts_with("https://music.example.com/rest/getAlbum?u=me%20%26%20you&t="));
        assert!(url.ends_with("&f=json&id=a%2Fb%20c"));
        assert!(!url.contains("sesame"));
        assert_eq!(client().server_name(), "music.example.com");
    }

    #[test]
    fn parses_artists_index() {
        #[derive(Deserialize)]
        struct Resp {
            artists: serde_json::Value,
        }
        let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1",
            "artists":{"index":[{"name":"A","artist":[{"id":"1","name":"Abba","albumCount":3}]}]}}}"#;
        let resp: Resp = parse_response(body).unwrap();
        let artist: Artist =
            serde_json::from_value(resp.artists["index"][0]["artist"][0].clone()).unwrap();
        assert_eq!(artist.name, "Abba");
        assert_eq!(artist.album_count, Some(3));
    }

    #[test]
    fn parses_song_with_missing_fields() {
        let song: Song = serde_json::from_str(
            r#"{"id":"s1","title":"Song","duration":185,"suffix":"flac","isDir":false}"#,
        )
        .unwrap();
        assert_eq!(song.duration(), Some(Duration::from_secs(185)));
        assert_eq!(song.artist, None);
    }

    #[test]
    fn api_error_becomes_err() {
        let body = r#"{"subsonic-response":{"status":"failed","version":"1.16.1",
            "error":{"code":40,"message":"Wrong username or password"}}}"#;
        let err = parse_response::<serde_json::Value>(body).unwrap_err();
        assert!(err.to_string().contains("Wrong username or password"));
    }
}
