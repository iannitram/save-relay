//! Where things live on the server, under one folder (the root) of the user's Nextcloud files:
//!
//! ```text
//! <root>/<game>/state.json                     current revision (state::ServerState)
//! <root>/<game>/playing.json                   the open-elsewhere mark (presence::Mark)
//! <root>/<game>/history/<stamp>-<device>/...   one dated copy per push; the current saves are
//!                                              the entry state.json names
//! <root>/<game>/incoming/<stamp>-<device>/     an upload in progress (transfer)
//! <root>/<game>/rev/<n>                        the claim on revision n, body: the entry it names
//!                                              (WebDAV only, #149; `Remote::needs_rev_marker`)
//! ```
//!
//! Paths are relative to the WebDAV files URL (`webdav::files_url`), `/`-separated, with no
//! leading or trailing `/`. Pure.

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: String,
}

impl Layout {
    /// `root` with leading and trailing `/` trimmed. Empty, a `\`, or a `.` or `..` segment
    /// is `Error::Usage`. `"/Games/save-relay/"` -> root `"Games/save-relay"`.
    pub fn new(root: &str) -> Result<Layout> {
        let root = root.trim_matches('/');
        if root.is_empty()
            || root.contains('\\')
            || root
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            return Err(Error::Usage("invalid server root".to_string()));
        }
        Ok(Layout {
            root: root.to_string(),
        })
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    /// `<root>/<game>`
    pub fn game_dir(&self, game: &str) -> String {
        format!("{}/{}", self.root, game)
    }

    /// `<root>/<game>/state.json`
    pub fn state_path(&self, game: &str) -> String {
        format!("{}/state.json", self.game_dir(game))
    }

    /// `<root>/<game>/playing.json` (`presence::FILE`)
    pub fn playing_path(&self, game: &str) -> String {
        format!("{}/{}", self.game_dir(game), crate::presence::FILE)
    }

    /// `<root>/<game>/history`
    pub fn history_dir(&self, game: &str) -> String {
        format!("{}/history", self.game_dir(game))
    }

    /// `<root>/<game>/history/<entry>`
    pub fn entry_dir(&self, game: &str, entry: &str) -> String {
        format!("{}/{}", self.history_dir(game), entry)
    }

    /// `<root>/<game>/incoming/<id>`
    pub fn incoming_dir(&self, game: &str, id: &str) -> String {
        format!("{}/incoming/{}", self.game_dir(game), id)
    }

    /// `<root>/<game>/rev`: the revision claims (`rev_path`).
    pub fn rev_dir(&self, game: &str) -> String {
        format!("{}/rev", self.game_dir(game))
    }

    /// `<root>/<game>/rev/<name>`: the claim named `name`, a revision number in decimal.
    pub fn rev_path(&self, game: &str, name: &str) -> String {
        format!("{}/{}", self.rev_dir(game), name)
    }

    /// `<root>/<game>/set-aside`: complete history entries that `repair` moved out of `history/`
    /// with their stranded claim (#159). Nothing reads this folder; it is kept for recovery by hand.
    pub fn set_aside_dir(&self, game: &str) -> String {
        format!("{}/set-aside", self.game_dir(game))
    }

    /// `<root>/<game>/set-aside/rev<number>-<entry>`: where `repair` put the entry `entry` that the
    /// claim `rev/<number>` named.
    pub fn set_aside_path(&self, game: &str, number: u64, entry: &str) -> String {
        format!("{}/rev{}-{}", self.set_aside_dir(game), number, entry)
    }
}

/// `"<stamp>-<device>"`: `entry_name("20261031T195100Z", "amd-handheld")` ->
/// `"20261031T195100Z-amd-handheld"`.
pub fn entry_name(stamp: &str, device: &str) -> String {
    format!("{}-{}", stamp, device)
}

/// Split an entry name into `(stamp, device)`. The stamp is the first 16 characters and must
/// parse with `stamp::parse_entry_stamp`; then one `-`; then a non-empty device. Else `None`.
pub fn parse_entry_name(name: &str) -> Option<(String, String)> {
    if !name.is_ascii() || name.len() <= 17 {
        return None;
    }
    let stamp = &name[..16];
    if crate::stamp::parse_entry_stamp(stamp).is_none() || name.as_bytes()[16] != b'-' {
        return None;
    }
    let device = &name[17..];
    if device.is_empty() {
        return None;
    }
    Some((stamp.to_string(), device.to_string()))
}

/// A device name made safe for paths: lowercase ASCII; every run of characters other than
/// `a-z` and `0-9` becomes one `-`; no `-` at either end; at most 32 characters (cut, then
/// trailing `-` trimmed again); empty becomes `"device"`. `"Steam Deck (OLED)"` -> `"steam-deck-oled"`.
pub fn device_slug(name: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character);
            separator = false;
        } else {
            separator = true;
        }
    }
    let slug = slug.chars().take(32).collect::<String>();
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "device".to_string()
    } else {
        slug
    }
}
