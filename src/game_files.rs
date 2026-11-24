//! Resumable per-game asset downloads through `save-relay`'s Dropbox App folder (#305).
//!
//! Layout in the Dropbox App folder: `game-files/<game-id>/<rel-path>` (isolated from save sync).
//! Each file in a [`GameFilesManifest`] is downloaded into `<dest_dir>/<rel-path>.part` in
//! bounded chunks using HTTP `Range: bytes=<start>-<end>`, flushed to disk after each chunk,
//! verified against its expected byte size and SHA-256 hex digest, and atomically renamed into
//! place. Already-complete files with matching size and SHA-256 are skipped on resume.

use crate::dropbox::Dropbox;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Default chunk size for resumable range downloads (4 MiB).
pub const DEFAULT_CHUNK_SIZE: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameFileEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameFilesManifest {
    #[serde(default = "default_version")]
    pub version: u32,
    pub game_id: String,
    #[serde(default)]
    pub remote_root: Option<String>,
    pub files: Vec<GameFileEntry>,
}

fn default_version() -> u32 {
    1
}

impl GameFilesManifest {
    pub fn parse(json: &str) -> Result<Self> {
        let manifest: Self = serde_json::from_str(json)
            .map_err(|e| Error::Usage(format!("invalid game-files manifest: {e}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn remote_prefix(&self) -> String {
        match &self.remote_root {
            Some(root) if !root.is_empty() => root.trim_matches('/').to_string(),
            _ => format!("game-files/{}", self.game_id),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(Error::Usage(format!(
                "unsupported game-files manifest version {}",
                self.version
            )));
        }
        validate_rel_path(&self.game_id, "game_id")?;
        if let Some(root) = &self.remote_root {
            if !root.is_empty() {
                validate_rel_path(root, "remote_root")?;
            }
        }
        for entry in &self.files {
            validate_rel_path(&entry.path, "file path")?;
            if entry.sha256.len() != 64 || !entry.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Error::Usage(format!(
                    "invalid sha256 for {}: must be 64 hex chars",
                    entry.path
                )));
            }
        }
        Ok(())
    }
}

fn validate_rel_path(path: &str, label: &str) -> Result<()> {
    if path.is_empty()
        || path.contains('\0')
        || path.contains('\\')
        || path.starts_with('/')
        || path.ends_with('/')
        || path.split('/').any(|seg| matches!(seg, "" | "." | ".."))
    {
        return Err(Error::Usage(format!("invalid {label}: {path}")));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameFilesReport {
    pub game_id: String,
    pub total_files: usize,
    pub downloaded_files: usize,
    pub skipped_files: usize,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameFilesStatus {
    pub game_id: String,
    pub total_files: usize,
    pub completed_files: usize,
    pub partial_files: usize,
    pub bytes_on_disk: u64,
    pub total_bytes: u64,
    pub complete: bool,
}

pub trait GameFileRemote {
    fn get_range(
        &self,
        remote_path: &str,
        start: u64,
        end_inclusive: u64,
    ) -> Result<(Vec<u8>, bool)>;
}

impl GameFileRemote for Dropbox {
    fn get_range(
        &self,
        remote_path: &str,
        start: u64,
        end_inclusive: u64,
    ) -> Result<(Vec<u8>, bool)> {
        Dropbox::get_range(self, remote_path, start, end_inclusive)
    }
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn part_path_for(final_path: &Path) -> PathBuf {
    let name = final_path
        .file_name()
        .map(|n| format!("{}.part", n.to_string_lossy()))
        .unwrap_or_else(|| "download.part".into());
    final_path.with_file_name(name)
}

fn file_matches(path: &Path, entry: &GameFileEntry) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() != entry.size {
        return false;
    }
    sha256_file(path)
        .map(|h| h.eq_ignore_ascii_case(&entry.sha256))
        .unwrap_or(false)
}

/// Inspect local completion & resumable `.part` progress without touching the network.
pub fn status_game_files(manifest: &GameFilesManifest, dest_dir: &Path) -> Result<GameFilesStatus> {
    manifest.validate()?;
    let mut completed_files = 0usize;
    let mut partial_files = 0usize;
    let mut bytes_on_disk = 0u64;
    let mut total_bytes = 0u64;

    for entry in &manifest.files {
        total_bytes = total_bytes.saturating_add(entry.size);
        let final_path = dest_dir.join(&entry.path);
        if file_matches(&final_path, entry) {
            completed_files += 1;
            bytes_on_disk = bytes_on_disk.saturating_add(entry.size);
            continue;
        }
        let part_path = part_path_for(&final_path);
        if let Ok(meta) = fs::metadata(&part_path) {
            if meta.is_file() {
                partial_files += 1;
                bytes_on_disk = bytes_on_disk.saturating_add(meta.len().min(entry.size));
            }
        }
    }

    Ok(GameFilesStatus {
        game_id: manifest.game_id.clone(),
        total_files: manifest.files.len(),
        completed_files,
        partial_files,
        bytes_on_disk,
        total_bytes,
        complete: completed_files == manifest.files.len(),
    })
}

/// Download all files in `manifest` into `dest_dir` resumably via byte-offset `.part` files.
pub fn download_game_files(
    remote: &dyn GameFileRemote,
    manifest: &GameFilesManifest,
    dest_dir: &Path,
    chunk_size: u64,
) -> Result<GameFilesReport> {
    manifest.validate()?;
    let chunk_size = chunk_size.max(1024);
    let prefix = manifest.remote_prefix();
    let mut downloaded_files = 0usize;
    let mut skipped_files = 0usize;
    let mut bytes_downloaded = 0u64;
    let mut total_bytes = 0u64;

    for entry in &manifest.files {
        total_bytes = total_bytes.saturating_add(entry.size);
        let final_path = dest_dir.join(&entry.path);
        if file_matches(&final_path, entry) {
            skipped_files += 1;
            continue;
        }

        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let part_path = part_path_for(&final_path);
        let mut part_len = fs::metadata(&part_path)
            .map(|m| if m.is_file() { m.len() } else { 0 })
            .unwrap_or(0);

        if part_len > entry.size {
            let _ = fs::remove_file(&part_path);
            part_len = 0;
        }

        if entry.size == 0 {
            File::create(&part_path)?.sync_data()?;
        } else if part_len < entry.size {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(&part_path)?;
            file.seek(SeekFrom::Start(part_len))?;

            let remote_path = format!("{prefix}/{}", entry.path);
            let mut offset = part_len;
            while offset < entry.size {
                let end = offset
                    .saturating_add(chunk_size)
                    .min(entry.size)
                    .saturating_sub(1);
                let (bytes, is_partial) = remote.get_range(&remote_path, offset, end)?;
                if bytes.is_empty() {
                    return Err(Error::Server(format!(
                        "empty response downloading {}",
                        entry.path
                    )));
                }
                if !is_partial && offset > 0 {
                    // Server ignored Range and sent full file from byte 0: restart cleanly.
                    file.set_len(0)?;
                    file.seek(SeekFrom::Start(0))?;
                    let take = (bytes.len() as u64).min(entry.size) as usize;
                    file.write_all(&bytes[..take])?;
                    file.sync_data()?;
                    bytes_downloaded = bytes_downloaded.saturating_add(take as u64);
                    break;
                }
                let expected_max = (entry.size - offset) as usize;
                let write_len = bytes.len().min(expected_max);
                file.write_all(&bytes[..write_len])?;
                file.sync_data()?;
                offset = offset.saturating_add(write_len as u64);
                bytes_downloaded = bytes_downloaded.saturating_add(write_len as u64);
            }
        }

        let actual_sha = sha256_file(&part_path)?;
        if !actual_sha.eq_ignore_ascii_case(&entry.sha256) {
            let _ = fs::remove_file(&part_path);
            return Err(Error::Server(format!(
                "sha256 mismatch for {}: expected {}, got {actual_sha}",
                entry.path, entry.sha256
            )));
        }
        fs::rename(&part_path, &final_path)?;
        downloaded_files += 1;
    }

    Ok(GameFilesReport {
        game_id: manifest.game_id.clone(),
        total_files: manifest.files.len(),
        downloaded_files,
        skipped_files,
        bytes_downloaded,
        total_bytes,
        complete: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("sr-gf-{}-{label}-{id}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sha_hex(data: &[u8]) -> String {
        let digest = Sha256::digest(data);
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    struct FakeRangeRemote {
        files: HashMap<String, Vec<u8>>,
        fail_after_bytes: Option<u64>,
        sent_bytes: std::sync::Mutex<u64>,
    }

    impl GameFileRemote for FakeRangeRemote {
        fn get_range(
            &self,
            remote_path: &str,
            start: u64,
            end_inclusive: u64,
        ) -> Result<(Vec<u8>, bool)> {
            let data = self
                .files
                .get(remote_path)
                .ok_or_else(|| Error::NotFound(remote_path.into()))?;
            let s = start as usize;
            let e = (end_inclusive as usize + 1).min(data.len());
            let slice = data[s..e].to_vec();
            let mut sent = self.sent_bytes.lock().unwrap();
            if let Some(limit) = self.fail_after_bytes {
                if *sent >= limit {
                    return Err(Error::Unreachable("simulated power loss".into()));
                }
            }
            *sent += slice.len() as u64;
            Ok((slice, true))
        }
    }

    #[test]
    fn resumable_download_recovers_from_partial_and_skips_completed() {
        let dir = temp_dir("resume");
        let payload_a = vec![0xABu8; 3000];
        let payload_b = vec![0xCDu8; 2500];
        let manifest = GameFilesManifest {
            version: 1,
            game_id: "uncharted-1".into(),
            remote_root: None,
            files: vec![
                GameFileEntry {
                    path: "u1data/a.bin".into(),
                    size: payload_a.len() as u64,
                    sha256: sha_hex(&payload_a),
                },
                GameFileEntry {
                    path: "u1data/b.bin".into(),
                    size: payload_b.len() as u64,
                    sha256: sha_hex(&payload_b),
                },
            ],
        };

        let mut files = HashMap::new();
        files.insert(
            "game-files/uncharted-1/u1data/a.bin".into(),
            payload_a.clone(),
        );
        files.insert(
            "game-files/uncharted-1/u1data/b.bin".into(),
            payload_b.clone(),
        );

        // Pass 1: fail after first 1024-byte chunk (simulating mid-file battery death).
        let remote1 = FakeRangeRemote {
            files: files.clone(),
            fail_after_bytes: Some(1024),
            sent_bytes: std::sync::Mutex::new(0),
        };
        assert!(download_game_files(&remote1, &manifest, &dir, 1024).is_err());

        let status = status_game_files(&manifest, &dir).unwrap();
        assert_eq!(status.completed_files, 0);
        assert_eq!(status.partial_files, 1);
        assert_eq!(status.bytes_on_disk, 1024);

        // Pass 2: resume from 1024 without re-downloading the first 1024 bytes.
        let remote2 = FakeRangeRemote {
            files,
            fail_after_bytes: None,
            sent_bytes: std::sync::Mutex::new(0),
        };
        let report = download_game_files(&remote2, &manifest, &dir, 1024).unwrap();
        assert_eq!(report.downloaded_files, 2);
        assert_eq!(report.skipped_files, 0);
        assert_eq!(report.bytes_downloaded, (3000 - 1024) + 2500);
        assert_eq!(fs::read(dir.join("u1data/a.bin")).unwrap(), payload_a);
        assert_eq!(fs::read(dir.join("u1data/b.bin")).unwrap(), payload_b);

        // Pass 3: everything complete -> skipped_files == 2, 0 bytes downloaded.
        let report2 = download_game_files(&remote2, &manifest, &dir, 1024).unwrap();
        assert_eq!(report2.skipped_files, 2);
        assert_eq!(report2.bytes_downloaded, 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
