//! Login dialog for the OpenSubsonic server.

use std::sync::mpsc::{self, Receiver};
use std::thread;

use anyhow::{Result, bail};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::ServerConfig;
use crate::subsonic::Client;

pub const LABELS: [&str; 3] = ["Server URL", "Username", "Password"];

pub enum Outcome {
    Pending,
    Cancelled,
    /// Credentials were verified with a ping.
    LoggedIn(Client),
}

pub struct Login {
    pub fields: [String; 3],
    pub focus: usize,
    pub error: Option<String>,
    check: Option<Receiver<Result<Client>>>,
}

impl Login {
    pub fn new(prefill: Option<&ServerConfig>) -> Self {
        let fields = match prefill {
            Some(c) => [c.url.clone(), c.user.clone(), c.password.clone()],
            None => [String::new(), String::new(), String::new()],
        };
        // Start in the first empty field.
        let focus = fields.iter().position(String::is_empty).unwrap_or(0);
        Login {
            fields,
            focus,
            error: None,
            check: None,
        }
    }

    pub fn is_checking(&self) -> bool {
        self.check.is_some()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Outcome {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Outcome::Cancelled;
        }
        match key.code {
            KeyCode::Esc => return Outcome::Cancelled,
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % 3,
            KeyCode::BackTab | KeyCode::Up => self.focus = (self.focus + 2) % 3,
            KeyCode::Enter if self.focus < 2 => self.focus += 1,
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                self.fields[self.focus].pop();
            }
            KeyCode::Char(c) => self.fields[self.focus].push(c),
            _ => {}
        }
        Outcome::Pending
    }

    fn submit(&mut self) {
        if self.check.is_some() {
            return;
        }
        let config = match self.config() {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        self.error = None;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let client = Client::new(config);
            let _ = tx.send(client.ping().map(|_| client));
        });
        self.check = Some(rx);
    }

    fn config(&self) -> Result<ServerConfig> {
        let [url, user, password] = self.fields.clone().map(|f| f.trim().to_string());
        if url.is_empty() || user.is_empty() {
            bail!("URL and username are required");
        }
        let url = if url.contains("://") {
            url
        } else {
            format!("https://{url}")
        };
        Ok(ServerConfig {
            url: url.trim_end_matches('/').to_string(),
            user,
            password,
        })
    }

    /// Check whether the background login finished.
    pub fn poll(&mut self) -> Outcome {
        let Some(rx) = &self.check else {
            return Outcome::Pending;
        };
        match rx.try_recv() {
            Ok(Ok(client)) => {
                self.check = None;
                Outcome::LoggedIn(client)
            }
            Ok(Err(e)) => {
                self.check = None;
                self.error = Some(format!("{e:#}"));
                Outcome::Pending
            }
            Err(_) => Outcome::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_normalizes_url() {
        let mut login = Login::new(None);
        login.fields = ["music.local:4533/".into(), " me ".into(), "pw".into()];
        let c = login.config().unwrap();
        assert_eq!(c.url, "https://music.local:4533");
        assert_eq!(c.user, "me");

        login.fields[0] = "http://10.0.0.2:4533".into();
        assert_eq!(login.config().unwrap().url, "http://10.0.0.2:4533");

        login.fields[1].clear();
        assert!(login.config().is_err());
    }

    #[test]
    fn starts_in_first_empty_field() {
        let prefill = ServerConfig {
            url: "https://x".into(),
            user: String::new(),
            password: String::new(),
        };
        assert_eq!(Login::new(Some(&prefill)).focus, 1);
    }
}
