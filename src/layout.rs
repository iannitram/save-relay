//! Where things live on the server, under one folder (the root) of the user's Nextcloud files:
//!
//! ```text
//! <root>/<game>/state.json                     current revision (state::ServerState)
//! <root>/<game>/history/<stamp>-<device>/...   one dated copy per push; the current saves are
//!                                              the entry state.json names
//! <root>/<game>/incoming/<stamp>-<device>/     an upload in progress (transfer)
//! ```
//!
//! Paths are relative to the WebDAV files URL (`webdav::files_url`), `/`-separated, with no
//! leading or trailing `/`. Pure.

use crate::error::{not_built, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: String,
}

impl Layout {
    /// `root` with leading and trailing `/` trimmed. Empty, a `\`, or a `.` or `..` segment
    /// is `Error::Usage`. `"/Games/save-relay/"` -> root `"Games/save-relay"`.
    pub fn new(root: &str) -> Result<Layout> {
        not_built("layout::Layout::new")
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    /// `<root>/<game>`
    pub fn game_dir(&self, game: &str) -> String {
        crate::not_built!("layout::Layout::game_dir")
    }

    /// `<root>/<game>/state.json`
    pub fn state_path(&self, game: &str) -> String {
        crate::not_built!("layout::Layout::state_path")
    }

    /// `<root>/<game>/history`
    pub fn history_dir(&self, game: &str) -> String {
        crate::not_built!("layout::Layout::history_dir")
    }

    /// `<root>/<game>/history/<entry>`
    pub fn entry_dir(&self, game: &str, entry: &str) -> String {
        crate::not_built!("layout::Layout::entry_dir")
    }

    /// `<root>/<game>/incoming/<id>`
    pub fn incoming_dir(&self, game: &str, id: &str) -> String {
        crate::not_built!("layout::Layout::incoming_dir")
    }
}

/// `"<stamp>-<device>"`: `entry_name("20261031T195100Z", "amd-handheld")` ->
/// `"20261031T195100Z-amd-handheld"`.
pub fn entry_name(stamp: &str, device: &str) -> String {
    crate::not_built!("layout::entry_name")
}

/// Split an entry name into `(stamp, device)`. The stamp is the first 16 characters and must
/// parse with `stamp::parse_entry_stamp`; then one `-`; then a non-empty device. Else `None`.
pub fn parse_entry_name(name: &str) -> Option<(String, String)> {
    crate::not_built!("layout::parse_entry_name")
}

/// A device name made safe for paths: lowercase ASCII; every run of characters other than
/// `a-z` and `0-9` becomes one `-`; no `-` at either end; at most 32 characters (cut, then
/// trailing `-` trimmed again); empty becomes `"device"`. `"Steam Deck (OLED)"` -> `"steam-deck-oled"`.
pub fn device_slug(name: &str) -> String {
    crate::not_built!("layout::device_slug")
}
