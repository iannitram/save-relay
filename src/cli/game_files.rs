//! `save-relay game-files plan|download|cleanup` (#310): the installer engine (`gamefiles`) from
//! the command line, over the service and credential store this machine is set up with
//! (`service::open_game_source`). The envelopes are the C interface's (`game-files-plan`,
//! `game-files-download`) plus `game-files-cleanup`, and failures carry `error.class`.
//!
//! The operator names the folders: `--dest` (the game's files) and `--state` (the library's
//! checkpoints). Neither may be, contain or sit inside a saves folder this machine syncs, this
//! program's own config and data folders, or the other. A remote manifest never chooses a root:
//! `--legacy-root` is an explicit, relative, validated operator argument.

use super::{outcome, Command, Context, Outcome};
use crate::error::{Error, Result};
use crate::gamefiles::engine::{self, Options};
use crate::gamefiles::{self, manifest, Layout, Progress, State, DEFAULT_CHUNK, MAX_CHUNK};
use crate::local::LocalStore;
use crate::platform::AppDirs;
use crate::service::{self, Service};
use serde::Serialize;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The exit code of a download that stopped on Ctrl-C after a durable checkpoint: nothing is
/// wrong, nothing is finished; run the same command again to continue.
pub const EXIT_CANCELLED: i32 = 7;

/// The most retries `--retries` accepts.
const MAX_RETRIES: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameFilesCommand {
    /// `game-files plan <app-id> --dest <dir> --state <dir> [--legacy-root <root>]`
    Plan {
        app_id: String,
        dest: PathBuf,
        state: PathBuf,
        legacy_root: Option<String>,
    },
    /// `game-files download <app-id> --dest <dir> --state <dir> [--chunk <bytes>]
    /// [--retries <n>] [--legacy-root <root>]`
    Download {
        app_id: String,
        dest: PathBuf,
        state: PathBuf,
        chunk: Option<u64>,
        retries: Option<u32>,
        legacy_root: Option<String>,
    },
    /// `game-files cleanup --state <dir>`
    Cleanup { state: PathBuf },
}

impl GameFilesCommand {
    /// The envelope's command name (the C interface's names).
    pub fn name(&self) -> &'static str {
        match self {
            GameFilesCommand::Plan { .. } => "game-files-plan",
            GameFilesCommand::Download { .. } => "game-files-download",
            GameFilesCommand::Cleanup { .. } => "game-files-cleanup",
        }
    }
}

/// The envelope name for arguments that did not parse: `game-files-<sub>` when the second word
/// is a known subcommand, else `game-files`.
pub(super) fn guess_name(args: &[String]) -> String {
    let mut words = args.iter().filter(|a| a.as_str() != "--json");
    words.next();
    match words.next().map(String::as_str) {
        Some(sub @ ("plan" | "download" | "cleanup")) => format!("game-files-{sub}"),
        _ => "game-files".to_string(),
    }
}

/// Parse the words after `game-files`.
pub(super) fn parse(args: &[&str]) -> Result<GameFilesCommand> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(Error::Usage(
            "game-files needs plan, download or cleanup".into(),
        ));
    };
    match *sub {
        "plan" => {
            let flags = super::split(rest, &["--dest", "--state", "--legacy-root"], &[])?;
            super::require(&flags.positionals, 1, &["<app-id>"])?;
            Ok(GameFilesCommand::Plan {
                app_id: app_id(flags.positionals[0])?,
                dest: folder(&flags, "--dest")?,
                state: folder(&flags, "--state")?,
                legacy_root: legacy_root(&flags)?,
            })
        }
        "download" => {
            let flags = super::split(
                rest,
                &["--dest", "--state", "--chunk", "--retries", "--legacy-root"],
                &[],
            )?;
            super::require(&flags.positionals, 1, &["<app-id>"])?;
            let chunk = match flags.flag("--chunk") {
                None => None,
                Some(text) => match text.parse::<u64>() {
                    Ok(n) if (1..=MAX_CHUNK).contains(&n) => Some(n),
                    _ => {
                        return Err(Error::Usage(format!(
                            "--chunk must be a whole number of bytes from 1 to {MAX_CHUNK}"
                        )))
                    }
                },
            };
            let retries = match flags.flag("--retries") {
                None => None,
                Some(text) => match text.parse::<u32>() {
                    Ok(n) if n <= MAX_RETRIES => Some(n),
                    _ => {
                        return Err(Error::Usage(format!(
                            "--retries must be a whole number from 0 to {MAX_RETRIES}"
                        )))
                    }
                },
            };
            Ok(GameFilesCommand::Download {
                app_id: app_id(flags.positionals[0])?,
                dest: folder(&flags, "--dest")?,
                state: folder(&flags, "--state")?,
                chunk,
                retries,
                legacy_root: legacy_root(&flags)?,
            })
        }
        "cleanup" => {
            let flags = super::split(rest, &["--state"], &[])?;
            super::require(&flags.positionals, 0, &[])?;
            Ok(GameFilesCommand::Cleanup {
                state: folder(&flags, "--state")?,
            })
        }
        other => Err(Error::Usage(format!(
            "unknown game-files command '{other}': use plan, download or cleanup"
        ))),
    }
}

fn app_id(text: &str) -> Result<String> {
    manifest::check_app_id(text).map_err(|e| Error::Usage(format!("<app-id>: {e}")))?;
    Ok(text.to_string())
}

fn folder(flags: &super::Flags<'_>, name: &str) -> Result<PathBuf> {
    match flags.flag(name) {
        None => super::missing(name),
        Some("") => Err(Error::Usage(format!("{name} must not be empty"))),
        Some(path) if path.contains('\0') => {
            Err(Error::Usage(format!("{name} must not contain NUL")))
        }
        Some(path) => Ok(PathBuf::from(path)),
    }
}

/// `--legacy-root`: explicit, relative and valid by the same rules as a payload path. It names
/// where an owner staged an old app's files (`acnh-universal-game/android`); only the operator
/// passes it, never a manifest.
fn legacy_root(flags: &super::Flags<'_>) -> Result<Option<String>> {
    match flags.flag("--legacy-root") {
        None => Ok(None),
        Some(root) => {
            manifest::check_remote_root(root)?;
            Ok(Some(root.to_string()))
        }
    }
}

// ---------------------------------------------------------------------------------------
// Folder refusals
// ---------------------------------------------------------------------------------------

/// `path` made absolute and free of `.` and `..`, with the part that exists resolved through
/// links (so a link into a saves folder is seen), and the rest appended as written. A `..` is
/// applied to the folder the links before it lead to (`link/..` is the parent of the link's
/// target, as the file system reads it), never to the name written before it.
fn resolve(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)
        .map_err(|e| Error::Usage(format!("cannot use {}: {e}", path.display())))?;
    let mut normal = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::ParentDir => {
                if let Ok(real) = std::fs::canonicalize(&normal) {
                    normal = real;
                }
                normal.pop();
            }
            Component::CurDir => {}
            other => normal.push(other.as_os_str()),
        }
    }
    let mut head = normal.clone();
    let mut tail = Vec::new();
    let mut base = loop {
        match std::fs::canonicalize(&head) {
            Ok(resolved) => break resolved,
            Err(_) => match head.file_name().map(|n| n.to_owned()) {
                Some(name) => {
                    tail.push(name);
                    head.pop();
                }
                None => break head,
            },
        }
    };
    for name in tail.iter().rev() {
        base.push(name);
    }
    Ok(base)
}

/// The components of a resolved path, case-folded where the file system usually is.
fn key(path: &Path) -> Vec<String> {
    path.components()
        .map(|c| {
            let text = c.as_os_str().to_string_lossy().into_owned();
            if cfg!(any(
                windows,
                target_os = "macos",
                target_os = "ios",
                target_os = "android"
            )) {
                text.to_lowercase()
            } else {
                text
            }
        })
        .collect()
}

/// True when one path is the other, or inside it.
fn overlaps(a: &Path, b: &Path) -> bool {
    let (a, b) = (key(a), key(b));
    a.starts_with(&b) || b.starts_with(&a)
}

/// The folders this machine syncs saves from (`gamefiles::saves_folders`, fail closed), and
/// its own config and data folders.
fn protected(store: &LocalStore, cx: &Context) -> Result<Vec<(String, PathBuf, bool)>> {
    let dirs = AppDirs::from_vars(cx.vars);
    // The flag says whether a game folder may hold it: a saves folder may sit inside one (a
    // port keeps `user/` in its install folder; the engine then checks the manifest), this
    // program's own folders may not.
    let mut found: Vec<(String, PathBuf, bool)> = vec![
        ("this program's config folder".into(), dirs.config, false),
        ("this program's data folder".into(), dirs.data, false),
    ];
    found.extend(
        gamefiles::saves_folders(store, cx.platform, cx.vars)?
            .into_iter()
            .map(|(what, path)| (what, path, true)),
    );
    Ok(found)
}

/// Refuse a `--state` that is, holds or sits inside a protected folder, and a `--dest` that is or
/// sits inside one (a saves folder may sit inside `--dest`: the engine then checks the manifest);
/// this program's own folders may not be held by `--dest` either. And a
/// `dest` and `state` that overlap each other. An early refusal before any request: the engine
/// judges the same folders again (`Options::protected`, returned here for it) with the same
/// `destination_conflict` code.
fn refuse_overlaps(
    dest: Option<&Path>,
    state: &Path,
    store: &LocalStore,
    cx: &Context,
) -> Result<Vec<PathBuf>> {
    let state = resolve(state)?;
    let dest = dest.map(resolve).transpose()?;
    let guarded = protected(store, cx)?;
    let folders = guarded.iter().map(|(_, path, _)| path.clone()).collect();
    for (what, saves, may_be_held) in guarded {
        let saves = resolve(&saves)?;
        if overlaps(&state, &saves) {
            return Err(Error::DestinationConflict(format!(
                "--state overlaps {what}; use a folder of its own"
            )));
        }
        let refused = dest.as_deref().is_some_and(|d| {
            if may_be_held {
                key(d).starts_with(&key(&saves))
            } else {
                overlaps(d, &saves)
            }
        });
        if refused {
            return Err(Error::DestinationConflict(format!(
                "--dest overlaps {what}; use a folder of its own"
            )));
        }
    }
    if dest.as_deref().is_some_and(|d| overlaps(d, &state)) {
        return Err(Error::DestinationConflict(
            "--dest and --state must be separate folders, neither inside the other".into(),
        ));
    }
    if let Some(dest) = dest.as_deref() {
        if !same_volume(dest, &state) {
            return Err(Error::DestinationConflict(
                "--dest and --state must be on the same drive or volume: a finished file is moved from the state folder into place"
                    .into(),
            ));
        }
    }
    Ok(folders)
}

/// Whether two resolved paths sit on one volume, judged by their nearest existing folders
/// (device id on unix, drive or share elsewhere). Unknown counts as the same.
fn same_volume(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fn device(path: &Path) -> Option<u64> {
            path.ancestors()
                .find_map(|p| std::fs::metadata(p).ok())
                .map(|m| m.dev())
        }
        match (device(a), device(b)) {
            (Some(x), Some(y)) => x == y,
            _ => true,
        }
    }
    #[cfg(not(unix))]
    {
        use std::path::Prefix;
        fn drive(path: &Path) -> Option<String> {
            match path.components().next()? {
                Component::Prefix(p) => Some(match p.kind() {
                    Prefix::Disk(d) | Prefix::VerbatimDisk(d) => {
                        (d as char).to_ascii_lowercase().to_string()
                    }
                    _ => p.as_os_str().to_string_lossy().to_lowercase(),
                }),
                _ => None,
            }
        }
        match (drive(a), drive(b)) {
            (Some(x), Some(y)) => x == y,
            _ => true,
        }
    }
}

/// A state folder holds only what the engine writes there. Refuse to run or clean in a folder
/// with anything else (the game folder, a home folder), so nothing foreign is ever swept or
/// deleted. The names are the engine's own (`engine::is_state_name`, `engine::is_part_name`:
/// the ownership marker, `lock`, `session.json`, `parts/<32 hex>.part` and `.json`, and an
/// interrupted atomic write's `.tmp`), so this early refusal never contradicts the engine's
/// ownership rules; it is stricter only in kind: `parts` must be a real folder (not a link or
/// junction) and every other name a plain file. The engine judges ownership again (marker or
/// adoption) with the same `destination_conflict` code. A missing folder is fine.
fn refuse_foreign_state(state: &Path) -> Result<()> {
    let entries = match std::fs::read_dir(state) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Error::Io(format!("cannot read --state: {e}"))),
    };
    for entry in entries {
        let entry = entry.map_err(|e| Error::Io(format!("cannot read --state: {e}")))?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(foreign_state)?;
        let kind = std::fs::symlink_metadata(entry.path())
            .map_err(|e| Error::Io(format!("cannot read --state: {e}")))?;
        if name == "parts" {
            if !kind.is_dir() || is_link(&kind) {
                return Err(foreign_state());
            }
            refuse_foreign_parts(&entry.path())?;
        } else if !engine::is_state_name(name) || !kind.is_file() || is_link(&kind) {
            return Err(foreign_state());
        }
    }
    Ok(())
}

fn foreign_state() -> Error {
    Error::DestinationConflict(
        "--state holds files that are not game-file checkpoints; use an empty folder of its own"
            .into(),
    )
}

fn refuse_foreign_parts(parts: &Path) -> Result<()> {
    let entries =
        std::fs::read_dir(parts).map_err(|e| Error::Io(format!("cannot read --state: {e}")))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::Io(format!("cannot read --state: {e}")))?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(foreign_state)?;
        let kind = std::fs::symlink_metadata(entry.path())
            .map_err(|e| Error::Io(format!("cannot read --state: {e}")))?;
        if !engine::is_part_name(name) || !kind.is_file() || is_link(&kind) {
            return Err(foreign_state());
        }
    }
    Ok(())
}

/// A symbolic link, or a Windows junction or other reparse point.
fn is_link(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_symlink() || is_reparse_point(meta)
}

#[cfg(windows)]
fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_meta: &std::fs::Metadata) -> bool {
    false
}

// ---------------------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------------------

/// Set by Ctrl-C once `install_interrupt` ran; read between chunks. A new download clears it.
static INTERRUPT: AtomicBool = AtomicBool::new(false);

/// Ask the running download to stop after its next durable checkpoint (what Ctrl-C does).
pub fn request_interrupt() {
    INTERRUPT.store(true, Ordering::SeqCst);
}

/// Route Ctrl-C to `request_interrupt`: the first stops after a durable chunk, a second ends
/// the process at once (the checkpoint on disk is still good).
pub fn install_interrupt() {
    // Clear before the handler exists, so a press that lands later is never wiped.
    INTERRUPT.store(false, Ordering::SeqCst);
    HANDLER_INSTALLED.store(true, Ordering::SeqCst);
    interrupt::install();
}

/// True once `install_interrupt` ran: from then on only Ctrl-C sets `INTERRUPT`.
static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
mod interrupt {
    use std::sync::atomic::{AtomicBool, Ordering};
    const SIGINT: i32 = 2;
    static SEEN: AtomicBool = AtomicBool::new(false);
    extern "C" {
        fn signal(sig: i32, handler: usize) -> usize;
        fn _exit(status: i32) -> !;
    }
    extern "C" fn on_signal(_: i32) {
        if SEEN.swap(true, Ordering::SeqCst) {
            // SAFETY: `_exit` is async-signal-safe and never returns.
            unsafe { _exit(130) }
        }
        super::request_interrupt();
    }
    pub fn install() {
        SEEN.store(false, Ordering::SeqCst);
        let handler = on_signal as extern "C" fn(i32) as usize;
        // SAFETY: the handler only touches atomics and calls `_exit`.
        unsafe {
            signal(SIGINT, handler);
        }
    }
}

#[cfg(windows)]
mod interrupt {
    use std::sync::atomic::{AtomicBool, Ordering};
    type Handler = unsafe extern "system" fn(u32) -> i32;
    static SEEN: AtomicBool = AtomicBool::new(false);
    extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<Handler>, add: i32) -> i32;
    }
    unsafe extern "system" fn on_event(event: u32) -> i32 {
        // CTRL_C_EVENT 0 and CTRL_BREAK_EVENT 1; the first is ours, the second ends the process.
        if event > 1 || SEEN.swap(true, Ordering::SeqCst) {
            return 0;
        }
        super::request_interrupt();
        1
    }
    pub fn install() {
        SEEN.store(false, Ordering::SeqCst);
        // SAFETY: `on_event` is a valid console handler that only touches atomics.
        unsafe {
            SetConsoleCtrlHandler(Some(on_event), 1);
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod interrupt {
    pub fn install() {}
}

#[derive(Serialize)]
struct DownloadData {
    #[serde(flatten)]
    progress: Progress,
    /// Files this run fetched and published.
    downloaded_files: u64,
    /// Files already in the destination and verified.
    skipped_files: u64,
    /// Payload bytes this run received (a resumed prefix is not counted).
    bytes_fetched: u64,
}

#[derive(Serialize)]
struct CleanupData {
    cleaned: bool,
}

fn options(
    chunk: Option<u64>,
    retries: Option<u32>,
    legacy_root: &Option<String>,
    protected: Vec<PathBuf>,
) -> Options {
    let defaults = Options::default();
    Options {
        protected,
        chunk: chunk.unwrap_or(DEFAULT_CHUNK),
        retries: retries.unwrap_or(defaults.retries),
        layout: match legacy_root {
            Some(root) => Layout::Legacy { root: root.clone() },
            None => Layout::Standard,
        },
        ..defaults
    }
}

/// Run `work` with the game source of the configured service, then keep a rotated refresh token.
fn with_source<T>(
    cx: &Context,
    work: impl FnOnce(&dyn crate::gamefiles::GameSource) -> Result<T>,
) -> Result<T> {
    let config = super::load_config(cx)?;
    let opened = service::open_game_source(
        &config,
        cx.creds,
        cx.net.transport.clone(),
        cx.net.clock,
        &cx.net.clients,
    )?;
    let result = work(opened.source.as_ref());
    let stored = match opened.tokens.as_ref().and_then(|tokens| tokens.rotated()) {
        Some(rotated) => {
            service::token_account(Service::from_server(&config.server)?, &config.user)
                .and_then(|account| cx.creds.set(&account, &rotated))
        }
        None => Ok(()),
    };
    let value = result?;
    stored?;
    Ok(value)
}

/// Run one `game-files` command. Text mode prints progress lines to `stdout` while a download
/// runs; `--json` prints nothing until the one envelope the caller prints.
pub(super) fn execute(
    command: &GameFilesCommand,
    json: bool,
    stdout: &mut dyn Write,
    cx: &Context,
) -> Result<Outcome> {
    match command {
        GameFilesCommand::Plan {
            app_id,
            dest,
            state,
            legacy_root,
        } => {
            super::load_config(cx)?;
            let protected = refuse_overlaps(Some(dest), state, cx.store, cx)?;
            let plan = with_source(cx, |source| {
                engine::plan(
                    source,
                    app_id,
                    dest,
                    state,
                    &options(None, None, legacy_root, protected),
                )
            })?;
            let text = format!(
                "{}: {} files, {} to fetch ({} already present, {} partly fetched, {} missing{}).",
                plan.app_id,
                plan.files_total,
                bytes(plan.bytes_to_fetch),
                plan.files_present,
                plan.files_partial,
                plan.files_missing,
                if plan.conflict_count > 0 {
                    format!(", {} in the way", plan.conflict_count)
                } else {
                    String::new()
                }
            );
            Ok(outcome("game-files-plan", &plan, text, 0))
        }
        GameFilesCommand::Download {
            app_id,
            dest,
            state,
            chunk,
            retries,
            legacy_root,
        } => {
            super::load_config(cx)?;
            let protected = refuse_overlaps(Some(dest), state, cx.store, cx)?;
            refuse_foreign_state(state)?;
            // The real binary cleared it before installing its handler (`install_interrupt`);
            // an early Ctrl-C must survive until here. Embedded runs start clean.
            if !HANDLER_INSTALLED.load(Ordering::SeqCst) {
                INTERRUPT.store(false, Ordering::SeqCst);
            }
            let mut shown = Shown::default();
            let report = with_source(cx, |source| {
                engine::run_report(
                    source,
                    app_id,
                    dest,
                    state,
                    &options(*chunk, *retries, legacy_root, protected),
                    &INTERRUPT,
                    &mut |progress| {
                        if !json {
                            shown.line(progress, stdout);
                        }
                    },
                )
            })?;
            let (text, code) = match report.progress.state {
                State::Cancelled => (
                    format!(
                        "Stopped at {} of {} files ({} of {}). Run the same command again to continue.",
                        report.progress.files_done,
                        report.progress.files_total,
                        bytes(report.progress.bytes_done),
                        bytes(report.progress.bytes_total),
                    ),
                    EXIT_CANCELLED,
                ),
                _ => (
                    format!(
                        "{}: complete, {} files ({} fetched now, {} already there).",
                        report.progress.app_id,
                        report.progress.files_total,
                        report.downloaded_files,
                        report.skipped_files
                    ),
                    0,
                ),
            };
            let data = DownloadData {
                progress: report.progress,
                downloaded_files: report.downloaded_files,
                skipped_files: report.skipped_files,
                bytes_fetched: report.bytes_fetched,
            };
            Ok(outcome("game-files-download", &data, text, code))
        }
        GameFilesCommand::Cleanup { state } => {
            refuse_overlaps(None, state, cx.store, cx)?;
            refuse_foreign_state(state)?;
            engine::cleanup(state)?;
            Ok(outcome(
                "game-files-cleanup",
                &CleanupData { cleaned: true },
                "Removed the partial downloads and checkpoints. Downloaded game files were kept."
                    .into(),
                0,
            ))
        }
    }
}

/// Throttles the text progress: a line when a file finishes, the file in flight changes, or
/// five more percent of the bytes are in.
#[derive(Default)]
struct Shown {
    files: Option<u64>,
    current: Option<String>,
    step: u64,
}

impl Shown {
    fn line(&mut self, progress: &Progress, stdout: &mut dyn Write) {
        if progress.state != State::Running {
            return;
        }
        let step = progress
            .bytes_done
            .saturating_mul(20)
            .checked_div(progress.bytes_total)
            .unwrap_or(0);
        if self.files == Some(progress.files_done)
            && self.current == progress.current
            && self.step == step
        {
            return;
        }
        self.files = Some(progress.files_done);
        self.current = progress.current.clone();
        self.step = step;
        let _ = writeln!(
            stdout,
            "{}: {}/{} files, {} of {}{}",
            progress.app_id,
            progress.files_done,
            progress.files_total,
            bytes(progress.bytes_done),
            bytes(progress.bytes_total),
            progress
                .current
                .as_deref()
                .map(|c| format!(" ({c})"))
                .unwrap_or_default(),
        );
    }
}

/// `1.5 MiB` style, for people.
fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

impl From<GameFilesCommand> for Command {
    fn from(command: GameFilesCommand) -> Command {
        Command::GameFiles(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ctrl_c_before_install_is_cleared_and_one_after_survives() {
        request_interrupt();
        install_interrupt();
        assert!(!INTERRUPT.load(Ordering::SeqCst), "a stale stop is cleared");
        request_interrupt();
        assert!(
            INTERRUPT.load(Ordering::SeqCst),
            "a press after install stays"
        );
        // Later runs of this test process are embedded runs again.
        INTERRUPT.store(false, Ordering::SeqCst);
        HANDLER_INSTALLED.store(false, Ordering::SeqCst);
    }

    #[test]
    fn engine_names_are_recognised_and_nothing_else() {
        let id = "0123456789abcdef0123456789abcdef";
        for name in [
            ".save-relay-state",
            "lock",
            "session.json",
            "session.json.tmp",
        ] {
            assert!(engine::is_state_name(name), "{name}");
        }
        assert!(!engine::is_state_name("notes.txt"));
        assert!(!engine::is_state_name(".session.json.12-3.craft_tmp"));
        for ext in ["part", "json", "json.tmp"] {
            assert!(engine::is_part_name(&format!("{id}.{ext}")), "{ext}");
        }
        assert!(!engine::is_part_name(
            "0123456789ABCDEF0123456789abcdef.part"
        ));
        assert!(!engine::is_part_name("notes"));
        assert!(!engine::is_part_name(&format!(".{id}.json.12-3.craft_tmp")));
    }

    #[cfg(windows)]
    #[test]
    fn different_drives_are_different_volumes() {
        assert!(same_volume(Path::new(r"C:\a"), Path::new(r"c:\b\c")));
        assert!(!same_volume(Path::new(r"C:\a"), Path::new(r"D:\a")));
    }
}
