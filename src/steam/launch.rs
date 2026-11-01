//! `save-relay install-wrap` (SPEC.md M13): put `save-relay wrap -- %command%` in the launch
//! options of the Steam games picked, in every Steam user's
//! `<root>/userdata/<id>/config/localconfig.vdf`
//! (`UserLocalConfigStore/Software/Valve/Steam/apps/<app id>/LaunchOptions`).
//!
//! Steam must not be running (it rewrites the file when it quits): refused with
//! `Error::SteamRunning`, and Steam is never closed for the person. A dated backup goes next to
//! the file before the first change. An existing launch option is kept by wrapping it, the
//! change is idempotent, and `--remove` undoes only save-relay's part. The text rules are pure.

use crate::api::InstallWrapData;
use crate::error::{not_built, Result};
use crate::platform::Platform;
use crate::steam::vdf::Vdf;
use std::path::Path;

/// The words that mark save-relay's part of a launch option.
pub const MARKER: &str = "wrap --";

/// The launch option with save-relay around it. `binary` is the save-relay program's path,
/// in double quotes when it holds a space or on Windows. `current` empty or `None`:
/// `<binary> wrap -- %command%`. `current` holding `%command%` (`gamemoderun %command% -fps`):
/// `%command%` becomes `<binary> wrap -- %command%`, the rest kept. `current` with no
/// `%command%` (game arguments only, `-novid`): `<binary> wrap -- %command% -novid`. Already
/// wrapped (`is_wrapped`): unchanged.
pub fn wrapped(current: Option<&str>, binary: &str, platform: Platform) -> String {
    crate::not_built!("steam::launch::wrapped")
}

/// True when `current` has save-relay's part: a program path ending in `save-relay` or
/// `save-relay.exe` (quoted or not) followed by `MARKER`.
pub fn is_wrapped(current: &str) -> bool {
    crate::not_built!("steam::launch::is_wrapped")
}

/// The inverse of `wrapped`: the launch option with save-relay's part taken out, exactly as it
/// was before (`None` when nothing else was there). Not wrapped: `current` unchanged.
pub fn unwrapped(current: &str) -> Option<String> {
    crate::not_built!("steam::launch::unwrapped")
}

/// The `LaunchOptions` of `app_id` in a parsed `localconfig.vdf`.
pub fn launch_options(config: &Vdf, app_id: u32) -> Option<String> {
    crate::not_built!("steam::launch::launch_options")
}

/// Set (or with `None` remove) the `LaunchOptions` of `app_id`.
pub fn set_launch_options(config: &mut Vdf, app_id: u32, value: Option<&str>) {
    crate::not_built!("steam::launch::set_launch_options")
}

/// The backup's file name: `localconfig.vdf.save-relay-<entry stamp>.bak`.
pub fn backup_name(file_name: &str, now: u64) -> String {
    crate::not_built!("steam::launch::backup_name")
}

/// True when Steam is running: Linux and Mac, the pid in `<home>/.steam/steam.pid` is a live
/// process (Linux: `/proc/<pid>` exists; Mac: `kill -0`); Windows, `tasklist` lists
/// `steam.exe`. A check that cannot run is `Error::Io`.
pub fn steam_running(platform: Platform, home: &Path) -> Result<bool> {
    not_built("steam::launch::steam_running")
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
    not_built("steam::launch::install")
}
