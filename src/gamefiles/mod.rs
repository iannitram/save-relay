//! Resumable per-game file downloads (docs/assets.md, #305, #309): a thin mobile app asks for
//! the rest of its game, and this module fetches it from the owner's own service folder in
//! bounded byte ranges, so a dead battery, a killed app or a dropped network costs at most one
//! chunk. Dropbox is the first `GameSource` (`dropbox::Dropbox`); the engine knows nothing
//! provider-specific.
//!
//! - `manifest`: the strict v1 manifest (relative path, size, SHA-256) and its path rules.
//! - `engine`: plan, run (checkpointed `.part` files, resume, verify, no-replace publish),
//!   cancel and cleanup.
//!
//! The layout on the service is `game-files/<app-id>/manifest.json` and the payload under
//! `game-files/<app-id>/files/`, apart from the save folders.

pub mod engine;
pub mod manifest;

use crate::error::Result;
use serde::Serialize;

/// Where an app's manifest lives, as a `Remote` path.
pub fn remote_manifest_path(app_id: &str) -> String {
    format!("game-files/{app_id}/manifest.json")
}

/// Where one payload file lives, as a `Remote` path. `rel` must already be validated.
pub fn remote_file_path(app_id: &str, rel: &str) -> String {
    format!("game-files/{app_id}/files/{rel}")
}

/// The longest manifest read (docs/assets.md: 4 MiB before parsing).
pub const MANIFEST_MAX_BYTES: usize = 4 * 1024 * 1024;
/// The request size of a chunk unless the caller picks another.
pub const DEFAULT_CHUNK: u64 = 4 * 1024 * 1024;
/// The largest chunk a caller may ask for; it bounds the memory one request holds.
pub const MAX_CHUNK: u64 = 32 * 1024 * 1024;

/// What a service answered to one ranged read, unjudged: the engine checks every field
/// (`engine::validate`) so all services meet the same contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeAnswer {
    pub status: u16,
    pub content_range: Option<String>,
    pub content_encoding: Option<String>,
    pub content_type: Option<String>,
    /// The service's identity for this version of the file (Dropbox: the `rev`), when it
    /// gives one. A change between chunks stops the download.
    pub identity: Option<String>,
    pub body: Vec<u8>,
}

/// The seam between the engine and a service. Blocking, one request per call; the app's network
/// callback sits below it.
pub trait GameSource {
    /// A short stable key naming the service (`"dropbox"`); a saved session from another
    /// service never resumes.
    fn service(&self) -> &'static str;
    /// A whole small file (the manifest), failing if it is longer than `max` bytes.
    /// A missing file is `Error::NotFound`.
    fn read_bounded(&self, path: &str, max: usize) -> Result<Vec<u8>>;
    /// Bytes `start..start + len` of the file at `path` (`Range: bytes=start-end`), with
    /// reading bounded to `len` bytes even when the service ignores the range.
    /// A missing file is `Error::NotFound`.
    fn fetch(&self, path: &str, start: u64, len: u64) -> Result<RangeAnswer>;
}

/// Where a run stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Ready,
    Running,
    /// Stopped on request after a durable checkpoint; run again to continue.
    Cancelled,
    Complete,
    Error,
}

/// Progress of a run (`save_relay_game_files_progress`, and the observer's argument).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub app_id: String,
    pub state: State,
    pub files_done: u64,
    pub files_total: u64,
    /// Bytes of verified files plus the durable bytes of the file in flight.
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// The relative path in flight, or `null`.
    pub current: Option<String>,
}

impl Progress {
    pub fn new(app_id: &str) -> Progress {
        Progress {
            app_id: app_id.to_string(),
            state: State::Ready,
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            current: None,
        }
    }
}

/// What a run would do (`engine::plan`), from file sizes alone: nothing is hashed or written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub app_id: String,
    pub files_total: u64,
    pub bytes_total: u64,
    /// Present with the manifest's size; the run still rehashes each before trusting it.
    pub files_present: u64,
    pub bytes_present: u64,
    /// With a resumable partial copy.
    pub files_partial: u64,
    pub bytes_partial: u64,
    pub files_missing: u64,
    /// Bytes a run would still fetch.
    pub bytes_to_fetch: u64,
    /// Files in the way that a run would refuse to overwrite (at most 100 listed).
    pub conflicts: Vec<String>,
    pub conflict_count: u64,
}
