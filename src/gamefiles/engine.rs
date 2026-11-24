//! The download engine (docs/assets.md "Destination, checkpoint and publication"): plan what is
//! missing, then fetch it chunk by chunk into app-private `.part` files, checkpointing after each
//! chunk so a kill at any point loses at most one chunk, verify each finished file against the
//! manifest's size and SHA-256, and publish it with an atomic no-replace step.
//!
//! State directory (`state_dir`, private to this library):
//!
//! ```text
//! lock             held (OS file lock) for the whole run; one writer per state directory
//! session.json     service, app id, manifest digest, `ready` once every file is published
//! parts/<id>.part  a file's bytes so far (<id> = the first 32 hex of SHA-256 of its path)
//! parts/<id>.json  size, SHA-256, service identity (Dropbox rev) and the last durable offset
//! ```
//!
//! The `.part` bytes beyond the checkpoint's durable offset are never trusted: a restart cuts
//! them off. A file is fetched only when it is not already in the destination; a file that is
//! there must hash to the manifest's SHA-256 (it is then adopted) or the run stops with
//! `destination_conflict`, and nothing in the destination is ever overwritten.
//!
//! Known limits (docs/assets.md lists the full design): destination paths are checked with
//! `symlink_metadata` before use rather than opened through directory handles, so a hostile
//! process that can already write inside the app-private folders could race it; every file is
//! fetched one at a time; updating a file this library installed earlier is a conflict.

use super::manifest::{self, Entry, Manifest};
use super::{
    remote_file_path, remote_manifest_path, GameSource, Plan, Progress, RangeAnswer, State,
    DEFAULT_CHUNK, MANIFEST_MAX_BYTES, MAX_CHUNK,
};
use crate::error::{Class, Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How a run behaves; `Options::default()` is what the C interface uses.
#[derive(Clone)]
pub struct Options {
    /// Bytes asked for per request, 1 to `MAX_CHUNK`.
    pub chunk: u64,
    /// Retries of one request after a retryable failure (waits 1 s, 2 s, 4 s, ... up to 30 s).
    /// A request that succeeds starts the count again, so a long file survives many drops.
    pub retries: u32,
    /// How to wait; tests pass a no-op.
    pub sleep: Arc<dyn Fn(Duration) + Send + Sync>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            chunk: DEFAULT_CHUNK,
            retries: 5,
            sleep: Arc::new(std::thread::sleep),
        }
    }
}

struct Dirs {
    dest: PathBuf,
    state: PathBuf,
    parts: PathBuf,
}

fn io(what: &str, path: &Path, error: std::io::Error) -> Error {
    Error::Io(format!("{what} {}: {error}", path.display()))
}

/// Create both directories, and refuse a pair where one holds the other.
fn prepare(dest: &Path, state: &Path) -> Result<Dirs> {
    for dir in [dest, state] {
        fs::create_dir_all(dir).map_err(|e| io("cannot create", dir, e))?;
    }
    let dest = fs::canonicalize(dest).map_err(|e| io("cannot resolve", dest, e))?;
    let state = fs::canonicalize(state).map_err(|e| io("cannot resolve", state, e))?;
    if dest.starts_with(&state) || state.starts_with(&dest) {
        return Err(Error::DestinationConflict(
            "the game folder and the state folder must be separate folders, neither inside the other"
                .into(),
        ));
    }
    let parts = state.join("parts");
    fs::create_dir_all(&parts).map_err(|e| io("cannot create", &parts, e))?;
    Ok(Dirs { dest, state, parts })
}

/// The OS-held lock that makes one run the only writer of a state folder. A crash releases it.
struct Lock(#[allow(dead_code)] File);

fn acquire(state: &Path) -> Result<Lock> {
    let path = state.join("lock");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| io("cannot open", &path, e))?;
    match file.try_lock() {
        Ok(()) => Ok(Lock(file)),
        Err(TryLockError::WouldBlock) => Err(Error::Busy(
            "another download is using this game folder".into(),
        )),
        Err(TryLockError::Error(e)) => Err(io("cannot lock", &path, e)),
    }
}

#[derive(Serialize, Deserialize)]
struct Session {
    version: u32,
    app_id: String,
    service: String,
    manifest_sha256: String,
    ready: bool,
}

fn read_session(state: &Path) -> Option<Session> {
    let bytes = fs::read(state.join("session.json")).ok()?;
    serde_json::from_slice(&bytes)
        .ok()
        .filter(|s: &Session| s.version == 1)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|e| Error::Io(e.to_string()))?;
    craft_atomic::atomic_write(path, &bytes).map_err(|e| io("cannot write", path, e))
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

fn read_partial(parts: &Path, path: &str) -> Option<Partial> {
    let bytes = fs::read(parts.join(format!("{}.json", part_id(path)))).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// SHA-256 and length of a file, streamed.
fn sha256_file(path: &Path) -> std::io::Result<(u64, String)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut length = 0u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        length += n as u64;
    }
    Ok((length, manifest::hex(&hasher.finalize())))
}

fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

fn final_path(dest: &Path, rel: &str) -> PathBuf {
    let mut path = dest.to_path_buf();
    path.extend(rel.split('/'));
    path
}

/// Create the folders above `rel` inside `dest`, refusing a link or a file where a folder
/// belongs.
fn ensure_parents(dest: &Path, rel: &str) -> Result<()> {
    let mut current = dest.to_path_buf();
    let segments: Vec<&str> = rel.split('/').collect();
    for segment in &segments[..segments.len() - 1] {
        current.push(segment);
        match fs::symlink_metadata(&current) {
            Ok(md) if md.file_type().is_dir() => {}
            Ok(_) => {
                return Err(Error::DestinationConflict(format!(
                    "{} is in the way of {rel}",
                    current.display()
                )))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|e| io("cannot create", &current, e))?
            }
            Err(e) => return Err(io("cannot read", &current, e)),
        }
    }
    Ok(())
}

fn conflict(rel: &str, why: &str) -> Error {
    Error::DestinationConflict(format!("{rel}: {why}; it is left as it is"))
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
                (options.sleep)(backoff(attempt));
                if stop.load(Ordering::SeqCst) {
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
    let path = remote_manifest_path(app_id);
    let fetched = retrying(options, stop, || {
        source
            .read_bounded(&path, MANIFEST_MAX_BYTES)
            .map_err(|error| match error {
                Error::NotFound(_) => Error::NotYetUploaded(format!(
                    "the manifest for {app_id} is not in the service folder"
                )),
                other => other,
            })
    })?;
    match fetched {
        Some(bytes) => manifest::parse(&bytes, app_id).map(Some),
        None => Ok(None),
    }
}

/// What a run would do, from the manifest on the service and the sizes of what is on disk.
pub fn plan(
    source: &dyn GameSource,
    app_id: &str,
    dest: &Path,
    state_dir: &Path,
    options: &Options,
) -> Result<Plan> {
    let never = AtomicBool::new(false);
    let manifest = load_manifest(source, app_id, options, &never)?
        .ok_or_else(|| Error::Io("stopped".into()))?;
    let parts = state_dir.join("parts");
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
        match fs::symlink_metadata(final_path(dest, &entry.path)) {
            Ok(md) if md.file_type().is_file() && md.len() == entry.size => {
                plan.files_present += 1;
                plan.bytes_present += entry.size;
                continue;
            }
            Ok(_) => {
                note_conflict(&mut plan, &entry.path);
                continue;
            }
            Err(_) => {}
        }
        let have = read_partial(&parts, &entry.path)
            .filter(|p| p.matches(entry) && p.durable <= entry.size)
            .filter(|p| {
                fs::metadata(parts.join(format!("{}.part", part_id(&entry.path))))
                    .is_ok_and(|md| md.len() >= p.durable)
            })
            .map_or(0, |p| p.durable);
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
/// anything in the game folder. The next run starts every unfinished file again.
pub fn cleanup(state_dir: &Path) -> Result<()> {
    if !state_dir.exists() {
        return Ok(());
    }
    let _lock = acquire(state_dir)?;
    for name in ["parts", "session.json"] {
        let path = state_dir.join(name);
        let removed = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        match removed {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io("cannot remove", &path, e)),
        }
    }
    Ok(())
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
}

impl Run<'_> {
    fn notify(&mut self, in_flight: u64) {
        self.progress.bytes_done = self.done_bytes + in_flight;
        (self.observe)(&self.progress);
    }

    fn part_paths(&self, rel: &str) -> (PathBuf, PathBuf) {
        let id = part_id(rel);
        (
            self.dirs.parts.join(format!("{id}.part")),
            self.dirs.parts.join(format!("{id}.json")),
        )
    }

    fn drop_partial(&self, rel: &str) {
        let (part, json) = self.part_paths(rel);
        let _ = fs::remove_file(part);
        let _ = fs::remove_file(json);
    }

    fn finish(&mut self, entry: &Entry) {
        self.drop_partial(&entry.path);
        self.done_bytes += entry.size;
        self.progress.files_done += 1;
        self.notify(0);
    }

    /// Delete `.part` files and checkpoints that do not belong to this manifest, this service
    /// or a still-matching file.
    fn sweep(&self, manifest: &Manifest, reset_all: bool) {
        let wanted: HashMap<String, &Entry> = manifest
            .files
            .iter()
            .map(|e| (part_id(&e.path), e))
            .collect();
        let Ok(listing) = fs::read_dir(&self.dirs.parts) else {
            return;
        };
        let mut keep = std::collections::HashSet::new();
        let names: Vec<String> = listing
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        for name in &names {
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            let kept = !reset_all
                && wanted.get(id).is_some_and(|entry| {
                    fs::read(self.dirs.parts.join(name))
                        .ok()
                        .and_then(|b| serde_json::from_slice::<Partial>(&b).ok())
                        .is_some_and(|p| p.matches(entry))
                });
            if kept {
                keep.insert(format!("{id}.json"));
                keep.insert(format!("{id}.part"));
            }
        }
        for name in names {
            if !keep.contains(&name) {
                let _ = fs::remove_file(self.dirs.parts.join(name));
            }
        }
    }

    fn fetch_chunk(&mut self, entry: &Entry, start: u64, len: u64) -> Result<Option<RangeAnswer>> {
        let path = remote_file_path(self.app_id, &entry.path);
        let source = self.source;
        retrying(self.options, self.stop, || {
            let answer = source
                .fetch(&path, start, len)
                .map_err(|error| match error {
                    Error::NotFound(_) => Error::NotYetUploaded(format!(
                        "{} is not in the service folder",
                        entry.path
                    )),
                    other => other,
                })?;
            validate(answer, start, len, entry.size)
        })
    }

    /// Settle the `.part` against its checkpoint and open it for writing at `partial.durable`.
    fn open_part(&self, part: &Path, partial: &mut Partial) -> Result<File> {
        let usable = match fs::symlink_metadata(part) {
            Ok(md) if md.file_type().is_file() => {
                md.len() >= partial.durable && partial.durable <= partial.size
            }
            Ok(_) => {
                // A link or folder where our file belongs: it is ours to remove, not to follow.
                let _ = fs::remove_file(part).or_else(|_| fs::remove_dir_all(part));
                false
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(io("cannot read", part, e)),
        };
        if !usable {
            partial.durable = 0;
            partial.rev = None;
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(part)
            .map_err(|e| io("cannot open", part, e))?;
        // Bytes past the durable offset may be torn or unsynced: cut them off.
        file.set_len(partial.durable)
            .map_err(|e| io("cannot resize", part, e))?;
        Ok(file)
    }

    fn download(&mut self, entry: &Entry) -> Result<Fetched> {
        let (part, json) = self.part_paths(&entry.path);
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
            let Some(answer) = self.fetch_chunk(entry, partial.durable, len)? else {
                return Ok(Fetched::Stopped);
            };
            if let Some(identity) = &answer.identity {
                match &partial.rev {
                    Some(old) if old != identity => {
                        drop(file);
                        self.drop_partial(&entry.path);
                        return Err(Error::RemoteChanged(format!(
                            "{} is a different version now; run again to fetch it from the start",
                            entry.path
                        )));
                    }
                    _ => partial.rev = Some(identity.clone()),
                }
            }
            file.seek(SeekFrom::Start(partial.durable))
                .and_then(|_| file.write_all(&answer.body))
                .and_then(|_| file.sync_data())
                .map_err(|e| io("cannot write", &part, e))?;
            partial.durable += answer.body.len() as u64;
            if partial.durable < entry.size {
                write_json(&json, &partial)?;
            }
            self.notify(partial.durable);
        }
        file.sync_all().map_err(|e| io("cannot sync", &part, e))?;
        drop(file);
        let intact = sha256_file(&part)
            .map_err(|e| io("cannot read", &part, e))
            .map(|(length, hash)| length == entry.size && hash == entry.sha256)?;
        if intact {
            Ok(Fetched::Verified)
        } else {
            self.drop_partial(&entry.path);
            Ok(Fetched::Corrupt(resumed))
        }
    }

    /// Make `entry` present and verified. `false` when stopped on request.
    fn install(&mut self, entry: &Entry) -> Result<bool> {
        let target = final_path(&self.dirs.dest, &entry.path);
        ensure_parents(&self.dirs.dest, &entry.path)?;
        self.progress.current = Some(entry.path.clone());
        match fs::symlink_metadata(&target) {
            Ok(md) if !md.file_type().is_file() => {
                return Err(conflict(
                    &entry.path,
                    "something that is not a file is there",
                ))
            }
            Ok(md) if md.len() != entry.size => {
                return Err(conflict(&entry.path, "a file of another size is there"))
            }
            Ok(_) => {
                self.notify(0);
                let (_, hash) = sha256_file(&target).map_err(|e| io("cannot read", &target, e))?;
                if hash != entry.sha256 {
                    return Err(conflict(&entry.path, "a different file is there"));
                }
                self.finish(entry);
                return Ok(true);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io("cannot read", &target, e)),
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
        self.publish(entry, &target)?;
        self.finish(entry);
        Ok(true)
    }

    /// Move the verified part to its final name without replacing anything: a hard link
    /// fails if the name exists; where links are unavailable, a rename after a check.
    fn publish(&self, entry: &Entry, target: &Path) -> Result<()> {
        let (part, _) = self.part_paths(&entry.path);
        match fs::hard_link(&part, target) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let same = sha256_file(target)
                    .is_ok_and(|(length, hash)| length == entry.size && hash == entry.sha256);
                if !same {
                    return Err(conflict(&entry.path, "a different file appeared meanwhile"));
                }
            }
            Err(_) => {
                if fs::symlink_metadata(target).is_ok() {
                    return Err(conflict(&entry.path, "a file appeared meanwhile"));
                }
                fs::rename(&part, target).map_err(|e| io("cannot move into place", target, e))?;
            }
        }
        if let Some(parent) = target.parent() {
            sync_dir(parent);
        }
        Ok(())
    }
}

/// Fetch every file of `app_id` that is not yet in `dest`, resuming from `state_dir`.
///
/// Returns the final `Progress`: `Complete`, or `Cancelled` when `stop` was set (checked
/// between chunks, so the stop lands after a durable checkpoint and a later run continues).
/// Any other end is an `Err`; the checkpoint stays for the next run, and the observer is told
/// `State::Error` first. `observe` is called after every chunk and file.
pub fn run(
    source: &dyn GameSource,
    app_id: &str,
    dest: &Path,
    state_dir: &Path,
    options: &Options,
    stop: &AtomicBool,
    observe: &mut dyn FnMut(&Progress),
) -> Result<Progress> {
    manifest::check_app_id(app_id)?;
    let mut options = options.clone();
    options.chunk = options.chunk.clamp(1, MAX_CHUNK);
    let dirs = prepare(dest, state_dir)?;
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
    };
    let outcome = run.go();
    match &outcome {
        Ok(progress) => (run.observe)(progress),
        Err(_) => {
            run.progress.state = State::Error;
            (run.observe)(&run.progress);
        }
    }
    outcome
}

impl Run<'_> {
    fn go(&mut self) -> Result<Progress> {
        self.progress.state = State::Running;
        (self.observe)(&self.progress);
        let Some(manifest) = load_manifest(self.source, self.app_id, self.options, self.stop)?
        else {
            return Ok(self.stopped());
        };
        self.progress.files_total = manifest.files.len() as u64;
        self.progress.bytes_total = manifest.total_bytes;
        let service = self.source.service();
        let mut reset_all = false;
        if let Some(old) = read_session(&self.dirs.state) {
            if old.app_id != self.app_id {
                return Err(Error::DestinationConflict(format!(
                    "this state folder belongs to another game ({})",
                    old.app_id
                )));
            }
            // Another service's identities (revisions) mean nothing here.
            reset_all = old.service != service;
        }
        self.sweep(&manifest, reset_all);
        let session = Session {
            version: 1,
            app_id: self.app_id.to_string(),
            service: service.to_string(),
            manifest_sha256: manifest.digest.clone(),
            ready: false,
        };
        write_json(&self.dirs.state.join("session.json"), &session)?;
        self.notify(0);
        for entry in &manifest.files {
            if self.stop.load(Ordering::SeqCst) || !self.install(entry)? {
                return Ok(self.stopped());
            }
        }
        write_json(
            &self.dirs.state.join("session.json"),
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
