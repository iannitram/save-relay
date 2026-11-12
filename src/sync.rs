//! The engine: each CLI command as one function over a `Remote`, a `LocalStore` and the saves
//! folder. It reads, asks `plan::decide`, then acts through `transfer`, writes `state.json` with
//! `Precondition::Match` on the etag it read (so a second machine pushing at once gets
//! `Error::Changed`, never a lost revision: If-Match on WebDAV, behind a `rev/<n>` claim where
//! (#149), the rev on Dropbox, the claim guard on Google Drive, M9), applies `retention`, and
//! records `LastSynced`. It never knows which service it talks to.
//!
//! `pull` reads the game's open-elsewhere mark (`presence`, M11), asks `plan::decide_with_mark` and
//! writes this machine's mark after a launch that synced; `push` clears it again
//! (docs/adr/0003-baton-wrap-cards-panel.md). Each new history entry gets a save card
//! (`transfer::put_card`, M14), and `pull` starts this machine's play session (`local::Session`)
//! so the card can carry the playtime.

use crate::api::{
    Action, Held, HistoryData, HistoryEntry, PictureData, RepairData, RepairMarker, RestoreData,
    Side, StatusData, SyncData, SyncReport, WhereData, WhereGame,
};
use crate::card::{self, Card, PictureKind};
use crate::error::{Error, Result};
use crate::layout::{entry_name, parse_entry_name, Layout};
use crate::local::LocalStore;
use crate::manifest::Manifest;
use crate::plan::{self, Decision, Intent, ServerView};
use crate::platform::{PathVars, Platform};
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
/// new revision with this device and time, then the record, then retention, best-effort);
/// unreachable is `Action::Deferred`; the rest as in `pull`. The mark is not read to decide: at exit `decide_with_mark` ignores it.
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
        Intent::Launch => launch_hold(s, id)?,
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
    let folder = canonical_saves_folder(&game.saves_dir);
    let mut decision = plan::decide_with_mark_for_folder(
        intent,
        &hold,
        anyway,
        &local,
        last.as_ref(),
        &folder,
        &view,
    );
    if decision == Decision::Conflict {
        match resolve {
            Some(Resolve::KeepServer) => {
                let taken = history_names(s, id)?;
                let entry = fresh_entry(s.now, &s.device, &taken);
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
            record_sync(s, store, id, state.revision, &files, &game.saves_dir)?;
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
            let taken = history_names(s, id)?;
            let entry = fresh_entry(s.now, &s.device, &taken);
            data.counts =
                transfer::upload_entry(s.remote, &s.layout, id, &entry, &game.saves_dir, &local)?;
            // The playtime is the session's, and only an exit has one: a launch's push is no play.
            let started = match intent {
                Intent::Exit => session_start(store, id, &mut found.notes),
                Intent::Launch => None,
            };
            if found.picture.is_none() {
                if let Some(started) = started {
                    add_steam_screenshot(id, started, s.now, &mut found);
                }
            }
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
            if let Err(error) = write_state(s, Some(store), id, revision, &entry, &next, &pre) {
                if error == Error::Changed
                    && claim_looks_stranded(s, id, revision, &pre, state.as_ref())
                {
                    return Err(Error::ChangedRepair(format!("another machine changed the saves on the server meanwhile; if a revision claim was stranded, inspect it with `save-relay repair {id}`")));
                }
                return Err(error);
            }
            server_entry = Some(entry.clone());
            record_sync(s, store, id, revision, &local, &game.saves_dir)?;
            prune_after_publish(s, id, data.backup.as_deref(), &entry, revision);
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

/// For a wrapped Steam game, use its newest screenshot captured during this play session when
/// the port did not supply a checked sidecar picture. Any lookup or validation failure only
/// leaves the optional card picture out.
fn add_steam_screenshot(game: &str, started: u64, now: u64, found: &mut Found) {
    let Some(app_id) = game.strip_prefix("steam-").and_then(|id| id.parse().ok()) else {
        return;
    };
    let env = |name: &str| std::env::var(name).ok();
    let platform = Platform::current();
    let vars = match PathVars::from_env(platform, &env) {
        Ok(vars) => vars,
        Err(error) => {
            found
                .notes
                .push(format!("Steam screenshot left out: {error}"));
            return;
        }
    };
    let root = match crate::steam::find_root(platform, &vars, &env) {
        Ok(root) => root,
        Err(error) => {
            found
                .notes
                .push(format!("Steam screenshot left out: {error}"));
            return;
        }
    };
    let Some(path) = crate::steam::screenshot_file(&root, app_id, started, now) else {
        return;
    };
    match read_picture(&path)
        .and_then(|bytes| card::check_picture(&bytes).map(|kind| (kind, bytes)))
    {
        Ok(picture) => found.picture = Some(picture),
        Err(note) => found.notes.push(note),
    }
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

/// This game's mark (M11). No file, and a file that is not a valid `Mark` (a broken mark is no
/// mark), are `None`. An unreachable server is an error: it is not "no mark".
fn read_mark(s: &Session, game: &str) -> Result<Option<Mark>> {
    fetch_mark(s, game)
}

/// The hold a launch (or `status`) sees. An unreachable mark read is "no mark" only when the
/// server is unreachable as a whole (the launch then goes offline); if `state.json` answers, a
/// failed mark read was a blip and the other machine's mark may be there, so it is an error.
fn launch_hold(s: &Session, game: &str) -> Result<Hold> {
    match read_mark(s, game) {
        Ok(mark) => Ok(presence::check(mark.as_ref(), &s.device)),
        Err(error @ Error::Unreachable(_)) => match read_server(s, game)? {
            (_, _, false) => Ok(Hold::Free),
            _ => Err(error),
        },
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
    let mark = match read_mark(s, game) {
        Err(Error::Unreachable(_)) => return Ok(()),
        other => other?,
    };
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

/// Writes `state.json` for revision `revision`, which names `entry`, with `pre`. On a backend that
/// needs the revision claim (`Remote::needs_rev_marker`, #149) the write comes second: only the
/// write that creates `rev/<revision>` may write `state.json`. A refused claim or write is
/// `Error::Changed` (or Drive's confirmed missing-claim failure), and `entry` is removed: a
/// rejected conditional write cannot have published it. Other failures may be ambiguous, so
/// `entry` stays in history. A failed state write releases the claim only when the claim names
/// `entry` (its own), on a best-effort basis; a claim another machine took is never touched. A
/// crash, a dropped link or a failed cleanup can strand it: the same machine's next push releases
/// its own stranded claim (`claim_revision`, #242 S1), and `repair` releases the rest safely
/// (#159, ADR 0002). After a publish on WebDAV, an entry that `repair` set aside while this push
/// was paused is moved back into `history/`, so `state.json` never names a set-aside entry (S2).
fn write_state(
    s: &Session,
    store: Option<&LocalStore>,
    id: &str,
    revision: u64,
    entry: &str,
    next: &ServerState,
    pre: &Precondition,
) -> Result<()> {
    let name = revision.to_string();
    if let Err(error) = claim_revision(s, id, &name, entry) {
        if matches!(error, Error::Changed | Error::MissingDriveClaim { .. }) {
            let _ = s.remote.delete(&s.layout.entry_dir(id, entry));
        }
        return Err(error);
    }
    if let Err(error) = s
        .remote
        .put(&s.layout.state_path(id), next.to_json().as_bytes(), pre)
    {
        if let Error::MissingDriveClaim {
            path,
            file_id,
            version,
            claim,
        } = &error
        {
            if let Some(store) = store {
                let _ = store.save_drive_claim_evidence(
                    id,
                    &crate::local::DriveClaimEvidence {
                        format: crate::local::FORMAT,
                        state_path: path.clone(),
                        file_id: file_id.clone(),
                        version: *version,
                        claim: claim.clone(),
                    },
                );
            }
        }
        if matches!(error, Error::Changed | Error::MissingDriveClaim { .. }) {
            let _ = s.remote.delete(&s.layout.entry_dir(id, entry));
        }
        if s.remote.needs_rev_marker() && claim_body(s, id, &name).as_deref() == Some(entry) {
            let _ = s.remote.delete(&s.layout.rev_path(id, &name));
        }
        return Err(error);
    }
    if s.remote.needs_rev_marker() {
        restore_set_aside_entry(s, id, revision, entry)?;
    }
    Ok(())
}

/// The body of `rev/<name>` (the entry it names), when it can be read.
fn claim_body(s: &Session, id: &str, name: &str) -> Option<String> {
    s.remote
        .get(&s.layout.rev_path(id, name))
        .ok()
        .and_then(|(bytes, _)| String::from_utf8(bytes).ok())
}

/// Moves `entry` back from `set-aside/` into `history/` when `repair` moved it while this push
/// was paused (S2). `repair` checks `state.json` after its move and moves the entry back itself
/// when the state moved; this check covers the other order, a publish after that check. A move
/// that finds the entry already back is not an error.
fn restore_set_aside_entry(s: &Session, id: &str, revision: u64, entry: &str) -> Result<()> {
    if history_names(s, id)?.iter().any(|name| name == entry) {
        return Ok(());
    }
    let from = s.layout.set_aside_path(id, revision, entry);
    match s.remote.move_to(&from, &s.layout.entry_dir(id, entry)) {
        Ok(()) | Err(Error::NotFound(_)) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Creates `rev/<name>` (its body: the entry it names) with `Precondition::Absent`, on a backend
/// that needs the claim (#149). Other backends: nothing. A refused create is `Error::Changed`,
/// unless the refusing claim names this machine's own device: a push that lost its link after
/// its claim left that claim behind, and no one else can release it, so this machine releases it
/// and claims again (#242 S1). Another machine's claim is never released here (`repair` does that).
fn claim_revision(s: &Session, id: &str, name: &str, entry: &str) -> Result<()> {
    if !s.remote.needs_rev_marker() {
        return Ok(());
    }
    ensure_dir(s.remote, &s.layout.rev_dir(id))?;
    let path = s.layout.rev_path(id, name);
    match s.remote.put(&path, entry.as_bytes(), &Precondition::Absent) {
        Err(Error::Changed) if own_stranded_claim(s, id, name) => {
            s.remote.delete(&path)?;
            s.remote
                .put(&path, entry.as_bytes(), &Precondition::Absent)
                .map(|_| ())
        }
        result => result.map(|_| ()),
    }
}

/// True when `rev/<name>` names an entry of this machine's own device (#242 S1).
fn own_stranded_claim(s: &Session, id: &str, name: &str) -> bool {
    claim_body(s, id, name)
        .and_then(|body| parse_entry_name(&body))
        .is_some_and(|(_, device)| device == s.device)
}

/// Retention and marker pruning, run after `state.json` is published and `LastSynced` is saved
/// (S9). Best-effort: the push or restore has happened, so a failed DELETE leaves old entries or
/// claims for a later push to prune and does not fail the command.
fn prune_after_publish(s: &Session, id: &str, backup: Option<&str>, newest: &str, revision: u64) {
    let _ = retain_entries(s, id, backup, newest);
    let _ = prune_markers(s, id, revision);
}

/// Drops the oldest history entries beyond `retention::KEEP`. The backup (the conflict's losing
/// side, or a restore's backup) takes a slot, even with an old clock.
fn retain_entries(s: &Session, id: &str, backup: Option<&str>, newest: &str) -> Result<()> {
    let entries = history_children(s, id)?
        .into_iter()
        .filter(|child| child.is_dir && backup != Some(child.name.as_str()))
        .map(|child| child.name)
        .collect::<Vec<_>>();
    let keep = retention::KEEP - usize::from(backup.is_some());
    for old in retention::to_drop(&entries, keep, newest) {
        s.remote.delete(&s.layout.entry_dir(id, &old))?;
    }
    Ok(())
}

/// Deletes the revision claims beyond the newest `retention::KEEP`, with the entries (#149). A
/// claim older than the current revision is never needed again: a push reads the current
/// revision and claims the next one. Other backends: nothing.
fn prune_markers(s: &Session, id: &str, revision: u64) -> Result<()> {
    if !s.remote.needs_rev_marker() {
        return Ok(());
    }
    let names: Vec<String> = match s.remote.list(&s.layout.rev_dir(id)) {
        Ok(children) => children
            .into_iter()
            .filter(|child| !child.is_dir)
            .map(|child| child.name)
            .collect(),
        Err(Error::NotFound(_)) => return Ok(()),
        Err(error) => return Err(error),
    };
    for old in retention::markers_to_drop(&names, retention::KEEP, revision) {
        s.remote.delete(&s.layout.rev_path(id, &old))?;
    }
    Ok(())
}

/// The entry names under `history/`; none yet is an empty list.
fn history_names(s: &Session, game: &str) -> Result<Vec<String>> {
    Ok(history_children(s, game)?
        .into_iter()
        .map(|entry| entry.name)
        .collect())
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
    saves_dir: &Path,
) -> Result<()> {
    store.save_last(&LastSynced {
        format: FORMAT,
        game: game.into(),
        revision,
        time: stamp::rfc3339(s.now),
        saves_folder: Some(canonical_saves_folder(saves_dir)),
        files: files.files.clone(),
    })
}

/// Stable folder identity for a last-sync record, including when the folder is not mounted yet.
fn canonical_saves_folder(folder: &Path) -> String {
    if let Ok(path) = fs::canonicalize(folder) {
        return path.to_string_lossy().into_owned();
    }
    let absolute = if folder.is_absolute() {
        folder.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(folder)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    let mut ancestor = normalized.as_path();
    let mut missing = Vec::new();
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name() else {
            break;
        };
        missing.push(name.to_os_string());
        let Some(parent) = ancestor.parent() else {
            break;
        };
        ancestor = parent;
    }
    let mut canonical = fs::canonicalize(ancestor).unwrap_or_else(|_| ancestor.to_path_buf());
    for component in missing.into_iter().rev() {
        canonical.push(component);
    }
    canonical.to_string_lossy().into_owned()
}

/// What `pull` would do, changing nothing. Unreachable is `reachable: false`, not an error.
/// `held` is another machine's mark; `pending` is this machine's pending choice, if any.
pub fn status(s: &Session, game: &Game, store: &LocalStore) -> Result<StatusData> {
    let id = &game.manifest.id;
    let hold = launch_hold(s, id)?;
    let (state, _, reachable) = read_server(s, id)?;
    let local = scan(
        &game.saves_dir,
        &game.manifest.include,
        &game.manifest.exclude,
    )?;
    let last = store.load_last(id)?;
    let view = server_view(state.as_ref(), reachable);
    let folder = canonical_saves_folder(&game.saves_dir);
    let would = plan::decide_with_mark_for_folder(
        Intent::Launch,
        &hold,
        false,
        &local,
        last.as_ref(),
        &folder,
        &view,
    );
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
        // Another machine's mark (M11) and this machine's pending choice (M12).
        held,
        pending: store.load_pending(id)?,
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

/// Inspect and, when `release` is set, repair Nextcloud revision claims (#159) and, with this
/// machine's verified evidence, a missing Google Drive state claim. A Nextcloud claim is
/// eligible only when it is exactly the next revision, `n+1`, where `n` is the revision in
/// `state.json`. Its named history entry is either incomplete or missing, and then the claim is
/// released; or it is complete, and then the entry is moved to `set-aside/rev<n+1>-<entry>` (kept
/// for recovery by hand, never deleted) before the claim is released. Claims above `n+1` and
/// claims whose body is not an entry name are kept. `state.json` is checked before each change
/// and again after a move, so a publication that races with repair is never overwritten.
pub fn repair(s: &Session, game: &str, release: bool) -> Result<RepairData> {
    repair_inner(s, game, release, None)
}

/// Repair with this machine's local Drive claim evidence available.
pub fn repair_with_store(
    s: &Session,
    game: &str,
    release: bool,
    store: &LocalStore,
) -> Result<RepairData> {
    repair_inner(s, game, release, Some(store))
}

fn repair_inner(
    s: &Session,
    game: &str,
    release: bool,
    store: Option<&LocalStore>,
) -> Result<RepairData> {
    let evidence = store
        .map(|store| store.load_drive_claim_evidence(game))
        .transpose()?
        .flatten();
    let claim_recovery = s.remote.repair_missing_state_claim(
        &s.layout.state_path(game),
        evidence.as_ref(),
        release,
    )?;
    if release
        && claim_recovery
            .as_deref()
            .is_some_and(|message| message.starts_with("rotated missing Drive claim "))
    {
        if let Some(store) = store {
            store.clear_drive_claim_evidence(game)?;
        }
    }
    if !s.remote.needs_rev_marker() {
        return Ok(RepairData {
            game: game.into(),
            revision: 0,
            markers: vec![],
            message: claim_recovery
                .clone()
                .unwrap_or_else(|| "nothing to repair".into()),
            claim_recovery,
        });
    }
    let (state, original_pre, reachable) = read_server(s, game)?;
    if !reachable {
        return Err(Error::Unreachable(format!(
            "cannot inspect claims for {game}"
        )));
    }
    let revision = state.as_ref().map_or(0, |state| state.revision);
    let children = match s.remote.list(&s.layout.rev_dir(game)) {
        Ok(children) => children,
        Err(Error::NotFound(_)) => vec![],
        Err(error) => return Err(error),
    };
    let mut markers = Vec::new();
    for child in children.into_iter().filter(|child| !child.is_dir) {
        let Some(number) = child
            .name
            .parse::<u64>()
            .ok()
            .filter(|n| *n > revision && n.to_string() == child.name)
        else {
            continue;
        };
        let entry = s
            .remote
            .get(&s.layout.rev_path(game, &child.name))
            .ok()
            .and_then(|(bytes, _)| String::from_utf8(bytes).ok())
            .filter(|name| parse_entry_name(name).is_some());
        let complete = entry
            .as_deref()
            .is_some_and(|name| complete_entry(s, game, name));
        let mut status = if number != revision.saturating_add(1) {
            "kept_later"
        } else if entry.is_none() {
            "unreadable"
        } else if complete {
            "releasable_set_aside"
        } else {
            "releasable"
        };
        let mut set_aside = None;
        if let (true, Some(name)) = (
            release && (status == "releasable" || status == "releasable_set_aside"),
            entry.as_deref(),
        ) {
            (status, set_aside) =
                release_claim(s, game, number, name, complete, &state, &original_pre)?;
        }
        markers.push(RepairMarker {
            revision: number,
            entry,
            status: status.into(),
            set_aside,
        });
    }
    let message = if markers.is_empty() {
        "nothing to repair".into()
    } else {
        format!(
            "Found {} revision marker(s): {}.",
            markers.len(),
            markers
                .iter()
                .map(|m| format!(
                    "revision {}: {}{}{}",
                    m.revision,
                    m.status,
                    m.entry
                        .as_ref()
                        .map(|e| format!(" ({e})"))
                        .unwrap_or_default(),
                    m.set_aside
                        .as_ref()
                        .map(|p| format!(", moved to {p}"))
                        .unwrap_or_default()
                ))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    Ok(RepairData {
        game: game.into(),
        revision,
        markers,
        claim_recovery,
        message,
    })
}

/// Releases the claim `rev/<number>` that names `entry`, once `state.json` is confirmed to be as
/// it was read. A complete entry is first moved to `set-aside/`; if `state.json` moved during
/// that move, the entry goes back to `history/` and nothing is released. Returns the status and
/// the set-aside path.
fn release_claim(
    s: &Session,
    game: &str,
    number: u64,
    entry: &str,
    complete: bool,
    state: &Option<ServerState>,
    pre: &Precondition,
) -> Result<(&'static str, Option<String>)> {
    if !state_unchanged(s, game, state, pre)? {
        return Ok(("skipped_state_moved", None));
    }
    let mut set_aside = None;
    if complete {
        let from = s.layout.entry_dir(game, entry);
        let to = s.layout.set_aside_path(game, number, entry);
        ensure_dir(s.remote, &s.layout.set_aside_dir(game))?;
        s.remote.move_to(&from, &to)?;
        if !state_unchanged(s, game, state, pre)? {
            // A paused push may have moved the entry back already (#242 S2): not an error.
            match s.remote.move_to(&to, &from) {
                Ok(()) | Err(Error::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
            return Ok(("skipped_state_moved", None));
        }
        set_aside = Some(to);
    }
    match s
        .remote
        .delete(&s.layout.rev_path(game, &number.to_string()))
    {
        Ok(()) | Err(Error::NotFound(_)) => {
            Ok((if complete { "set_aside" } else { "released" }, set_aside))
        }
        Err(error) => Err(error),
    }
}

/// True when `state.json` still reads as `state` with precondition `pre`.
fn state_unchanged(
    s: &Session,
    game: &str,
    state: &Option<ServerState>,
    pre: &Precondition,
) -> Result<bool> {
    let (latest, latest_pre, _) = read_server(s, game)?;
    Ok(latest == *state && latest_pre == *pre)
}

/// After a refused push, true when a claim names a revision that `state.json` did not reach, so
/// `repair` may be able to clear it. The state must not have moved since the push read it.
fn claim_looks_stranded(
    s: &Session,
    game: &str,
    number: u64,
    original_pre: &Precondition,
    original_state: Option<&ServerState>,
) -> bool {
    if !s.remote.needs_rev_marker() {
        return false;
    }
    let Ok((latest, latest_pre, _)) = read_server(s, game) else {
        return false;
    };
    if latest_pre != *original_pre || latest.as_ref() != original_state {
        return false;
    }
    s.remote
        .get(&s.layout.rev_path(game, &number.to_string()))
        .is_ok()
}

/// An entry is complete only when its file list parses and every listed file is present with
/// the recorded hash. A card is optional and does not determine publication.
fn complete_entry(s: &Session, game: &str, entry: &str) -> bool {
    let dir = s.layout.entry_dir(game, entry);
    let (record, _) = match s.remote.get(&format!("{dir}/{}", transfer::RECORD)) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let set: SaveSet = match serde_json::from_slice(&record) {
        Ok(set) => set,
        Err(_) => return false,
    };
    set.files.iter().all(|file| {
        s.remote
            .get(&format!("{dir}/{}", file.path))
            .is_ok_and(|(bytes, _)| crate::scan::hash_bytes(&bytes) == file.sha256)
    })
}

/// Bring back history entry `entry` (M4): keep unsynced local changes as a backup entry first,
/// copy `entry` server-side to a new entry, write it as a new revision, download it, record it.
///
/// Order: the entry is checked against the listing (`Error::NoSuchEntry`, nothing written); its
/// file list is read; the backup (if the local files changed since the last sync) is uploaded;
/// the folder is copied; `state.json` moves to the copy with `Precondition::Match` (a `Changed`
/// removes the copy again, as `push` does); the files are downloaded; `LastSynced` is saved; then
/// retention and marker pruning run, best-effort, with the backup taking a slot as in `push`.
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
    write_state(s, Some(store), id, revision, &new_entry, &next, &pre)?;
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
    record_sync(s, store, id, revision, &wanted, &game.saves_dir)?;
    prune_after_publish(s, id, backup.as_deref(), &new_entry, revision);
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
/// `read_mark`, it does not turn an unreachable mark read into "no mark" for an offline launch.
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

#[cfg(test)]
mod repair_tests {
    use super::*;
    use crate::remote::{MemoryRemote, RemoteEntry};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The entry the seeded claim `rev/2` names.
    const TAKEN: &str = "20261101T195100Z-device";
    /// The entry a racing writer publishes as revision 2.
    const LATER: &str = "20261102T195100Z-other";

    /// A WebDAV-like remote with the revision claim. When `publish_at` is `Some(k)`, the k-th read
    /// of `state.json` first publishes `LATER` as revision 2, as a racing writer would.
    struct MarkedRemote {
        inner: MemoryRemote,
        state_reads: AtomicUsize,
        publish_at: Option<usize>,
    }

    impl MarkedRemote {
        fn new() -> Self {
            Self {
                inner: MemoryRemote::new(),
                state_reads: AtomicUsize::new(0),
                publish_at: None,
            }
        }

        /// Revision 1 is published; `rev/2` names `TAKEN`; `TAKEN` is complete when `complete`.
        fn seed(&self, complete: bool) {
            for dir in ["root", "root/game", "root/game/history", "root/game/rev"] {
                self.inner.mkcol(dir).unwrap();
            }
            let state = ServerState {
                format: FORMAT,
                game: "game".into(),
                revision: 1,
                device: "device".into(),
                time: "2026-10-31T19:51:00Z".into(),
                entry: "20261031T195100Z-device".into(),
                files: vec![],
            };
            self.inner
                .put(
                    "root/game/state.json",
                    state.to_json().as_bytes(),
                    &Precondition::Absent,
                )
                .unwrap();
            self.inner
                .put("root/game/rev/2", TAKEN.as_bytes(), &Precondition::Absent)
                .unwrap();
            if complete {
                self.add_entry(TAKEN);
            }
        }

        /// Adds a complete history entry: a record with no files.
        fn add_entry(&self, entry: &str) {
            self.inner
                .mkcol(&format!("root/game/history/{entry}"))
                .unwrap();
            self.inner
                .put(
                    &format!("root/game/history/{entry}/{}", transfer::RECORD),
                    b"{\"files\":[]}",
                    &Precondition::Any,
                )
                .unwrap();
        }

        fn publish_later(&self) -> Result<()> {
            self.add_entry(LATER);
            let next = ServerState {
                format: FORMAT,
                game: "game".into(),
                revision: 2,
                device: "device".into(),
                time: "2026-11-02T19:51:00Z".into(),
                entry: LATER.into(),
                files: vec![],
            };
            self.inner.put(
                "root/game/state.json",
                next.to_json().as_bytes(),
                &Precondition::Any,
            )?;
            Ok(())
        }
    }

    impl Remote for MarkedRemote {
        fn list(&self, p: &str) -> Result<Vec<RemoteEntry>> {
            self.inner.list(p)
        }
        fn get(&self, p: &str) -> Result<(Vec<u8>, String)> {
            if p == "root/game/state.json" {
                let read = self.state_reads.fetch_add(1, Ordering::SeqCst) + 1;
                if self.publish_at == Some(read) {
                    self.publish_later()?;
                }
            }
            self.inner.get(p)
        }
        fn put(&self, p: &str, b: &[u8], c: &Precondition) -> Result<String> {
            self.inner.put(p, b, c)
        }
        fn mkcol(&self, p: &str) -> Result<()> {
            self.inner.mkcol(p)
        }
        fn copy(&self, a: &str, b: &str) -> Result<()> {
            self.inner.copy(a, b)
        }
        fn move_to(&self, a: &str, b: &str) -> Result<()> {
            self.inner.move_to(a, b)
        }
        fn delete(&self, p: &str) -> Result<()> {
            self.inner.delete(p)
        }
        fn needs_rev_marker(&self) -> bool {
            true
        }
    }

    fn session(remote: &dyn Remote) -> Session<'_> {
        Session {
            remote,
            layout: Layout::new("root").unwrap(),
            device: "repair-test".into(),
            now: 1_800_000_000,
        }
    }

    fn set_aside_path() -> String {
        format!("root/game/set-aside/rev2-{TAKEN}")
    }

    #[test]
    fn incomplete_entry_claim_is_released_and_the_next_write_can_claim() {
        let remote = MarkedRemote::new();
        remote.seed(false);
        let report = repair(&session(&remote), "game", true).unwrap();
        assert_eq!(report.markers[0].status, "released");
        assert_eq!(report.markers[0].set_aside, None);
        assert!(remote.inner.get("root/game/rev/2").is_err());
        let next = ServerState {
            format: FORMAT,
            game: "game".into(),
            revision: 2,
            device: "device".into(),
            time: "2026-11-02T19:51:00Z".into(),
            entry: "20261102T195100Z-device".into(),
            files: vec![],
        };
        let (_, pre, _) = read_server(&session(&remote), "game").unwrap();
        assert!(write_state(&session(&remote), None, "game", 2, &next.entry, &next, &pre).is_ok());
    }

    #[test]
    fn complete_entry_is_previewed_then_set_aside_and_its_claim_released() {
        let remote = MarkedRemote::new();
        remote.seed(true);
        let preview = repair(&session(&remote), "game", false).unwrap();
        assert_eq!(preview.markers[0].status, "releasable_set_aside");
        assert!(remote.inner.get("root/game/rev/2").is_ok());
        assert!(remote
            .inner
            .list(&format!("root/game/history/{TAKEN}"))
            .is_ok());

        let report = repair(&session(&remote), "game", true).unwrap();
        let marker = &report.markers[0];
        assert_eq!(marker.status, "set_aside");
        assert_eq!(marker.set_aside.as_deref(), Some(set_aside_path().as_str()));
        assert!(remote.inner.get("root/game/rev/2").is_err());
        assert!(remote
            .inner
            .list(&format!("root/game/history/{TAKEN}"))
            .is_err());
        assert_eq!(
            remote
                .inner
                .get(&format!("{}/{}", set_aside_path(), transfer::RECORD))
                .unwrap()
                .0,
            b"{\"files\":[]}"
        );
    }

    #[test]
    fn marker_above_next_revision_is_kept() {
        let remote = MarkedRemote::new();
        remote.seed(true);
        remote
            .inner
            .put("root/game/rev/3", b"incomplete", &Precondition::Absent)
            .unwrap();
        let report = repair(&session(&remote), "game", true).unwrap();
        let marker = report.markers.iter().find(|m| m.revision == 3).unwrap();
        assert_eq!(marker.status, "kept_later");
        assert!(remote.inner.get("root/game/rev/3").is_ok());
    }

    #[test]
    fn publish_between_read_and_release_skips_it_and_keeps_the_entry() {
        let mut remote = MarkedRemote::new();
        remote.seed(false);
        remote.publish_at = Some(2);
        let report = repair(&session(&remote), "game", true).unwrap();
        assert_eq!(report.markers[0].status, "skipped_state_moved");
        assert!(remote.inner.get("root/game/rev/2").is_ok());
        assert!(remote
            .inner
            .get(&format!("root/game/history/{LATER}/{}", transfer::RECORD))
            .is_ok());
    }

    #[test]
    fn publish_after_the_move_puts_the_entry_back_and_keeps_the_claim() {
        let mut remote = MarkedRemote::new();
        remote.seed(true);
        remote.publish_at = Some(3);
        let report = repair(&session(&remote), "game", true).unwrap();
        let marker = &report.markers[0];
        assert_eq!(marker.status, "skipped_state_moved");
        assert_eq!(marker.set_aside, None);
        assert!(remote
            .inner
            .get(&format!("root/game/history/{TAKEN}/{}", transfer::RECORD))
            .is_ok());
        assert!(remote.inner.get(&set_aside_path()).is_err());
        assert!(remote.inner.get("root/game/rev/2").is_ok());
    }

    #[test]
    fn memory_remote_reports_nothing_to_repair() {
        let remote = MemoryRemote::new();
        let result = repair(&session(&remote), "game", true).unwrap();
        assert_eq!(result.message, "nothing to repair");
        assert!(result.markers.is_empty());
    }
}
