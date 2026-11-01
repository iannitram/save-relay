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

use crate::error::{not_built, Result};
use crate::manifest::Manifest;
use crate::pending::PendingChoice;
use crate::platform::AppDirs;
use crate::state::LastSynced;
use serde::{Deserialize, Serialize};
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
        not_built("local::LocalStore::load_config")
    }

    /// Creates the folder as needed.
    pub fn save_config(&self, config: &Config) -> Result<()> {
        not_built("local::LocalStore::save_config")
    }

    /// `None` when this machine never synced `game`.
    pub fn load_last(&self, game: &str) -> Result<Option<LastSynced>> {
        not_built("local::LocalStore::load_last")
    }

    /// Saved under `last.game`.
    pub fn save_last(&self, last: &LastSynced) -> Result<()> {
        not_built("local::LocalStore::save_last")
    }

    /// `None` when `game` was never seen here.
    pub fn load_game(&self, game: &str) -> Result<Option<GameRecord>> {
        not_built("local::LocalStore::load_game")
    }

    /// Saved under `record.manifest.id`.
    pub fn save_game(&self, record: &GameRecord) -> Result<()> {
        not_built("local::LocalStore::save_game")
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
