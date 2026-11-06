//! The server, as a trait: the six WebDAV operations save-relay needs. `webdav::WebDav` is the
//! real one; `MemoryRemote` is the fake every sync test runs against. A mobile build can bring
//! its own implementation (the core takes no network dependency).
//!
//! Paths are relative to the files URL, `/`-separated, no leading or trailing `/`; `""` is the
//! files root, which always exists.

use crate::error::{Error, Result};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

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
    /// True when an etag can repeat for two writes of one file (a WebDAV server whose etag is
    /// built from the modification time in whole seconds and the size; Nextcloud 33 is not one,
    /// #149). Then `sync` first creates
    /// `<game>/rev/<revision>` with `Precondition::Absent` and writes `state.json` only if that
    /// create succeeded. Backends whose guard is exact keep the default.
    fn needs_rev_marker(&self) -> bool {
        false
    }
}

/// `mkcol` `dir` and every missing folder above it, top down.
pub fn ensure_dir(remote: &dyn Remote, dir: &str) -> Result<()> {
    let mut current = String::new();
    for part in dir.split('/').filter(|part| !part.is_empty()) {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(part);
        remote.mkcol(&current)?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
enum Node {
    File { bytes: Vec<u8>, etag: String },
    Dir,
}

#[derive(Debug)]
struct Inner {
    nodes: BTreeMap<String, Node>,
    next_etag: u64,
    reachable: bool,
}

impl Default for Inner {
    fn default() -> Inner {
        Inner {
            nodes: BTreeMap::new(),
            next_etag: 1,
            reachable: true,
        }
    }
}

fn parent_of(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((parent, _)) => parent,
        None => "",
    }
}

impl Inner {
    fn offline(&self) -> Result<()> {
        if self.reachable {
            Ok(())
        } else {
            Err(Error::Unreachable("memory remote is offline".to_string()))
        }
    }

    fn is_dir(&self, path: &str) -> bool {
        path.is_empty() || matches!(self.nodes.get(path), Some(Node::Dir))
    }

    fn is_file(&self, path: &str) -> bool {
        matches!(self.nodes.get(path), Some(Node::File { .. }))
    }

    fn require_dir(&self, path: &str) -> Result<()> {
        if self.is_dir(path) {
            Ok(())
        } else {
            Err(Error::NotFound(path.to_string()))
        }
    }

    fn tree_paths(&self, from: &str) -> Vec<String> {
        let prefix = format!("{from}/");
        self.nodes
            .keys()
            .filter(|path| *path == from || path.starts_with(&prefix))
            .cloned()
            .collect()
    }

    fn remove_tree(&mut self, path: &str) {
        let prefix = format!("{path}/");
        self.nodes
            .retain(|key, _| key != path && !key.starts_with(&prefix));
    }

    fn copy_tree(&mut self, from: &str, to: &str) {
        let entries: Vec<(String, Node)> = self
            .tree_paths(from)
            .into_iter()
            .map(|path| {
                let rest = &path[from.len()..];
                let node = self.nodes.get(&path).cloned().expect("tree path exists");
                (format!("{to}{rest}"), node)
            })
            .collect();
        for (path, node) in entries {
            self.nodes.insert(path, node);
        }
    }
}

/// An in-memory server for tests: files and folders in a map, etags that change on every
/// write, and a switch that makes every call fail as unreachable (M5 tests).
/// Interior mutability (a `Mutex`), so it is used through `&self` like a real server.
#[derive(Debug, Default)]
pub struct MemoryRemote {
    inner: Mutex<Inner>,
}

impl MemoryRemote {
    pub fn new() -> MemoryRemote {
        MemoryRemote::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("memory remote lock poisoned")
    }

    /// While `false`, every `Remote` call returns `Error::Unreachable("memory remote is offline")`.
    pub fn set_reachable(&self, reachable: bool) {
        self.lock().reachable = reachable;
    }

    /// Every file path, sorted (folders left out). Works while unreachable.
    pub fn file_paths(&self) -> Vec<String> {
        let inner = self.lock();
        inner
            .nodes
            .iter()
            .filter_map(|(path, node)| match node {
                Node::File { .. } => Some(path.clone()),
                Node::Dir => None,
            })
            .collect()
    }

    /// A file's bytes, or `None`. Works while unreachable.
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        match self.lock().nodes.get(path) {
            Some(Node::File { bytes, .. }) => Some(bytes.clone()),
            _ => None,
        }
    }
}

impl Remote for MemoryRemote {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        let inner = self.lock();
        inner.offline()?;
        inner.require_dir(dir)?;
        let prefix = format!("{dir}/");
        let mut entries: Vec<RemoteEntry> = inner
            .nodes
            .iter()
            .filter_map(|(path, node)| {
                let name = if dir.is_empty() {
                    if path.contains('/') {
                        return None;
                    }
                    path.as_str()
                } else {
                    path.strip_prefix(&prefix)?
                };
                if name.is_empty() || name.contains('/') {
                    return None;
                }
                let entry = match node {
                    Node::File { bytes, etag } => RemoteEntry {
                        name: name.to_string(),
                        is_dir: false,
                        size: bytes.len() as u64,
                        etag: Some(etag.clone()),
                    },
                    Node::Dir => RemoteEntry {
                        name: name.to_string(),
                        is_dir: true,
                        size: 0,
                        etag: None,
                    },
                };
                Some(entry)
            })
            .collect();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    fn get(&self, path: &str) -> Result<(Vec<u8>, String)> {
        let inner = self.lock();
        inner.offline()?;
        match inner.nodes.get(path) {
            Some(Node::File { bytes, etag }) => Ok((bytes.clone(), etag.clone())),
            _ => Err(Error::NotFound(path.to_string())),
        }
    }

    fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> Result<String> {
        let mut inner = self.lock();
        inner.offline()?;
        if path.is_empty() {
            return Err(Error::Server("cannot write the files root".to_string()));
        }
        let existing = inner.nodes.get(path);
        match pre {
            Precondition::Any => {}
            Precondition::Match(etag) => {
                let matches = matches!(
                    existing,
                    Some(Node::File { etag: current, .. }) if current == etag
                );
                if !matches {
                    return Err(Error::Changed);
                }
            }
            Precondition::Absent => {
                if existing.is_some() {
                    return Err(Error::Changed);
                }
            }
        }
        if matches!(existing, Some(Node::Dir)) {
            return Err(Error::Server(format!("a folder is already at '{path}'")));
        }
        let parent = parent_of(path);
        if !inner.is_dir(parent) {
            return Err(Error::NotFound(parent.to_string()));
        }
        let etag = format!("\"{}\"", inner.next_etag);
        inner.next_etag += 1;
        inner.nodes.insert(
            path.to_string(),
            Node::File {
                bytes: bytes.to_vec(),
                etag: etag.clone(),
            },
        );
        Ok(etag)
    }

    fn mkcol(&self, dir: &str) -> Result<()> {
        let mut inner = self.lock();
        inner.offline()?;
        if dir.is_empty() {
            return Ok(());
        }
        let parent = parent_of(dir);
        if !inner.is_dir(parent) {
            return Err(Error::NotFound(parent.to_string()));
        }
        let existing = inner.nodes.get(dir);
        if matches!(existing, Some(Node::Dir)) {
            return Ok(());
        }
        if matches!(existing, Some(Node::File { .. })) {
            return Err(Error::Server(format!("a file is already at '{dir}'")));
        }
        inner.nodes.insert(dir.to_string(), Node::Dir);
        Ok(())
    }

    fn copy(&self, from: &str, to: &str) -> Result<()> {
        let mut inner = self.lock();
        inner.offline()?;
        if !inner.is_dir(from) && !inner.is_file(from) {
            return Err(Error::NotFound(from.to_string()));
        }
        if to.is_empty() || inner.nodes.contains_key(to) {
            return Err(Error::Server(format!("something is already at '{to}'")));
        }
        let parent = parent_of(to);
        if !inner.is_dir(parent) {
            return Err(Error::NotFound(parent.to_string()));
        }
        inner.copy_tree(from, to);
        Ok(())
    }

    fn move_to(&self, from: &str, to: &str) -> Result<()> {
        let mut inner = self.lock();
        inner.offline()?;
        if !inner.is_dir(from) && !inner.is_file(from) {
            return Err(Error::NotFound(from.to_string()));
        }
        if inner.is_dir(from) && (to == from || to.starts_with(&format!("{from}/"))) {
            return Err(Error::Server(format!(
                "a folder cannot be moved into itself or its subtree: '{from}' to '{to}'"
            )));
        }
        if to.is_empty() || inner.nodes.contains_key(to) {
            return Err(Error::Server(format!("something is already at '{to}'")));
        }
        let parent = parent_of(to);
        if !inner.is_dir(parent) {
            return Err(Error::NotFound(parent.to_string()));
        }
        inner.copy_tree(from, to);
        inner.remove_tree(from);
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<()> {
        let mut inner = self.lock();
        inner.offline()?;
        if !inner.is_dir(path) && !inner.is_file(path) {
            return Err(Error::NotFound(path.to_string()));
        }
        inner.remove_tree(path);
        Ok(())
    }
}
