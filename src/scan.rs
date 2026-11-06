//! The local save set: every file under a game's saves folder that its manifest selects, with
//! size, modification time and SHA-256.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

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
    /// Same paths with the same hashes; `mtime` is ignored.
    pub fn same_content(&self, other: &SaveSet) -> bool {
        self.files.len() == other.files.len()
            && self
                .files
                .iter()
                .zip(&other.files)
                .all(|(a, b)| a.path == b.path && a.sha256 == b.sha256)
    }

    /// The newest `mtime`, or `None` for an empty set.
    pub fn newest_mtime(&self) -> Option<u64> {
        self.files.iter().map(|f| f.mtime).max()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// SHA-256 of `bytes`, lowercase hex.
pub fn hash_bytes(bytes: &[u8]) -> String {
    hex(Sha256::digest(bytes))
}

/// SHA-256 of the file at `path`, lowercase hex, read in fixed-size chunks so a large save is
/// never held in memory whole.
fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    Ok(hex(hasher.finalize()))
}

fn hex(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Walk `dir` and return the files `glob::selected(include, exclude, path)` keeps, sorted by
/// path. A file with a reserved name (`card::is_reserved`: the sidecar, a part file) is never
/// kept, before the include and exclude patterns are tried. A missing `dir` is an empty set, not an error. Symbolic links are skipped, never
/// followed. Folders are not entries. Read errors are `Error::Io` naming the file; a `dir` that
/// is a file is `Error::Io("<dir>: not a folder")`. A pattern with a `..` segment is
/// `Error::ManifestInvalid`, and no file is read.
pub fn scan(dir: &Path, include: &[String], exclude: &[String]) -> Result<SaveSet> {
    // A pattern never names a path outside the saves folder: `..` is refused, as the manifest
    // refuses it, so nothing selected can lead out of `dir`.
    for pattern in include.iter().chain(exclude) {
        if pattern.split(['/', '\\']).any(|segment| segment == "..") {
            return Err(Error::ManifestInvalid(format!(
                "pattern `{pattern}` has `..`, which may not leave the saves folder"
            )));
        }
    }
    let metadata = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SaveSet::default());
        }
        Err(error) => return Err(Error::Io(format!("{}: {error}", dir.display()))),
    };
    if !metadata.is_dir() {
        return Err(Error::Io(format!("{}: not a folder", dir.display())));
    }

    let mut files = Vec::new();
    scan_directory(dir, dir, include, exclude, &mut files)?;
    files.sort_by(|a: &FileEntry, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    Ok(SaveSet { files })
}

fn scan_directory(
    root: &Path,
    dir: &Path,
    include: &[String],
    exclude: &[String],
    files: &mut Vec<FileEntry>,
) -> Result<()> {
    let entries =
        fs::read_dir(dir).map_err(|error| Error::Io(format!("{}: {error}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|error| Error::Io(format!("{}: {error}", dir.display())))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| Error::Io(format!("{}: {error}", path.display())))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            scan_directory(root, &path, include, exclude, files)?;
        } else if metadata.is_file() {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            let relative = path_string(relative);
            if crate::card::is_reserved(&relative)
                || !crate::glob::selected(include, exclude, &relative)
            {
                continue;
            }
            let sha256 = hash_file(&path)
                .map_err(|error| Error::Io(format!("{}: {error}", path.display())))?;
            let modified = metadata
                .modified()
                .map_err(|error| Error::Io(format!("{}: {error}", path.display())))?;
            let mtime = modified
                .duration_since(UNIX_EPOCH)
                .map_err(|error| Error::Io(format!("{}: {error}", path.display())))?
                .as_secs();
            files.push(FileEntry {
                path: relative,
                size: metadata.len(),
                mtime,
                sha256,
            });
        }
    }
    Ok(())
}

fn path_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
