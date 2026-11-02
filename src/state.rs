//! The two state files. `ServerState` is `<root>/<game>/state.json` on the server: which
//! revision is current, who pushed it, when, and its file list. `LastSynced` is this machine's
//! record of the revision it last pulled or pushed, with the file list as of then; a conflict is
//! "the server's revision moved and the local files changed since that record" (SPEC.md).

use crate::error::{Error, Result};
use crate::scan::{FileEntry, SaveSet};
use serde::{Deserialize, Serialize};

/// The state file format this build reads and writes.
pub const FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerState {
    pub format: u32,
    pub game: String,
    /// Starts at 1 with the first push; each push and restore adds 1.
    pub revision: u64,
    /// The device that made this revision (`layout::device_slug` form).
    pub device: String,
    /// When, `stamp::rfc3339` form.
    pub time: String,
    /// The history entry holding this revision's files (`layout::entry_name` form).
    pub entry: String,
    pub files: Vec<FileEntry>,
}

impl ServerState {
    /// Parse and check: `format == FORMAT` (a newer one says "made by a newer save-relay"),
    /// `revision >= 1`, `game`, `device`, `entry` not empty, `time` parses as RFC 3339, files
    /// sorted by path with no duplicates, each `sha256` 64 lowercase hex characters, each path
    /// relative with no `..` segment. Any failure is `Error::StateInvalid`.
    pub fn from_json(text: &str) -> Result<ServerState> {
        let state: ServerState =
            serde_json::from_str(text).map_err(|e| Error::StateInvalid(e.to_string()))?;
        check_common(state.format, &state.game, &state.time, &state.files)?;
        if state.revision == 0 {
            return Err(Error::StateInvalid("revision must be at least 1".into()));
        }
        if state.device.is_empty() {
            return Err(Error::StateInvalid("device must not be empty".into()));
        }
        if state.entry.is_empty() {
            return Err(Error::StateInvalid("entry must not be empty".into()));
        }
        if !is_entry_name(&state.entry) {
            return Err(Error::StateInvalid(format!(
                "entry '{}' is not <stamp>-<device>",
                state.entry
            )));
        }
        Ok(state)
    }

    /// Pretty JSON, fields in declaration order, ending in one newline.
    pub fn to_json(&self) -> String {
        format!(
            "{}\n",
            serde_json::to_string_pretty(self).expect("state serialization cannot fail")
        )
    }

    /// The file list as a save set. Built in the skeleton.
    pub fn save_set(&self) -> SaveSet {
        SaveSet {
            files: self.files.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastSynced {
    pub format: u32,
    pub game: String,
    /// The server revision this machine last pulled or pushed.
    pub revision: u64,
    /// When, `stamp::rfc3339` form.
    pub time: String,
    /// The local files as of that sync.
    pub files: Vec<FileEntry>,
}

impl LastSynced {
    /// The same checks as `ServerState::from_json` (revision may be any value).
    pub fn from_json(text: &str) -> Result<LastSynced> {
        let state: LastSynced =
            serde_json::from_str(text).map_err(|e| Error::StateInvalid(e.to_string()))?;
        check_common(state.format, &state.game, &state.time, &state.files)?;
        Ok(state)
    }

    /// Pretty JSON, fields in declaration order, ending in one newline.
    pub fn to_json(&self) -> String {
        format!(
            "{}\n",
            serde_json::to_string_pretty(self).expect("state serialization cannot fail")
        )
    }

    /// The file list as a save set. Built in the skeleton.
    pub fn save_set(&self) -> SaveSet {
        SaveSet {
            files: self.files.clone(),
        }
    }
}

fn check_common(format: u32, game: &str, time: &str, files: &[FileEntry]) -> Result<()> {
    if format > FORMAT {
        return Err(Error::StateInvalid(format!(
            "format {format} was made by a newer save-relay"
        )));
    }
    if format != FORMAT {
        return Err(Error::StateInvalid(format!(
            "unsupported format {format}; expected {FORMAT}"
        )));
    }
    if game.is_empty() {
        return Err(Error::StateInvalid("game must not be empty".into()));
    }
    if crate::stamp::parse_rfc3339(time).is_none() {
        return Err(Error::StateInvalid("time must be RFC 3339 UTC".into()));
    }
    for (i, file) in files.iter().enumerate() {
        if i > 0 && files[i - 1].path >= file.path {
            return Err(Error::StateInvalid(
                "files must be strictly increasing by path".into(),
            ));
        }
        if file.sha256.len() != 64
            || !file
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::StateInvalid(format!(
                "invalid sha256 for path '{}'",
                file.path
            )));
        }
        if file.path.is_empty()
            || file.path.starts_with('/')
            || file.path.split('/').any(|segment| segment == "..")
        {
            return Err(Error::StateInvalid(format!(
                "invalid relative path '{}'",
                file.path
            )));
        }
    }
    Ok(())
}

/// `<stamp>-<device>` (the `layout::entry_name` form) with a device of `a-z`, `0-9` and `-` only,
/// so the entry is one plain path segment and cannot climb out of the history folder.
fn is_entry_name(entry: &str) -> bool {
    if !entry.is_ascii() || entry.len() <= 17 || entry.as_bytes()[16] != b'-' {
        return false;
    }
    let (stamp, device) = (&entry[..16], &entry[17..]);
    crate::stamp::parse_entry_stamp(stamp).is_some()
        && device
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
