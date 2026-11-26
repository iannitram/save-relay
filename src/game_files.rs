//! Compatibility shim for the two C functions released with #324,
//! `save_relay_download_game_files` and `save_relay_game_files_status` (#309).
//!
//! Deprecated for new code: use `gamefiles` and `save_relay_game_files_plan`, `_download`, ...
//!
//! There is one download engine: `gamefiles::engine`. This module only turns the inline
//! manifest those functions take (`game_id`, optional `remote_root`, `files`) into the engine's
//! `Layout::Inline`, and the engine's `Report` and `LocalStatus` back into the two JSON shapes
//! they returned.
//!
//! What changed from the first #324 code, for safety (docs/assets.md):
//! - the remote layout is the same (`game-files/<game_id>/<path>`, or `remote_root`), but
//!   `remote_root` must now be a plain relative path and `game_id` an app id
//!   (`[a-z0-9][a-z0-9._-]*`);
//! - checkpoints live in `<dest_dir>/.save-relay-game-files/`, not in `<file>.part` beside each
//!   file; old `.part` files from #324 are ignored and left alone;
//! - an existing file that does not match the manifest is a `destination_conflict`, never
//!   overwritten (#324 replaced it);
//! - the manifest follows `gamefiles::manifest` (digests are lowercased first, then the path
//!   collision rules and the 4 MiB / 100,000 file bounds apply);
//! - every range answer is checked (`engine::validate`), so a server that ignores `Range` fails
//!   as `range_unsupported` instead of being cut short.

use crate::error::{Error, Result};
use crate::gamefiles::engine::{self, Options};
use crate::gamefiles::manifest::{self, Manifest};
use crate::gamefiles::{GameSource, Layout, State, INLINE_STATE_DIR};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// Default chunk size for resumable range downloads (4 MiB).
pub const DEFAULT_CHUNK_SIZE: u64 = crate::gamefiles::DEFAULT_CHUNK;

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
    /// Parse and fully validate (the same rules the engine applies).
    pub fn parse(json: &str) -> Result<Self> {
        let parsed: Self = serde_json::from_str(json)
            .map_err(|e| Error::ManifestInvalid(format!("invalid game-files manifest: {e}")))?;
        parsed.build()?;
        Ok(parsed)
    }

    /// The remote folder holding the files: `remote_root` (slashes trimmed) or
    /// `game-files/<game_id>`.
    pub fn remote_prefix(&self) -> String {
        match &self.remote_root {
            Some(root) if !root.trim_matches('/').is_empty() => root.trim_matches('/').to_string(),
            _ => format!("game-files/{}", self.game_id),
        }
    }

    /// Validate by building the engine's layout and manifest.
    pub fn validate(&self) -> Result<()> {
        self.build().map(|_| ())
    }

    /// The engine layout carrying this manifest as strict v1 JSON bound to `game_id`, and the
    /// manifest as the engine parses it.
    fn build(&self) -> Result<(Layout, Manifest)> {
        if self.version != 1 {
            return Err(Error::ManifestInvalid(format!(
                "unsupported game-files manifest version {}",
                self.version
            )));
        }
        manifest::check_app_id(&self.game_id)?;
        let prefix = self.remote_prefix();
        let files: Vec<serde_json::Value> = self
            .files
            .iter()
            .map(|f| {
                serde_json::json!({
                    "path": f.path,
                    "size": f.size,
                    // #324 accepted upper-case digests; the strict manifest wants lowercase.
                    "sha256": f.sha256.to_ascii_lowercase(),
                })
            })
            .collect();
        let bytes = serde_json::to_vec(
            &serde_json::json!({"version": 1, "app_id": self.game_id, "files": files}),
        )
        .map_err(|e| Error::ManifestInvalid(e.to_string()))?;
        let layout = Layout::Inline {
            manifest: bytes.clone(),
            prefix,
        };
        layout.validate()?;
        let parsed = layout.parse(&bytes, &self.game_id)?;
        Ok((layout, parsed))
    }
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

/// The state folder these functions keep inside the game folder.
pub fn state_dir(dest_dir: &Path) -> PathBuf {
    dest_dir.join(INLINE_STATE_DIR)
}

/// Inspect local completion and resumable progress without touching the network.
pub fn status_game_files(manifest: &GameFilesManifest, dest_dir: &Path) -> Result<GameFilesStatus> {
    let (_, parsed) = manifest.build()?;
    let local = engine::local_status(&parsed, dest_dir, &state_dir(dest_dir))?;
    Ok(GameFilesStatus {
        game_id: manifest.game_id.clone(),
        total_files: local.files_total as usize,
        completed_files: local.files_complete as usize,
        partial_files: local.files_partial as usize,
        bytes_on_disk: local.bytes_on_disk,
        total_bytes: local.bytes_total,
        complete: local.files_complete == local.files_total,
    })
}

/// Download all files in `manifest` into `dest_dir` resumably, through the one engine.
/// `source` serves the files at `game-files/<game_id>/<path>` (or `remote_root`). `protected`
/// is every saves folder that must never be (or sit inside) `dest_dir`, and that no manifest
/// path may land in or above if it sits inside `dest_dir`
/// (`gamefiles::saves_folders`; the C interface fills it from the remembered games):
/// `destination_conflict` otherwise.
pub fn download_game_files(
    source: &dyn GameSource,
    manifest: &GameFilesManifest,
    dest_dir: &Path,
    chunk_size: u64,
    protected: &[PathBuf],
) -> Result<GameFilesReport> {
    download_game_files_until(
        source,
        manifest,
        dest_dir,
        chunk_size,
        protected,
        &AtomicBool::new(false),
    )
}

/// `download_game_files` that stops once `stop` is set: after the chunk it is on is durable
/// (or at once, while waiting to retry). The report then says `complete: false`; a later call
/// continues from there.
pub fn download_game_files_until(
    source: &dyn GameSource,
    manifest: &GameFilesManifest,
    dest_dir: &Path,
    chunk_size: u64,
    protected: &[PathBuf],
    stop: &AtomicBool,
) -> Result<GameFilesReport> {
    let (layout, _) = manifest.build()?;
    let options = Options {
        chunk: chunk_size.max(1024),
        layout,
        protected: protected.to_vec(),
        ..Options::default()
    };
    let report = engine::run_report(
        source,
        &manifest.game_id,
        dest_dir,
        &state_dir(dest_dir),
        &options,
        stop,
        &mut |_| {},
    )?;
    Ok(GameFilesReport {
        game_id: manifest.game_id.clone(),
        total_files: report.progress.files_total as usize,
        downloaded_files: report.downloaded_files as usize,
        skipped_files: report.skipped_files as usize,
        bytes_downloaded: report.bytes_fetched,
        total_bytes: report.progress.bytes_total,
        complete: report.progress.state == State::Complete,
    })
}
