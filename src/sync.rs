//! The engine: each CLI command as one function over a `Remote`, a `LocalStore` and the saves
//! folder. It reads, asks `plan::decide`, then acts through `transfer`, writes `state.json` with
//! `Precondition::Match` on the etag it read (so a second machine pushing at once gets
//! `Error::Changed`, never a lost revision: If-Match on Nextcloud, the rev on Dropbox, the
//! claim guard on Google Drive, M9), applies `retention`, and records `LastSynced`. It never
//! knows which service it talks to.
//!
//! M11-M14 (docs/adr/0003-baton-wrap-cards-panel.md): `pull` reads and writes the game's
//! open-elsewhere mark (`presence`) and asks `plan::decide_with_mark`; `push` writes a save
//! card into each new history entry (`transfer::put_card`) and removes this machine's mark.

use crate::api::{HistoryData, PictureData, RestoreData, StatusData, SyncReport, WhereData};
use crate::error::{not_built, Result};
use crate::layout::Layout;
use crate::local::LocalStore;
use crate::manifest::Manifest;
use crate::remote::Remote;
use std::path::{Path, PathBuf};

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
///
/// M11: first reads `playing.json`; another machine's mark is `Action::Held` (touch nothing,
/// `held` set) unless `anyway` (`pull --anyway`). After its own work, even a `noop`, it writes
/// this machine's mark (`Precondition::Any`) and the session start (`LocalStore::save_session`);
/// offline it writes neither. The report carries both sides' cards when there are two (M14).
pub fn pull(
    s: &Session,
    game: &Game,
    store: &LocalStore,
    resolve: Option<Resolve>,
    anyway: bool,
) -> Result<SyncReport> {
    not_built("sync::pull")
}

/// After the game quits (M2, M3, M5). `plan::decide(Intent::Exit, ..)`, then: Push (upload,
/// new revision with this device and time, then retention); unreachable is `Action::Deferred`;
/// the rest as in `pull`.
///
/// M14: each new entry gets a card (`card::for_push`: the session start for playtime, the
/// sidecar's place and picture) through `transfer::put_card` after the MOVE and before
/// `state.json`. M11: after the push, removes `playing.json` only when it names this device,
/// and clears the session start; a `deferred` push leaves both.
pub fn push(
    s: &Session,
    game: &Game,
    store: &LocalStore,
    resolve: Option<Resolve>,
) -> Result<SyncReport> {
    not_built("sync::push")
}

/// What `pull` would do, changing nothing. Unreachable is `reachable: false`, not an error.
/// `held` is another machine's mark; `pending` is this machine's pending choice, if any.
pub fn status(s: &Session, game: &Game, store: &LocalStore) -> Result<StatusData> {
    not_built("sync::status")
}

/// The game's history entries, newest first (M4), each with its card (`transfer::read_card`,
/// `null` for old entries). Unreachable is an error here.
pub fn history(s: &Session, game: &str) -> Result<HistoryData> {
    not_built("sync::history")
}

/// Bring back history entry `entry` (M4): keep unsynced local changes as a backup entry first,
/// copy `entry` server-side to a new entry, write it as a new revision, download it, record it.
pub fn restore(s: &Session, game: &Game, store: &LocalStore, entry: &str) -> Result<RestoreData> {
    not_built("sync::restore")
}

/// `save-relay where` (M11): for each game in `games` (`LocalStore::list_games`), its mark and
/// the server's current side. A missing or broken mark is no mark. Unreachable is an error.
pub fn where_open(s: &Session, games: &[String]) -> Result<WhereData> {
    not_built("sync::where_open")
}

/// `save-relay picture <game> <entry> --out <file>` (M14): copy the entry's card picture to
/// `out` (through a part file). No card, or a card with no picture, is `Error::NotFound`.
pub fn picture(s: &Session, game: &str, entry: &str, out: &Path) -> Result<PictureData> {
    not_built("sync::picture")
}
