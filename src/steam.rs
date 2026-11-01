//! Steam's own files, read for `wrap` and `install-wrap` (SPEC.md M12-M14;
//! docs/adr/0003-baton-wrap-cards-panel.md): where Steam is, its libraries, which library
//! holds an app (and so its Proton prefix), the Steam user ids, and the screenshot a wrapped
//! game's save card takes. Steam's text files are KeyValues (`vdf`); writing the launch option
//! is `launch`. Pure functions over text and file lists, plus small reads; never a network
//! call, never Steam's binary files.

pub mod launch;
pub mod vdf;

use crate::error::{not_built, Result};
use crate::platform::{PathVars, Platform};
use std::path::{Path, PathBuf};

/// An environment variable naming the Steam folder; it wins over every default.
pub const STEAM_DIR_VAR: &str = "SAVE_RELAY_STEAM_DIR";

/// The variables Steam sets for a game it starts, read in this order; the first set one wins.
pub const APP_ID_VARS: [&str; 3] = ["SteamAppId", "SteamGameId", "STEAM_COMPAT_APP_ID"];

/// The app id Steam passed to a game it started: the first of `APP_ID_VARS` that is set and
/// is a whole number above 0 (`SteamGameId` can be a 64-bit id for a non-Steam shortcut: a
/// value above `u32::MAX` is skipped). `None` when none fits.
pub fn app_id_from_env(var: &dyn Fn(&str) -> Option<String>) -> Option<u32> {
    crate::not_built!("steam::app_id_from_env")
}

/// The game id a wrapped Steam game gets: `"steam-<app id>"` (a valid `manifest::Manifest` id).
pub fn game_id(app_id: u32) -> String {
    crate::not_built!("steam::game_id")
}

/// The folders Steam may live in, in the order tried:
/// `$SAVE_RELAY_STEAM_DIR` when set; then
/// - Linux: `<home>/.steam/steam`, `<home>/.local/share/Steam`,
///   `<home>/.var/app/com.valvesoftware.Steam/.local/share/Steam` (Flatpak);
/// - Windows: `%ProgramFiles(x86)%\Steam`, else `C:\Program Files (x86)\Steam` (no registry);
/// - Mac: `<home>/Library/Application Support/Steam`;
/// - Android, iOS: none.
pub fn root_candidates(
    platform: Platform,
    vars: &PathVars,
    var: &dyn Fn(&str) -> Option<String>,
) -> Vec<PathBuf> {
    crate::not_built!("steam::root_candidates")
}

/// The first of `root_candidates` that holds `steamapps/libraryfolders.vdf` (symbolic links
/// followed: `~/.steam/steam` is one). None is `Error::SteamNotFound` naming the folders tried.
pub fn find_root(
    platform: Platform,
    vars: &PathVars,
    var: &dyn Fn(&str) -> Option<String>,
) -> Result<PathBuf> {
    not_built("steam::find_root")
}

/// One Steam library folder and the app ids installed in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    /// The library folder (it holds `steamapps/`).
    pub path: PathBuf,
    pub apps: Vec<u32>,
}

/// The libraries in `steamapps/libraryfolders.vdf`'s text: each numbered entry's `path` and
/// the keys of its `apps`, in file order. Text that is not KeyValues, or an entry with no
/// `path`, is `Error::Io("libraryfolders.vdf: ...")`.
pub fn parse_libraries(text: &str) -> Result<Vec<Library>> {
    not_built("steam::parse_libraries")
}

/// The library that holds `app_id`, if any.
pub fn library_for(libraries: &[Library], app_id: u32) -> Option<&Library> {
    crate::not_built!("steam::library_for")
}

/// The install folder name from `steamapps/appmanifest_<id>.acf`'s text (`AppState/installdir`),
/// so `<library>/steamapps/common/<installdir>` is the game's folder (ludusavi's `<base>`).
/// `None` when it is missing.
pub fn parse_install_dir(text: &str) -> Option<String> {
    crate::not_built!("steam::parse_install_dir")
}

/// `<library>/steamapps/compatdata/<app id>/pfx`: the Proton prefix of a Windows game on
/// Linux, whose `drive_c/users/steamuser/` holds `AppData/Roaming`, `AppData/Local` and
/// `Documents`.
pub fn proton_prefix(library: &Path, app_id: u32) -> PathBuf {
    crate::not_built!("steam::proton_prefix")
}

/// The Steam user ids (the folder names under `<root>/userdata/` that are whole numbers),
/// sorted. A missing `userdata` folder is an empty list.
pub fn user_ids(root: &Path) -> Result<Vec<String>> {
    not_built("steam::user_ids")
}

/// `<root>/userdata/<user id>/760/remote/<app id>/screenshots/thumbnails`: Steam's small
/// copies of the screenshots taken in that game.
pub fn screenshot_dir(root: &Path, user_id: &str, app_id: u32) -> PathBuf {
    crate::not_built!("steam::screenshot_dir")
}

/// The picture for a wrapped game's save card: of `files` (name, modification time in Unix
/// seconds), the newest `.jpg` whose time is within `[session_start, now]`; on a tie the
/// greater name. `None` when none was taken during the session.
pub fn pick_screenshot(files: &[(String, u64)], session_start: u64, now: u64) -> Option<String> {
    crate::not_built!("steam::pick_screenshot")
}
