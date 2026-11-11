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
//! Every write goes to a temp file of its own in the same folder first and is renamed over the
//! file, so a crash never leaves half a record and two writes of one file never share a temp file.

use crate::error::{Error, Result};
use crate::manifest::Manifest;
use crate::pending::PendingChoice;
use crate::platform::AppDirs;
use crate::state::LastSynced;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

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
    /// The resolved saves folder last used, including a folder found from ludusavi data.
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
        let entries = match fs::read_dir(self.dirs.data.join("games")) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() || path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            if let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) {
                ids.push(id.to_string());
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// `None` when no choice is pending for `game`. Unreadable is `Error::StateInvalid`.
    pub fn load_pending(&self, game: &str) -> Result<Option<PendingChoice>> {
        let path = self.dirs.data.join("pending").join(format!("{game}.json"));
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::StateInvalid(error.to_string())),
        };
        PendingChoice::from_json(&text).map(Some)
    }

    /// Saved under `choice.game`, replacing any earlier one.
    pub fn save_pending(&self, choice: &PendingChoice) -> Result<()> {
        write_json(
            &self
                .dirs
                .data
                .join("pending")
                .join(format!("{}.json", choice.game)),
            choice,
        )
    }

    /// Removes `pending/<game>.json`; none there is fine.
    pub fn clear_pending(&self, game: &str) -> Result<()> {
        match fs::remove_file(self.dirs.data.join("pending").join(format!("{game}.json"))) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Every pending choice on this machine, by game id (the panel's list).
    pub fn list_pending(&self) -> Result<Vec<PendingChoice>> {
        let entries = match fs::read_dir(self.dirs.data.join("pending")) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut choices = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                let text = fs::read_to_string(path)
                    .map_err(|error| Error::StateInvalid(error.to_string()))?;
                choices.push(PendingChoice::from_json(&text)?);
            }
        }
        choices.sort_by(|a, b| a.game.cmp(&b.game));
        Ok(choices)
    }

    /// `None` when no `pull` ran since the last push. A session of another format is
    /// `Error::StateInvalid`.
    pub fn load_session(&self, game: &str) -> Result<Option<Session>> {
        let session: Option<Session> =
            read_json(&self.dirs.data.join("session").join(format!("{game}.json")))?;
        match session {
            Some(session) if session.format != FORMAT => Err(Error::StateInvalid(format!(
                "session format {} is not {FORMAT}",
                session.format
            ))),
            other => Ok(other),
        }
    }

    /// Saved under `session.game`.
    pub fn save_session(&self, session: &Session) -> Result<()> {
        write_json(
            &self
                .dirs
                .data
                .join("session")
                .join(format!("{}.json", session.game)),
            session,
        )
    }

    /// Removes `session/<game>.json`; none there is fine.
    pub fn clear_session(&self, game: &str) -> Result<()> {
        match fs::remove_file(self.dirs.data.join("session").join(format!("{game}.json"))) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
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

/// Counts the temp files this process has made, so no two writes in it share a name.
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn write_json<T: Serialize>(path: &std::path::Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Io("local file has no parent folder".into()))?;
    fs::create_dir_all(parent)?;
    // Each write gets a temp file of its own, in the same folder: `<file>.<process id>.<n>.tmp`.
    // Two writes of one file (two `answer` calls, or two threads) never share a name, so neither
    // renames the other's temp file away. Two live processes never share a process id. A stale
    // temp file from a crash is truncated only by a later write with the same name, and no live
    // write owns that name.
    let file_name = path
        .file_name()
        .ok_or_else(|| Error::Io("local file has no name".into()))?;
    let mut temp_name = file_name.to_os_string();
    temp_name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let temp = parent.join(temp_name);
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|error| Error::StateInvalid(error.to_string()))?;
    text.push('\n');
    let result = write_synced(&temp, text.as_bytes())
        .and_then(|()| fs::rename(&temp, path).map_err(Error::from));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Writes `bytes` to `temp` and flushes them to disk, so the rename that follows never
/// publishes a file whose contents are still only in memory.
fn write_synced(temp: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = fs::File::create(temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
