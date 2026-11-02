//! This machine's own files, under `platform::AppDirs`:
//!
//! ```text
//! <config>/save-relay/config.json        Config: service (in `server`), user, root, device;
//!                                        never a password or token
//! <data>/save-relay/last/<game>.json     state::LastSynced: what this machine last synced
//! <data>/save-relay/games/<game>.json    GameRecord: the manifest and saves folder last used
//! <data>/save-relay/session/<game>.json  Session: when this machine's last `pull` ran (M14 playtime)
//! <data>/save-relay/pending/<game>.json  pending::PendingChoice: a choice `wrap` waits on (M12)
//! <data>/save-relay/ludusavi/            the ludusavi data cache (`ludusavi::Cache`, M13)
//! ```
//!
//! Every write goes to `<file>.tmp` first and is renamed over the file, so a crash never leaves
//! half a record.

use crate::error::{not_built, Error, Result};
use crate::manifest::Manifest;
use crate::pending::PendingChoice;
use crate::platform::AppDirs;
use crate::state::LastSynced;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// The config format this build reads and writes.
pub const FORMAT: u32 = 1;

/// Written by `save-relay setup`. Holds no password or token: those are in the credential
/// store (M6). The chosen service is `service::Service::from_server(&config.server)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub format: u32,
    /// The Nextcloud address as given to setup, e.g. `https://cloud.example.org`; or the
    /// service key `gdrive` or `dropbox` for those services.
    pub server: String,
    /// The Nextcloud user; for Google Drive and Dropbox the account's e-mail address as the
    /// service reports it (the refresh token is filed under `service::token_account`).
    pub user: String,
    /// The folder under the user's files that holds every game (`layout::Layout::new`).
    pub root: String,
    /// This machine's name in history entries (`layout::device_slug` form).
    pub device: String,
}

/// A game seen on this machine, so later commands can name it by id alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameRecord {
    pub manifest: Manifest,
    /// The `--saves-dir` the launcher passed, if any.
    pub saves_dir: Option<PathBuf>,
}

/// `session/<game>.json`: when this machine's `pull` ran, so the push can give the card a
/// playtime (`card::playtime`). Written by `pull`, cleared by `push`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub format: u32,
    pub game: String,
    /// Seconds since the Unix epoch.
    pub started: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalStore {
    dirs: AppDirs,
}

impl LocalStore {
    pub fn new(dirs: AppDirs) -> LocalStore {
        LocalStore { dirs }
    }

    /// `None` when there is no config file yet. Unreadable JSON is `Error::StateInvalid`.
    pub fn load_config(&self) -> Result<Option<Config>> {
        read_json(&self.dirs.config.join("config.json"))
    }

    /// Creates the folder as needed.
    pub fn save_config(&self, config: &Config) -> Result<()> {
        write_json(&self.dirs.config.join("config.json"), config)
    }

    /// `None` when this machine never synced `game`.
    pub fn load_last(&self, game: &str) -> Result<Option<LastSynced>> {
        read_json(&self.dirs.data.join("last").join(format!("{game}.json")))
    }

    /// Saved under `last.game`.
    pub fn save_last(&self, last: &LastSynced) -> Result<()> {
        write_json(
            &self
                .dirs
                .data
                .join("last")
                .join(format!("{}.json", last.game)),
            last,
        )
    }

    /// `None` when `game` was never seen here.
    pub fn load_game(&self, game: &str) -> Result<Option<GameRecord>> {
        read_json(&self.dirs.data.join("games").join(format!("{game}.json")))
    }

    /// Saved under `record.manifest.id`.
    pub fn save_game(&self, record: &GameRecord) -> Result<()> {
        write_json(
            &self
                .dirs
                .data
                .join("games")
                .join(format!("{}.json", record.manifest.id)),
            record,
        )
    }

    /// Every game id with a `games/<game>.json`, sorted (`where`, the panel).
    pub fn list_games(&self) -> Result<Vec<String>> {
        not_built("local::LocalStore::list_games")
    }

    /// `None` when no choice is pending for `game`. Unreadable is `Error::StateInvalid`.
    pub fn load_pending(&self, game: &str) -> Result<Option<PendingChoice>> {
        not_built("local::LocalStore::load_pending")
    }

    /// Saved under `choice.game`, replacing any earlier one.
    pub fn save_pending(&self, choice: &PendingChoice) -> Result<()> {
        not_built("local::LocalStore::save_pending")
    }

    /// Removes `pending/<game>.json`; none there is fine.
    pub fn clear_pending(&self, game: &str) -> Result<()> {
        not_built("local::LocalStore::clear_pending")
    }

    /// Every pending choice on this machine, by game id (the panel's list).
    pub fn list_pending(&self) -> Result<Vec<PendingChoice>> {
        not_built("local::LocalStore::list_pending")
    }

    /// `None` when no `pull` ran since the last push.
    pub fn load_session(&self, game: &str) -> Result<Option<Session>> {
        not_built("local::LocalStore::load_session")
    }

    /// Saved under `session.game`.
    pub fn save_session(&self, session: &Session) -> Result<()> {
        not_built("local::LocalStore::save_session")
    }

    /// Removes `session/<game>.json`; none there is fine.
    pub fn clear_session(&self, game: &str) -> Result<()> {
        not_built("local::LocalStore::clear_session")
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &std::path::Path) -> Result<Option<T>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::from(error)),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| Error::StateInvalid(error.to_string()))
}

fn write_json<T: Serialize>(path: &std::path::Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Io("local file has no parent folder".into()))?;
    fs::create_dir_all(parent)?;
    let temp = path.with_extension("json.tmp");
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|error| Error::StateInvalid(error.to_string()))?;
    text.push('\n');
    fs::write(&temp, text)?;
    fs::rename(temp, path)?;
    Ok(())
}
