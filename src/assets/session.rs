//! Resumable, serial installer sessions. The host runs `step` off its UI thread.
use super::{parse_manifest, safe_dir::SafeDir, AssetManifest};
use crate::{
    error::{Error, Result},
    remote::{Remote, MAX_RANGE_BYTES},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::Arc,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    manifest_hash: String,
    app_id: String,
    source_root: String,
    scope: String,
    destination: String,
    index: usize,
    offset: u64,
    identity: Option<String>,
}

/// The result after a bounded step. `durable_bytes` includes verified existing files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub completed_files: usize,
    pub total_files: usize,
    pub durable_bytes: u64,
    pub total_bytes: u64,
    pub current_file: Option<String>,
    pub complete: bool,
}

/// Trusted host configuration. `source_root` is selected by the host, never by the manifest.
pub struct AssetOptions<'a> {
    pub app_id: &'a str,
    pub allow_legacy: bool,
    pub source_root: &'a str,
    pub scope: &'a str,
    pub destination: &'a Path,
    pub state_dir: &'a Path,
}

/// A single locked installer. Dropping it leaves its durable checkpoint for the next launch.
pub struct AssetSession {
    remote: Arc<dyn Remote>,
    manifest: AssetManifest,
    state: Checkpoint,
    destination: SafeDir,
    staging: SafeDir,
    _destination_lock: File,
    _state_lock: File,
    total: u64,
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash_file(file: &mut File) -> Result<(u64, String)> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| Error::StateInvalid("file size overflow".into()))?;
        hash.update(&buffer[..count]);
    }
    Ok((size, format!("{:x}", hash.finalize())))
}
fn source_valid(root: &str) -> bool {
    !root.is_empty()
        && !root.starts_with('/')
        && root.split('/').all(|v| {
            !v.is_empty()
                && v != "."
                && v != ".."
                && !v.contains('\\')
                && !v.chars().any(char::is_control)
        })
}
fn state_error() -> Error {
    Error::StateInvalid("asset checkpoint does not match this session".into())
}

impl AssetSession {
    /// Open an existing app-private destination and a separate same-filesystem state directory.
    /// `scope` identifies the configured service/account without including credentials. The caller
    /// supplies a trusted root (including any explicit legacy mapping), never a manifest URL.
    pub fn open(
        remote: Arc<dyn Remote>,
        raw_manifest: &[u8],
        options: AssetOptions<'_>,
    ) -> Result<Self> {
        let AssetOptions {
            app_id,
            allow_legacy,
            source_root,
            scope,
            destination,
            state_dir,
        } = options;
        let manifest = parse_manifest(raw_manifest, app_id, allow_legacy)?;
        if !source_valid(source_root) || scope.is_empty() {
            return Err(Error::Usage("invalid asset source or scope".into()));
        }
        let dest_path = destination.canonicalize()?;
        let state_path = state_dir.canonicalize()?;
        if dest_path.starts_with(&state_path) || state_path.starts_with(&dest_path) {
            return Err(Error::Usage(
                "asset destination and state directory overlap".into(),
            ));
        }
        let destination = SafeDir::open(&dest_path)?;
        let staging = SafeDir::open(&state_path)?;
        if destination.device()? != staging.device()? {
            return Err(Error::Usage(
                "asset destination and state must be on the same filesystem".into(),
            ));
        }
        let destination_lock = destination.lock_file(".save-relay-assets.lock")?;
        let state_lock = staging.lock_file(".save-relay-state.lock")?;
        let initial = Checkpoint {
            manifest_hash: hash_bytes(raw_manifest),
            app_id: app_id.into(),
            source_root: source_root.into(),
            scope: scope.into(),
            destination: dest_path.to_string_lossy().into_owned(),
            index: 0,
            offset: 0,
            identity: None,
        };
        let state = match staging.read_file("state.json") {
            Ok(file) => {
                let mut raw = Vec::new();
                file.take(64 * 1024).read_to_end(&mut raw)?;
                let state: Checkpoint = serde_json::from_slice(&raw).map_err(|_| state_error())?;
                if state.manifest_hash != initial.manifest_hash
                    || state.app_id != initial.app_id
                    || state.source_root != initial.source_root
                    || state.scope != initial.scope
                    || state.destination != initial.destination
                    || state.index > manifest.files.len()
                    || (state.index == manifest.files.len()
                        && (state.offset != 0 || state.identity.is_some()))
                    || (state.index < manifest.files.len()
                        && (state.offset > manifest.files[state.index].size
                            || (state.offset > 0 && state.identity.is_none())))
                {
                    return Err(state_error());
                }
                state
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => initial,
            Err(e) => return Err(e.into()),
        };
        let total = manifest.files.iter().map(|f| f.size).sum();
        let session = Self {
            remote,
            manifest,
            state,
            destination,
            staging,
            _destination_lock: destination_lock,
            _state_lock: state_lock,
            total,
        };
        // Previously published files are rehashed; checkpoint alone is never proof of content.
        for index in 0..session.state.index {
            session.verify_final(index)?;
        }
        session.checkpoint()?;
        Ok(session)
    }
    fn checkpoint(&self) -> Result<()> {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random)
            .map_err(|_| Error::Io("cannot create checkpoint name".into()))?;
        let name = format!("checkpoint-{:x}.tmp", u128::from_le_bytes(random));
        let mut file = self.staging.create_new(&name)?;
        file.write_all(&serde_json::to_vec(&self.state).map_err(|_| state_error())?)?;
        file.sync_all()?;
        self.staging.replace(&name, "state.json")?;
        Ok(())
    }
    fn parent(&self, path: &str) -> Result<(SafeDir, String)> {
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        Ok((self.destination.directory(parent)?, name.to_string()))
    }
    fn verify_final(&self, index: usize) -> Result<()> {
        let item = &self.manifest.files[index];
        let (parent, name) = self.parent(&item.path)?;
        let mut file = parent.read_file(&name)?;
        let (size, hash) = hash_file(&mut file)?;
        if size != item.size || hash != item.sha256 {
            return Err(Error::StateInvalid(
                "existing asset does not match manifest".into(),
            ));
        }
        Ok(())
    }
    pub fn progress(&self) -> Progress {
        let completed: u64 = self.manifest.files[..self.state.index]
            .iter()
            .map(|f| f.size)
            .sum();
        Progress {
            completed_files: self.state.index,
            total_files: self.manifest.files.len(),
            durable_bytes: completed + self.state.offset,
            total_bytes: self.total,
            current_file: self
                .manifest
                .files
                .get(self.state.index)
                .map(|f| f.path.clone()),
            complete: self.state.index == self.manifest.files.len(),
        }
    }
    /// Cancel the session. Cleanup removes only this session's current staging and checkpoint;
    /// published files are retained. Without cleanup the session remains resumable.
    pub fn cancel(self, cleanup: bool) -> Result<()> {
        if cleanup {
            self.staging
                .remove_file(&format!("part-{}", self.state.index))?;
            self.staging.remove_file("state.json")?;
        }
        Ok(())
    }

    /// Fetch at most one bounded chunk and checkpoint it, or verify and publish one complete file.
    pub fn step(&mut self) -> Result<Progress> {
        if self.state.index == self.manifest.files.len() {
            return Ok(self.progress());
        }
        let item = &self.manifest.files[self.state.index];
        let (parent, name) = self.parent(&item.path)?;
        match parent.read_file(&name) {
            Ok(_) => {
                self.verify_final(self.state.index)?;
                self.state.index += 1;
                self.state.offset = 0;
                self.state.identity = None;
                self.checkpoint()?;
                return Ok(self.progress());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let part_name = format!("part-{}", self.state.index);
        let mut part = match self.staging.open_file(&part_name) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && self.state.offset == 0 => {
                self.staging.create_new(&part_name)?
            }
            Err(e) => return Err(e.into()),
        };
        let length = part.metadata()?.len();
        if length < self.state.offset {
            return Err(state_error());
        }
        if length > self.state.offset {
            part.set_len(self.state.offset)?;
            part.sync_all()?;
        }
        if self.state.offset > 0 {
            let (prefix_size, _) = hash_file(&mut part)?;
            if prefix_size != self.state.offset {
                return Err(state_error());
            }
        }
        if self.state.offset < item.size {
            let want = (item.size - self.state.offset).min(MAX_RANGE_BYTES as u64) as usize;
            let path = format!("{}/{}", self.state.source_root, item.path);
            let chunk = self.remote.read_range(
                &path,
                self.state.offset,
                want,
                self.state.identity.as_deref(),
            )?;
            if chunk.total != item.size || chunk.bytes.is_empty() || chunk.bytes.len() > want {
                return Err(Error::Server("invalid asset range".into()));
            }
            if self
                .state
                .identity
                .as_ref()
                .is_some_and(|id| id != &chunk.identity)
            {
                return Err(Error::Changed);
            }
            part.seek(SeekFrom::Start(self.state.offset))?;
            part.write_all(&chunk.bytes)?;
            part.sync_all()?;
            self.state.offset += chunk.bytes.len() as u64;
            self.state.identity = Some(chunk.identity);
            self.checkpoint()?;
            return Ok(self.progress());
        }
        let (size, hash) = hash_file(&mut part)?;
        if size != item.size || hash != item.sha256 {
            return Err(Error::StateInvalid("asset integrity check failed".into()));
        }
        part.sync_all()?;
        self.staging.publish(&part_name, &parent, &name)?;
        self.state.index += 1;
        self.state.offset = 0;
        self.state.identity = None;
        self.checkpoint()?;
        Ok(self.progress())
    }
}
