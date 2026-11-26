//! The download engine (docs/assets.md "Destination, checkpoint and publication"): plan what is
//! missing, then fetch it chunk by chunk into app-private `.part` files, checkpointing after each
//! chunk so a kill at any point loses at most one chunk, verify each finished file against the
//! manifest's size and SHA-256, and publish it with an atomic no-replace step.
//!
//! State directory (`state_dir`, private to this library):
//!
//! ```text
//! .save-relay-state  ownership marker, written when the library first uses the folder
//! lock               held (OS file lock) for the whole run; one writer per state directory
//! session.json       service, app id, destination, manifest digest, `ready` once all published
//! parts/<id>.part    a file's bytes so far (<id> = the first 32 hex of SHA-256 of its path)
//! parts/<id>.json    size, SHA-256, service identity (Dropbox rev, Nextcloud strong ETag,
//!                    Drive `<id>:<version>`) and the last durable offset
//! ```
//!
//! The library deletes inside a state folder only when it carries the ownership marker (or is
//! empty, or holds nothing but this library's own files and a valid session), and then only the
//! names it creates. The `.part` bytes beyond the checkpoint's durable offset are never trusted:
//! a restart cuts them off. A file is fetched only when it is not already in the destination; a
//! file that is there must hash to the manifest's SHA-256 (it is then adopted) or the run stops
//! with `destination_conflict`, and nothing in the destination is ever overwritten.
//!
//! Every destination and state path is traversed, created, opened and published through
//! directory handles that never follow a link (`fsx` lists what each platform guarantees and
//! the one residual Windows risk). Other limits (docs/assets.md): every file is fetched one at
//! a time; updating a file this library installed earlier is a conflict.

use super::fsx::{self, is_not_plain, Dir, Kind, Meta, SMALL_MAX};
use super::manifest::{self, Entry, Manifest};
use super::{
    GameSource, Layout, Plan, Progress, RangeAnswer, Report, State, DEFAULT_CHUNK,
    INLINE_STATE_DIR, MANIFEST_MAX_BYTES, MAX_CHUNK,
};
use crate::error::{Class, Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{File, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// A point in a run where a crash is simulated by a test (`Options::crash_at`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    /// A chunk is written and synced; its checkpoint is not.
    ChunkSynced,
    /// A chunk's checkpoint is durable.
    Checkpointed,
    /// The finished part hashed correctly; the checkpoint is still there.
    Verified,
    /// The checkpoint is removed; the part is not published yet.
    CheckpointRemoved,
    /// The file is in the destination; its part is still in the state folder.
    Published,
    /// Every file is published; `session.json` does not say `ready` yet.
    BeforeReady,
}

/// How a run behaves; `Options::default()` is what the C interface uses.
#[derive(Clone)]
pub struct Options {
    /// Bytes asked for per request, 1 to `MAX_CHUNK`.
    pub chunk: u64,
    /// Retries of one request after a retryable failure (waits 1 s, 2 s, 4 s, ... up to 30 s).
    /// A request that succeeds starts the count again, so a long file survives many drops.
    pub retries: u32,
    /// Where the manifest and payload live (`Layout::Standard` for the C interface).
    pub layout: Layout,
    /// How to wait; tests pass a no-op. The default is interruptible: it sleeps in short
    /// slices and the run's cancel flag ends the wait. A custom function is called once per
    /// wait and the flag is checked after it.
    pub sleep: Arc<dyn Fn(Duration) + Send + Sync>,
    /// Folders that must never be the destination or the state folder, nor hold or sit
    /// inside one (the configured save folders). The service or FFI layer fills this; an
    /// empty list checks nothing.
    pub protected: Vec<PathBuf>,
    /// Test hook: when it returns true at a `Boundary`, the run stops there with an error and
    /// cleans nothing up, as a killed process would.
    pub crash_at: Option<Arc<dyn Fn(Boundary) -> bool + Send + Sync>>,
}

type SleepFn = Arc<dyn Fn(Duration) + Send + Sync>;

fn default_sleep() -> &'static SleepFn {
    static SLEEP: OnceLock<SleepFn> = OnceLock::new();
    SLEEP.get_or_init(|| Arc::new(std::thread::sleep))
}

impl Default for Options {
    fn default() -> Options {
        Options {
            chunk: DEFAULT_CHUNK,
            retries: 5,
            layout: Layout::Standard,
            sleep: default_sleep().clone(),
            protected: Vec::new(),
            crash_at: None,
        }
    }
}

/// Wait `duration` (cut short when `stop` is set, with the default sleeper). `true` when stopped.
fn wait(options: &Options, stop: &AtomicBool, duration: Duration) -> bool {
    if !Arc::ptr_eq(&options.sleep, default_sleep()) {
        (options.sleep)(duration);
        return stop.load(Ordering::SeqCst);
    }
    let slice = Duration::from_millis(50);
    let mut left = duration;
    while !stop.load(Ordering::SeqCst) {
        if left.is_zero() {
            return false;
        }
        let step = left.min(slice);
        std::thread::sleep(step);
        left -= step;
    }
    true
}

struct Dirs {
    dest: Dir,
    state: Dir,
    parts: Dir,
    /// The canonical destination, recorded in the session.
    dest_id: String,
}

fn io(what: &str, path: &Path, error: std::io::Error) -> Error {
    Error::Io(format!("{what} {}: {error}", path.display()))
}

fn io_in(what: &str, name: &str, error: std::io::Error) -> Error {
    if is_not_plain(&error) {
        Error::DestinationConflict(format!(
            "{what} {name}: a link or the wrong kind of entry is in the way"
        ))
    } else {
        Error::Io(format!("{what} {name}: {error}"))
    }
}

/// Whether `inner` is `outer` or below it (component-wise; case-blind where the file system is).
fn within(inner: &Path, outer: &Path) -> bool {
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "android"))]
    {
        let fold = |p: &Path| -> Vec<String> {
            p.components()
                .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
                .collect()
        };
        let (i, o) = (fold(inner), fold(outer));
        i.len() >= o.len() && i[..o.len()] == o[..]
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "ios", target_os = "android")))]
    {
        inner.starts_with(outer)
    }
}

/// Resolve and judge the pair of roots before anything is created: separate (except the
/// nested layout), and clear of the protected save folders.
fn judge_roots(dest: &Path, state: &Path, layout: &Layout, protected: &[PathBuf]) -> Result<()> {
    let dest = fsx::resolve_lenient(dest).map_err(|e| io("cannot resolve", dest, e))?;
    let state = fsx::resolve_lenient(state).map_err(|e| io("cannot resolve", state, e))?;
    let nested_ok = layout.nested_state() && state == dest.join(INLINE_STATE_DIR);
    if !nested_ok && (within(&dest, &state) || within(&state, &dest)) {
        return Err(Error::DestinationConflict(
            "the game folder and the state folder must be separate folders, neither inside the other"
                .into(),
        ));
    }
    for saves in protected {
        let saves = fsx::resolve_lenient(saves).unwrap_or_else(|_| saves.clone());
        // The state folder may neither be, hold nor sit inside a saves folder. The game folder
        // may not be or sit inside one; it may hold one (a port keeps its saves in `<game>/user`),
        // but then no manifest path may land in or above it (`check_manifest_clear_of_saves`).
        if within(&state, &saves) || within(&saves, &state) {
            return Err(Error::DestinationConflict(format!(
                "the state folder may not be, hold or sit inside a save folder ({})",
                saves.display()
            )));
        }
        if within(&dest, &saves) {
            return Err(Error::DestinationConflict(format!(
                "the game folder may not be or sit inside a save folder ({})",
                saves.display()
            )));
        }
    }
    Ok(())
}

/// For a game folder that holds a saves folder: refuse a manifest with a path in or above that
/// saves folder, so a download can never write, replace or shadow what save sync owns. Paths
/// compare by the manifest's own fold (case, Unicode form), stricter than any file system.
fn check_manifest_clear_of_saves(
    dest: &Path,
    protected: &[PathBuf],
    manifest: &Manifest,
) -> Result<()> {
    let dest = fsx::resolve_lenient(dest).map_err(|e| io("cannot resolve", dest, e))?;
    let depth = dest.components().count();
    for saves in protected {
        let saves = fsx::resolve_lenient(saves).unwrap_or_else(|_| saves.clone());
        if !within(&saves, &dest) {
            continue;
        }
        let rel: Vec<String> = saves
            .components()
            .skip(depth)
            .map(|c| manifest::fold_segment(&c.as_os_str().to_string_lossy()))
            .collect();
        if rel.is_empty() {
            continue;
        }
        for entry in &manifest.files {
            let key: Vec<String> = entry.path.split('/').map(manifest::fold_segment).collect();
            if key.starts_with(&rel) || rel.starts_with(&key) {
                return Err(Error::DestinationConflict(format!(
                    "the manifest names {}, which is in or above a save folder inside the game folder ({}); a download never writes there",
                    entry.path,
                    saves.display()
                )));
            }
        }
    }
    Ok(())
}

pub(crate) const MARKER: &str = ".save-relay-state";
const MARKER_TEXT: &[u8] =
    b"save-relay game-files state folder; the library deletes only its own staging here\n";

/// Names this library creates in a state folder.
pub(crate) fn is_state_name(name: &str) -> bool {
    matches!(
        name,
        "lock" | "parts" | "session.json" | "session.json.tmp" | MARKER
    )
}

/// `<32 hex>.part`, `.json` or `.json.tmp`: what the library creates under `parts/`.
pub(crate) fn is_part_name(name: &str) -> bool {
    let Some((id, ext)) = name.split_once('.') else {
        return false;
    };
    id.len() == 32
        && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        && matches!(ext, "part" | "json" | "json.tmp")
}

/// Whether the library owns this state folder. A marker says so; so does an empty folder, and
/// a folder that holds nothing but this library's names (`lock`, `parts`, `session.json`)
/// with a valid session or only this library's part names under `parts` (folders made before
/// the marker existed). `claim` writes the marker; nothing else is ever adopted.
fn owned(state: &Dir) -> Result<bool> {
    match state
        .stat(MARKER)
        .map_err(|e| io_in("cannot read", MARKER, e))?
    {
        Some(Meta {
            kind: Kind::File, ..
        }) => return Ok(true),
        Some(_) => return Ok(false),
        None => {}
    }
    let names = state
        .names()
        .map_err(|e| io_in("cannot list", "the state folder", e))?;
    if names.is_empty() {
        return Ok(true);
    }
    if !names.iter().all(|n| is_state_name(n)) {
        return Ok(false);
    }
    // Only this library's names are here. A valid session proves it; so does a folder whose
    // `parts` holds nothing but this library's part names (a run that stopped before it wrote
    // `session.json`, or whose cleanup took it away, leaves `lock` and perhaps `parts`).
    if read_session(state).is_some() {
        return Ok(true);
    }
    // A `session.json` (or its `.tmp`) that is not ours is somebody's file with our name:
    // never adopted, so a run cannot replace it nor a cleanup delete it.
    if names
        .iter()
        .any(|n| n == "session.json" || n == "session.json.tmp")
    {
        return Ok(false);
    }
    match state.sub("parts", false) {
        Ok(parts) => Ok(parts
            .names()
            .map_err(|e| io_in("cannot list", "parts", e))?
            .iter()
            .all(|n| is_part_name(n))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(_) => Ok(false),
    }
}

fn not_owned() -> Error {
    Error::DestinationConflict(
        "that folder is not a save-relay state folder (no ownership marker), so nothing in it is touched"
            .into(),
    )
}

fn claim(state: &Dir) -> Result<()> {
    if !owned(state)? {
        return Err(not_owned());
    }
    match state.create_new(MARKER) {
        Ok(mut file) => {
            file.write_all(MARKER_TEXT)
                .and_then(|_| file.sync_all())
                .map_err(|e| io_in("cannot write", MARKER, e))?;
            state
                .sync()
                .map_err(|e| io_in("cannot sync", "the state folder", e))
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(io_in("cannot create", MARKER, e)),
    }
}

/// Create (if needed) and open both roots, refuse an unowned state folder, create `parts`,
/// and check that staging and destination share a file system.
fn prepare(dest: &Path, state: &Path, layout: &Layout, protected: &[PathBuf]) -> Result<Dirs> {
    judge_roots(dest, state, layout, protected)?;
    for dir in [dest, state] {
        fsx::create_dir_all_synced(dir).map_err(|e| io("cannot create", dir, e))?;
    }
    let dest_c = std::fs::canonicalize(dest).map_err(|e| io("cannot resolve", dest, e))?;
    let state_c = std::fs::canonicalize(state).map_err(|e| io("cannot resolve", state, e))?;
    let dest_dir = Dir::open(&dest_c).map_err(|e| io("cannot open", &dest_c, e))?;
    let state_dir = Dir::open(&state_c).map_err(|e| io("cannot open", &state_c, e))?;
    claim(&state_dir)?;
    let parts = state_dir
        .sub("parts", true)
        .map_err(|e| io_in("cannot open", "parts", e))?;
    let same = dest_dir
        .device()
        .and_then(|d| parts.device().map(|p| d == p))
        .map_err(|e| io("cannot inspect", &dest_c, e))?;
    if !same {
        return Err(Error::DestinationConflict(
            "the game folder and the state folder are on different volumes; a download is staged in the state folder and moved into the game folder, so both must be on one volume"
                .into(),
        ));
    }
    Ok(Dirs {
        dest: dest_dir,
        state: state_dir,
        parts,
        dest_id: dest_c.to_string_lossy().into_owned(),
    })
}

/// The OS-held lock that makes one run the only writer of a state folder. A crash releases it.
struct Lock(#[allow(dead_code)] File);

fn acquire(state: &Dir) -> Result<Lock> {
    let file = state
        .open_rw("lock", true)
        .map_err(|e| io_in("cannot open", "lock", e))?;
    match file.try_lock() {
        Ok(()) => Ok(Lock(file)),
        Err(TryLockError::WouldBlock) => Err(Error::Busy(
            "another download is using this game folder".into(),
        )),
        Err(TryLockError::Error(e)) => Err(io_in("cannot lock", "lock", e)),
    }
}

#[derive(Serialize, Deserialize)]
struct Session {
    version: u32,
    app_id: String,
    service: String,
    /// The layout key (`Layout::key`); sessions written before it existed were `standard`.
    #[serde(default = "standard_layout")]
    layout: String,
    /// The canonical destination the partials were staged for; a different one resets them.
    #[serde(default)]
    dest: String,
    manifest_sha256: String,
    ready: bool,
}

fn standard_layout() -> String {
    "standard".into()
}

fn read_session(state: &Dir) -> Option<Session> {
    let bytes = state.read_small("session.json", SMALL_MAX).ok()??;
    serde_json::from_slice(&bytes)
        .ok()
        .filter(|s: &Session| s.version == 1)
}

fn write_json(dir: &Dir, name: &str, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|e| Error::Io(e.to_string()))?;
    dir.write_atomic(name, &bytes)
        .map_err(|e| io_in("cannot write", name, e))
}

/// A file's checkpoint: what its `.part` is a prefix of, and how much of it is durable.
#[derive(Serialize, Deserialize, Clone)]
struct Partial {
    path: String,
    size: u64,
    sha256: String,
    rev: Option<String>,
    durable: u64,
}

impl Partial {
    fn fresh(entry: &Entry) -> Partial {
        Partial {
            path: entry.path.clone(),
            size: entry.size,
            sha256: entry.sha256.clone(),
            rev: None,
            durable: 0,
        }
    }
    fn matches(&self, entry: &Entry) -> bool {
        self.path == entry.path && self.size == entry.size && self.sha256 == entry.sha256
    }
}

fn part_id(path: &str) -> String {
    manifest::hex(&Sha256::digest(path.as_bytes()))[..32].to_string()
}

fn part_name(path: &str) -> String {
    format!("{}.part", part_id(path))
}

fn json_name(path: &str) -> String {
    format!("{}.json", part_id(path))
}

fn read_partial(parts: &Dir, path: &str) -> Option<Partial> {
    let bytes = parts.read_small(&json_name(path), SMALL_MAX).ok()??;
    serde_json::from_slice(&bytes).ok()
}

/// SHA-256 and length of an open file, streamed; `None` when `stop` was set meanwhile.
fn sha256_stream(mut file: File, stop: &AtomicBool) -> std::io::Result<Option<(u64, String)>> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut length = 0u64;
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        length += n as u64;
    }
    Ok(Some((length, manifest::hex(&hasher.finalize()))))
}

/// Hash a file in `dir` without being stoppable (read-only status calls and the rare
/// re-check after a publish race).
fn sha256_named(dir: &Dir, name: &str) -> std::io::Result<(u64, String)> {
    let never = AtomicBool::new(false);
    let file = dir.open_read(name)?;
    Ok(sha256_stream(file, &never)?.expect("never stopped"))
}

/// What one chunk request brought: a validated answer, or word that the file is another
/// version now.
enum Chunk {
    Answer(RangeAnswer),
    Revised,
}

/// What a missing file may also mean on this service. Under Drive's `drive.file` scope the
/// app sees only files it created or was given: a file put there through Drive's web site or
/// desktop app stays invisible however long the app waits, so the message says so.
fn hidden_hint(source: &dyn GameSource) -> &'static str {
    if source.service() == "gdrive" {
        " (or Drive hides it: this app sees only files it created or was given, and a file uploaded through Drive's web site or desktop app is not given to it)"
    } else {
        ""
    }
}

fn conflict(rel: &str, why: &str) -> Error {
    Error::DestinationConflict(format!("{rel}: {why}; it is left as it is"))
}

/// What the destination holds at a manifest path.
enum Found {
    Missing,
    Entry(Meta),
    /// A link or a file stands where a folder belongs.
    Blocked,
}

fn segments(rel: &str) -> (Vec<&str>, &str) {
    let mut parts: Vec<&str> = rel.split('/').collect();
    let leaf = parts.pop().unwrap_or(rel);
    (parts, leaf)
}

/// Look at `rel` under `dest` without creating or following anything.
fn probe(dest: &Dir, rel: &str) -> Result<Found> {
    let (folders, leaf) = segments(rel);
    let mut current: Option<Dir> = None;
    for folder in folders {
        let base = current.as_ref().unwrap_or(dest);
        match base.sub(folder, false) {
            Ok(next) => current = Some(next),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Found::Missing),
            Err(e) if is_not_plain(&e) => return Ok(Found::Blocked),
            Err(e) => return Err(io_in("cannot read", folder, e)),
        }
    }
    let base = current.as_ref().unwrap_or(dest);
    match base.stat(leaf).map_err(|e| io_in("cannot read", leaf, e))? {
        Some(meta) => Ok(Found::Entry(meta)),
        None => Ok(Found::Missing),
    }
}

/// The folder that holds `rel` and its file name, creating missing folders (each synced) and
/// refusing a link or a file where a folder belongs.
fn parent_of<'r>(dest: &Dir, rel: &'r str) -> Result<(Dir, &'r str)> {
    let (folders, leaf) = segments(rel);
    let mut current = dest
        .try_clone()
        .map_err(|e| Error::Io(format!("cannot open the game folder: {e}")))?;
    for folder in folders {
        current = current.sub(folder, true).map_err(|e| {
            if is_not_plain(&e) {
                Error::DestinationConflict(format!("{folder} is in the way of {rel}"))
            } else {
                io_in("cannot create", folder, e)
            }
        })?;
    }
    Ok((current, leaf))
}

/// Judge one ranged answer for the request `start..start + len` of a file of `size` bytes
/// (docs/assets.md "Range and identity contract"): only an identity-encoded, single-part 206
/// that begins at `start`, stays inside the request, names the manifest's size and carries
/// exactly the bytes it claims. Returns the body.
pub fn validate(answer: RangeAnswer, start: u64, len: u64, size: u64) -> Result<RangeAnswer> {
    let unsupported = |why: &str| Error::RangeUnsupported(why.to_string());
    match answer.status {
        206 => {}
        200 => return Err(unsupported("the service answered 200 to a byte range")),
        416 => return Err(unsupported("the service refused the byte range (416)")),
        other => return Err(Error::Server(format!("answer {other} to a byte range"))),
    }
    if answer
        .content_encoding
        .as_deref()
        .is_some_and(|e| !e.trim().eq_ignore_ascii_case("identity"))
    {
        return Err(unsupported("the answer is content-encoded"));
    }
    if answer
        .content_type
        .as_deref()
        .is_some_and(|t| t.to_ascii_lowercase().contains("multipart/byteranges"))
    {
        return Err(unsupported("the answer is multipart"));
    }
    let range = answer
        .content_range
        .as_deref()
        .and_then(parse_content_range)
        .ok_or_else(|| unsupported("the answer has no usable Content-Range"))?;
    let (first, last, total) = range;
    if first != start || last < first || last > start + len - 1 {
        return Err(unsupported(
            "the answer is not the range that was asked for",
        ));
    }
    if total != size {
        return Err(Error::RemoteChanged(format!(
            "the service reports {total} bytes, the manifest {size}"
        )));
    }
    if answer.body.len() as u64 != last - first + 1 {
        return Err(unsupported(
            "the body is not as long as its Content-Range says",
        ));
    }
    Ok(answer)
}

/// `bytes 0-99/1000` as `(0, 99, 1000)`.
fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (first, last) = range.split_once('-')?;
    Some((
        first.trim().parse().ok()?,
        last.trim().parse().ok()?,
        total.trim().parse().ok()?,
    ))
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_secs((1u64 << (attempt - 1).min(5)).min(30))
}

/// The longest a service's `Retry-After` is honoured for one wait; a hostile or buggy header
/// cannot park a run for longer.
const MAX_RETRY_AFTER_SECS: u64 = 60;

/// How long to wait before retry number `attempt` after `error`: the backoff, or the service's
/// own `Retry-After` when that is longer (never over `MAX_RETRY_AFTER_SECS`).
fn retry_wait(error: &Error, attempt: u32) -> Duration {
    let asked = match error {
        Error::RateLimited {
            retry_after_secs: Some(seconds),
            ..
        } => Duration::from_secs((*seconds).min(MAX_RETRY_AFTER_SECS)),
        _ => Duration::ZERO,
    };
    backoff(attempt).max(asked)
}

/// Run `request`, retrying a retryable failure with a wait. `None` when stopped while waiting.
fn retrying<T>(
    options: &Options,
    stop: &AtomicBool,
    mut request: impl FnMut() -> Result<T>,
) -> Result<Option<T>> {
    let mut attempt = 0;
    loop {
        match request() {
            Ok(value) => return Ok(Some(value)),
            Err(error) if error.class() == Class::Retryable && attempt < options.retries => {
                attempt += 1;
                if wait(options, stop, retry_wait(&error, attempt)) {
                    return Ok(None);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

fn load_manifest(
    source: &dyn GameSource,
    app_id: &str,
    options: &Options,
    stop: &AtomicBool,
) -> Result<Option<Manifest>> {
    // Before any remote path is built from it.
    manifest::check_app_id(app_id)?;
    options.layout.validate()?;
    let Some(path) = options.layout.manifest_path(app_id) else {
        // The caller supplied the manifest: no request, no retry.
        let Layout::Inline { manifest, .. } = &options.layout else {
            unreachable!("only an inline layout has no remote manifest");
        };
        return options.layout.parse(manifest, app_id).map(Some);
    };
    let fetched = retrying(options, stop, || {
        source
            .read_bounded(&path, MANIFEST_MAX_BYTES)
            .map_err(|error| match error {
                Error::NotFound(_) => Error::NotYetUploaded(format!(
                    "the manifest for {app_id} is not in the service folder{}",
                    hidden_hint(source)
                )),
                other => other,
            })
    })?;
    match fetched {
        Some(bytes) => options.layout.parse(&bytes, app_id).map(Some),
        None => Ok(None),
    }
}

/// Open an existing folder for reading only (never creating anything); `None` when it is not
/// there or cannot be opened without following a link.
fn open_existing(path: &Path) -> Option<Dir> {
    Dir::open(&std::fs::canonicalize(path).ok()?).ok()
}

fn open_parts_readonly(state_dir: &Path) -> Option<Dir> {
    open_existing(state_dir)?.sub("parts", false).ok()
}

/// Bytes of a file's `.part` a run would resume from: the checkpoint's durable offset when the
/// checkpoint is this file's and the part (a plain file, not shared with another name) holds it.
fn resumable(parts: Option<&Dir>, entry: &Entry) -> u64 {
    let Some(parts) = parts else {
        return 0;
    };
    read_partial(parts, &entry.path)
        .filter(|p| p.matches(entry) && p.durable <= entry.size)
        .filter(|p| {
            parts
                .stat(&part_name(&entry.path))
                .ok()
                .flatten()
                .is_some_and(|m| m.kind == Kind::File && m.nlink == 1 && m.len >= p.durable)
        })
        .map_or(0, |p| p.durable)
}

/// The existing folder that holds `rel` and its file name; nothing is created.
fn existing_parent<'r>(dest: &Dir, rel: &'r str) -> Option<(Dir, &'r str)> {
    let (folders, leaf) = segments(rel);
    let mut current = dest.try_clone().ok()?;
    for folder in folders {
        current = current.sub(folder, false).ok()?;
    }
    Some((current, leaf))
}

/// What a run would do, from the manifest on the service and the sizes of what is on disk.
pub fn plan(
    source: &dyn GameSource,
    app_id: &str,
    dest: &Path,
    state_dir: &Path,
    options: &Options,
) -> Result<Plan> {
    // Read-only, but the same folder rules as a run: a plan never reports on a saves folder.
    judge_roots(dest, state_dir, &options.layout, &options.protected)?;
    let never = AtomicBool::new(false);
    let manifest = load_manifest(source, app_id, options, &never)?
        .ok_or_else(|| Error::Io("stopped".into()))?;
    check_manifest_clear_of_saves(dest, &options.protected, &manifest)?;
    let dest = open_existing(dest);
    let parts = open_parts_readonly(state_dir);
    let mut plan = Plan {
        app_id: app_id.to_string(),
        files_total: manifest.files.len() as u64,
        bytes_total: manifest.total_bytes,
        files_present: 0,
        bytes_present: 0,
        files_partial: 0,
        bytes_partial: 0,
        files_missing: 0,
        bytes_to_fetch: 0,
        conflicts: Vec::new(),
        conflict_count: 0,
    };
    let note_conflict = |plan: &mut Plan, path: &str| {
        plan.conflict_count += 1;
        if plan.conflicts.len() < 100 {
            plan.conflicts.push(path.to_string());
        }
    };
    for entry in &manifest.files {
        let found = match &dest {
            Some(dest) => probe(dest, &entry.path)?,
            None => Found::Missing,
        };
        match found {
            Found::Entry(meta) if meta.kind == Kind::File && meta.len == entry.size => {
                plan.files_present += 1;
                plan.bytes_present += entry.size;
                continue;
            }
            Found::Entry(_) | Found::Blocked => {
                note_conflict(&mut plan, &entry.path);
                continue;
            }
            Found::Missing => {}
        }
        let have = resumable(parts.as_ref(), entry);
        if have > 0 {
            plan.files_partial += 1;
            plan.bytes_partial += have;
        } else {
            plan.files_missing += 1;
        }
        plan.bytes_to_fetch += entry.size - have;
    }
    Ok(plan)
}

/// Remove this library's staging for a state folder (the `.part` files and checkpoints), never
/// anything in the game folder and never in a folder the library does not own (one without its
/// ownership marker is refused with `destination_conflict`). The next run starts every
/// unfinished file again.
pub fn cleanup(state_dir: &Path) -> Result<()> {
    let Ok(canonical) = std::fs::canonicalize(state_dir) else {
        return Ok(());
    };
    let state = Dir::open(&canonical).map_err(|e| io("cannot open", &canonical, e))?;
    if !owned(&state)? {
        return Err(not_owned());
    }
    let _lock = acquire(&state)?;
    match state.sub("parts", false) {
        Ok(parts) => {
            let names = parts
                .names()
                .map_err(|e| io_in("cannot list", "parts", e))?;
            for name in names.iter().filter(|n| is_part_name(n)) {
                parts
                    .remove(name)
                    .map_err(|e| io_in("cannot remove", name, e))?;
            }
            drop(parts);
            // Only an empty folder goes; anything else in it is not ours to delete.
            let _ = state.remove_dir("parts");
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_in("cannot open", "parts", e)),
    }
    state
        .remove("session.json")
        .and_then(|_| state.sync())
        .map_err(|e| io_in("cannot remove", "session.json", e))
}

enum Fetched {
    Verified,
    /// The finished part did not match; carries whether it had resumed old bytes.
    Corrupt(bool),
    Stopped,
}

struct Run<'a> {
    source: &'a dyn GameSource,
    app_id: &'a str,
    dirs: Dirs,
    options: &'a Options,
    stop: &'a AtomicBool,
    observe: &'a mut dyn FnMut(&Progress),
    progress: Progress,
    done_bytes: u64,
    downloaded_files: u64,
    skipped_files: u64,
    bytes_fetched: u64,
}

impl Run<'_> {
    fn notify(&mut self, in_flight: u64) {
        self.progress.bytes_done = self.done_bytes + in_flight;
        (self.observe)(&self.progress);
    }

    /// Stop here like a killed process would, when a test asked for it.
    fn boundary(&self, at: Boundary) -> Result<()> {
        match &self.options.crash_at {
            Some(hook) if hook(at) => Err(Error::Io(format!("crash injected at {at:?}"))),
            _ => Ok(()),
        }
    }

    fn drop_partial(&self, rel: &str) {
        let _ = self.dirs.parts.remove(&part_name(rel));
        let _ = self.dirs.parts.remove(&json_name(rel));
    }

    fn finish(&mut self, entry: &Entry) {
        self.drop_partial(&entry.path);
        self.done_bytes += entry.size;
        self.progress.files_done += 1;
        self.notify(0);
    }

    /// Delete `.part` files and checkpoints that do not belong to this manifest, this service
    /// or a still-matching file. Only names this library makes are ever deleted.
    fn sweep(&self, manifest: &Manifest, reset_all: bool) {
        let wanted: HashMap<String, &Entry> = manifest
            .files
            .iter()
            .map(|e| (part_id(&e.path), e))
            .collect();
        let Ok(names) = self.dirs.parts.names() else {
            return;
        };
        let mut keep = HashSet::new();
        for name in &names {
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            let kept = !reset_all
                && wanted.get(id).is_some_and(|entry| {
                    self.dirs
                        .parts
                        .read_small(name, SMALL_MAX)
                        .ok()
                        .flatten()
                        .and_then(|b| serde_json::from_slice::<Partial>(&b).ok())
                        .is_some_and(|p| p.matches(entry))
                });
            if kept {
                keep.insert(format!("{id}.json"));
                keep.insert(format!("{id}.part"));
            }
        }
        for name in names {
            if is_part_name(&name) && !keep.contains(&name) {
                let _ = self.dirs.parts.remove(&name);
            }
        }
    }

    /// One validated chunk, `Ok(None)` when stopped while waiting. `expected` is the identity
    /// the earlier chunks came from. A source that reports another version
    /// (`Error::RemoteRevised`) is not retried: the same request would be refused again.
    fn fetch_chunk(
        &mut self,
        entry: &Entry,
        start: u64,
        len: u64,
        expected: Option<&str>,
    ) -> Result<Option<Chunk>> {
        let path = self.options.layout.file_path(self.app_id, &entry.path);
        let source = self.source;
        retrying(self.options, self.stop, || {
            let answer = match source.fetch_matching(&path, start, len, expected) {
                Ok(answer) => answer,
                Err(Error::RemoteRevised(_)) => return Ok(Chunk::Revised),
                Err(Error::NotFound(_)) => {
                    return Err(Error::NotYetUploaded(format!(
                        "{} is not in the service folder{}",
                        entry.path,
                        hidden_hint(source)
                    )))
                }
                Err(other) => return Err(other),
            };
            validate(answer, start, len, entry.size).map(Chunk::Answer)
        })
    }

    /// Settle the `.part` against its checkpoint and open it for writing at `partial.durable`.
    /// A part that is a link, a folder, shorter than its checkpoint, or has another name
    /// (a hard link count above 1: the installed file may share its inode) is discarded, never
    /// written through.
    fn open_part(&self, name: &str, partial: &mut Partial) -> Result<File> {
        let parts = &self.dirs.parts;
        let read = |e| io_in("cannot read", name, e);
        let mut usable = match parts.stat(name).map_err(read)? {
            Some(md) if md.kind == Kind::File && md.nlink == 1 => {
                md.len >= partial.durable && partial.durable <= partial.size
            }
            Some(md) => {
                // A link, a folder or a shared inode where our file belongs: unlink the name
                // (the other name of a shared inode is untouched) and start the file again.
                if md.kind == Kind::Dir {
                    let _ = parts.remove_dir(name);
                } else {
                    parts.remove(name).map_err(read)?;
                }
                false
            }
            None => false,
        };
        for _ in 0..2 {
            if !usable {
                partial.durable = 0;
                partial.rev = None;
            }
            let created = parts.stat(name).map_err(read)?.is_none();
            let file = parts
                .open_rw(name, true)
                .map_err(|e| io_in("cannot open", name, e))?;
            if fsx::nlink(&file).map_err(read)? > 1 {
                // Raced: another name appeared between the check and the open.
                drop(file);
                parts.remove(name).map_err(read)?;
                usable = false;
                continue;
            }
            if created {
                parts.sync().map_err(read)?;
            }
            // Bytes past the durable offset may be torn or unsynced: cut them off.
            file.set_len(partial.durable)
                .map_err(|e| io_in("cannot resize", name, e))?;
            return Ok(file);
        }
        Err(conflict(
            &partial.path,
            "the staging file keeps another name",
        ))
    }

    /// The file changed (or cannot be bound to what is staged): drop the partial; the next run
    /// restarts this file. Retryable, code `remote_changed`.
    fn restart(&self, file: File, entry: &Entry, why: &str) -> Error {
        drop(file);
        self.drop_partial(&entry.path);
        Error::RemoteRevised(format!(
            "{} {why}; run again to fetch it from the start",
            entry.path
        ))
    }

    fn download(&mut self, entry: &Entry) -> Result<Fetched> {
        let part = part_name(&entry.path);
        let json = json_name(&entry.path);
        let mut partial = read_partial(&self.dirs.parts, &entry.path)
            .filter(|p| p.matches(entry))
            .unwrap_or_else(|| Partial::fresh(entry));
        let mut file = self.open_part(&part, &mut partial)?;
        let resumed = partial.durable > 0;
        self.notify(partial.durable);
        while partial.durable < entry.size {
            if self.stop.load(Ordering::SeqCst) {
                return Ok(Fetched::Stopped);
            }
            let len = self.options.chunk.min(entry.size - partial.durable);
            let expected = partial.rev.clone();
            let answer = match self.fetch_chunk(entry, partial.durable, len, expected.as_deref())? {
                None => return Ok(Fetched::Stopped),
                Some(Chunk::Answer(answer)) => answer,
                Some(Chunk::Revised) => {
                    return Err(self.restart(file, entry, "is a different version now"))
                }
            };
            // Identity is bound before a byte is appended. An answer without one cannot be
            // bound to bytes already held, and an answer with one cannot be bound to held
            // bytes that never had one: both fail closed and restart the file.
            match (&answer.identity, &partial.rev) {
                (Some(new), Some(old)) if new != old => {
                    return Err(self.restart(file, entry, "is a different version now"))
                }
                (Some(_), None) if partial.durable > 0 => {
                    return Err(self.restart(
                        file,
                        entry,
                        "now reports a version, but the bytes already held have none",
                    ))
                }
                (Some(new), _) => partial.rev = Some(new.clone()),
                (None, Some(_)) => {
                    return Err(self.restart(
                        file,
                        entry,
                        "was answered without its version, which the bytes already held need",
                    ))
                }
                (None, None) => {}
            }
            file.seek(SeekFrom::Start(partial.durable))
                .and_then(|_| file.write_all(&answer.body))
                .and_then(|_| file.sync_data())
                .map_err(|e| io_in("cannot write", &part, e))?;
            self.boundary(Boundary::ChunkSynced)?;
            partial.durable += answer.body.len() as u64;
            self.bytes_fetched += answer.body.len() as u64;
            if partial.durable < entry.size {
                write_json(&self.dirs.parts, &json, &partial)?;
                self.boundary(Boundary::Checkpointed)?;
            }
            self.notify(partial.durable);
        }
        file.sync_all()
            .map_err(|e| io_in("cannot sync", &part, e))?;
        drop(file);
        let reader = self
            .dirs
            .parts
            .open_read(&part)
            .map_err(|e| io_in("cannot read", &part, e))?;
        let Some((length, hash)) =
            sha256_stream(reader, self.stop).map_err(|e| io_in("cannot read", &part, e))?
        else {
            return Ok(Fetched::Stopped);
        };
        if length == entry.size && hash == entry.sha256 {
            Ok(Fetched::Verified)
        } else {
            self.drop_partial(&entry.path);
            Ok(Fetched::Corrupt(resumed))
        }
    }

    /// Make `entry` present and verified. `false` when stopped on request.
    fn install(&mut self, entry: &Entry) -> Result<bool> {
        let (parent, leaf) = parent_of(&self.dirs.dest, &entry.path)?;
        self.progress.current = Some(entry.path.clone());
        let read = |e| io_in("cannot read", &entry.path, e);
        match parent.stat(leaf).map_err(read)? {
            Some(md) if md.kind != Kind::File => {
                return Err(conflict(
                    &entry.path,
                    "something that is not a file is there",
                ))
            }
            Some(md) if md.len != entry.size => {
                return Err(conflict(&entry.path, "a file of another size is there"))
            }
            Some(_) => {
                self.notify(0);
                let file = parent.open_read(leaf).map_err(read)?;
                let Some((_, hash)) = sha256_stream(file, self.stop).map_err(read)? else {
                    return Ok(false);
                };
                if hash != entry.sha256 {
                    return Err(conflict(&entry.path, "a different file is there"));
                }
                // The bytes are the manifest's own, so the execute bit it names is safe to
                // apply: a copy that lost its mode (FAT, zip) must still launch.
                if entry.exec {
                    let mark = |e| io_in("cannot mark executable", &entry.path, e);
                    let file = parent.open_read(leaf).map_err(mark)?;
                    manifest::make_executable(&file).map_err(mark)?;
                }
                self.skipped_files += 1;
                self.finish(entry);
                return Ok(true);
            }
            None => {}
        }
        for attempt in 0..2 {
            match self.download(entry)? {
                Fetched::Stopped => return Ok(false),
                Fetched::Verified => break,
                Fetched::Corrupt(true) if attempt == 0 => continue,
                Fetched::Corrupt(_) => {
                    return Err(Error::IntegrityFailed(format!(
                        "{} does not match the manifest's size and SHA-256",
                        entry.path
                    )))
                }
            }
        }
        self.boundary(Boundary::Verified)?;
        self.publish(entry, &parent, leaf)?;
        self.boundary(Boundary::Published)?;
        self.downloaded_files += 1;
        self.finish(entry);
        Ok(true)
    }

    /// Move the verified part to its final name without replacing anything (`fsx::Dir::publish`:
    /// a no-replace rename, never an overwrite). The checkpoint goes first and durably, so a
    /// crash can never leave a checkpointed part that shares its inode with the installed file;
    /// a crash after that costs this one file's staging, not correctness.
    fn publish(&self, entry: &Entry, parent: &Dir, leaf: &str) -> Result<()> {
        let parts = &self.dirs.parts;
        let sync = |e| io_in("cannot sync", "the staging folder", e);
        parts.remove(&json_name(&entry.path)).map_err(sync)?;
        parts.sync().map_err(sync)?;
        self.boundary(Boundary::CheckpointRemoved)?;
        if entry.exec {
            let mark = |e| io_in("cannot mark executable", &entry.path, e);
            let file = parts.open_read(&part_name(&entry.path)).map_err(mark)?;
            manifest::make_executable(&file).map_err(mark)?;
        }
        match parts.publish(&part_name(&entry.path), parent, leaf) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let same = sha256_named(parent, leaf)
                    .is_ok_and(|(length, hash)| length == entry.size && hash == entry.sha256);
                if same {
                    Ok(())
                } else {
                    Err(conflict(&entry.path, "a different file appeared meanwhile"))
                }
            }
            Err(e) => Err(Error::Io(format!(
                "cannot move {} into place without replacing anything: {e}",
                entry.path
            ))),
        }
    }
}

/// Fetch every file of `app_id` that is not yet in `dest`, resuming from `state_dir`.
///
/// Returns the final `Progress`: `Complete`, or `Cancelled` when `stop` was set (checked
/// between chunks, while hashing and while waiting to retry, so the stop lands after a durable
/// checkpoint and a later run continues). Any other end is an `Err`; the checkpoint stays for
/// the next run, and the observer is told `State::Error` first. `observe` is called after
/// every chunk and file.
pub fn run(
    source: &dyn GameSource,
    app_id: &str,
    dest: &Path,
    state_dir: &Path,
    options: &Options,
    stop: &AtomicBool,
    observe: &mut dyn FnMut(&Progress),
) -> Result<Progress> {
    run_report(source, app_id, dest, state_dir, options, stop, observe).map(|r| r.progress)
}

/// `run`, also saying how many files were fetched and how many were already there.
pub fn run_report(
    source: &dyn GameSource,
    app_id: &str,
    dest: &Path,
    state_dir: &Path,
    options: &Options,
    stop: &AtomicBool,
    observe: &mut dyn FnMut(&Progress),
) -> Result<Report> {
    manifest::check_app_id(app_id)?;
    options.layout.validate()?;
    let mut options = options.clone();
    options.chunk = options.chunk.clamp(1, MAX_CHUNK);
    let dirs = prepare(dest, state_dir, &options.layout, &options.protected)?;
    let _lock = acquire(&dirs.state)?;
    let mut run = Run {
        source,
        app_id,
        dirs,
        options: &options,
        stop,
        observe,
        progress: Progress::new(app_id),
        done_bytes: 0,
        downloaded_files: 0,
        skipped_files: 0,
        bytes_fetched: 0,
    };
    let outcome = run.go();
    match &outcome {
        Ok(progress) => (run.observe)(progress),
        Err(_) => {
            run.progress.state = State::Error;
            (run.observe)(&run.progress);
        }
    }
    outcome.map(|progress| Report {
        progress,
        downloaded_files: run.downloaded_files,
        skipped_files: run.skipped_files,
        bytes_fetched: run.bytes_fetched,
    })
}

/// What `dest` holds of `manifest` without any network: files that already match the
/// manifest's size and SHA-256 (hashed), files with a resumable checkpoint, and bytes on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalStatus {
    pub files_total: u64,
    pub files_complete: u64,
    pub files_partial: u64,
    pub bytes_on_disk: u64,
    pub bytes_total: u64,
}

pub fn local_status(manifest: &Manifest, dest: &Path, state_dir: &Path) -> Result<LocalStatus> {
    let dest = open_existing(dest);
    let parts = open_parts_readonly(state_dir);
    let mut status = LocalStatus {
        files_total: manifest.files.len() as u64,
        files_complete: 0,
        files_partial: 0,
        bytes_on_disk: 0,
        bytes_total: manifest.total_bytes,
    };
    for entry in &manifest.files {
        let intact = match &dest {
            Some(dest) => match probe(dest, &entry.path)? {
                Found::Entry(meta) if meta.kind == Kind::File && meta.len == entry.size => {
                    existing_parent(dest, &entry.path).is_some_and(|(parent, leaf)| {
                        sha256_named(&parent, leaf).is_ok_and(|(_, hash)| hash == entry.sha256)
                    })
                }
                _ => false,
            },
            None => false,
        };
        if intact {
            status.files_complete += 1;
            status.bytes_on_disk += entry.size;
            continue;
        }
        let have = resumable(parts.as_ref(), entry);
        if have > 0 {
            status.files_partial += 1;
            status.bytes_on_disk += have;
        }
    }
    Ok(status)
}

impl Run<'_> {
    fn go(&mut self) -> Result<Progress> {
        self.progress.state = State::Running;
        (self.observe)(&self.progress);
        let Some(manifest) = load_manifest(self.source, self.app_id, self.options, self.stop)?
        else {
            return Ok(self.stopped());
        };
        check_manifest_clear_of_saves(
            Path::new(&self.dirs.dest_id),
            &self.options.protected,
            &manifest,
        )?;
        self.progress.files_total = manifest.files.len() as u64;
        self.progress.bytes_total = manifest.total_bytes;
        let service = self.source.service();
        let layout = self.options.layout.key();
        let mut reset_all = false;
        if let Some(old) = read_session(&self.dirs.state) {
            if old.app_id != self.app_id {
                return Err(Error::DestinationConflict(format!(
                    "this state folder belongs to another game ({})",
                    old.app_id
                )));
            }
            // Another service's identities (revisions) mean nothing here. Nor do partials
            // fetched from another remote location, or staged for another destination.
            reset_all =
                old.service != service || old.layout != layout || old.dest != self.dirs.dest_id;
        }
        self.sweep(&manifest, reset_all);
        let session = Session {
            version: 1,
            app_id: self.app_id.to_string(),
            service: service.to_string(),
            layout,
            dest: self.dirs.dest_id.clone(),
            manifest_sha256: manifest.digest.clone(),
            ready: false,
        };
        write_json(&self.dirs.state, "session.json", &session)?;
        self.notify(0);
        for entry in &manifest.files {
            if self.stop.load(Ordering::SeqCst) || !self.install(entry)? {
                return Ok(self.stopped());
            }
        }
        self.boundary(Boundary::BeforeReady)?;
        write_json(
            &self.dirs.state,
            "session.json",
            &Session {
                ready: true,
                ..session
            },
        )?;
        self.progress.state = State::Complete;
        self.progress.current = None;
        self.notify(0);
        Ok(self.progress.clone())
    }

    fn stopped(&mut self) -> Progress {
        self.progress.state = State::Cancelled;
        self.progress.clone()
    }
}
