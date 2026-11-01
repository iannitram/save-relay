//! The two state files. `ServerState` is `<root>/<game>/state.json` on the server: which
//! revision is current, who pushed it, when, and its file list. `LastSynced` is this machine's
//! record of the revision it last pulled or pushed, with the file list as of then; a conflict is
//! "the server's revision moved and the local files changed since that record" (SPEC.md).

use crate::error::{not_built, Result};
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
        not_built("state::ServerState::from_json")
    }

    /// Pretty JSON, fields in declaration order, ending in one newline.
    pub fn to_json(&self) -> String {
        crate::not_built!("state::ServerState::to_json")
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
        not_built("state::LastSynced::from_json")
    }

    /// Pretty JSON, fields in declaration order, ending in one newline.
    pub fn to_json(&self) -> String {
        crate::not_built!("state::LastSynced::to_json")
    }

    /// The file list as a save set. Built in the skeleton.
    pub fn save_set(&self) -> SaveSet {
        SaveSet {
            files: self.files.clone(),
        }
    }
}
