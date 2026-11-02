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

use crate::error::{Error, Result};
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
        let manifest: Manifest = serde_json::from_str(text)
            .map_err(|error| Error::ManifestInvalid(format!("json: {error}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Checks: `format == FORMAT`; `id` as documented; `name` not empty; `saves` not empty;
    /// `include` not empty; every pattern relative (no leading `/`), with no `\` and no `..`
    /// segment. The first failure is returned.
    pub fn validate(&self) -> Result<()> {
        if self.format != FORMAT {
            return Err(Error::ManifestInvalid(format!("format: expected {FORMAT}")));
        }
        if self.id.is_empty()
            || self.id.len() > 64
            || self.id.starts_with('-')
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(Error::ManifestInvalid(
                "id: must be 1-64 lowercase letters, digits, or hyphens, starting with a letter or digit".into(),
            ));
        }
        if is_windows_reserved(&self.id) {
            return Err(Error::ManifestInvalid(
                "id: is a name Windows reserves for devices (con, prn, aux, nul, com0-9, lpt0-9)"
                    .into(),
            ));
        }
        if self.name.is_empty() {
            return Err(Error::ManifestInvalid("name: must not be empty".into()));
        }
        if self.saves.is_empty() {
            return Err(Error::ManifestInvalid("saves: must not be empty".into()));
        }
        if self.include.is_empty() {
            return Err(Error::ManifestInvalid("include: must not be empty".into()));
        }
        for pattern in &self.include {
            validate_pattern(pattern, "include")?;
        }
        for pattern in &self.exclude {
            validate_pattern(pattern, "exclude")?;
        }
        Ok(())
    }

    /// The saves folder on `platform`: the template for it, expanded. No template for that
    /// platform is `Error::ManifestInvalid("saves: no folder for <key>")`.
    pub fn saves_dir(&self, platform: Platform, vars: &PathVars) -> Result<PathBuf> {
        let template = self.saves.get(&platform).ok_or_else(|| {
            Error::ManifestInvalid(format!("saves: no folder for {}", platform.key()))
        })?;
        expand(template, vars)
    }
}

/// Replace `{home}`, `{data}`, `{config}` and `{documents}` in `template` with those folders
/// and return the result as a path. Any other `{...}`, an unclosed `{`, or `{documents}` when
/// it is `None` is `Error::ManifestInvalid`.
pub fn expand(template: &str, vars: &PathVars) -> Result<PathBuf> {
    let mut result = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(open) = remaining.find('{') {
        result.push_str(&remaining[..open]);
        let placeholder = &remaining[open + 1..];
        let close = placeholder
            .find('}')
            .ok_or_else(|| Error::ManifestInvalid("saves: unclosed placeholder".into()))?;
        let name = &placeholder[..close];
        let path = match name {
            "home" => &vars.home,
            "data" => &vars.data,
            "config" => &vars.config,
            "documents" => vars.documents.as_ref().ok_or_else(|| {
                Error::ManifestInvalid("saves: documents folder is not available".into())
            })?,
            _ => {
                return Err(Error::ManifestInvalid(format!(
                    "saves: unknown placeholder {{{name}}}"
                )))
            }
        };
        result.push_str(&path.to_string_lossy());
        remaining = &placeholder[close + 1..];
    }
    result.push_str(remaining);
    Ok(PathBuf::from(result))
}

fn validate_pattern(pattern: &str, field: &str) -> Result<()> {
    if pattern.is_empty()
        || pattern.starts_with('/')
        || pattern.contains('\\')
        || pattern.split('/').any(|segment| segment == "..")
    {
        return Err(Error::ManifestInvalid(format!(
            "{field}: invalid relative pattern {pattern:?}"
        )));
    }
    Ok(())
}

/// The game id is a folder name on the server, and the Windows sync client cannot make a folder
/// with one of these names, so they are refused as ids (lowercase, as ids are).
fn is_windows_reserved(id: &str) -> bool {
    matches!(id, "con" | "prn" | "aux" | "nul")
        || (id.len() == 4
            && (id.starts_with("com") || id.starts_with("lpt"))
            && id.as_bytes()[3].is_ascii_digit())
}
