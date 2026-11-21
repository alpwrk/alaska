use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub url: String,
    pub user: String,
    pub password: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub subsonic: Option<ServerConfig>,
}

impl Config {
    pub fn file() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("alaska").join("config.toml"))
    }

    /// Load the config; a missing file yields the default config.
    pub fn load() -> Result<Config> {
        let Some(file) = Self::file() else {
            return Ok(Config::default());
        };
        match fs::read_to_string(&file) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("invalid {}", file.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", file.display())),
        }
    }

    /// Save the config readable only by the user, since it contains a password.
    pub fn save(&self) -> Result<()> {
        let file = Self::file().context("no config directory")?;
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        // Create the temp file with mode 600 so the password is never world-readable.
        let tmp = file.with_extension("tmp");
        let _ = fs::remove_file(&tmp);
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(toml::to_string(self)?.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, &file)?;
        // Also tighten a pre-existing file that was created by hand.
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_toml() {
        let config = Config {
            subsonic: Some(ServerConfig {
                url: "https://music.example.com".into(),
                user: "me".into(),
                password: "secret".into(),
            }),
        };
        let text = toml::to_string(&config).unwrap();
        assert!(text.contains("[subsonic]"));
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
        assert_eq!(toml::from_str::<Config>("").unwrap(), Config::default());
    }
}
