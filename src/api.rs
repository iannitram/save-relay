//! The launcher contract: the `--json` envelope, each command's `data`, and exit codes.
//! The full shape, with examples, is `docs/code-map.md` § API shape. Field names and codes here
//! are a public contract: add fields, never rename or remove one.
//!
//! ```text
//! {"ok":true,"command":"pull","data":{...}}
//! {"ok":false,"command":"pull","error":{"code":"server_error","message":"..."}}
//! ```

use crate::error::Error;
use crate::plan::Decision;
use crate::transfer::Counts;
use serde::{Deserialize, Serialize};

/// One side of a sync: who made the saves and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Side {
    pub device: String,
    /// RFC 3339 UTC. For the local side: the newest save file's modification time.
    pub time: String,
    /// The server revision; `null` for local saves never pushed.
    pub revision: Option<u64>,
}

/// What `pull` or `push` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Already in sync.
    Noop,
    /// Same files on both sides; only the local record was updated.
    Adopted,
    /// The server's saves were downloaded.
    Pulled,
    /// The local saves were uploaded as a new revision.
    Pushed,
    /// `pull` with the server unreachable: start the game with local saves.
    Offline,
    /// `push` with the server unreachable: the saves stay local and go up next time.
    Deferred,
    /// Both sides changed: nothing was touched; ask the player, then call again with `--resolve`.
    Conflict,
}

/// `data` of `pull` and `push`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncData {
    pub game: String,
    pub action: Action,
    /// The server revision the local saves now match; `null` when offline, deferred or in conflict.
    pub revision: Option<u64>,
    /// The server's current saves; `null` when unreachable or empty.
    pub server: Option<Side>,
    /// The local saves before the command; `null` when there are none.
    pub local: Option<Side>,
    /// The history entry that keeps the side not chosen in a `--resolve`; `null` otherwise.
    pub backup: Option<String>,
    pub counts: Counts,
    /// One line for a person, e.g. "Pulled revision 7 from steam-deck (2026-10-31T19:51:00Z)."
    pub message: String,
}

/// `data` of `status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusData {
    pub game: String,
    /// The service this machine was set up with: "nextcloud", "gdrive", "dropbox" (M6).
    pub service: String,
    pub reachable: bool,
    /// What `pull` would do now.
    pub would: Decision,
    pub server: Option<Side>,
    pub local: Option<Side>,
    /// The revision this machine last synced; `null` when never.
    pub last_synced_revision: Option<u64>,
    /// Local files differ from the last sync (or exist with no sync yet).
    pub local_changed: bool,
    pub message: String,
}

/// One row of `history`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// The entry name, which `restore` takes.
    pub entry: String,
    pub device: String,
    /// RFC 3339 UTC.
    pub time: String,
    /// It holds the server's current revision.
    pub current: bool,
}

/// `data` of `history`: newest first, at most `retention::KEEP` plus any conflict backups not
/// yet dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryData {
    pub game: String,
    pub current: Option<String>,
    pub entries: Vec<HistoryEntry>,
}

/// `data` of `restore`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreData {
    pub game: String,
    /// The entry restored from.
    pub entry: String,
    /// The new server revision, a copy of `entry`.
    pub revision: u64,
    /// The new entry holding that copy.
    pub new_entry: String,
    /// Where unsynced local changes were kept first; `null` when there were none.
    pub backup: Option<String>,
    pub counts: Counts,
    pub message: String,
}

/// `data` of `setup`. Never the password or a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupData {
    /// "nextcloud", "gdrive", "dropbox".
    pub service: String,
    /// The Nextcloud address; for Google Drive and Dropbox the service key again.
    pub server: String,
    /// The Nextcloud user; for Google Drive and Dropbox the account's e-mail address.
    pub user: String,
    pub device: String,
    pub root: String,
    /// `CredentialStore::kind` of the store the password or refresh token went into.
    pub credential_store: String,
}

/// `data` of `setup --no-browser` (Google Drive, Dropbox): the sign-in to finish on any
/// device, then `setup --service <s> --finish`. A launcher shows `url` and `code` (as text or
/// a QR code); nothing is set up yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignInPrompt {
    pub service: String,
    /// The page to open.
    pub url: String,
    /// Google: the code to type on that page. Dropbox: `null` (Dropbox shows a code to type
    /// back into `setup --finish` instead).
    pub code: Option<String>,
    /// RFC 3339 UTC; after it, start again.
    pub expires_at: String,
    pub message: String,
}

/// `{"ok":true,"command":<command>,"data":<data>}` on one line, no trailing newline.
pub fn success<T: Serialize>(command: &str, data: &T) -> String {
    crate::not_built!("api::success")
}

/// `{"ok":false,"command":<command>,"error":{"code":<error.code()>,"message":<error's Display>}}`
/// on one line, no trailing newline.
pub fn failure(command: &str, error: &Error) -> String {
    crate::not_built!("api::failure")
}

/// The exit code of a command that succeeded: 3 for `Action::Conflict`, else 0.
pub fn exit_code_for(action: Action) -> i32 {
    crate::not_built!("api::exit_code_for")
}

/// The exit code of a failed command:
/// 2 `usage`, `manifest_invalid`, `game_unknown`, `no_such_entry`;
/// 4 `not_set_up`, `auth_failed`, `credentials`, `reauth_needed`, `sign_in_failed`;
/// 5 `server_unreachable`, `server_error`, `state_invalid`, `changed_on_server`, `not_found`;
/// 1 anything else (`io`, `not_built`).
pub fn exit_code(error: &Error) -> i32 {
    crate::not_built!("api::exit_code")
}
