//! `save-relay install-wrap` (SPEC.md M13): put `save-relay wrap -- %command%` in the launch
//! options of the Steam games picked, in every Steam user's
//! `<root>/userdata/<id>/config/localconfig.vdf`
//! (`UserLocalConfigStore/Software/Valve/Steam/apps/<app id>/LaunchOptions`).
//!
//! Steam must not be running (it rewrites the file when it quits): refused with
//! `Error::SteamRunning`, and Steam is never closed for the person. A dated backup goes next to
//! the file before the first change. An existing launch option is kept by wrapping it, the
//! change is idempotent, and `--remove` undoes only save-relay's part. The text rules are pure.

use crate::api::{InstallWrapApp, InstallWrapData};
use crate::error::{Error, Result};
use crate::platform::Platform;
use crate::stamp;
use crate::steam;
use crate::steam::vdf::{self, Vdf};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The words that mark save-relay's part of a launch option.
pub const MARKER: &str = "wrap --";

/// Steam's placeholder for the game's own command line.
const COMMAND: &str = "%command%";

/// Where Steam keeps its process id on Linux, relative to the home folder: the classic folder,
/// the XDG one, and the Flatpak one. Any of them naming a live process means Steam runs.
const LINUX_PID_FILES: [&str; 3] = [
    ".steam/steam.pid",
    ".local/share/Steam/steam.pid",
    ".var/app/com.valvesoftware.Steam/.local/share/Steam/steam.pid",
];

/// Steam's process id on Mac, relative to the home folder (unverified on a real Mac: see the
/// steam_running doc).
const MAC_PID_FILE: &str = "Library/Application Support/Steam/steam.pid";

/// The launch option with save-relay around it. `binary` is the save-relay program's path,
/// in double quotes when it holds a space or on Windows. `current` empty or `None`:
/// `<binary> wrap -- %command%`. `current` holding `%command%` (`gamemoderun %command% -fps`):
/// `%command%` becomes `<binary> wrap -- %command%`, the rest kept. `current` with no
/// `%command%` (game arguments only, `-novid`): `<binary> wrap -- %command% -novid`. Already
/// wrapped (`is_wrapped`): unchanged.
pub fn wrapped(current: Option<&str>, binary: &str, platform: Platform) -> String {
    if let Some(current) = current {
        if is_wrapped(current) {
            return current.to_string();
        }
    }
    let invocation = format!("{} {MARKER} {COMMAND}", program(binary, platform));
    let base = match current {
        Some(current) if current.contains(COMMAND) => current.to_string(),
        Some(current) if !current.is_empty() => format!("{COMMAND} {current}"),
        _ => COMMAND.to_string(),
    };
    base.replacen(COMMAND, &invocation, 1)
}

/// True when `current` has save-relay's part: a program path ending in `save-relay` or
/// `save-relay.exe` (quoted or not) followed by `MARKER %command%`.
pub fn is_wrapped(current: &str) -> bool {
    locate(current).is_some()
}

/// The inverse of `wrapped`: the launch option with save-relay's part taken out (`None` when
/// nothing else was there). Not wrapped: `current` unchanged (`None` for an empty one).
///
/// Exact for every text that holds `%command%` or is empty. One text has the same meaning to
/// Steam but comes back shorter: game arguments with no `%command%` (`-novid`) and `%command%`
/// followed by a space and the same arguments (`%command% -novid`) are both wrapped to the same
/// text, and both come back as `-novid`.
pub fn unwrapped(current: &str) -> Option<String> {
    let Some((start, command)) = locate(current) else {
        return (!current.is_empty()).then(|| current.to_string());
    };
    let head = &current[..start];
    let tail = &current[command + COMMAND.len()..];
    let text = if head.is_empty() {
        match tail.strip_prefix(' ') {
            Some(rest) => rest.to_string(),
            None => format!("{COMMAND}{tail}"),
        }
    } else {
        format!("{head}{COMMAND}{tail}")
    };
    (!text.is_empty() && text != COMMAND).then_some(text)
}

/// The `LaunchOptions` of `app_id` in a parsed `localconfig.vdf`.
pub fn launch_options(config: &Vdf, app_id: u32) -> Option<String> {
    let id = app_id.to_string();
    config
        .get_path(&option_path(&id))?
        .as_str()
        .map(str::to_owned)
}

/// Set (or with `None` remove) the `LaunchOptions` of `app_id`.
pub fn set_launch_options(config: &mut Vdf, app_id: u32, value: Option<&str>) {
    let id = app_id.to_string();
    config.set_path(
        &option_path(&id),
        value.map(|text| Vdf::Str(text.to_string())),
    );
}

/// The backup's file name: `localconfig.vdf.save-relay-<entry stamp>.bak`.
pub fn backup_name(file_name: &str, now: u64) -> String {
    format!("{file_name}.save-relay-{}.bak", stamp::entry_stamp(now))
}

/// True when Steam is running. Linux: a pid in one of `LINUX_PID_FILES` under `home` names a
/// live process (`/proc/<pid>` exists). Mac: the pid in `MAC_PID_FILE` answers `kill -0`
/// (unverified on a real Mac: the pid file's place is from Steam's usual layout). Windows:
/// `tasklist` lists `steam.exe`. Android and iOS have no Steam: false. A check that cannot run,
/// or a pid file that is not a number, is `Error::Io`.
pub fn steam_running(platform: Platform, home: &Path) -> Result<bool> {
    match platform {
        Platform::Linux => {
            for file in LINUX_PID_FILES {
                if let Some(pid) = read_pid(&home.join(file))? {
                    if Path::new("/proc").join(pid.to_string()).exists() {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
        Platform::Mac => match read_pid(&home.join(MAC_PID_FILE))? {
            Some(pid) => Ok(Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .status()?
                .success()),
            None => Ok(false),
        },
        Platform::Windows => {
            let listing = Command::new("tasklist")
                .args(["/FI", "IMAGENAME eq steam.exe", "/NH"])
                .output()?;
            Ok(String::from_utf8_lossy(&listing.stdout)
                .lines()
                .any(|line| {
                    line.split_whitespace()
                        .next()
                        .is_some_and(|name| name.eq_ignore_ascii_case("steam.exe"))
                }))
        }
        Platform::Android | Platform::Ios => Ok(false),
    }
}

/// The whole command, after the caller refused with `Error::SteamRunning` when
/// `steam_running` (`cli::run` does): for every user's
/// `localconfig.vdf` and every app in `app_ids`, apply `wrapped` (or `unwrapped` with
/// `remove`); unless `dry_run`, write a backup (once per file per run) and then the file via a
/// temp file and rename. Returns what changed, apps already in the wanted state left out.
pub fn install(
    root: &Path,
    app_ids: &[u32],
    binary: &Path,
    remove: bool,
    dry_run: bool,
    platform: Platform,
    now: u64,
) -> Result<InstallWrapData> {
    let program_path = binary.display().to_string();
    let users = steam::user_ids(root)?;
    let mut changed = Vec::new();
    let mut backups = Vec::new();
    let mut pending: Vec<(PathBuf, Vec<u8>, Vdf)> = Vec::new();
    for user_id in &users {
        let path = root
            .join("userdata")
            .join(user_id)
            .join("config")
            .join("localconfig.vdf");
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let text = String::from_utf8(bytes.clone())
            .map_err(|_| Error::Io(format!("{}: not UTF-8", path.display())))?;
        let mut config =
            vdf::parse(&text).map_err(|error| Error::Io(format!("{}: {error}", path.display())))?;
        let mut file_changed = false;
        for &app_id in app_ids {
            let before = launch_options(&config, app_id);
            let after = next_option(before.as_deref(), &program_path, remove, platform);
            if after == before {
                continue;
            }
            // A removed option is kept as "" so the key stays; `after` reports it as None.
            set_launch_options(&mut config, app_id, Some(after.as_deref().unwrap_or("")));
            changed.push(InstallWrapApp {
                app_id,
                user_id: user_id.clone(),
                before,
                after,
            });
            file_changed = true;
        }
        if file_changed {
            pending.push((path, bytes, config));
        }
    }
    // Every file is read and parsed before the first write, so a file that cannot be read or
    // parsed leaves the others as they were. A write that fails part way can still leave the
    // files before it rewritten; their backups are listed in the result.
    if !dry_run {
        for (path, bytes, config) in &pending {
            backups.push(write_backup(path, bytes, now)?.display().to_string());
            write_config(path, config)?;
        }
    }
    let count = changed.len();
    let verb = match (remove, dry_run) {
        (false, false) => "wrapped",
        (false, true) => "would wrap",
        (true, false) => "removed save-relay from",
        (true, true) => "would remove save-relay from",
    };
    let message = if users.is_empty() {
        format!(
            "no Steam user under {}/userdata: sign in to Steam once, then run install-wrap again",
            root.display()
        )
    } else if count == 0 {
        "no launch option needed a change".to_string()
    } else {
        format!("{verb} {count} launch option(s)")
    };
    Ok(InstallWrapData {
        dry_run,
        changed,
        backups,
        message,
    })
}

/// Where save-relay's part starts in `current` (its program path's first byte) and the byte
/// index of the `%command%` after the marker. Only the first occurrence with a save-relay
/// program path counts.
fn locate(current: &str) -> Option<(usize, usize)> {
    let needle = format!("{MARKER} {COMMAND}");
    let mut from = 0;
    while let Some(offset) = current[from..].find(&needle) {
        let marker = from + offset;
        // "wrap" is ASCII, so the byte after its `w` is a character boundary.
        from = marker + 1;
        let head = current[..marker].trim_end();
        let gap = &current[head.len()..marker];
        if gap.is_empty() || !gap.chars().all(char::is_whitespace) {
            continue;
        }
        if let Some(start) = program_start(head) {
            return Some((start, marker + MARKER.len() + 1));
        }
    }
    None
}

/// Where the program path ending `head` starts, when it is a save-relay program: quoted (the
/// opening quote's index) or not (after the last whitespace).
fn program_start(head: &str) -> Option<usize> {
    let (start, token) = match head.strip_suffix('"') {
        Some(inner) => {
            let open = inner.rfind('"')?;
            (open, &inner[open + 1..])
        }
        None => {
            let start = match head.char_indices().rev().find(|(_, c)| c.is_whitespace()) {
                Some((index, c)) => index + c.len_utf8(),
                None => 0,
            };
            (start, &head[start..])
        }
    };
    let lower = token.to_ascii_lowercase();
    (lower.ends_with("save-relay") || lower.ends_with("save-relay.exe")).then_some(start)
}

/// The program path as it goes in the launch option: in double quotes on Windows or when it
/// holds a space.
fn program(binary: &str, platform: Platform) -> String {
    if platform == Platform::Windows || binary.contains(' ') {
        format!("\"{binary}\"")
    } else {
        binary.to_string()
    }
}

/// The `LaunchOptions` path under `apps` for an app id text.
fn option_path(id: &str) -> [&str; 7] {
    [
        "UserLocalConfigStore",
        "Software",
        "Valve",
        "Steam",
        "apps",
        id,
        "LaunchOptions",
    ]
}

/// The option after `install`: wrapped, or with save-relay's part taken out. `None` when the
/// option is not there (or not wrapped, for `remove`), so nothing changes.
fn next_option(
    before: Option<&str>,
    program_path: &str,
    remove: bool,
    platform: Platform,
) -> Option<String> {
    if remove {
        match before {
            Some(current) if is_wrapped(current) => unwrapped(current),
            _ => before.map(str::to_owned),
        }
    } else {
        Some(wrapped(before, program_path, platform))
    }
}

/// The pid in a Steam pid file: `None` when the file is missing.
fn read_pid(path: &Path) -> Result<Option<u32>> {
    match fs::read_to_string(path) {
        Ok(text) => text
            .trim()
            .parse::<u32>()
            .map(Some)
            .map_err(|_| Error::Io(format!("{}: not a process id", path.display()))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Write the original bytes to a new backup file beside `path`, named with `now`; a name
/// already taken (two runs in one second) moves on to the next second. Refuses to overwrite.
fn write_backup(path: &Path, bytes: &[u8], now: u64) -> Result<PathBuf> {
    let file_name = file_name_of(path);
    for step in 0..100 {
        let backup = path.with_file_name(backup_name(&file_name, now + step));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
        {
            Ok(mut file) => {
                file.write_all(bytes)?;
                return Ok(backup);
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::Io(format!(
        "no free backup name beside {}",
        path.display()
    )))
}

/// Write `config` to `path` through a temp file beside it and a rename.
fn write_config(path: &Path, config: &Vdf) -> Result<()> {
    let temp = path.with_file_name(format!("{}.save-relay-tmp", file_name_of(path)));
    fs::write(&temp, vdf::to_text(config))?;
    fs::rename(&temp, path)?;
    Ok(())
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "localconfig.vdf".to_string())
}
