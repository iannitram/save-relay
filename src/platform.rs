//! Which OS this is, and the folders a manifest's saves template and this tool's own files
//! resolve against. Values come from environment variables through a lookup function, so the
//! rules are tested on every OS without touching the real environment.

use crate::error::{not_built, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A platform a manifest can name a saves folder for. 1.0 builds Linux and Windows
/// (macOS is built in CI too); Android and iOS come with the game ports' 2.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Linux,
    Windows,
    Mac,
    Android,
    Ios,
}

impl Platform {
    /// The platform this binary was built for.
    pub fn current() -> Platform {
        if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(target_os = "android") {
            Platform::Android
        } else if cfg!(target_os = "ios") {
            Platform::Ios
        } else {
            Platform::Linux
        }
    }

    /// The manifest key: "linux", "windows", "mac", "android", "ios".
    pub fn key(self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::Windows => "windows",
            Platform::Mac => "mac",
            Platform::Android => "android",
            Platform::Ios => "ios",
        }
    }
}

/// The folders a saves template may name: `{home}`, `{data}`, `{config}`, `{documents}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathVars {
    pub home: PathBuf,
    pub data: PathBuf,
    pub config: PathBuf,
    pub documents: Option<PathBuf>,
}

impl PathVars {
    /// Resolve the folders for `platform` from environment variables read through `var`
    /// (pass `|k| std::env::var(k).ok()` for the real environment). An empty value counts as unset.
    ///
    /// - Linux: home `$HOME`; data `$XDG_DATA_HOME` else `<home>/.local/share`;
    ///   config `$XDG_CONFIG_HOME` else `<home>/.config`; documents `<home>/Documents`.
    /// - Mac: home `$HOME`; data and config `<home>/Library/Application Support`;
    ///   documents `<home>/Documents`.
    /// - Windows: home `%USERPROFILE%`; data `%LOCALAPPDATA%`; config `%APPDATA%`;
    ///   documents `<home>\Documents`.
    /// - Android, iOS: `Error::Usage` (the host app supplies its folders there).
    ///
    /// A required variable that is unset is `Error::Io("<NAME> is not set")`.
    pub fn from_env(platform: Platform, var: &dyn Fn(&str) -> Option<String>) -> Result<PathVars> {
        not_built("platform::PathVars::from_env")
    }
}

/// Where this tool keeps its own files on this machine (docs/code-map.md, `local`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDirs {
    /// `<config>/save-relay`: `config.json`.
    pub config: PathBuf,
    /// `<data>/save-relay`: `last/<game>.json`, `games/<game>.json`.
    pub data: PathBuf,
}

impl AppDirs {
    /// `<vars.config>/save-relay` and `<vars.data>/save-relay`.
    pub fn from_vars(vars: &PathVars) -> AppDirs {
        crate::not_built!("platform::AppDirs::from_vars")
    }
}
