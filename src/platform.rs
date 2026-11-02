//! Which OS this is, and the folders a manifest's saves template and this tool's own files
//! resolve against. Values come from environment variables through a lookup function, so the
//! rules are tested on every OS without touching the real environment.

use crate::error::{Error, Result};
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
        // An empty value counts as unset.
        let set = |name: &str| var(name).filter(|value| !value.is_empty());
        let required = |name: &str| {
            set(name)
                .map(PathBuf::from)
                .ok_or_else(|| Error::Io(format!("{name} is not set")))
        };
        match platform {
            Platform::Linux => {
                let home = required("HOME")?;
                let data = set("XDG_DATA_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".local/share"));
                let config = set("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".config"));
                Ok(PathVars {
                    documents: Some(home.join("Documents")),
                    home,
                    data,
                    config,
                })
            }
            Platform::Mac => {
                let home = required("HOME")?;
                let support = home.join("Library/Application Support");
                Ok(PathVars {
                    documents: Some(home.join("Documents")),
                    home,
                    data: support.clone(),
                    config: support,
                })
            }
            Platform::Windows => {
                let home = required("USERPROFILE")?;
                let data = required("LOCALAPPDATA")?;
                let config = required("APPDATA")?;
                Ok(PathVars {
                    documents: Some(home.join("Documents")),
                    home,
                    data,
                    config,
                })
            }
            Platform::Android | Platform::Ios => Err(Error::Usage(format!(
                "{} has no default folders: the host app supplies them",
                platform.key()
            ))),
        }
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
        AppDirs {
            config: vars.config.join("save-relay"),
            data: vars.data.join("save-relay"),
        }
    }
}
