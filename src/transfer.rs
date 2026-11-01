//! Moving a save set between the saves folder and the server, safely. An upload goes to
//! `incoming/<entry>/` and is renamed into `history/<entry>/` only when every file is there, so
//! a broken upload never becomes a revision (SPEC.md). A download writes each file to a part
//! name, checks its hash, then renames it over the old one.
//!
//! A save card (`card::CARD_FILE`) and its picture go into `history/<entry>/` after the MOVE
//! and before `state.json` (`put_card`): a crash in between leaves an entry with no card,
//! which reads as `card: null`, never a broken revision.

use crate::card::Card;
use crate::error::{not_built, Result};
use crate::layout::Layout;
use crate::remote::Remote;
use crate::scan::SaveSet;
use serde::{Deserialize, Serialize};
use std::path::Path;

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
/// `sha256` (the game wrote meanwhile) is `Error::Io`.
pub fn upload_entry(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
    saves_dir: &Path,
    set: &SaveSet,
) -> Result<Counts> {
    not_built("transfer::upload_entry")
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
    not_built("transfer::download_entry")
}

/// The file list of history entry `entry` (its `RECORD`). Missing: `Error::NotFound`;
/// unreadable: `Error::StateInvalid`.
pub fn read_record(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
) -> Result<SaveSet> {
    not_built("transfer::read_record")
}

/// Write `card` as `history/<entry>/` + `card::CARD_FILE`, and the picture first when given
/// (`(file name, bytes)`, the name being `card::PictureKind::file_name`). Called by `sync::push`
/// after `upload_entry` and before `state.json`; `upload_entry` is unchanged.
pub fn put_card(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
    card: &Card,
    picture: Option<(&str, &[u8])>,
) -> Result<()> {
    not_built("transfer::put_card")
}

/// The card of history entry `entry`. Missing or unreadable is `Ok(None)` (old entries have
/// none); only a server failure is an error.
pub fn read_card(
    remote: &dyn Remote,
    layout: &Layout,
    game: &str,
    entry: &str,
) -> Result<Option<Card>> {
    not_built("transfer::read_card")
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
    not_built("transfer::get_picture")
}
