//! The engine: each CLI command as one function over a `Remote`, a `LocalStore` and the saves
//! folder. It reads, asks `plan::decide`, then acts through `transfer`, writes `state.json` with
//! `Precondition::Match` on the etag it read (so a second machine pushing at once gets
//! `Error::Changed`, never a lost revision: If-Match on Nextcloud, the rev on Dropbox, the
//! claim guard on Google Drive, M9), applies `retention`, and records `LastSynced`. It never
//! knows which service it talks to.

use crate::api::{HistoryData, RestoreData, StatusData, SyncData};
use crate::error::{not_built, Result};
use crate::layout::Layout;
use crate::local::LocalStore;
use crate::manifest::Manifest;
use crate::remote::Remote;
use std::path::PathBuf;

/// One command's connection and identity.
pub struct Session<'a> {
    pub remote: &'a dyn Remote,
    pub layout: Layout,
    /// This machine, `layout::device_slug` form.
    pub device: String,
    /// Seconds since the Unix epoch; passed in so tests fix the clock.
    pub now: u64,
}

/// The game a command works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub manifest: Manifest,
    pub saves_dir: PathBuf,
}

/// The player's answer to a conflict (M3). The side not chosen is kept as a history entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolve {
    /// Take the server's saves; upload the local ones as a backup entry first.
    KeepServer,
    /// Push the local saves; the server's current entry stays in history.
    KeepLocal,
}

/// Before Play (M1, M3, M5). `plan::decide(Intent::Launch, ..)`, then: Noop; Adopt (record
/// only); Pull (download the current entry); Push (offline play caught up: upload as a new
/// revision); Offline (touch nothing, `Action::Offline`); Conflict (touch nothing,
/// `Action::Conflict`) unless `resolve` is given, which settles it as described on `Resolve`.
pub fn pull(
    s: &Session,
    game: &Game,
    store: &LocalStore,
    resolve: Option<Resolve>,
) -> Result<SyncData> {
    not_built("sync::pull")
}

/// After the game quits (M2, M3, M5). `plan::decide(Intent::Exit, ..)`, then: Push (upload,
/// new revision with this device and time, then retention); unreachable is `Action::Deferred`;
/// the rest as in `pull`.
pub fn push(
    s: &Session,
    game: &Game,
    store: &LocalStore,
    resolve: Option<Resolve>,
) -> Result<SyncData> {
    not_built("sync::push")
}

/// What `pull` would do, changing nothing. Unreachable is `reachable: false`, not an error.
pub fn status(s: &Session, game: &Game, store: &LocalStore) -> Result<StatusData> {
    not_built("sync::status")
}

/// The game's history entries, newest first (M4). Unreachable is an error here.
pub fn history(s: &Session, game: &str) -> Result<HistoryData> {
    not_built("sync::history")
}

/// Bring back history entry `entry` (M4): keep unsynced local changes as a backup entry first,
/// copy `entry` server-side to a new entry, write it as a new revision, download it, record it.
pub fn restore(s: &Session, game: &Game, store: &LocalStore, entry: &str) -> Result<RestoreData> {
    not_built("sync::restore")
}
