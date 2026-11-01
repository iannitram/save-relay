//! The server, as a trait: the six WebDAV operations save-relay needs. `webdav::WebDav` is the
//! real one; `MemoryRemote` is the fake every sync test runs against. A mobile build can bring
//! its own implementation (the core takes no network dependency).
//!
//! Paths are relative to the files URL, `/`-separated, no leading or trailing `/`; `""` is the
//! files root, which always exists.

use crate::error::{not_built, Result};

/// One child of a listed folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    /// The child's own name (one segment, decoded).
    pub name: String,
    pub is_dir: bool,
    /// Bytes; 0 for a folder.
    pub size: u64,
    pub etag: Option<String>,
}

/// A condition on `put`, so two machines pushing at once cannot both win (HTTP If-Match).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Precondition {
    /// Write whatever is there.
    Any,
    /// Only if the file's etag is still this (`If-Match`).
    Match(String),
    /// Only if the file does not exist yet (`If-None-Match: *`).
    Absent,
}

pub trait Remote {
    /// The direct children of folder `dir`, sorted by name. Missing: `Error::NotFound(dir)`.
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>>;
    /// A file's bytes and etag. Missing, or a folder: `Error::NotFound(path)`.
    fn get(&self, path: &str) -> Result<(Vec<u8>, String)>;
    /// Write a file and return its new etag. A precondition that fails is `Error::Changed`;
    /// a missing parent folder is `Error::NotFound(parent)`.
    fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> Result<String>;
    /// Create one folder. Already a folder: `Ok`. Missing parent: `Error::NotFound(parent)`.
    /// A file is there: `Error::Server`.
    fn mkcol(&self, dir: &str) -> Result<()>;
    /// Copy a file or a whole folder. `from` missing: `Error::NotFound(from)`; `to` exists:
    /// `Error::Server`; `to`'s parent missing: `Error::NotFound(parent)`.
    fn copy(&self, from: &str, to: &str) -> Result<()>;
    /// Rename a file or a whole folder; the same errors as `copy`.
    fn move_to(&self, from: &str, to: &str) -> Result<()>;
    /// Delete a file or a whole folder. Missing: `Error::NotFound(path)`.
    fn delete(&self, path: &str) -> Result<()>;
}

/// `mkcol` `dir` and every missing folder above it, top down.
pub fn ensure_dir(remote: &dyn Remote, dir: &str) -> Result<()> {
    not_built("remote::ensure_dir")
}

/// An in-memory server for tests: files and folders in a map, etags that change on every
/// write, and a switch that makes every call fail as unreachable (M5 tests).
/// Interior mutability (a `Mutex`), so it is used through `&self` like a real server.
#[derive(Debug, Default)]
pub struct MemoryRemote {
    _private: (),
}

impl MemoryRemote {
    pub fn new() -> MemoryRemote {
        MemoryRemote::default()
    }

    /// While `false`, every `Remote` call returns `Error::Unreachable("memory remote is offline")`.
    pub fn set_reachable(&self, reachable: bool) {
        crate::not_built!("remote::MemoryRemote::set_reachable")
    }

    /// Every file path, sorted (folders left out). Works while unreachable.
    pub fn file_paths(&self) -> Vec<String> {
        crate::not_built!("remote::MemoryRemote::file_paths")
    }

    /// A file's bytes, or `None`. Works while unreachable.
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        crate::not_built!("remote::MemoryRemote::read")
    }
}

impl Remote for MemoryRemote {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        not_built("remote::MemoryRemote::list")
    }
    fn get(&self, path: &str) -> Result<(Vec<u8>, String)> {
        not_built("remote::MemoryRemote::get")
    }
    fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> Result<String> {
        not_built("remote::MemoryRemote::put")
    }
    fn mkcol(&self, dir: &str) -> Result<()> {
        not_built("remote::MemoryRemote::mkcol")
    }
    fn copy(&self, from: &str, to: &str) -> Result<()> {
        not_built("remote::MemoryRemote::copy")
    }
    fn move_to(&self, from: &str, to: &str) -> Result<()> {
        not_built("remote::MemoryRemote::move_to")
    }
    fn delete(&self, path: &str) -> Result<()> {
        not_built("remote::MemoryRemote::delete")
    }
}
