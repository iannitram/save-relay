//! The engine: each CLI command as one function over a `Remote`, a `LocalStore` and the saves
//! folder. It reads, asks `plan::decide`, then acts through `transfer`, writes `state.json` with
//! `Precondition::Match` on the etag it read (so a second machine pushing at once gets
//! `Error::Changed`, never a lost revision: If-Match on Nextcloud, the rev on Dropbox, the
//! claim guard on Google Drive, M9), applies `retention`, and records `LastSynced`. It never
//! knows which service it talks to.
//!
//! `pull` reads the game's open-elsewhere mark (`presence`, M11), asks `plan::decide_with_mark` and
//! writes this machine's mark after a launch that synced; `push` clears it again
//! (docs/adr/0003-baton-wrap-cards-panel.md). Each new history entry gets a save card
//! (`transfer::put_card`, M14), and `pull` starts this machine's play session (`local::Session`)
//! so the card can carry the playtime.

use crate::api::{
    Action, Held, HistoryData, HistoryEntry, PictureData, RestoreData, Side, StatusData, SyncData,
    SyncReport, WhereData, WhereGame,
};
use crate::card::{self, Card, PictureKind};
use crate::error::{Error, Result};
use crate::layout::{entry_name, parse_entry_name, Layout};
use crate::local::LocalStore;
use crate::manifest::Manifest;
use crate::plan::{self, Decision, Intent, ServerView};
use crate::presence::{self, Hold, Mark};
use crate::remote::{ensure_dir, Precondition, Remote, RemoteEntry};
use crate::scan::{scan, SaveSet};
use crate::state::{LastSynced, ServerState, FORMAT};
use crate::{retention, stamp, transfer};
use std::fs;
use std::io::{Read, Write};
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

/// Before Play (M1, M3, M5, M11). The mark is read first (`read_mark`: no file, or one that does
/// not parse, is no mark), then `plan::decide_with_mark(Intent::Launch, ..)`: Noop; Adopt (record
/// only); Pull (download the current entry); Push (offline play caught up: upload as a new
/// revision); Offline (touch nothing, `Action::Offline`); Conflict (touch nothing,
/// `Action::Conflict`) unless `resolve` is given, which settles it as described on `Resolve`;
/// Held (another machine's mark and not `anyway`: touch nothing, `Action::Held`, `held` set).
/// After Noop, Adopt, Pull or Push, even a resolved conflict, this machine writes its mark
/// (`write_mark`) and starts its play session (`local::Session`, M14); offline and an unresolved
/// conflict write nothing. The report carries both sides' cards when there are two (M14).
pub fn pull(
    s: &Session,
    game: &Game,
    store: &LocalStore,
    resolve: Option<Resolve>,
    anyway: bool,
) -> Result<SyncReport> {
    sync(s, game, store, resolve, Intent::Launch, anyway)
}

/// After the game quits (M2, M3, M5, M11). `plan::decide(Intent::Exit, ..)`, then: Push (upload,
/// new revision with this device and time, then retention); unreachable is `Action::Deferred`;
/// the rest as in `pull`. The mark is not read to decide: at exit `decide_with_mark` ignores it.
/// After Noop, Adopt, Pull (a resolved conflict) or Push, `clear_mark` removes `playing.json`, but
/// only when it names this device: another machine's mark stays. A `deferred` push and an
/// unresolved conflict leave it, since the saves are still only here.
///
/// Each new entry gets a card (`card::for_push`: the session start for playtime, the sidecar's
/// place and picture) through `transfer::put_card` after the MOVE and before `state.json`. After
/// a synced exit the session start is cleared; a `deferred` or unresolved push keeps it.
pub fn push(
    s: &Session,
    game: &Game,
    store: &LocalStore,
    resolve: Option<Resolve>,
) -> Result<SyncReport> {
    sync(s, game, store, resolve, Intent::Exit, false)
}

// The mark (M11) is read and written by `pull`, and cleared by `push`; the session start (M14)
// follows it; the cards (M14) travel with each new entry.
fn sync(
    s: &Session,
    game: &Game,
    store: &LocalStore,
    resolve: Option<Resolve>,
    intent: Intent,
    anyway: bool,
) -> Result<SyncReport> {
    let id = &game.manifest.id;
    // Only a launch reads the mark: a push decides without it.
    let hold = match intent {
        Intent::Launch => presence::check(read_mark(s, id)?.as_ref(), &s.device),
        Intent::Exit => Hold::Free,
    };
    let (state, pre, reachable) = read_server(s, id)?;
    let local = scan(
        &game.saves_dir,
        &game.manifest.include,
        &game.manifest.exclude,
    )?;
    let last = store.load_last(id)?;
    let view = server_view(state.as_ref(), reachable);
    // The sidecar's place and picture for the cards, and a note for each part left out.
    let mut found = found_in(&game.saves_dir);
    let has_local = !local.is_empty();
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
    let mut held = None;
    // The entry `state.json` names once this command is done; `None` without a server state.
    let mut server_entry = state.as_ref().map(|state| state.entry.clone());
    let mut decision = plan::decide_with_mark(intent, &hold, anyway, &local, last.as_ref(), &view);
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
                // The kept backup carries this machine's card too; it is no session's push, so no playtime.
                let card = card_of(s, &found, None);
                write_card(s, id, &entry, &card, &mut found);
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
                let wanted = transfer::without_reserved(&state.save_set());
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
            // The playtime is the session's, and only an exit has one: a launch's push is no play.
            let started = match intent {
                Intent::Exit => session_start(store, id, &mut found.notes),
                Intent::Launch => None,
            };
            let card = card_of(s, &found, started);
            write_card(s, id, &entry, &card, &mut found);
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
            server_entry = Some(entry.clone());
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
        Decision::Held => match hold {
            Hold::Elsewhere { device, since } => {
                let by = Held { device, since };
                data.action = Action::Held;
                data.revision = None;
                data.message = held_message(&by);
                held = Some(by);
            }
            Hold::Free | Hold::Mine => {
                unreachable!("decide_with_mark answers Held only for another machine's mark")
            }
        },
    }
    if synced(data.action) {
        let outcome = match intent {
            Intent::Launch => write_mark(s, id, data.revision),
            Intent::Exit => clear_mark(s, id),
        };
        if let Err(error) = outcome {
            let verb = match intent {
                Intent::Launch => "write",
                Intent::Exit => "clear",
            };
            // The saves are already in step; the mark is advice, so this is not the command's error.
            data.message = format!("{} Could not {verb} playing.json: {error}.", data.message);
        }
        // The playtime start follows the mark (M14): a launch that synced starts the session, an
        // exit that synced ends it. Not the command's error either.
        let session = match intent {
            Intent::Launch => store.save_session(&crate::local::Session {
                format: crate::local::FORMAT,
                game: id.clone(),
                started: s.now,
            }),
            Intent::Exit => store.clear_session(id),
        };
        if let Err(error) = session {
            data.message = format!(
                "{} Could not update the session start: {error}.",
                data.message
            );
        }
    }
    let server_card = match &server_entry {
        Some(entry) => match transfer::read_card(s.remote, &s.layout, id, entry) {
            Ok(card) => card,
            Err(error) => {
                found.notes.push(format!("card not read: {error}"));
                None
            }
        },
        None => None,
    };
    let local_card = has_local.then(|| card_of(s, &found, None));
    for note in &found.notes {
        data.message = format!("{} {}.", data.message, one_line(note));
    }
    Ok(SyncReport {
        sync: data,
        held,
        server_card,
        local_card,
    })
}

/// What this machine's saves folder says about a card: the sidecar's place and checked picture,
/// and one note (a short line for `message`) for each part that was left out.
#[derive(Debug, Default)]
struct Found {
    /// The sidecar's place as written; `card::for_push` cleans it.
    place: Option<String>,
    picture: Option<(PictureKind, Vec<u8>)>,
    notes: Vec<String>,
}

/// A note as one line of `message`: each control character, a newline included, becomes a space.
/// A note can quote the sidecar's picture path, and that path is not checked for control characters.
fn one_line(note: &str) -> String {
    note.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Read the sidecar (`card::SIDECAR`) in `saves_dir` and the picture it names. Nothing there is
/// nothing found, with no note; a bad sidecar or picture is a note, never an error.
fn found_in(saves_dir: &Path) -> Found {
    let mut found = Found::default();
    let text = match fs::read_to_string(saves_dir.join(card::SIDECAR)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return found,
        Err(error) => {
            found.notes.push(format!("sidecar left out: {error}"));
            return found;
        }
    };
    let sidecar = match card::parse_sidecar(&text) {
        Ok(sidecar) => sidecar,
        Err(note) => {
            found.notes.push(note);
            return found;
        }
    };
    found.place = sidecar.place;
    if let Some(relative) = sidecar.picture {
        match read_picture(&saves_dir.join(&relative)) {
            Ok(bytes) => match card::check_picture(&bytes) {
                Ok(kind) => found.picture = Some((kind, bytes)),
                Err(note) => found.notes.push(note),
            },
            Err(note) => found.notes.push(note),
        }
    }
    found
}

/// The picture's bytes, read to one byte past the size limit, so a huge file is refused by
/// `card::check_picture` without being loaded whole.
fn read_picture(path: &Path) -> std::result::Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|error| format!("picture left out: {error}"))?;
    let mut bytes = Vec::new();
    file.take(card::MAX_PICTURE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("picture left out: {error}"))?;
    Ok(bytes)
}

/// The card for this machine's saves at `s.now`, with a playtime from `started` (`None` for no
/// playtime).
fn card_of(s: &Session, found: &Found, started: Option<u64>) -> Card {
    card::for_push(
        &s.device,
        s.now,
        started,
        found.place.as_deref(),
        found.picture.as_ref().map(|(kind, _)| *kind),
    )
}

/// The session start (M14) for a push at exit. Unreadable is a note and no playtime, never an
/// error: the card is optional.
fn session_start(store: &LocalStore, game: &str, notes: &mut Vec<String>) -> Option<u64> {
    match store.load_session(game) {
        Ok(session) => session.map(|session| session.started),
        Err(error) => {
            notes.push(format!("playtime left out: {error}"));
            None
        }
    }
}

/// Write the card and picture of a new entry. A failure is a note, not an error: the entry is
/// complete, so the save still goes up (a crash in between also reads as `card: null`).
fn write_card(s: &Session, game: &str, entry: &str, card: &Card, found: &mut Found) {
    let picture = found
        .picture
        .as_ref()
        .map(|(kind, bytes)| (kind.file_name(), bytes.as_slice()));
    if let Err(error) = transfer::put_card(s.remote, &s.layout, game, entry, card, picture) {
        found.notes.push(format!("card left out: {error}"));
    }
}

/// The line `pull` and `status` give for another machine's mark (M11).
fn held_message(held: &Held) -> String {
    format!("{} has the game open since {}.", held.device, held.since)
}

/// The actions after which this machine's saves and the server's revision agree, so the mark is
/// written (launch) or cleared (exit). Not after `offline`, `deferred`, an unresolved `conflict`
/// or `held`: then the saves are still only here, or nothing was decided.
fn synced(action: Action) -> bool {
    matches!(
        action,
        Action::Noop | Action::Adopted | Action::Pulled | Action::Pushed
    )
}

/// This game's mark (M11). No file, a file that is not a valid `Mark` (a broken mark is no mark)
/// and an unreachable server are all `None`.
fn read_mark(s: &Session, game: &str) -> Result<Option<Mark>> {
    match s.remote.get(&s.layout.playing_path(game)) {
        Ok((bytes, _)) => Ok(std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| Mark::from_json(text).ok())),
        Err(Error::NotFound(_) | Error::Unreachable(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes this machine's mark after a launch that synced (M11). `Precondition::Any`: the mark is
/// advice and takes no part in the compare-and-swap of `state.json`. The game's folder is made
/// first, for a game nothing was ever pushed for.
fn write_mark(s: &Session, game: &str, revision_read: Option<u64>) -> Result<()> {
    let mark = Mark {
        format: presence::FORMAT,
        device: s.device.clone(),
        since: stamp::rfc3339(s.now),
        revision_read,
    };
    ensure_dir(s.remote, &s.layout.game_dir(game))?;
    s.remote.put(
        &s.layout.playing_path(game),
        mark.to_json().as_bytes(),
        &Precondition::Any,
    )?;
    Ok(())
}

/// Removes `playing.json` only when it names this device (M11): read, `presence::check` is `Mine`,
/// then delete. Another machine's mark, and a broken one, stay. A file already gone is fine.
fn clear_mark(s: &Session, game: &str) -> Result<()> {
    let mark = read_mark(s, game)?;
    if presence::check(mark.as_ref(), &s.device) != Hold::Mine {
        return Ok(());
    }
    match s.remote.delete(&s.layout.playing_path(game)) {
        Err(Error::NotFound(_)) => Ok(()),
        other => other,
    }
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

/// What `plan::decide_with_mark` gets as the server's side (reserved names left out).
fn server_view(state: Option<&ServerState>, reachable: bool) -> ServerView {
    match state {
        Some(state) => ServerView::At {
            revision: state.revision,
            files: transfer::without_reserved(&state.save_set()),
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
    let hold = presence::check(read_mark(s, id)?.as_ref(), &s.device);
    let (state, _, reachable) = read_server(s, id)?;
    let local = scan(
        &game.saves_dir,
        &game.manifest.include,
        &game.manifest.exclude,
    )?;
    let last = store.load_last(id)?;
    let view = server_view(state.as_ref(), reachable);
    let would = plan::decide_with_mark(Intent::Launch, &hold, false, &local, last.as_ref(), &view);
    let held = match hold {
        Hold::Elsewhere { device, since } if would == Decision::Held => {
            Some(Held { device, since })
        }
        _ => None,
    };
    let message = match &held {
        Some(by) => held_message(by),
        None => status_message(would),
    };
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
        message,
        // Another machine's mark (M11); pending choices (M12) are not read yet.
        held,
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
    let mut entries: Vec<HistoryEntry> = Vec::new();
    for child in history_children(s, game)?
        .into_iter()
        .filter(|child| child.is_dir)
    {
        let Some((stamp_text, device)) = parse_entry_name(&child.name) else {
            continue;
        };
        let Some(unix) = stamp::parse_entry_stamp(&stamp_text) else {
            continue;
        };
        let card = transfer::read_card(s.remote, &s.layout, game, &child.name)?;
        entries.push(HistoryEntry {
            current: current.as_deref() == Some(child.name.as_str()),
            device,
            time: stamp::rfc3339(unix),
            card,
            entry: child.name,
        });
    }
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
    let wanted =
        transfer::without_reserved(&transfer::read_record(s.remote, &s.layout, id, entry)?);
    let local = scan(
        &game.saves_dir,
        &game.manifest.include,
        &game.manifest.exclude,
    )?;
    let last = store.load_last(id)?;
    let mut taken = listed;
    let mut counts = transfer::Counts::default();
    // Notes from the backup's card (sidecar, picture, a failed card write); never an error.
    let mut notes = Vec::new();
    let backup = if local_changed(&local, last.as_ref()) {
        let name = fresh_entry(s.now, &s.device, &taken);
        counts = transfer::upload_entry(s.remote, &s.layout, id, &name, &game.saves_dir, &local)?;
        // The backup carries this machine's card, as `pull`'s keep-server backup does; no playtime.
        let mut found = found_in(&game.saves_dir);
        let card = card_of(s, &found, None);
        write_card(s, id, &name, &card, &mut found);
        notes = found.notes;
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
    let mut message = format!("Restored {entry} as revision {revision}.");
    for note in &notes {
        message = format!("{message} {}.", one_line(note));
    }
    Ok(RestoreData {
        game: id.clone(),
        entry: entry.to_string(),
        revision,
        message,
        new_entry,
        backup,
        counts,
    })
}

/// `save-relay where` (M11): for each game in `games` (`LocalStore::list_games`), its mark and
/// the server's current side. A missing or broken mark is no mark. Unreachable is an error.
pub fn where_open(s: &Session, games: &[String]) -> Result<WhereData> {
    let mut rows = Vec::with_capacity(games.len());
    for game in games {
        let held = fetch_mark(s, game)?.map(|mark| Held {
            device: mark.device,
            since: mark.since,
        });
        let (state, _, reachable) = read_server(s, game)?;
        if !reachable {
            return Err(Error::Unreachable(format!(
                "cannot read where {game} is open"
            )));
        }
        rows.push(WhereGame {
            game: game.clone(),
            held,
            server: state.as_ref().map(server_side),
        });
    }
    Ok(WhereData { games: rows })
}

/// This game's mark for `where`: no file, or one that is not a valid `Mark`, is `None`. Unlike
/// `read_mark`, an unreachable server is an error, not "no mark".
fn fetch_mark(s: &Session, game: &str) -> Result<Option<Mark>> {
    match s.remote.get(&s.layout.playing_path(game)) {
        Ok((bytes, _)) => Ok(std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| Mark::from_json(text).ok())),
        Err(Error::NotFound(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// `save-relay picture <game> <entry> --out <file>` (M14): copy the entry's card picture to
/// `out` (through a part file). No card, or a card with no picture, is `Error::NotFound`.
pub fn picture(s: &Session, game: &str, entry: &str, out: &Path) -> Result<PictureData> {
    let missing = || Error::NotFound(format!("a picture for {game} {entry}"));
    // A name that is not an entry name has no card either.
    if parse_entry_name(entry).is_none() {
        return Err(missing());
    }
    let card = transfer::read_card(s.remote, &s.layout, game, entry)?.ok_or_else(missing)?;
    let (bytes, _mime) = transfer::get_picture(s.remote, &s.layout, game, entry, &card)?;
    write_through_part(out, &bytes)?;
    Ok(PictureData {
        path: out.display().to_string(),
        bytes: bytes.len() as u64,
    })
}

/// Write `bytes` to `out` through a part file beside it, renamed into place once complete.
fn write_through_part(out: &Path, bytes: &[u8]) -> Result<()> {
    let mut part_name = out.as_os_str().to_os_string();
    part_name.push(transfer::PART);
    let part = PathBuf::from(part_name);
    let written = fs::File::create(&part).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    let result = written
        .map_err(|error| Error::Io(format!("{}: {error}", part.display())))
        .and_then(|()| {
            fs::rename(&part, out).map_err(|error| Error::Io(format!("{}: {error}", out.display())))
        });
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}
