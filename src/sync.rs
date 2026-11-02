//! The engine: each CLI command as one function over a `Remote`, a `LocalStore` and the saves
//! folder. It reads, asks `plan::decide`, then acts through `transfer`, writes `state.json` with
//! `Precondition::Match` on the etag it read (so a second machine pushing at once gets
//! `Error::Changed`, never a lost revision: If-Match on Nextcloud, the rev on Dropbox, the
//! claim guard on Google Drive, M9), applies `retention`, and records `LastSynced`. It never
//! knows which service it talks to.
//!
//! Planned for M11-M14 (docs/adr/0003-baton-wrap-cards-panel.md): `pull` reads and writes the game's
//! open-elsewhere mark (`presence`) and asks `plan::decide_with_mark`; `push` writes a save
//! card into each new history entry (`transfer::put_card`) and removes this machine's mark.

use crate::api::{
    Action, HistoryData, PictureData, RestoreData, Side, StatusData, SyncData, SyncReport,
    WhereData,
};
use crate::error::{not_built, Error, Result};
use crate::layout::{entry_name, Layout};
use crate::local::LocalStore;
use crate::manifest::Manifest;
use crate::plan::{self, Decision, Intent, ServerView};
use crate::remote::{Precondition, Remote};
use crate::scan::{scan, SaveSet};
use crate::state::{LastSynced, ServerState, FORMAT};
use crate::{retention, stamp, transfer};
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
/// Planned for M11: first reads `playing.json`; another machine's mark is `Action::Held` (touch nothing,
/// `held` set) unless `anyway` (`pull --anyway`). After its own work, even a `noop`, it writes
/// this machine's mark (`Precondition::Any`) and the session start (`LocalStore::save_session`);
/// offline it writes neither. The report carries both sides' cards when there are two (M14).
pub fn pull(
    s: &Session,
    game: &Game,
    store: &LocalStore,
    resolve: Option<Resolve>,
    _anyway: bool,
) -> Result<SyncReport> {
    sync(s, game, store, resolve, Intent::Launch)
}

/// After the game quits (M2, M3, M5). `plan::decide(Intent::Exit, ..)`, then: Push (upload,
/// new revision with this device and time, then retention); unreachable is `Action::Deferred`;
/// the rest as in `pull`.
///
/// Planned for M14: each new entry gets a card (`card::for_push`: the session start for playtime, the
/// sidecar's place and picture) through `transfer::put_card` after the MOVE and before
/// `state.json`. M11: after the push, removes `playing.json` only when it names this device,
/// and clears the session start; a `deferred` push leaves both.
pub fn push(
    s: &Session,
    game: &Game,
    store: &LocalStore,
    resolve: Option<Resolve>,
) -> Result<SyncReport> {
    sync(s, game, store, resolve, Intent::Exit)
}

// M11's mark/anyway handling and M14's cards are added by their own issues.
fn sync(
    s: &Session,
    game: &Game,
    store: &LocalStore,
    resolve: Option<Resolve>,
    intent: Intent,
) -> Result<SyncReport> {
    let id = &game.manifest.id;
    let (state, pre, reachable) = match s.remote.get(&s.layout.state_path(id)) {
        Ok((bytes, etag)) => {
            let text = std::str::from_utf8(&bytes)
                .map_err(|error| Error::StateInvalid(error.to_string()))?;
            let state = ServerState::from_json(text)?;
            if state.game != *id {
                return Err(Error::StateInvalid("state belongs to another game".into()));
            }
            (Some(state), Precondition::Match(etag), true)
        }
        Err(Error::NotFound(_)) => (None, Precondition::Absent, true),
        Err(Error::Unreachable(_)) => (None, Precondition::Absent, false),
        Err(error) => return Err(error),
    };
    let local = scan(
        &game.saves_dir,
        &game.manifest.include,
        &game.manifest.exclude,
    )?;
    let last = store.load_last(id)?;
    let view = match &state {
        Some(state) => ServerView::At {
            revision: state.revision,
            files: state.save_set(),
        },
        None if reachable => ServerView::Empty,
        None => ServerView::Unreachable,
    };
    let mut data = SyncData {
        game: id.clone(),
        action: Action::Noop,
        revision: state.as_ref().and(last.as_ref()).map(|last| last.revision),
        server: state.as_ref().map(server_side),
        local: local.newest_mtime().map(|mtime| Side {
            device: s.device.clone(),
            time: stamp::rfc3339(mtime),
            revision: last.as_ref().map(|last| last.revision),
        }),
        backup: None,
        counts: transfer::Counts::default(),
        message: "No saves transferred.".into(),
    };
    let mut decision = plan::decide(intent, &local, last.as_ref(), &view);
    if decision == Decision::Conflict {
        match resolve {
            Some(Resolve::KeepServer) => {
                let entry = entry_name(&stamp::entry_stamp(s.now), &s.device);
                data.counts = transfer::upload_entry(
                    s.remote,
                    &s.layout,
                    id,
                    &entry,
                    &game.saves_dir,
                    &local,
                )?;
                data.backup = Some(entry);
                decision = Decision::Pull;
            }
            Some(Resolve::KeepLocal) => {
                data.backup = state.as_ref().map(|state| state.entry.clone());
                decision = Decision::Push;
            }
            None => {}
        }
    }
    match decision {
        Decision::Noop => {}
        Decision::Adopt | Decision::Pull => {
            let state = state.as_ref().ok_or_else(|| {
                Error::StateInvalid("sync decision requires a server revision".into())
            })?;
            let files = if decision == Decision::Pull {
                let wanted = state.save_set();
                let counts = transfer::download_entry(
                    s.remote,
                    &s.layout,
                    id,
                    &state.entry,
                    &wanted,
                    &game.saves_dir,
                    &local,
                )?;
                data.counts.downloaded = counts.downloaded;
                data.counts.deleted = counts.deleted;
                data.action = Action::Pulled;
                wanted
            } else {
                data.action = Action::Adopted;
                local
            };
            record_sync(s, store, id, state.revision, &files)?;
            data.revision = Some(state.revision);
            let verb = if decision == Decision::Pull {
                "Pulled"
            } else {
                "Adopted"
            };
            data.message = format!(
                "{verb} revision {} from {} ({}).",
                state.revision, state.device, state.time
            );
        }
        Decision::Push => {
            let revision = state
                .as_ref()
                .map_or(0, |state| state.revision)
                .checked_add(1)
                .ok_or_else(|| Error::StateInvalid("revision overflow".into()))?;
            let entry = entry_name(&stamp::entry_stamp(s.now), &s.device);
            data.counts =
                transfer::upload_entry(s.remote, &s.layout, id, &entry, &game.saves_dir, &local)?;
            let next = ServerState {
                format: FORMAT,
                game: id.clone(),
                revision,
                device: s.device.clone(),
                time: stamp::rfc3339(s.now),
                entry: entry.clone(),
                files: local.files.clone(),
            };
            if let Err(error) =
                s.remote
                    .put(&s.layout.state_path(id), next.to_json().as_bytes(), &pre)
            {
                if error == Error::Changed {
                    // A rejected conditional write cannot have published this entry.
                    let _ = s.remote.delete(&s.layout.entry_dir(id, &entry));
                }
                // Other failures may be ambiguous: keep the complete entry in history.
                return Err(error);
            }
            let entries = s
                .remote
                .list(&s.layout.history_dir(id))?
                .into_iter()
                .filter(|entry| entry.is_dir && data.backup.as_deref() != Some(entry.name.as_str()))
                .map(|entry| entry.name)
                .collect::<Vec<_>>();
            // Reserve a slot for the conflict's losing side, even with an old clock.
            let keep = retention::KEEP - usize::from(data.backup.is_some());
            for old in retention::to_drop(&entries, keep, &entry) {
                s.remote.delete(&s.layout.entry_dir(id, &old))?;
            }
            record_sync(s, store, id, revision, &local)?;
            data.action = Action::Pushed;
            data.revision = Some(revision);
            data.server = Some(server_side(&next));
            data.message = format!(
                "Pushed revision {revision} from {} ({}).",
                next.device, next.time
            );
        }
        Decision::Offline => {
            data.action = match intent {
                Intent::Launch => Action::Offline,
                Intent::Exit => Action::Deferred,
            };
            data.revision = None;
            data.message = "Playing offline: saves will upload next time.".into();
        }
        Decision::Conflict => {
            data.action = Action::Conflict;
            data.revision = None;
            data.message = "Local and server saves changed: choose which to keep.".into();
        }
        Decision::Held => unreachable!("plan::decide does not return Held"),
    }
    Ok(SyncReport {
        sync: data,
        held: None,
        server_card: None,
        local_card: None,
    })
}

fn server_side(state: &ServerState) -> Side {
    Side {
        device: state.device.clone(),
        time: state.time.clone(),
        revision: Some(state.revision),
    }
}

fn record_sync(
    s: &Session,
    store: &LocalStore,
    game: &str,
    revision: u64,
    files: &SaveSet,
) -> Result<()> {
    store.save_last(&LastSynced {
        format: FORMAT,
        game: game.into(),
        revision,
        time: stamp::rfc3339(s.now),
        files: files.files.clone(),
    })
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
