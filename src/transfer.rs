//! Moving a save set between the saves folder and the server, safely. An upload goes to
//! `incoming/<entry>/` and is renamed into `history/<entry>/` only when every file is there, so
//! a broken upload never becomes a revision (SPEC.md). A download writes each file to a part
//! name, checks its hash, then renames it over the old one.
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
fn local_path(saves_dir: &Path, path: &str) -> PathBuf {
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

/// Make `saves_dir` hold exactly `wanted` (the files of history entry `entry`): download each
/// file whose hash differs from `local`'s into `<name>.save-relay-part`, check its SHA-256
/// (a mismatch is `Error::Server` and nothing is replaced), rename it into place and set its
/// modification time to the entry's `mtime` (`File::set_modified`), so the game's load list
/// shows when the save was made; then delete the files in `local` that `wanted` lacks.
/// Creates folders as needed.
pub fn download_entry(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
    wanted: &SaveSet,
    saves_dir: &Path,
    local: &SaveSet,
) -> Result<Counts> {
    // Reserved names (a sidecar, a part file) are neither fetched nor replaced nor deleted.
    let wanted = without_reserved(wanted);
    let local = without_reserved(local);
    for file in wanted.files.iter().chain(&local.files) {
        check_save_path(&file.path)?;
    }
    let entry_dir = layout.entry_dir(game, entry);
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    if let Err(err) = stage_downloads(remote, &entry_dir, &wanted, saves_dir, &local, &mut staged) {
        for (part, _) in &staged {
            let _ = fs::remove_file(part);
        }
        return Err(err);
    }
    for (index, (part, target)) in staged.iter().enumerate() {
        if let Err(e) = fs::rename(part, target) {
            for (left, _) in &staged[index..] {
                let _ = fs::remove_file(left);
            }
            return Err(io_err(target, e));
        }
    }
    let mut deleted = 0u32;
    for old in &local.files {
        if wanted.files.iter().any(|w| w.path == old.path) {
            continue;
        }
        let path = local_path(saves_dir, &old.path);
        match fs::remove_file(&path) {
            Ok(()) => deleted += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(&path, e)),
        }
        let mut folder = path.parent().map(Path::to_path_buf);
        while let Some(dir) = folder {
            if dir == saves_dir || !dir.starts_with(saves_dir) || fs::remove_dir(&dir).is_err() {
                break;
            }
            folder = dir.parent().map(Path::to_path_buf);
        }
    }
    Ok(Counts {
        downloaded: staged.len() as u32,
        deleted,
        ..Counts::default()
    })
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
