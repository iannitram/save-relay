//! A game's manifest: the small JSON file each port ships so a game joins with no code here
//! (SPEC.md M7). Example (`docs/manifest.md` once written):
//!
//! ```json
//! {
//!   "format": 1,
//!   "id": "uncharted-1",
//!   "name": "Uncharted: Drake's Fortune",
//!   "saves": { "linux": "{data}/uncharted-pc/user", "windows": "{data}/uncharted-pc/user" },
//!   "include": ["Uncharted 1/SAVEDATA*/**"],
//!   "exclude": ["**/*.tmp"]
//! }
//! ```
//!
//! A launcher that lets the player move the saves folder passes `--saves-dir`, which wins over
//! the template.

use crate::error::{not_built, Result};
use crate::platform::{PathVars, Platform};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The manifest format this build reads.
pub const FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Must be `FORMAT`.
    pub format: u32,
    /// The game id: 1-64 characters of `a-z`, `0-9`, `-`, starting with a letter or digit.
    /// It names the game's folder on the server.
    pub id: String,
    /// A display name, not empty.
    pub name: String,
    /// The saves folder template per platform; at least one.
    pub saves: BTreeMap<Platform, String>,
    /// Glob patterns (`glob::matches`) of the files that count as saves, relative to the saves
    /// folder; at least one.
    pub include: Vec<String>,
    /// Glob patterns left out even when included.
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl Manifest {
    /// Parse and `validate`. Bad JSON or a failed check is `Error::ManifestInvalid` whose
    /// message names the field (`"id: ..."`, `"include: ..."`).
    pub fn from_json(text: &str) -> Result<Manifest> {
        not_built("manifest::Manifest::from_json")
    }

    /// Checks: `format == FORMAT`; `id` as documented; `name` not empty; `saves` not empty;
    /// `include` not empty; every pattern relative (no leading `/`), with no `\` and no `..`
    /// segment. The first failure is returned.
    pub fn validate(&self) -> Result<()> {
        not_built("manifest::Manifest::validate")
    }

    /// The saves folder on `platform`: the template for it, expanded. No template for that
    /// platform is `Error::ManifestInvalid("saves: no folder for <key>")`.
    pub fn saves_dir(&self, platform: Platform, vars: &PathVars) -> Result<PathBuf> {
        not_built("manifest::Manifest::saves_dir")
    }
}

/// Replace `{home}`, `{data}`, `{config}` and `{documents}` in `template` with those folders
/// and return the result as a path. Any other `{...}`, an unclosed `{`, or `{documents}` when
/// it is `None` is `Error::ManifestInvalid`.
pub fn expand(template: &str, vars: &PathVars) -> Result<PathBuf> {
    not_built("manifest::expand")
}
