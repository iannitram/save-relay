use save_relay::platform::{PathVars, Platform};
use save_relay::steam::{self, Library, APP_ID_VARS, STEAM_DIR_VAR};
use save_relay::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |key| {
        pairs
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.to_string())
    }
}

fn vars(home: &Path) -> PathVars {
    PathVars {
        home: home.to_owned(),
        data: home.join("custom-data"),
        config: home.join("custom-config"),
        documents: None,
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = PathBuf::from(format!(
                ".steam-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create temporary folder: {error}"),
            }
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("clean temporary folder");
    }
}

#[test]
fn app_id_reads_each_variable_and_honors_order() {
    for name in APP_ID_VARS {
        assert_eq!(steam::app_id_from_env(&env(&[(name, "42")])), Some(42));
    }
    assert_eq!(
        steam::app_id_from_env(&env(&[
            ("SteamAppId", "1"),
            ("SteamGameId", "2"),
            ("STEAM_COMPAT_APP_ID", "3"),
        ])),
        Some(1)
    );
    assert_eq!(
        steam::app_id_from_env(&env(&[("SteamGameId", "2"), ("STEAM_COMPAT_APP_ID", "3")])),
        Some(2)
    );
}

#[test]
fn app_id_skips_unusable_values_and_accepts_u32_max() {
    for invalid in [
        "",
        "abc",
        "0",
        "4294967296",
        "18446744073709551615",
        "-1",
        "1.5",
        " 42",
    ] {
        assert_eq!(
            steam::app_id_from_env(&env(&[("SteamAppId", invalid)])),
            None
        );
        assert_eq!(
            steam::app_id_from_env(&env(&[("SteamAppId", invalid), ("SteamGameId", "42")])),
            Some(42)
        );
        assert_eq!(
            steam::app_id_from_env(&env(&[
                ("SteamAppId", invalid),
                ("SteamGameId", invalid),
                ("STEAM_COMPAT_APP_ID", "42"),
            ])),
            Some(42)
        );
    }
    assert_eq!(steam::app_id_from_env(&env(&[])), None);
    assert_eq!(
        steam::app_id_from_env(&env(&[("SteamAppId", "4294967295")])),
        Some(u32::MAX)
    );
}

#[test]
fn game_id_uses_steam_prefix() {
    assert_eq!(steam::game_id(42), "steam-42");
}

#[test]
fn candidates_for_every_platform_with_override_first() {
    let home = PathBuf::from("fixture-home");
    let vars = vars(&home);
    for (platform, expected) in [
        (
            Platform::Linux,
            vec![
                home.join(".steam/steam"),
                home.join(".local/share/Steam"),
                home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
            ],
        ),
        (
            Platform::Windows,
            vec![PathBuf::from(r"C:\Program Files (x86)").join("Steam")],
        ),
        (
            Platform::Mac,
            vec![home.join("Library/Application Support/Steam")],
        ),
        (Platform::Android, vec![]),
        (Platform::Ios, vec![]),
    ] {
        assert_eq!(steam::root_candidates(platform, &vars, &env(&[])), expected);
        assert_eq!(
            steam::root_candidates(platform, &vars, &env(&[(STEAM_DIR_VAR, "")])),
            expected
        );
        let mut overridden = vec![PathBuf::from("override")];
        overridden.extend(expected);
        assert_eq!(
            steam::root_candidates(platform, &vars, &env(&[(STEAM_DIR_VAR, "override")])),
            overridden
        );
    }
}

#[test]
fn windows_candidates_use_program_files_when_set() {
    let vars = vars(Path::new("fixture-home"));
    for value in ["alternate-program-files", ""] {
        let base = if value.is_empty() {
            r"C:\Program Files (x86)"
        } else {
            value
        };
        assert_eq!(
            steam::root_candidates(
                Platform::Windows,
                &vars,
                &env(&[("ProgramFiles(x86)", value)])
            ),
            vec![PathBuf::from(base).join("Steam")]
        );
    }
}

#[test]
fn find_root_checks_second_candidate_and_reports_every_attempt() {
    let temp = TempDir::new();
    let vars = vars(&temp.0);
    let candidates = steam::root_candidates(Platform::Linux, &vars, &env(&[]));
    // A folder without libraryfolders.vdf is not enough.
    fs::create_dir_all(candidates[0].join("steamapps")).unwrap();
    let error = steam::find_root(Platform::Linux, &vars, &env(&[])).unwrap_err();
    assert!(
        matches!(error, Error::SteamNotFound(message) if candidates.iter().all(|path| message.contains(&path.display().to_string())))
    );
    fs::create_dir_all(candidates[1].join("steamapps")).unwrap();
    fs::write(candidates[1].join("steamapps/libraryfolders.vdf"), "").unwrap();
    assert_eq!(
        steam::find_root(Platform::Linux, &vars, &env(&[])).unwrap(),
        candidates[1]
    );
    fs::write(candidates[0].join("steamapps/libraryfolders.vdf"), "").unwrap();
    assert_eq!(
        steam::find_root(Platform::Linux, &vars, &env(&[])).unwrap(),
        candidates[0]
    );
}

#[cfg(unix)]
#[test]
fn find_root_follows_symbolic_links() {
    let temp = TempDir::new();
    let real = temp.0.join("real");
    fs::create_dir_all(real.join("steamapps")).unwrap();
    fs::write(real.join("steamapps/libraryfolders.vdf"), "").unwrap();
    fs::create_dir_all(temp.0.join(".steam")).unwrap();
    std::os::unix::fs::symlink("../real", temp.0.join(".steam/steam")).unwrap();
    assert_eq!(
        steam::find_root(Platform::Linux, &vars(&temp.0), &env(&[])).unwrap(),
        temp.0.join(".steam/steam")
    );
}

#[test]
fn parse_two_libraries_preserves_file_order_and_skips_non_numeric_keys() {
    let libraries = steam::parse_libraries(r#"
        "LibraryFolders" {
            "metadata" "ignored"
            "2" { "Path" "library-two" "Apps" { "42" "size" "abc" "0" "7" "size" "4294967296" "0" } }
            "0" { "path" "library-zero" "apps" { "99" "size" } }
        }
    "#).unwrap();
    assert_eq!(
        libraries,
        vec![
            Library {
                path: PathBuf::from("library-two"),
                apps: vec![42, 7]
            },
            Library {
                path: PathBuf::from("library-zero"),
                apps: vec![99]
            },
        ]
    );
    assert_eq!(steam::library_for(&libraries, 99), Some(&libraries[1]));
    assert_eq!(steam::library_for(&libraries, 100), None);
    assert_eq!(steam::library_for(&[], 42), None);
}

#[test]
fn parse_libraries_reports_file_context_for_bad_text_or_missing_path() {
    for text in [
        r#""libraryfolders" {"#,
        r#""libraryfolders" { "0" { "apps" {} } }"#,
    ] {
        assert!(
            matches!(steam::parse_libraries(text), Err(Error::Io(message)) if message.starts_with("libraryfolders.vdf: "))
        );
    }
    assert_eq!(
        steam::parse_libraries(r#""libraryfolders" {}"#).unwrap(),
        vec![]
    );
}

#[test]
fn parse_install_dir_reads_app_state_and_handles_missing_or_bad_text() {
    assert_eq!(
        steam::parse_install_dir(r#""AppState" { "installdir" "A Game" }"#),
        Some("A Game".into())
    );
    for text in [
        "",
        "{",
        r#""AppState" {}"#,
        r#""installdir" "wrong level""#,
        r#""AppState" { "installdir" {} }"#,
    ] {
        assert_eq!(steam::parse_install_dir(text), None);
    }
}

#[test]
fn proton_and_screenshot_paths() {
    let root = Path::new("steam-root");
    assert_eq!(
        steam::proton_prefix(root, 42),
        root.join("steamapps/compatdata/42/pfx")
    );
    assert_eq!(
        steam::screenshot_dir(root, "123", 42),
        root.join("userdata/123/760/remote/42/screenshots/thumbnails")
    );
}

#[test]
fn user_ids_are_digit_folders_sorted_as_text_and_missing_is_empty() {
    let temp = TempDir::new();
    assert_eq!(steam::user_ids(&temp.0).unwrap(), Vec::<String>::new());
    for name in ["2", "10", "001", "abc", "-1", "+3", "4a", "１２"] {
        fs::create_dir_all(temp.0.join("userdata").join(name)).unwrap();
    }
    fs::write(temp.0.join("userdata/123"), "stray file").unwrap();
    assert_eq!(steam::user_ids(&temp.0).unwrap(), vec!["001", "10", "2"]);
}

#[test]
fn screenshot_filters_extension_and_session_then_breaks_ties_by_name() {
    let temp = TempDir::new();
    let files: Vec<(String, u64)> = [
        ("latest.png", 200),
        ("before.jpg", 99),
        ("after.jpg", 201),
        ("start.jpg", 100),
        ("a.jpg", 200),
        ("z.JpG", 200),
        ("suffix.jpg.bak", 200),
        ("é", 200),
    ]
    .into_iter()
    .map(|(name, time)| {
        fs::write(temp.0.join(name), "picture").unwrap();
        (name.to_owned(), time)
    })
    .collect();
    assert_eq!(
        steam::pick_screenshot(&files, 100, 200),
        Some("z.JpG".into())
    );
    assert_eq!(
        steam::pick_screenshot(&files, 100, 100),
        Some("start.jpg".into())
    );
    assert_eq!(steam::pick_screenshot(&files, 101, 199), None);
    assert_eq!(steam::pick_screenshot(&files, 201, 100), None);
    assert_eq!(steam::pick_screenshot(&[], 100, 200), None);
    let mut reversed = files;
    reversed.reverse();
    assert_eq!(
        steam::pick_screenshot(&reversed, 100, 200),
        Some("z.JpG".into())
    );
}
