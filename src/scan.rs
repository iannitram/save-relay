//! The local save set: every file under a game's saves folder that its manifest selects, with
//! size, modification time and SHA-256.

use crate::error::{not_built, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One save file. `path` is relative to the saves folder, with `/` between segments on every OS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    /// Modification time, seconds since the Unix epoch.
    pub mtime: u64,
    /// Lowercase hex, 64 characters.
    pub sha256: String,
}

/// All of one game's save files on one machine, sorted by `path`, paths unique.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveSet {
    pub files: Vec<FileEntry>,
}

impl SaveSet {
    /// Same paths with the same hashes; `mtime` is ignored. Built in the skeleton.
    pub fn same_content(&self, other: &SaveSet) -> bool {
        self.files.len() == other.files.len()
            && self
                .files
                .iter()
                .zip(&other.files)
                .all(|(a, b)| a.path == b.path && a.sha256 == b.sha256)
    }

    /// The newest `mtime`, or `None` for an empty set. Built in the skeleton.
    pub fn newest_mtime(&self) -> Option<u64> {
        self.files.iter().map(|f| f.mtime).max()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// SHA-256 of `bytes`, lowercase hex.
pub fn hash_bytes(bytes: &[u8]) -> String {
    crate::not_built!("scan::hash_bytes")
}

/// Walk `dir` and return the files `glob::selected(include, exclude, path)` keeps, sorted by
/// path. A missing `dir` is an empty set, not an error. Symbolic links are skipped, never
/// followed. Folders are not entries. Read errors are `Error::Io` naming the file.
pub fn scan(dir: &Path, include: &[String], exclude: &[String]) -> Result<SaveSet> {
    not_built("scan::scan")
}
