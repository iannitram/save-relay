//! Where a Steam game keeps its saves, from the ludusavi-manifest data (SPEC.md M13;
//! docs/adr/0003-baton-wrap-cards-panel.md). The data is ludusavi-manifest's
//! `data/manifest.yaml` (MIT, mtkennerly/ludusavi-manifest): downloaded over `http::Transport`
//! with its ETag, kept at `<data>/save-relay/ludusavi/manifest.yaml` with a small index
//! `steam-index.json` (app id -> game name), refreshed at most once every `REFRESH_SECS`; an old
//! copy keeps working offline. It is data, never code, and never bundled in the binary.
//!
//! An entry is translated to our own `manifest::Manifest` for one machine: its placeholders,
//! its `when` filter, only paths tagged `save`, and on Linux a Windows-only game under Proton
//! maps into the app's prefix. One clear folder or nothing: no entry, no save path for this OS,
//! or more than one existing folder that could be it is `Error::SaveLocationUnclear`, never a
//! guess. A manifest of the person's own (`--manifest`, or one remembered for that game id)
//! always wins over this data.
//!
//! ```yaml
//! Hades:
//!   files:
//!     <winDocuments>/Saved Games/Hades:
//!       tags: [save]
//!       when: [{os: windows}]
//!   steam: {id: 1145360}
//! ```
//!
//! YAML is read with the `serde-saphyr` crate (pure Rust, MIT or Apache-2.0), added by its
//! issue (docs/adr/0003).

use crate::error::{not_built, Result};
use crate::http::Transport;
use crate::manifest::Manifest;
use crate::platform::{AppDirs, PathVars, Platform};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The data's address.
pub const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/mtkennerly/ludusavi-manifest/master/data/manifest.yaml";

/// Download a newer copy at most this often (a week).
pub const REFRESH_SECS: u64 = 7 * 24 * 60 * 60;

/// The placeholders an entry's paths may use; any other `<...>` makes that path unusable.
pub const PLACEHOLDERS: [&str; 11] = [
    "<home>",
    "<winAppData>",
    "<winLocalAppData>",
    "<winDocuments>",
    "<xdgData>",
    "<xdgConfig>",
    "<storeUserId>",
    "<osUserName>",
    "<base>",
    "<root>",
    "<game>",
];

/// One game in the data. Fields save-relay does not use (registry, cloud, other stores) are
/// skipped while reading.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Entry {
    /// Path template -> its rule.
    #[serde(default)]
    pub files: BTreeMap<String, FileRule>,
    /// Install folder names (`<base>` is `<library>/steamapps/common/<one of these>`).
    #[serde(default, rename = "installDir")]
    pub install_dir: BTreeMap<String, Ignored>,
    #[serde(default)]
    pub steam: Option<SteamRef>,
}

/// A value whose content is skipped while reading (`installDir`'s `{}`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ignored;

impl<'de> Deserialize<'de> for Ignored {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Ignored, D::Error> {
        serde::de::IgnoredAny::deserialize(d).map(|_| Ignored)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct FileRule {
    /// Only paths tagged `save` count.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Empty means every OS and store; otherwise one of these must fit.
    #[serde(default)]
    pub when: Vec<When>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct When {
    /// `windows`, `linux`, `mac`, `dos`; `None` is any.
    #[serde(default)]
    pub os: Option<String>,
    /// `steam`, `gog`, ...; `None` is any.
    #[serde(default)]
    pub store: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SteamRef {
    #[serde(default)]
    pub id: Option<u32>,
}

/// `steam-index.json` beside the cached data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteamIndex {
    pub format: u32,
    /// The ETag the cached copy was downloaded with.
    pub etag: Option<String>,
    /// When it was last downloaded or confirmed unchanged (304), Unix seconds.
    pub checked: u64,
    /// App id -> the game's name in the data.
    pub apps: BTreeMap<u32, String>,
}

/// Parse the whole data file into name -> entry. Not YAML, or not a map of games, is
/// `Error::StateInvalid("ludusavi data: ...")`. The file is large (about 20 MB): entries are
/// read straight into these types, unused fields skipped.
pub fn parse(text: &str) -> Result<BTreeMap<String, Entry>> {
    not_built("ludusavi::parse")
}

/// App id -> name for every entry with a `steam.id`. Two names with one id: the first by name.
pub fn steam_index(games: &BTreeMap<String, Entry>) -> BTreeMap<u32, String> {
    crate::not_built!("ludusavi::steam_index")
}

/// True when the cache should be downloaded again: never checked, or `checked + REFRESH_SECS <= now`.
pub fn needs_refresh(checked: Option<u64>, now: u64) -> bool {
    crate::not_built!("ludusavi::needs_refresh")
}

/// What this machine knows about the wrapped game, for `translate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub platform: Platform,
    pub vars: PathVars,
    pub app_id: u32,
    /// The Steam library that holds the app (`<root>`).
    pub library: PathBuf,
    /// `<library>/steamapps/common/<installdir>` (`<base>`), when known.
    pub install_dir: Option<PathBuf>,
    /// The Steam user id (`<storeUserId>`), when exactly one user is known.
    pub user_id: Option<String>,
    /// `<osUserName>`.
    pub os_user: String,
    /// Linux, a Windows game run through Proton: Windows placeholders map into
    /// `steam::proton_prefix(library, app_id)/drive_c/users/steamuser/` (`AppData/Roaming`,
    /// `AppData/Local`, `Documents`), `<home>` to that user folder, `<osUserName>` to `steamuser`.
    pub proton: bool,
}

/// The save path templates of `entry` that apply on this machine: tagged `save`, a `when` that
/// fits (`facts.platform`, or `windows` under Proton; store `steam` or none), every placeholder
/// known, expanded with `facts`. `/`-separated, glob characters (`*`, `?`, `**`) kept, sorted,
/// no duplicates.
pub fn candidates(entry: &Entry, facts: &Facts) -> Vec<String> {
    crate::not_built!("ludusavi::candidates")
}

/// A wrapped game as save-relay syncs it: our manifest (id `steam::game_id(app_id)`, the
/// entry's name, one include pattern per candidate under the folder) and its saves folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub manifest: Manifest,
    pub saves_dir: PathBuf,
}

/// Translate one entry: each candidate splits at its first segment with a glob character into
/// a folder and an include pattern (no glob: the folder is the path's parent and the pattern its
/// last segment, or `**` when `is_dir` says it is a folder). Folders that do not exist (`exists`)
/// are dropped. Exactly one folder left: `Located`, with every pattern under it. None, or more
/// than one: `Error::SaveLocationUnclear` naming them.
pub fn translate(
    name: &str,
    entry: &Entry,
    facts: &Facts,
    exists: &dyn Fn(&Path) -> bool,
    is_dir: &dyn Fn(&Path) -> bool,
) -> Result<Located> {
    not_built("ludusavi::translate")
}

/// The cache under `<data>/save-relay/ludusavi/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new(dirs: &AppDirs) -> Cache {
        Cache {
            dir: dirs.data.join("ludusavi"),
        }
    }

    /// Download the data when `needs_refresh` (or `force`), sending `If-None-Match` with the
    /// cached ETag; 304 only updates `checked`. Writes via temp file and rename, the index too.
    /// Unreachable with a cached copy: `Ok(false)` (the old copy is used); with none:
    /// `Error::Unreachable`. Returns true when a new copy was written.
    pub fn refresh(&self, transport: &dyn Transport, now: u64, force: bool) -> Result<bool> {
        not_built("ludusavi::Cache::refresh")
    }

    /// The index, `None` when nothing was downloaded yet.
    pub fn index(&self) -> Result<Option<SteamIndex>> {
        not_built("ludusavi::Cache::index")
    }

    /// The entry for `app_id`: its name from the index, then that entry from the data. No entry
    /// is `Error::SaveLocationUnclear("no entry for Steam app <id> in the ludusavi data")`.
    pub fn lookup(&self, app_id: u32) -> Result<(String, Entry)> {
        not_built("ludusavi::Cache::lookup")
    }
}
