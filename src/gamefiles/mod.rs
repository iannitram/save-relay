//! Resumable per-game file downloads (docs/assets.md, #305, #309): a thin mobile app asks for
//! the rest of its game, and this module fetches it from the owner's own service folder in
//! bounded byte ranges, so a dead battery, a killed app or a dropped network costs at most one
//! chunk. Three services implement `GameSource`: Dropbox (`dropbox::Dropbox`), Nextcloud over
//! WebDAV (`webdav::WebDav`) and Google Drive (`gdrive::Drive`); the engine knows nothing
//! provider-specific, and `tests/gamefiles_providers.rs` runs one contract against all three.
//!
//! - `manifest`: the strict v1 manifest (relative path, size, SHA-256) and its path rules.
//! - `engine`: plan, run (checkpointed `.part` files, resume, verify, no-replace publish),
//!   cancel and cleanup.
//!
//! The layout on the service is `game-files/<app-id>/manifest.json` and the payload under
//! `game-files/<app-id>/files/`, apart from the save folders. `Layout` selects the two other
//! shapes: a host-trusted legacy mapping (ACNH) and the compatibility shape behind
//! `save_relay_download_game_files` (`game_files`).

pub mod engine;
pub(crate) mod fsx;
pub mod manifest;

use crate::error::{Error, Result};
use serde::Serialize;

/// Every saves folder a remembered game record names on `platform`: the record's resolved
/// folder, else its manifest's template. This is what `engine::Options::protected` is filled
/// with by the command line, the C interface and the compatibility wrapper, so a game-file
/// download can never land in (or sweep) a folder save sync writes. Fail closed: a games list
/// or a record that cannot be read is `destination_conflict`, never a folder left out. A record
/// with no saves folder on this platform has nothing to protect (sync cannot write there).
pub fn saves_folders(
    store: &crate::local::LocalStore,
    platform: crate::platform::Platform,
    vars: &crate::platform::PathVars,
) -> Result<Vec<(String, std::path::PathBuf)>> {
    let unreadable = |what: &str, e: Error| {
        Error::DestinationConflict(format!(
            "cannot check the saves folders of the remembered games ({what}: {e}); fix or remove the record first"
        ))
    };
    let mut found = Vec::new();
    for id in store
        .list_games()
        .map_err(|e| unreadable("the games list", e))?
    {
        let Some(record) = store
            .load_game(&id)
            .map_err(|e| unreadable(&format!("game '{id}'"), e))?
        else {
            continue;
        };
        let saves = record
            .saves_dir
            .clone()
            .or_else(|| record.manifest.saves_dir(platform, vars).ok());
        if let Some(saves) = saves {
            found.push((format!("the saves folder of '{id}'"), saves));
        }
    }
    Ok(found)
}

/// The state folder name the compatibility wrappers keep inside the game folder
/// (`Layout::Inline`). Manifest paths may not use it (`manifest` rejects `.save-relay*`).
pub const INLINE_STATE_DIR: &str = ".save-relay-game-files";

/// Where an app's manifest and payload live, and how the manifest is trusted. The layout is
/// chosen by the host's code, never by a remote manifest (docs/assets.md).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Layout {
    /// `game-files/<app-id>/manifest.json`, payload under `game-files/<app-id>/files/`.
    #[default]
    Standard,
    /// An explicitly configured legacy app (ACNH, `acnh-universal-game/android`): the manifest
    /// is `<root>/manifest.json`, may omit `app_id`, and the payload sits directly under
    /// `root`. The player's `user/`, `config.ini` and logs are refused as manifest paths.
    Legacy { root: String },
    /// Compatibility shape of the two functions released with #324: the caller supplies the
    /// manifest bytes (no remote manifest is read), the payload sits directly under `prefix`,
    /// and the state folder is `<dest>/.save-relay-game-files`.
    Inline { manifest: Vec<u8>, prefix: String },
}

/// The operating system a host runs on, as the allowlisted legacy layout names it. The value
/// comes from the host's own code (`HostOs::current`, or a token its code validated), never
/// from a remote manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Android,
    Ios,
    Linux,
    Macos,
    Windows,
}

impl HostOs {
    /// The OS this build runs on (Android is its own target, not Linux).
    pub fn current() -> Option<HostOs> {
        if cfg!(target_os = "android") {
            Some(HostOs::Android)
        } else if cfg!(target_os = "ios") {
            Some(HostOs::Ios)
        } else if cfg!(target_os = "linux") {
            Some(HostOs::Linux)
        } else if cfg!(target_os = "macos") {
            Some(HostOs::Macos)
        } else if cfg!(target_os = "windows") {
            Some(HostOs::Windows)
        } else {
            None
        }
    }

    /// The folder token: exactly `android`, `ios`, `linux`, `macos` or `windows`.
    pub fn token(self) -> &'static str {
        match self {
            HostOs::Android => "android",
            HostOs::Ios => "ios",
            HostOs::Linux => "linux",
            HostOs::Macos => "macos",
            HostOs::Windows => "windows",
        }
    }

    /// Validate a token (case-sensitive, exact) from a host's own configuration.
    pub fn from_token(token: &str) -> Option<HostOs> {
        [
            HostOs::Android,
            HostOs::Ios,
            HostOs::Linux,
            HostOs::Macos,
            HostOs::Windows,
        ]
        .into_iter()
        .find(|os| os.token() == token)
    }
}

impl Layout {
    /// The legacy layout of a "Universal" port, `<app>-universal-game/<os>` (ACNH:
    /// `acnh-universal-game/android`). `app` is the short port name (`acnh`: lowercase
    /// letters and digits joined by single hyphens) and `os` the host's, so a remote
    /// manifest can never pick either part.
    pub fn legacy_universal(app: &str, os: HostOs) -> Result<Layout> {
        let ok = !app.is_empty()
            && app.len() <= 64
            && app.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            });
        if !ok {
            return Err(crate::error::Error::Usage(
                "a legacy app name is lowercase letters and digits, joined by single hyphens"
                    .into(),
            ));
        }
        Ok(Layout::Legacy {
            root: format!("{app}-universal-game/{}", os.token()),
        })
    }

    /// The `Remote` path of the manifest, or `None` when the caller supplied it.
    pub fn manifest_path(&self, app_id: &str) -> Option<String> {
        match self {
            Layout::Standard => Some(remote_manifest_path(app_id)),
            Layout::Legacy { root } => Some(format!("{root}/manifest.json")),
            Layout::Inline { .. } => None,
        }
    }

    /// The `Remote` path of one payload file. `rel` must already be validated.
    pub fn file_path(&self, app_id: &str, rel: &str) -> String {
        match self {
            Layout::Standard => remote_file_path(app_id, rel),
            Layout::Legacy { root } => format!("{root}/{rel}"),
            Layout::Inline { prefix, .. } => format!("{prefix}/{rel}"),
        }
    }

    /// Parse the manifest bytes under this layout's trust rules.
    pub fn parse(&self, bytes: &[u8], app_id: &str) -> Result<manifest::Manifest> {
        match self {
            Layout::Standard | Layout::Inline { .. } => manifest::parse(bytes, app_id),
            Layout::Legacy { root } => {
                manifest::parse_with(bytes, app_id, manifest::Mode::Legacy { root })
            }
        }
    }

    /// Check the layout's own fields (the legacy root, the inline prefix).
    pub fn validate(&self) -> Result<()> {
        match self {
            Layout::Standard => Ok(()),
            Layout::Legacy { root } => manifest::check_remote_root(root),
            Layout::Inline { prefix, .. } => manifest::check_remote_root(prefix),
        }
    }

    /// A short key stored in the session, so staging made under one layout never resumes under
    /// another.
    pub(crate) fn key(&self) -> String {
        match self {
            Layout::Standard => "standard".into(),
            Layout::Legacy { root } => format!("legacy:{root}"),
            Layout::Inline { prefix, .. } => format!("inline:{prefix}"),
        }
    }

    /// Whether the state folder lives inside the game folder (see `INLINE_STATE_DIR`).
    pub(crate) fn nested_state(&self) -> bool {
        matches!(self, Layout::Inline { .. })
    }
}

/// Where an app's manifest lives, as a `Remote` path.
pub fn remote_manifest_path(app_id: &str) -> String {
    format!("game-files/{app_id}/manifest.json")
}

/// Where one payload file lives, as a `Remote` path. `rel` must already be validated.
pub fn remote_file_path(app_id: &str, rel: &str) -> String {
    format!("game-files/{app_id}/files/{rel}")
}

/// What a finished run did, beyond its final `Progress`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub progress: Progress,
    /// Files fetched and published by this run.
    pub downloaded_files: u64,
    /// Files already in the destination and verified.
    pub skipped_files: u64,
    /// Payload bytes received by this run (a resumed prefix is not counted).
    pub bytes_fetched: u64,
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
    /// The service's identity for this version of the file, when it gives one: Dropbox's
    /// `rev`, Nextcloud's strong `ETag` (a weak one is `None`), Drive's `<file id>:<version>`.
    /// A change between chunks stops the download.
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
    /// `fetch`, for a file whose earlier chunks came from the version `expected` (the identity
    /// an earlier answer carried, kept in the checkpoint across restarts). A service that can
    /// make the server refuse another version (WebDAV `If-Match`) or check it before sending
    /// (Drive's version) does so, and answers `Error::RemoteRevised` when it differs: the
    /// engine then discards the partial copy without retrying the same request. The default
    /// ignores `expected`; the engine still compares every answer's identity.
    fn fetch_matching(
        &self,
        path: &str,
        start: u64,
        len: u64,
        expected: Option<&str>,
    ) -> Result<RangeAnswer> {
        let _ = expected;
        self.fetch(path, start, len)
    }
}

/// Room for a service error body on a range request, so a short range does not turn a 401,
/// 404 or 503 answer into "longer than the range". Every source bounds a range answer to
/// `range_response_bound(len)`; the exact length of a 206 body is still checked by
/// `engine::validate` against its Content-Range.
pub const RANGE_RESPONSE_FLOOR: usize = 64 * 1024;

/// The `max_response_bytes` of a range request for `len` bytes: `max(len, RANGE_RESPONSE_FLOOR)`.
pub fn range_response_bound(len: u64) -> usize {
    usize::try_from(len)
        .unwrap_or(usize::MAX)
        .max(RANGE_RESPONSE_FLOOR)
}

/// The last byte of `start..start + len` (`Range: bytes=start-end`); no bytes or an overflow is
/// `Error::Usage`.
pub(crate) fn range_end(start: u64, len: u64) -> Result<u64> {
    if len == 0 {
        return Err(Error::Usage("a range of no bytes".into()));
    }
    start
        .checked_add(len - 1)
        .ok_or_else(|| Error::Usage("range end overflows".into()))
}

/// A body past the bound the request set (a server that ignored `Range` and sent the whole
/// file) is a range problem, not a retryable network error. The message names the path only.
pub(crate) fn over_bound(error: Error, path: &str) -> Error {
    let longer = || {
        Error::RangeUnsupported(format!(
            "the answer for {path} is longer than the range asked for"
        ))
    };
    match &error {
        Error::Server(message) if message.contains("exceeds the") => longer(),
        Error::Unreachable(message) if message.contains("larger than allowed") => longer(),
        _ => error,
    }
}

/// `over_bound` for a whole-file read (the manifest): longer than `max` is `ManifestInvalid`.
pub(crate) fn over_manifest_bound(error: Error, path: &str, max: usize) -> Error {
    match over_bound(error, path) {
        Error::RangeUnsupported(_) => Error::ManifestInvalid(if max == MANIFEST_MAX_BYTES {
            "longer than 4 MiB".into()
        } else {
            format!("longer than {max} bytes")
        }),
        other => other,
    }
}

/// A service path for a game file: not empty, no `\`, and no empty, `.` or `..` segment.
/// Anything else is `Error::NotFound(path)`, before any request is sent.
pub(crate) fn check_file_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains('\\')
        || path
            .split('/')
            .any(|segment| matches!(segment, "" | "." | ".."))
    {
        return Err(Error::NotFound(path.into()));
    }
    Ok(())
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
