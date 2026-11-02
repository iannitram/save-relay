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

use crate::error::{not_built, Error, Result};
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
    named_candidates(entry, facts, None)
}

fn named_candidates(entry: &Entry, facts: &Facts, name: Option<&str>) -> Vec<String> {
    let proton = facts.platform == Platform::Linux && facts.proton;
    let os = if proton {
        "windows"
    } else {
        facts.platform.key()
    };
    let proton_home =
        crate::steam::proton_prefix(&facts.library, facts.app_id).join("drive_c/users/steamuser");
    let path_text = |path: &Path| path.to_str().map(|text| text.replace('\\', "/"));
    let mut paths = Vec::new();
    for (template, rule) in &entry.files {
        if !rule.tags.iter().any(|tag| tag == "save")
            || !(rule.when.is_empty()
                || rule.when.iter().any(|when| {
                    when.os.as_deref().is_none_or(|value| value == os)
                        && when.store.as_deref().is_none_or(|value| value == "steam")
                }))
        {
            continue;
        }
        let expand = || -> Option<String> {
            let mut result = String::new();
            let mut remaining = template.as_str();
            while let Some(open) = remaining.find('<') {
                result.push_str(&remaining[..open]);
                let close = remaining[open..].find('>')? + open;
                let placeholder = &remaining[open..=close];
                if !PLACEHOLDERS.contains(&placeholder) {
                    return None;
                }
                let value = match placeholder {
                    "<home>" => path_text(if proton {
                        &proton_home
                    } else {
                        &facts.vars.home
                    }),
                    "<winAppData>" if proton => path_text(&proton_home.join("AppData/Roaming")),
                    "<winLocalAppData>" if proton => path_text(&proton_home.join("AppData/Local")),
                    "<winDocuments>" if proton => path_text(&proton_home.join("Documents")),
                    "<winAppData>" if facts.platform == Platform::Windows => {
                        path_text(&facts.vars.config)
                    }
                    "<winLocalAppData>" if facts.platform == Platform::Windows => {
                        path_text(&facts.vars.data)
                    }
                    "<winDocuments>" if facts.platform == Platform::Windows => {
                        facts.vars.documents.as_deref().and_then(path_text)
                    }
                    "<xdgData>" if facts.platform == Platform::Linux => path_text(&facts.vars.data),
                    "<xdgConfig>" if facts.platform == Platform::Linux => {
                        path_text(&facts.vars.config)
                    }
                    "<storeUserId>" => facts.user_id.clone(),
                    "<osUserName>" => {
                        Some(if proton { "steamuser" } else { &facts.os_user }.to_owned())
                    }
                    "<base>" => facts.install_dir.as_deref().and_then(path_text),
                    "<root>" => path_text(&facts.library),
                    "<game>" => name.map(str::to_owned),
                    _ => None,
                }?;
                result.push_str(&value);
                remaining = &remaining[close + 1..];
            }
            result.push_str(remaining);
            Some(result.replace('\\', "/"))
        };
        if let Some(path) = expand() {
            paths.push(path);
        }
    }
    paths.sort();
    paths.dedup();
    paths
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
    let mut folders: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for candidate in named_candidates(entry, facts, Some(name)) {
        let (folder, pattern) = if let Some(glob) = candidate.find(['*', '?']) {
            let split = candidate[..glob].rfind('/').map_or(0, |index| index + 1);
            (&candidate[..split], candidate[split..].to_owned())
        } else if is_dir(Path::new(&candidate)) {
            (candidate.as_str(), "**".to_owned())
        } else {
            let split = candidate.rfind('/').map_or(0, |index| index + 1);
            (&candidate[..split], candidate[split..].to_owned())
        };
        // Keep a filesystem root's slash; other trailing slashes do not distinguish folders.
        let folder = if folder == "/" || folder.ends_with(":/") {
            folder
        } else {
            folder.trim_end_matches('/')
        };
        folders.entry(folder.to_owned()).or_default().push(pattern);
    }
    let tried = folders.keys().cloned().collect::<Vec<_>>().join(", ");
    folders.retain(|folder, _| exists(Path::new(folder)));
    if folders.len() != 1 {
        let reason = if folders.is_empty() {
            format!("no existing save folder for {name}; folders tried: {tried}")
        } else {
            format!(
                "multiple existing save folders for {name}: {}",
                folders.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        };
        return Err(Error::SaveLocationUnclear(reason));
    }
    let (folder, mut include) = folders.into_iter().next().expect("one folder");
    include.sort();
    include.dedup();
    Ok(Located {
        saves_dir: PathBuf::from(&folder),
        manifest: Manifest {
            format: crate::manifest::FORMAT,
            id: crate::steam::game_id(facts.app_id),
            name: name.to_owned(),
            saves: BTreeMap::from([(facts.platform, folder)]),
            include,
            exclude: Vec::new(),
        },
    })
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
