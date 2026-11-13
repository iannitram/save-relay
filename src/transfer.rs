//! Moving a save set between the saves folder and the server, safely. An upload goes to
//! `incoming/<entry>/` and is renamed into `history/<entry>/` only when every file is there, so
//! a broken upload never becomes a revision (SPEC.md). A download writes each file to a part
//! name, fsyncs it and checks its hash; then the whole set is put in place as one step, with the
//! old files moved aside until it is done, so a failure puts the pre-pull files back (S10).
//!
//! A save card (`card::CARD_FILE`) and its picture go into `history/<entry>/` after the MOVE
//! and before `state.json` (`put_card`): a crash in between leaves an entry with no card,
//! which reads as `card: null`, never a broken revision.

use crate::card::{is_reserved, Card, PictureKind, CARD_FILE};
use crate::error::{Error, Result};
use crate::layout::Layout;
use crate::remote::{ensure_dir, Precondition, Remote};
use crate::scan::{hash_bytes, SaveSet};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

/// The suffix of a file being downloaded, before it is renamed into place (also the part file
/// `sync::picture` writes through).
pub const PART: &str = ".save-relay-part";

/// `saves_dir` joined with a `/` save path, segment by segment.
pub(crate) fn local_path(saves_dir: &Path, path: &str) -> PathBuf {
    let mut full = saves_dir.to_path_buf();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        full.push(segment);
    }
    full
}

/// A save path from a record can never lead out of the saves folder: it must have at least one
/// segment (an all-slash path would name the folder itself, whose part file lands beside it),
/// and every segment must be a plain name on this OS (no `..`, `.`, drive prefix, root or `\`
/// separator), so `local_path` stays strictly under `saves_dir`. Anything else is
/// `Error::StateInvalid`.
fn check_save_path(path: &str) -> Result<()> {
    let mut segments = path.split('/').filter(|s| !s.is_empty()).peekable();
    let plain = segments.peek().is_some()
        && segments.all(|segment| {
            let mut parts = Path::new(segment).components();
            matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none()
        });
    if plain {
        Ok(())
    } else {
        Err(Error::StateInvalid(format!("unsafe save path: {path}")))
    }
}

fn io_err(path: &Path, err: std::io::Error) -> Error {
    Error::Io(format!("{}: {err}", path.display()))
}

/// `set` without the reserved names (`card::is_reserved`): a sidecar or a part file is never a
/// save, so an upload leaves it out, and a download neither fetches, replaces nor deletes it.
pub fn without_reserved(set: &SaveSet) -> SaveSet {
    SaveSet {
        files: set
            .files
            .iter()
            .filter(|file| !is_reserved(&file.path))
            .cloned()
            .collect(),
    }
}

/// The picture kind a file name in a card names, if it is one of the two.
fn kind_of(name: &str) -> Option<PictureKind> {
    [PictureKind::Jpeg, PictureKind::Png]
        .into_iter()
        .find(|kind| kind.file_name() == name)
}

fn upload_files(
    remote: &dyn Remote,
    incoming: &str,
    saves_dir: &Path,
    set: &SaveSet,
) -> Result<()> {
    ensure_dir(remote, incoming)?;
    for file in &set.files {
        if let Some((folders, _)) = file.path.rsplit_once('/') {
            ensure_dir(remote, &format!("{incoming}/{folders}"))?;
        }
        let local = local_path(saves_dir, &file.path);
        let bytes = fs::read(&local).map_err(|e| io_err(&local, e))?;
        if hash_bytes(&bytes) != file.sha256 {
            return Err(Error::Io(format!("{} changed while uploading", file.path)));
        }
        remote.put(
            &format!("{incoming}/{}", file.path),
            &bytes,
            &Precondition::Any,
        )?;
    }
    let record = serde_json::to_vec(set).map_err(|e| Error::Io(format!("file list: {e}")))?;
    remote.put(&format!("{incoming}/{RECORD}"), &record, &Precondition::Any)?;
    Ok(())
}

/// Download every changed file into a part file; fills `staged` with (part, target).
fn stage_downloads(
    remote: &dyn Remote,
    entry_dir: &str,
    wanted: &SaveSet,
    saves_dir: &Path,
    local: &SaveSet,
    staged: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<()> {
    for file in &wanted.files {
        if local
            .files
            .iter()
            .any(|l| l.path == file.path && l.sha256 == file.sha256)
        {
            continue;
        }
        let (bytes, _) = remote.get(&format!("{entry_dir}/{}", file.path))?;
        if hash_bytes(&bytes) != file.sha256 {
            return Err(Error::Server(format!("hash mismatch: {}", file.path)));
        }
        let target = local_path(saves_dir, &file.path);
        let mut part_name = target.as_os_str().to_os_string();
        part_name.push(PART);
        let part = PathBuf::from(part_name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
        }
        staged.push((part.clone(), target));
        let mut handle = fs::File::create(&part).map_err(|e| io_err(&part, e))?;
        handle.write_all(&bytes).map_err(|e| io_err(&part, e))?;
        handle
            .set_modified(UNIX_EPOCH + Duration::from_secs(file.mtime))
            .map_err(|e| io_err(&part, e))?;
        handle.sync_all().map_err(|e| io_err(&part, e))?;
    }
    Ok(())
}

/// The file inside every history entry that lists its save files (a `SaveSet` as JSON), so any
/// entry can be restored without reading the others. Written into `incoming/<entry>/` before the
/// MOVE, so an entry and its list appear together.
pub const RECORD: &str = ".save-relay-files.json";

/// What a transfer changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub uploaded: u32,
    pub downloaded: u32,
    pub deleted: u32,
}

/// Upload every file of `set` from `saves_dir` into `incoming/<entry>/` (creating folders),
/// then `set` itself as `RECORD`, then move the folder to `history/<entry>/`. On any failure: delete the incoming folder (best effort)
/// and return the error; `history/` is untouched. A file whose bytes no longer hash to its
/// `sha256` (the game wrote meanwhile) is `Error::Io`. A reserved name in `set` is left out
/// (`without_reserved`), so the record lists only saves.
pub fn upload_entry(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
    saves_dir: &Path,
    set: &SaveSet,
) -> Result<Counts> {
    let set = without_reserved(set);
    let incoming = layout.incoming_dir(game, entry);
    let moved = upload_files(remote, &incoming, saves_dir, &set).and_then(|()| {
        ensure_dir(remote, &layout.history_dir(game))?;
        remote.move_to(&incoming, &layout.entry_dir(game, entry))
    });
    match moved {
        Ok(()) => Ok(Counts {
            uploaded: set.files.len() as u32,
            ..Counts::default()
        }),
        Err(err) => {
            let _ = remote.delete(&incoming);
            Err(err)
        }
    }
}

/// Names compare without case where the usual file system folds case (NTFS, and APFS by
/// default); elsewhere they compare exactly.
const FOLD_CASE: bool = cfg!(any(windows, target_os = "macos"));

/// The prefix of a file moved aside while a download runs. A name that starts with
/// `.save-relay-` is reserved, so a leftover aside file (after a crash) is never a save.
const ASIDE: &str = ".save-relay-prev-";

#[cfg(test)]
thread_local! {
    /// Test hook: how many renames may still succeed before one fails (`None`: no injection).
    static RENAMES_LEFT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Whether the test hook makes this rename fail.
#[cfg(test)]
fn injected_failure() -> bool {
    RENAMES_LEFT.with(|cell| match cell.get() {
        Some(0) => true,
        Some(left) => {
            cell.set(Some(left - 1));
            false
        }
        None => false,
    })
}

#[cfg(not(test))]
fn injected_failure() -> bool {
    false
}

/// `fs::rename`, which a test can make fail (`injected_failure`). Rollback uses `fs::rename`.
fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    if injected_failure() {
        return Err(std::io::Error::other("injected rename failure"));
    }
    fs::rename(from, to)
}

/// Where `path` goes while it is set aside: the same folder, under an `ASIDE` name.
fn aside_path(path: &Path) -> PathBuf {
    let mut name = std::ffi::OsString::from(ASIDE);
    name.push(path.file_name().unwrap_or(std::ffi::OsStr::new("")));
    path.with_file_name(name)
}

/// Move `path` aside if a file is there (a name already moved, by a case-only twin, is nothing
/// to do). Each move is recorded as (original, aside) so a rollback can put it back.
fn set_aside(path: &Path, moved: &mut Vec<(PathBuf, PathBuf)>) -> Result<()> {
    if path.symlink_metadata().is_err() {
        return Ok(());
    }
    let aside = aside_path(path);
    let _ = fs::remove_file(&aside);
    rename(path, &aside).map_err(|e| io_err(path, e))?;
    moved.push((path.to_path_buf(), aside));
    Ok(())
}

/// Undo a download that failed: remove the files already placed, put every set-aside file back,
/// and remove the part files not placed yet. Best effort; the caller returns the first error.
fn rollback(placed: &[PathBuf], staged: &[(PathBuf, PathBuf)], moved: &[(PathBuf, PathBuf)]) {
    for target in placed.iter().rev() {
        let _ = fs::remove_file(target);
    }
    for (original, aside) in moved.iter().rev() {
        let _ = fs::rename(aside, original);
    }
    for (part, _) in &staged[placed.len()..] {
        let _ = fs::remove_file(part);
    }
}

/// Files downloaded and checked, waiting in part files to be renamed into place
/// (`commit_staged`). Dropping it without committing removes the part files.
#[derive(Debug)]
pub struct Staged {
    parts: Vec<(PathBuf, PathBuf)>,
    local: SaveSet,
    wanted: SaveSet,
    saves_dir: PathBuf,
    /// Names compare without case (`FOLD_CASE`, or what a test asks for).
    fold: bool,
}

impl Drop for Staged {
    fn drop(&mut self) {
        for (part, _) in &self.parts {
            let _ = fs::remove_file(part);
        }
    }
}

/// The first half of `download_entry`: check every path of `wanted`, download each file whose
/// hash differs from `local`'s into `<name>.save-relay-part`, fsync it, check its SHA-256 (a
/// mismatch is `Error::Server`) and set its modification time to the entry's `mtime`. Nothing is
/// replaced yet; `restore` does this before it publishes anything.
pub fn stage_entry(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
    wanted: &SaveSet,
    saves_dir: &Path,
    local: &SaveSet,
) -> Result<Staged> {
    stage(
        remote, layout, game, entry, wanted, saves_dir, local, FOLD_CASE,
    )
}

/// `stage_entry` with the name comparison given (`fold`: case-insensitive names).
#[allow(clippy::too_many_arguments)]
fn stage(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
    wanted: &SaveSet,
    saves_dir: &Path,
    local: &SaveSet,
    fold: bool,
) -> Result<Staged> {
    // Reserved names (a sidecar, a part file) are neither fetched nor replaced nor deleted.
    let wanted = without_reserved(wanted);
    let local = without_reserved(local);
    for file in wanted.files.iter().chain(&local.files) {
        check_save_path(&file.path)?;
    }
    let entry_dir = layout.entry_dir(game, entry);
    let mut staged = Staged {
        parts: Vec::new(),
        local: local.clone(),
        wanted: wanted.clone(),
        saves_dir: saves_dir.to_path_buf(),
        fold,
    };
    // A failure leaves part files in `staged.parts`; dropping `staged` removes them.
    stage_downloads(
        remote,
        &entry_dir,
        &wanted,
        saves_dir,
        &local,
        &mut staged.parts,
    )?;
    Ok(staged)
}

/// The second half of `download_entry`: make `saves_dir` hold exactly the staged `wanted`. The
/// whole change is one step: every old file that changes or goes is first moved aside, then the
/// parts are renamed into place, then the aside files are removed. A failure at any point puts
/// the pre-pull files back and leaves no part file (S10). Files whose names differ only by case
/// count as the same name on a case-folding file system (S11). Creates folders as needed.
pub fn commit_staged(mut staged: Staged) -> Result<Counts> {
    let parts = std::mem::take(&mut staged.parts);
    let (local, wanted) = (&staged.local, &staged.wanted);
    let saves_dir: &Path = &staged.saves_dir;
    let fold = staged.fold;
    let same_name = |a: &str, b: &str| {
        if fold {
            a.to_lowercase() == b.to_lowercase()
        } else {
            a == b
        }
    };
    // Set aside: each old file not kept as it is, then whatever sits on a download's target.
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut removed: Vec<PathBuf> = Vec::new();
    let mut to_move: Vec<PathBuf> = Vec::new();
    for old in &local.files {
        let kept = wanted
            .files
            .iter()
            .any(|w| w.path == old.path && w.sha256 == old.sha256);
        if kept {
            continue;
        }
        let path = local_path(saves_dir, &old.path);
        if !wanted.files.iter().any(|w| same_name(&w.path, &old.path)) {
            removed.push(path.clone());
        }
        to_move.push(path);
    }
    to_move.extend(parts.iter().map(|(_, target)| target.clone()));
    for path in &to_move {
        if let Err(err) = set_aside(path, &mut moved) {
            rollback(&[], &parts, &moved);
            return Err(err);
        }
    }
    // Place: rename each part over its target (which is free now).
    let mut placed: Vec<PathBuf> = Vec::new();
    for (part, target) in &parts {
        if let Err(e) = rename(part, target) {
            rollback(&placed, &parts, &moved);
            return Err(io_err(target, e));
        }
        placed.push(target.clone());
    }
    // Commit: the old files are gone for good, then their emptied folders go.
    for (_, aside) in &moved {
        let _ = fs::remove_file(aside);
    }
    for path in &removed {
        let mut folder = path.parent().map(Path::to_path_buf);
        while let Some(dir) = folder {
            if dir == saves_dir || !dir.starts_with(saves_dir) || fs::remove_dir(&dir).is_err() {
                break;
            }
            folder = dir.parent().map(Path::to_path_buf);
        }
    }
    Ok(Counts {
        downloaded: parts.len() as u32,
        deleted: removed.len() as u32,
        ..Counts::default()
    })
}

/// Make `saves_dir` hold exactly `wanted` (the files of history entry `entry`): `stage_entry`,
/// then `commit_staged`. A hash mismatch is `Error::Server` and nothing is replaced.
pub fn download_entry(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
    wanted: &SaveSet,
    saves_dir: &Path,
    local: &SaveSet,
) -> Result<Counts> {
    download(
        remote, layout, game, entry, wanted, saves_dir, local, FOLD_CASE,
    )
}

/// `download_entry` with the name comparison given (`fold`: case-insensitive names).
#[allow(clippy::too_many_arguments)]
fn download(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
    wanted: &SaveSet,
    saves_dir: &Path,
    local: &SaveSet,
    fold: bool,
) -> Result<Counts> {
    commit_staged(stage(
        remote, layout, game, entry, wanted, saves_dir, local, fold,
    )?)
}

/// The file list of history entry `entry` (its `RECORD`). Missing: `Error::NotFound`;
/// unreadable: `Error::StateInvalid`.
pub fn read_record(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
) -> Result<SaveSet> {
    let (bytes, _) = remote.get(&format!("{}/{RECORD}", layout.entry_dir(game, entry)))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| Error::StateInvalid(format!("{entry}/{RECORD}: {e}")))
}

/// Write `card` as `history/<entry>/` + `card::CARD_FILE`, and the picture first when given
/// (`(file name, bytes)`, the name being `card::PictureKind::file_name`). Called by `sync::push`
/// after `upload_entry` and before `state.json`; `upload_entry` is unchanged. A picture name
/// that is not one of the two is `Error::StateInvalid`.
pub fn put_card(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
    card: &Card,
    picture: Option<(&str, &[u8])>,
) -> Result<()> {
    let dir = layout.entry_dir(game, entry);
    if let Some((name, bytes)) = picture {
        if kind_of(name).is_none() {
            return Err(Error::StateInvalid(format!("not a picture name: {name}")));
        }
        remote.put(&format!("{dir}/{name}"), bytes, &Precondition::Any)?;
    }
    remote.put(
        &format!("{dir}/{CARD_FILE}"),
        card.to_json().as_bytes(),
        &Precondition::Any,
    )?;
    Ok(())
}

/// The card of history entry `entry`. Missing or unreadable is `Ok(None)` (old entries have
/// none); only a server failure is an error.
pub fn read_card(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
) -> Result<Option<Card>> {
    match remote.get(&format!("{}/{CARD_FILE}", layout.entry_dir(game, entry))) {
        Ok((bytes, _)) => Ok(std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| Card::from_json(text).ok())),
        Err(Error::NotFound(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// The picture named by `card.picture` in entry `entry`: its bytes and MIME type
/// (`card::PictureKind::mime`). No picture is `Error::NotFound`.
pub fn get_picture(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
    card: &Card,
) -> Result<(Vec<u8>, String)> {
    let missing = || Error::NotFound(format!("a picture in {entry}"));
    let kind = card
        .picture
        .as_deref()
        .and_then(kind_of)
        .ok_or_else(missing)?;
    let path = format!("{}/{}", layout.entry_dir(game, entry), kind.file_name());
    match remote.get(&path) {
        Ok((bytes, _)) => Ok((bytes, kind.mime().to_string())),
        Err(Error::NotFound(_)) => Err(missing()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::MemoryRemote;
    use crate::scan::scan;

    const GAME: &str = "fixture";

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("save-relay-transfer-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Publish `files` as one history entry on `remote`; returns the entry name and its set.
    fn publish(remote: &MemoryRemote, tag: &str, files: &[(&str, &[u8])]) -> (String, SaveSet) {
        let src = folder(&format!("{tag}-src"));
        for (path, bytes) in files {
            fs::write(src.join(path), bytes).unwrap();
        }
        let set = scan(&src, &["**".to_string()], &[]).unwrap();
        let layout = Layout::new("root").unwrap();
        let entry = format!("{tag}-entry");
        ensure_dir(remote, &layout.history_dir(GAME)).unwrap();
        upload_entry(remote, &layout, GAME, &entry, &src, &set).unwrap();
        (entry, set)
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// S10: the third place fails after two files were placed; the folder must hold the old
    /// revision, whole, with no part, aside or new file left.
    #[test]
    fn a_rename_failure_midway_leaves_the_old_revision_whole() {
        let remote = MemoryRemote::new();
        let (entry, wanted) = publish(
            &remote,
            "midway",
            &[
                ("a.sav", b"new-a"),
                ("b.sav", b"new-b"),
                ("c.sav", b"new-c"),
            ],
        );
        let saves = folder("midway-saves");
        fs::write(saves.join("a.sav"), b"old-a").unwrap();
        fs::write(saves.join("b.sav"), b"old-b").unwrap();
        let local = scan(&saves, &["**".to_string()], &[]).unwrap();
        // Moving the two old files aside takes two renames, then placing a and b takes two more:
        // the fifth rename, placing c, fails only if the first four succeed.
        RENAMES_LEFT.with(|cell| cell.set(Some(4)));
        let result = download_entry(
            &remote,
            &Layout::new("root").unwrap(),
            GAME,
            &entry,
            &wanted,
            &saves,
            &local,
        );
        RENAMES_LEFT.with(|cell| cell.set(None));
        assert!(result.is_err());
        assert_eq!(fs::read(saves.join("a.sav")).unwrap(), b"old-a");
        assert_eq!(fs::read(saves.join("b.sav")).unwrap(), b"old-b");
        assert_eq!(names(&saves), ["a.sav", "b.sav"]);
    }

    /// S11: on a case-folding file system `Save.sav` and `save.sav` are one file. The download
    /// replaces it and nothing is counted as deleted.
    #[test]
    fn a_case_only_difference_does_not_delete_the_download() {
        let remote = MemoryRemote::new();
        let (entry, wanted) = publish(&remote, "fold", &[("save.sav", b"new")]);
        let saves = folder("fold-saves");
        fs::write(saves.join("Save.sav"), b"old").unwrap();
        let local = scan(&saves, &["**".to_string()], &[]).unwrap();
        let counts = download(
            &remote,
            &Layout::new("root").unwrap(),
            GAME,
            &entry,
            &wanted,
            &saves,
            &local,
            true,
        )
        .unwrap();
        assert_eq!(counts.deleted, 0);
        assert_eq!(fs::read(saves.join("save.sav")).unwrap(), b"new");
        assert_eq!(names(&saves), ["save.sav"]);
    }
}
