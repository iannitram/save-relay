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
    Action, HistoryData, HistoryEntry, PictureData, RestoreData, Side, StatusData, SyncData,
    SyncReport, WhereData,
};
use crate::error::{not_built, Error, Result};
use crate::layout::{entry_name, parse_entry_name, Layout};
use crate::local::LocalStore;
use crate::manifest::Manifest;
use crate::plan::{self, Decision, Intent, ServerView};
use crate::remote::{Precondition, Remote, RemoteEntry};
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
    let (state, pre, reachable) = read_server(s, id)?;
    let local = scan(
        &game.saves_dir,
        &game.manifest.include,
        &game.manifest.exclude,
    )?;
    let last = store.load_last(id)?;
    let view = server_view(state.as_ref(), reachable);
    let mut data = SyncData {
        game: id.clone(),
        action: Action::Noop,
        revision: state.as_ref().and(last.as_ref()).map(|last| last.revision),
        server: state.as_ref().map(server_side),
        local: local_side(s, &local, last.as_ref()),
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

/// The server's `state.json` as `pull`, `push` and `status` read it: the state (`None` when
/// there is none), the precondition a write must meet (`Match` on the etag read, `Absent` when
/// there is no state), and whether the server answered. Unreachable is not an error here.
fn read_server(s: &Session, id: &str) -> Result<(Option<ServerState>, Precondition, bool)> {
    match s.remote.get(&s.layout.state_path(id)) {
        Ok((bytes, etag)) => {
            let text = std::str::from_utf8(&bytes)
                .map_err(|error| Error::StateInvalid(error.to_string()))?;
            let state = ServerState::from_json(text)?;
            if state.game != *id {
                return Err(Error::StateInvalid("state belongs to another game".into()));
            }
            Ok((Some(state), Precondition::Match(etag), true))
        }
        Err(Error::NotFound(_)) => Ok((None, Precondition::Absent, true)),
        Err(Error::Unreachable(_)) => Ok((None, Precondition::Absent, false)),
        Err(error) => Err(error),
    }
}

/// What `plan::decide` gets as the server's side.
fn server_view(state: Option<&ServerState>, reachable: bool) -> ServerView {
    match state {
        Some(state) => ServerView::At {
            revision: state.revision,
            files: state.save_set(),
        },
        None if reachable => ServerView::Empty,
        None => ServerView::Unreachable,
    }
}

/// This machine's side: the newest save file's time, `None` when there are no saves.
fn local_side(s: &Session, local: &SaveSet, last: Option<&LastSynced>) -> Option<Side> {
    local.newest_mtime().map(|mtime| Side {
        device: s.device.clone(),
        time: stamp::rfc3339(mtime),
        revision: last.map(|last| last.revision),
    })
}

/// The local files differ from the last sync, or exist with no sync yet.
fn local_changed(local: &SaveSet, last: Option<&LastSynced>) -> bool {
    match last {
        Some(last) => !local.same_content(&last.save_set()),
        None => !local.is_empty(),
    }
}

/// The first entry name for this device at `now` or later that is not in `taken`: a copy made
/// in the same second as an existing entry moves one second on, so time order still holds.
fn fresh_entry(now: u64, device: &str, taken: &[String]) -> String {
    let mut at = now;
    loop {
        let name = entry_name(&stamp::entry_stamp(at), device);
        if !taken.contains(&name) {
            return name;
        }
        at += 1;
    }
}

/// The folder names under `history/`. No folder yet (nothing pushed) is an empty list.
fn history_children(s: &Session, game: &str) -> Result<Vec<RemoteEntry>> {
    match s.remote.list(&s.layout.history_dir(game)) {
        Ok(children) => Ok(children),
        Err(Error::NotFound(_)) => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn status_message(would: Decision) -> String {
    match would {
        Decision::Noop => "In sync: nothing to do.".into(),
        Decision::Adopt => "Same files on both sides: a launch only records the revision.".into(),
        Decision::Pull => "The server has a newer revision: a launch would download it.".into(),
        Decision::Push => "Local saves changed: a push would upload them.".into(),
        Decision::Conflict => {
            "Local and server saves changed: a launch would ask which to keep.".into()
        }
        Decision::Offline => "The server is unreachable: play with the local saves.".into(),
        Decision::Held => "Another machine has the game open.".into(),
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
    let id = &game.manifest.id;
    let (state, _, reachable) = read_server(s, id)?;
    let local = scan(
        &game.saves_dir,
        &game.manifest.include,
        &game.manifest.exclude,
    )?;
    let last = store.load_last(id)?;
    let view = server_view(state.as_ref(), reachable);
    let would = plan::decide(Intent::Launch, &local, last.as_ref(), &view);
    Ok(StatusData {
        game: id.clone(),
        // The service is the CLI's to name (it holds the config); `sync` does not know it.
        service: String::new(),
        reachable,
        would,
        server: state.as_ref().map(server_side),
        local: local_side(s, &local, last.as_ref()),
        last_synced_revision: last.as_ref().map(|last| last.revision),
        local_changed: local_changed(&local, last.as_ref()),
        message: status_message(would),
        // The open-elsewhere mark (M11) and pending choices (M12) are not read yet.
        held: None,
        pending: None,
    })
}

/// The game's history entries, newest first (M4), each with its card (`transfer::read_card`,
/// `null` for old entries). Unreachable is an error here.
pub fn history(s: &Session, game: &str) -> Result<HistoryData> {
    let (state, _, reachable) = read_server(s, game)?;
    if !reachable {
        return Err(Error::Unreachable(format!(
            "cannot list the history of {game}"
        )));
    }
    let current = state.as_ref().map(|state| state.entry.clone());
    let mut entries: Vec<HistoryEntry> = history_children(s, game)?
        .into_iter()
        .filter(|child| child.is_dir)
        .filter_map(|child| {
            let (stamp_text, device) = parse_entry_name(&child.name)?;
            let unix = stamp::parse_entry_stamp(&stamp_text)?;
            Some(HistoryEntry {
                current: current.as_deref() == Some(child.name.as_str()),
                entry: child.name,
                device,
                time: stamp::rfc3339(unix),
                // Cards are not read yet (`transfer::read_card` is M14): every entry has none.
                card: None,
            })
        })
        .collect();
    entries.sort_by(|left, right| right.entry.cmp(&left.entry));
    Ok(HistoryData {
        game: game.to_string(),
        current,
        entries,
    })
}

/// Bring back history entry `entry` (M4): keep unsynced local changes as a backup entry first,
/// copy `entry` server-side to a new entry, write it as a new revision, download it, record it.
///
/// Order: the entry is checked against the listing (`Error::NoSuchEntry`, nothing written); its
/// file list is read; the backup (if the local files changed since the last sync) is uploaded;
/// the folder is copied; `state.json` moves to the copy with `Precondition::Match` (a `Changed`
/// removes the copy again, as `push` does); the files are downloaded; `LastSynced` is saved; then
/// retention runs, with the backup taking a slot as in `push`.
pub fn restore(s: &Session, game: &Game, store: &LocalStore, entry: &str) -> Result<RestoreData> {
    let id = &game.manifest.id;
    let (state, pre, reachable) = read_server(s, id)?;
    if !reachable {
        return Err(Error::Unreachable(format!(
            "cannot restore {entry} of {id}"
        )));
    }
    let listed: Vec<String> = history_children(s, id)?
        .into_iter()
        .filter(|child| child.is_dir)
        .map(|child| child.name)
        .collect();
    if !listed.iter().any(|name| name == entry) {
        return Err(Error::NoSuchEntry(entry.to_string()));
    }
    let wanted = transfer::read_record(s.remote, &s.layout, id, entry)?;
    let local = scan(
        &game.saves_dir,
        &game.manifest.include,
        &game.manifest.exclude,
    )?;
    let last = store.load_last(id)?;
    let mut taken = listed;
    let mut counts = transfer::Counts::default();
    let backup = if local_changed(&local, last.as_ref()) {
        let name = fresh_entry(s.now, &s.device, &taken);
        counts = transfer::upload_entry(s.remote, &s.layout, id, &name, &game.saves_dir, &local)?;
        taken.push(name.clone());
        Some(name)
    } else {
        None
    };
    let new_entry = fresh_entry(s.now, &s.device, &taken);
    s.remote.copy(
        &s.layout.entry_dir(id, entry),
        &s.layout.entry_dir(id, &new_entry),
    )?;
    let revision = state
        .as_ref()
        .map_or(0, |state| state.revision)
        .checked_add(1)
        .ok_or_else(|| Error::StateInvalid("revision overflow".into()))?;
    let next = ServerState {
        format: FORMAT,
        game: id.clone(),
        revision,
        device: s.device.clone(),
        time: stamp::rfc3339(s.now),
        entry: new_entry.clone(),
        files: wanted.files.clone(),
    };
    if let Err(error) = s
        .remote
        .put(&s.layout.state_path(id), next.to_json().as_bytes(), &pre)
    {
        if error == Error::Changed {
            // A rejected conditional write cannot have published this copy.
            let _ = s.remote.delete(&s.layout.entry_dir(id, &new_entry));
        }
        return Err(error);
    }
    let downloaded = transfer::download_entry(
        s.remote,
        &s.layout,
        id,
        &new_entry,
        &wanted,
        &game.saves_dir,
        &local,
    )?;
    counts.downloaded = downloaded.downloaded;
    counts.deleted = downloaded.deleted;
    record_sync(s, store, id, revision, &wanted)?;
    let entries = history_children(s, id)?
        .into_iter()
        .filter(|child| child.is_dir && backup.as_deref() != Some(child.name.as_str()))
        .map(|child| child.name)
        .collect::<Vec<_>>();
    // Reserve a slot for the backup, as `push` does.
    let keep = retention::KEEP - usize::from(backup.is_some());
    for old in retention::to_drop(&entries, keep, &new_entry) {
        s.remote.delete(&s.layout.entry_dir(id, &old))?;
    }
    Ok(RestoreData {
        game: id.clone(),
        entry: entry.to_string(),
        revision,
        message: format!("Restored {entry} as revision {revision}."),
        new_entry,
        backup,
        counts,
    })
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
