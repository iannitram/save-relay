//! Steam's own files, read for `wrap` and `install-wrap` (SPEC.md M12-M14;
//! docs/adr/0003-baton-wrap-cards-panel.md): where Steam is, its libraries, which library
//! holds an app (and so its Proton prefix), the Steam user ids, and the screenshot a wrapped
//! game's save card takes. Steam's text files are KeyValues (`vdf`); writing the launch option
//! is `launch`. Pure functions over text and file lists, plus small reads; never a network
//! call, never Steam's binary files.

pub mod launch;
pub mod vdf;

use crate::error::{Error, Result};
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
    APP_ID_VARS
        .iter()
        .filter_map(|name| var(name)?.parse::<u32>().ok())
        .find(|id| *id > 0)
}

/// The game id a wrapped Steam game gets: `"steam-<app id>"` (a valid `manifest::Manifest` id).
pub fn game_id(app_id: u32) -> String {
    format!("steam-{app_id}")
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
    let mut candidates: Vec<PathBuf> = var(STEAM_DIR_VAR)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .into_iter()
        .collect();
    match platform {
        Platform::Linux => candidates.extend([
            vars.home.join(".steam/steam"),
            vars.home.join(".local/share/Steam"),
            vars.home
                .join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        ]),
        Platform::Windows => candidates.push(
            var("ProgramFiles(x86)")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"))
                .join("Steam"),
        ),
        Platform::Mac => candidates.push(vars.home.join("Library/Application Support/Steam")),
        Platform::Android | Platform::Ios => {}
    }
    candidates
}

/// The first of `root_candidates` that holds `steamapps/libraryfolders.vdf` (symbolic links
/// followed: `~/.steam/steam` is one). None is `Error::SteamNotFound` naming the folders tried.
pub fn find_root(
    platform: Platform,
    vars: &PathVars,
    var: &dyn Fn(&str) -> Option<String>,
) -> Result<PathBuf> {
    let candidates = root_candidates(platform, vars, var);
    candidates
        .iter()
        .find(|root| root.join("steamapps/libraryfolders.vdf").exists())
        .cloned()
        .ok_or_else(|| {
            Error::SteamNotFound(
                candidates
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        })
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
    let root =
        vdf::parse(text).map_err(|error| Error::Io(format!("libraryfolders.vdf: {error}")))?;
    let entries = match root.get("libraryfolders") {
        Some(vdf::Vdf::Map(entries)) => entries,
        None => return Ok(Vec::new()),
        Some(_) => return Err(Error::Io("libraryfolders.vdf: expected a block".into())),
    };
    let mut libraries = Vec::new();
    for (key, entry) in entries {
        if key.is_empty() || !key.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let path = entry
            .get("path")
            .and_then(vdf::Vdf::as_str)
            .ok_or_else(|| Error::Io(format!("libraryfolders.vdf: entry {key} has no path")))?;
        let apps = match entry.get("apps") {
            Some(vdf::Vdf::Map(apps)) => apps
                .iter()
                .filter_map(|(id, _)| id.parse::<u32>().ok())
                .collect(),
            _ => Vec::new(),
        };
        libraries.push(Library {
            path: PathBuf::from(path),
            apps,
        });
    }
    Ok(libraries)
}

/// The library that holds `app_id`, if any.
pub fn library_for(libraries: &[Library], app_id: u32) -> Option<&Library> {
    libraries
        .iter()
        .find(|library| library.apps.contains(&app_id))
}

/// The install folder name from `steamapps/appmanifest_<id>.acf`'s text (`AppState/installdir`),
/// so `<library>/steamapps/common/<installdir>` is the game's folder (ludusavi's `<base>`).
/// `None` when it is missing.
pub fn parse_install_dir(text: &str) -> Option<String> {
    vdf::parse(text)
        .ok()?
        .get_path(&["AppState", "installdir"])?
        .as_str()
        .map(str::to_owned)
}

/// `<library>/steamapps/compatdata/<app id>/pfx`: the Proton prefix of a Windows game on
/// Linux, whose `drive_c/users/steamuser/` holds `AppData/Roaming`, `AppData/Local` and
/// `Documents`.
pub fn proton_prefix(library: &Path, app_id: u32) -> PathBuf {
    library
        .join("steamapps/compatdata")
        .join(app_id.to_string())
        .join("pfx")
}

/// The Steam user ids (the folder names under `<root>/userdata/` that are whole numbers),
/// sorted. A missing `userdata` folder is an empty list.
pub fn user_ids(root: &Path) -> Result<Vec<String>> {
    let entries = match std::fs::read_dir(root.join("userdata")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            if !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit()) {
                ids.push(name.to_owned());
            }
        }
    }
    ids.sort();
    Ok(ids)
}

/// `<root>/userdata/<user id>/760/remote/<app id>/screenshots/thumbnails`: Steam's small
/// copies of the screenshots taken in that game.
pub fn screenshot_dir(root: &Path, user_id: &str, app_id: u32) -> PathBuf {
    root.join("userdata")
        .join(user_id)
        .join("760/remote")
        .join(app_id.to_string())
        .join("screenshots/thumbnails")
}

/// The picture for a wrapped game's save card: of `files` (name, modification time in Unix
/// seconds), the newest `.jpg` whose time is within `[session_start, now]`; on a tie the
/// greater name. `None` when none was taken during the session.
pub fn pick_screenshot(files: &[(String, u64)], session_start: u64, now: u64) -> Option<String> {
    files
        .iter()
        .filter(|(name, time)| {
            name.get(name.len().saturating_sub(4)..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".jpg"))
                && *time >= session_start
                && *time <= now
        })
        .max_by(|(name_a, time_a), (name_b, time_b)| {
            time_a.cmp(time_b).then_with(|| name_a.cmp(name_b))
        })
        .map(|(name, _)| name.clone())
}

/// The newest eligible screenshot file across Steam users for an app. Its filesystem
/// modification time is used as the capture time; unreadable folders and files are skipped.
pub fn screenshot_file(root: &Path, app_id: u32, session_start: u64, now: u64) -> Option<PathBuf> {
    use std::time::UNIX_EPOCH;

    let mut candidates = Vec::new();
    for user_id in user_ids(root).ok()? {
        let directory = screenshot_dir(root, &user_id, app_id);
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
                continue;
            };
            let Ok(time) = modified.duration_since(UNIX_EPOCH) else {
                continue;
            };
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            candidates.push((format!("{user_id}/{name}"), time.as_secs(), path));
        }
    }
    let listed: Vec<_> = candidates
        .iter()
        .map(|(name, time, _)| (name.clone(), *time))
        .collect();
    let chosen = pick_screenshot(&listed, session_start, now)?;
    candidates
        .into_iter()
        .find(|(name, _, _)| *name == chosen)
        .map(|(_, _, path)| path)
}
