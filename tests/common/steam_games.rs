use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static ENVIRONMENT: Mutex<()> = Mutex::new(());

/// Copy the made-up F74/F75 Steam installation fixture into a temporary root and substitute
/// its library path. The copy lets each test write independent cache and local state.
pub fn fixture(root: &Path) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/steam-games");
    let dest = root.join("steam");
    fs::create_dir_all(&dest).unwrap();
    for entry in tree(&source) {
        let relative = entry.strip_prefix(&source).unwrap();
        let target = dest.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target).unwrap();
        } else {
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            let contents = fs::read_to_string(&entry).unwrap();
            let steam_root = dest.to_string_lossy().replace('\\', "/");
            fs::write(target, contents.replace("__STEAM_ROOT__", &steam_root)).unwrap();
        }
    }
    dest
}

fn tree(root: &Path) -> Vec<PathBuf> {
    let mut entries = vec![root.to_path_buf()];
    let mut index = 0;
    while index < entries.len() {
        if entries[index].is_dir() {
            entries.extend(
                fs::read_dir(&entries[index])
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
        }
        index += 1;
    }
    entries
}

/// Set the fixture Steam root for an end-to-end CLI test and restore the process environment
/// after the guard drops.
pub fn set_root(root: &Path) -> SteamRootGuard {
    let lock = ENVIRONMENT
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let previous = std::env::var_os("SAVE_RELAY_STEAM_DIR");
    #[cfg(windows)]
    let previous_program_files = std::env::var_os("ProgramFiles(x86)");
    std::env::set_var("SAVE_RELAY_STEAM_DIR", root);
    // Prevent a developer's installed Steam from being found after an intentionally missing
    // fixture root. The explicit fixture root still wins for tests that have one.
    #[cfg(windows)]
    std::env::set_var("ProgramFiles(x86)", root.join(".no-program-files"));
    SteamRootGuard {
        previous,
        #[cfg(windows)]
        previous_program_files,
        _lock: lock,
    }
}

pub struct SteamRootGuard {
    previous: Option<std::ffi::OsString>,
    #[cfg(windows)]
    previous_program_files: Option<std::ffi::OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl Drop for SteamRootGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("SAVE_RELAY_STEAM_DIR", value),
            None => std::env::remove_var("SAVE_RELAY_STEAM_DIR"),
        }
        #[cfg(windows)]
        match &self.previous_program_files {
            Some(value) => std::env::set_var("ProgramFiles(x86)", value),
            None => std::env::remove_var("ProgramFiles(x86)"),
        }
    }
}
