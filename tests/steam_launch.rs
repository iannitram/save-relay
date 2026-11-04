//! `steam::launch` (`save-relay install-wrap`, SPEC.md M13): the launch-option text rules, the
//! `localconfig.vdf` reads and writes, the dated backup, `--remove`, and the Steam-running check.
//! Install runs over temp copies of a fixture `localconfig.vdf`, on every OS: the Windows
//! quoting comes from the `Platform` value passed, not from the host.

use save_relay::platform::Platform;
use save_relay::steam::launch::{
    backup_name, install, is_wrapped, launch_options, set_launch_options, steam_running, unwrapped,
    wrapped,
};
use save_relay::steam::vdf::{parse, to_text};
use save_relay::steam::{installed_among, parse_libraries};
use save_relay::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const BIN: &str = "/opt/save-relay/save-relay";
const WRAP: &str = "/opt/save-relay/save-relay wrap -- %command%";
const WINDOWS_BIN: &str = r"C:\Games\save-relay.exe";
const NOW: u64 = 1_793_476_260; // 2026-10-31T19:51:00Z
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A `localconfig.vdf` with two apps: one with an empty option, one with a game command line.
const LOCALCONFIG: &str = r#""UserLocalConfigStore"
{
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
				"apps"
				{
					"480"
					{
						"LaunchOptions"		""
						"LastPlayed"		"1700000000"
					}
					"1145360"
					{
						"LaunchOptions"		"gamemoderun %command% -fps"
						"LastPlayed"		"1710000000"
					}
				}
			}
		}
	}
}
"#;

fn temp_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "save-relay-steam-launch-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// Write `text` as `<root>/userdata/<user>/config/localconfig.vdf`; returns the file.
fn add_user(root: &Path, user: &str, text: &str) -> PathBuf {
    let dir = root.join("userdata").join(user).join("config");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("localconfig.vdf");
    fs::write(&file, text).unwrap();
    file
}

/// The `LaunchOptions` of `app_id` as the file holds it now.
fn option_in(file: &Path, app_id: u32) -> Option<String> {
    let config = parse(&fs::read_to_string(file).unwrap()).unwrap();
    launch_options(&config, app_id)
}

#[test]
fn wrapped_of_nothing_is_the_bare_invocation() {
    assert_eq!(wrapped(None, BIN, Platform::Linux), WRAP);
    assert_eq!(wrapped(Some(""), BIN, Platform::Linux), WRAP);
}

#[test]
fn wrapped_keeps_the_text_around_command() {
    assert_eq!(
        wrapped(Some("gamemoderun %command% -fps"), BIN, Platform::Linux),
        "gamemoderun /opt/save-relay/save-relay wrap -- %command% -fps"
    );
}

#[test]
fn wrapped_puts_command_before_game_arguments() {
    assert_eq!(
        wrapped(Some("-novid"), BIN, Platform::Linux),
        "/opt/save-relay/save-relay wrap -- %command% -novid"
    );
}

#[test]
fn wrapped_quotes_on_windows_and_around_spaces() {
    assert_eq!(
        wrapped(None, WINDOWS_BIN, Platform::Windows),
        r#""C:\Games\save-relay.exe" wrap -- %command%"#
    );
    assert_eq!(
        wrapped(None, "/home/me/My Tools/save-relay", Platform::Linux),
        r#""/home/me/My Tools/save-relay" wrap -- %command%"#
    );
}

#[test]
fn wrapping_twice_changes_nothing() {
    let once = wrapped(Some("gamemoderun %command% -fps"), BIN, Platform::Linux);
    assert!(is_wrapped(&once));
    assert_eq!(wrapped(Some(once.as_str()), BIN, Platform::Linux), once);
}

#[test]
fn is_wrapped_names_only_save_relay() {
    for wrapped_text in [
        WRAP,
        r#""C:\Games\save-relay.exe" wrap -- %command%"#,
        "gamemoderun /usr/bin/save-relay wrap -- %command% -fps",
        "/opt/save-relay/SAVE-RELAY.EXE wrap -- %command%",
    ] {
        assert!(is_wrapped(wrapped_text), "{wrapped_text}");
    }
    for other in [
        "",
        "%command%",
        "gamemoderun %command%",
        "-novid",
        "/opt/tool/tool wrap -- %command%",
        "/opt/save-relay-old wrap -- %command%",
        "/opt/save-relay/save-relay run -- %command%",
        "/opt/save-relay/save-relay wrap -- game",
    ] {
        assert!(!is_wrapped(other), "{other}");
    }
}

#[test]
fn unwrapped_restores_each_text_that_holds_command_or_nothing() {
    let originals = [
        "gamemoderun %command% -fps",
        "%command%-novid",
        "a %command% b %command%",
        "-novid",
        " -novid",
        "  ",
    ];
    let binaries = [
        (BIN, Platform::Linux),
        ("/home/me/My Tools/save-relay", Platform::Linux),
        (WINDOWS_BIN, Platform::Windows),
    ];
    for original in originals {
        for (binary, platform) in binaries {
            let text = wrapped(Some(original), binary, platform);
            assert!(is_wrapped(&text), "{text}");
            assert_eq!(
                unwrapped(&text),
                Some(original.to_string()),
                "{original} via {text}"
            );
        }
    }
}

#[test]
fn unwrapped_of_nothing_or_bare_command_is_none() {
    assert_eq!(unwrapped(&wrapped(None, BIN, Platform::Linux)), None);
    assert_eq!(unwrapped(&wrapped(Some(""), BIN, Platform::Linux)), None);
    assert_eq!(
        unwrapped(&wrapped(Some("%command%"), BIN, Platform::Linux)),
        None
    );
}

#[test]
fn unwrapped_of_bare_arguments_with_command_is_the_same_for_steam() {
    // `%command% -fps` and `-fps` both mean "the game with -fps": the second comes back.
    let text = wrapped(Some("%command% -fps"), BIN, Platform::Linux);
    assert_eq!(unwrapped(&text), Some("-fps".to_string()));
}

#[test]
fn unwrapped_leaves_an_option_that_is_not_wrapped_alone() {
    assert_eq!(
        unwrapped("gamemoderun %command% -fps"),
        Some("gamemoderun %command% -fps".to_string())
    );
    assert_eq!(unwrapped(""), None);
}

#[test]
fn launch_options_reads_each_app() {
    let config = parse(LOCALCONFIG).unwrap();
    assert_eq!(launch_options(&config, 480), Some(String::new()));
    assert_eq!(
        launch_options(&config, 1145360),
        Some("gamemoderun %command% -fps".to_string())
    );
    assert_eq!(launch_options(&config, 999), None);
}

#[test]
fn set_launch_options_writes_removes_and_adds_apps() {
    let mut config = parse(LOCALCONFIG).unwrap();
    set_launch_options(&mut config, 480, Some("-novid"));
    assert_eq!(launch_options(&config, 480), Some("-novid".to_string()));
    set_launch_options(&mut config, 1145360, None);
    assert_eq!(launch_options(&config, 1145360), None);
    set_launch_options(&mut config, 7, Some(WRAP));
    assert_eq!(launch_options(&config, 7), Some(WRAP.to_string()));

    // The other keys of the app are untouched, and the file reads back the same.
    let text = to_text(&config);
    let again = parse(&text).unwrap();
    assert_eq!(again, config);
    assert!(text.contains("\"LastPlayed\"\t\t\"1700000000\""));
}

#[test]
fn backup_name_carries_the_entry_stamp() {
    assert_eq!(
        backup_name("localconfig.vdf", NOW),
        "localconfig.vdf.save-relay-20261031T195100Z.bak"
    );
}

#[test]
fn install_wraps_the_picked_apps_and_keeps_the_rest() {
    let root = temp_root();
    let file = add_user(&root, "42", LOCALCONFIG);
    let data = install(
        &root,
        &[480, 1145360, 1659420],
        Path::new(BIN),
        false,
        false,
        Platform::Linux,
        NOW,
    )
    .unwrap();

    assert!(!data.dry_run);
    assert_eq!(data.changed.len(), 3);
    let app = |id: u32| data.changed.iter().find(|c| c.app_id == id).unwrap();
    assert_eq!(app(480).before, Some(String::new()));
    assert_eq!(app(480).after, Some(WRAP.to_string()));
    assert_eq!(app(480).user_id, "42");
    assert_eq!(
        app(1145360).after,
        Some("gamemoderun /opt/save-relay/save-relay wrap -- %command% -fps".to_string())
    );
    assert_eq!(app(1659420).before, None);
    assert_eq!(app(1659420).after, Some(WRAP.to_string()));

    assert_eq!(option_in(&file, 480), Some(WRAP.to_string()));
    assert_eq!(option_in(&file, 1659420), Some(WRAP.to_string()));
    let on_disk = fs::read_to_string(&file).unwrap();
    assert!(on_disk.contains("\"LastPlayed\"\t\t\"1700000000\""));

    // One backup, holding the file exactly as it was.
    assert_eq!(data.backups.len(), 1);
    assert!(data.backups[0].ends_with("localconfig.vdf.save-relay-20261031T195100Z.bak"));
    assert_eq!(fs::read(&data.backups[0]).unwrap(), LOCALCONFIG.as_bytes());
}

#[test]
fn install_twice_changes_nothing_the_second_time() {
    let root = temp_root();
    let file = add_user(&root, "42", LOCALCONFIG);
    install(
        &root,
        &[480],
        Path::new(BIN),
        false,
        false,
        Platform::Linux,
        NOW,
    )
    .unwrap();
    let after_first = fs::read(&file).unwrap();

    let second = install(
        &root,
        &[480],
        Path::new(BIN),
        false,
        false,
        Platform::Linux,
        NOW,
    )
    .unwrap();
    assert!(second.changed.is_empty());
    assert!(second.backups.is_empty());
    assert_eq!(fs::read(&file).unwrap(), after_first);
}

#[test]
fn install_dry_run_reports_and_writes_nothing() {
    let root = temp_root();
    let file = add_user(&root, "42", LOCALCONFIG);
    let data = install(
        &root,
        &[480, 1145360],
        Path::new(BIN),
        false,
        true,
        Platform::Linux,
        NOW,
    )
    .unwrap();

    assert!(data.dry_run);
    assert_eq!(data.changed.len(), 2);
    assert!(data.backups.is_empty());
    assert_eq!(fs::read(&file).unwrap(), LOCALCONFIG.as_bytes());
    assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn install_dry_run_over_two_users_writes_nothing() {
    let root = temp_root();
    let first = add_user(&root, "42", LOCALCONFIG);
    let second = add_user(&root, "43", LOCALCONFIG);
    let data = install(
        &root,
        &[480, 1145360],
        Path::new(BIN),
        false,
        true,
        Platform::Linux,
        NOW,
    )
    .unwrap();

    assert!(data.dry_run);
    let changes: Vec<(&str, u32)> = data
        .changed
        .iter()
        .map(|c| (c.user_id.as_str(), c.app_id))
        .collect();
    assert_eq!(
        changes,
        [("42", 480), ("42", 1145360), ("43", 480), ("43", 1145360)]
    );
    assert!(data.backups.is_empty());
    assert_eq!(fs::read(&first).unwrap(), LOCALCONFIG.as_bytes());
    assert_eq!(fs::read(&second).unwrap(), LOCALCONFIG.as_bytes());
    assert_eq!(fs::read_dir(first.parent().unwrap()).unwrap().count(), 1);
    assert_eq!(fs::read_dir(second.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn install_refuses_a_program_not_named_save_relay() {
    let root = temp_root();
    let file = add_user(&root, "42", LOCALCONFIG);
    for program in [
        "/opt/save-relay/save-relay-linux-x64",
        "/opt/save-relay/save-relay-old",
        r"C:\Downloads\save-relay (1).exe",
        r"C:\Downloads\save-relay-windows-x64.exe",
    ] {
        // Refused again on a second attempt: the first one wrote nothing.
        for _ in 0..2 {
            let result = install(
                &root,
                &[480],
                Path::new(program),
                false,
                false,
                Platform::Linux,
                NOW,
            );
            assert!(
                matches!(result, Err(Error::Usage(_))),
                "{program}: {result:?}"
            );
        }
        assert_eq!(
            fs::read(&file).unwrap(),
            LOCALCONFIG.as_bytes(),
            "{program}"
        );
    }
    assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
    assert!(!root.join("userdata/.save-relay-install-wrap.lock").exists());
}

#[test]
fn install_refuses_while_another_run_holds_the_lock() {
    let root = temp_root();
    let file = add_user(&root, "42", LOCALCONFIG);
    let lock = root.join("userdata/.save-relay-install-wrap.lock");
    fs::write(&lock, "").unwrap();
    let result = install(
        &root,
        &[480],
        Path::new(BIN),
        false,
        false,
        Platform::Linux,
        NOW,
    );
    assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
    assert_eq!(fs::read(&file).unwrap(), LOCALCONFIG.as_bytes());
    assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
    // The lock is another run's to remove, so it stays.
    assert!(lock.exists());
}

#[test]
fn install_removes_its_lock_when_it_ends() {
    let root = temp_root();
    add_user(&root, "42", LOCALCONFIG);
    install(
        &root,
        &[480],
        Path::new(BIN),
        false,
        false,
        Platform::Linux,
        NOW,
    )
    .unwrap();
    assert!(!root.join("userdata/.save-relay-install-wrap.lock").exists());

    // A run that fails also lets go of its lock.
    let other = temp_root();
    add_user(
        &other,
        "42",
        "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n",
    );
    let result = install(
        &other,
        &[480],
        Path::new(BIN),
        false,
        false,
        Platform::Linux,
        NOW,
    );
    assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
    assert!(!other
        .join("userdata/.save-relay-install-wrap.lock")
        .exists());
}

/// Two libraries: apps 480 and 1145360 installed in the first, 2000001 in the second.
const LIBRARYFOLDERS: &str = r#""libraryfolders"
{
	"0"
	{
		"path"		"/steam/library-one"
		"apps"
		{
			"480"		"1000"
			"1145360"		"2000"
		}
	}
	"1"
	{
		"path"		"/steam/library-two"
		"apps"
		{
			"2000001"		"3000"
		}
	}
}
"#;

#[test]
fn installed_among_keeps_only_the_known_apps_that_are_installed() {
    let libraries = parse_libraries(LIBRARYFOLDERS).unwrap();
    // 999 is known but not installed; 3000 is installed but not known; 480 repeats.
    assert_eq!(
        installed_among(&libraries, [2000001, 999, 480, 480, 1145360, 3000]),
        vec![480, 1145360, 2000001]
    );
    assert!(installed_among(&libraries, [999]).is_empty());
}

#[test]
fn all_known_writes_only_the_installed_app_of_two_in_the_index() {
    let root = temp_root();
    let file = add_user(&root, "42", LOCALCONFIG);
    let libraries = parse_libraries(LIBRARYFOLDERS).unwrap();
    // The index knows 480 and 1659420; 1659420 is not installed, so only 480 is written.
    let app_ids = installed_among(&libraries, [1659420, 480]);
    assert_eq!(app_ids, vec![480]);
    install(
        &root,
        &app_ids,
        Path::new(BIN),
        false,
        false,
        Platform::Linux,
        NOW,
    )
    .unwrap();
    assert_eq!(option_in(&file, 480), Some(WRAP.to_string()));
    assert_eq!(option_in(&file, 1659420), None);
}

#[test]
fn install_remove_takes_only_save_relay_back_out() {
    let root = temp_root();
    let file = add_user(&root, "42", LOCALCONFIG);
    install(
        &root,
        &[480, 1145360],
        Path::new(BIN),
        false,
        false,
        Platform::Linux,
        NOW,
    )
    .unwrap();

    let data = install(
        &root,
        &[480, 1145360],
        Path::new(BIN),
        true,
        false,
        Platform::Linux,
        NOW,
    )
    .unwrap();
    assert_eq!(data.changed.len(), 2);
    // The second backup is named a second later: this run's `now` is taken by the first.
    assert!(data.backups[0].ends_with("localconfig.vdf.save-relay-20261031T195101Z.bak"));
    assert_eq!(option_in(&file, 480), Some(String::new()));
    assert_eq!(
        option_in(&file, 1145360),
        Some("gamemoderun %command% -fps".to_string())
    );

    // Nothing left to remove the second time.
    let again = install(
        &root,
        &[480, 1145360],
        Path::new(BIN),
        true,
        false,
        Platform::Linux,
        NOW,
    )
    .unwrap();
    assert!(again.changed.is_empty());
}

#[test]
fn install_writes_every_user_and_skips_one_without_a_file() {
    let root = temp_root();
    let first = add_user(&root, "42", LOCALCONFIG);
    let second = add_user(&root, "43", LOCALCONFIG);
    fs::create_dir_all(root.join("userdata").join("44")).unwrap();

    let data = install(
        &root,
        &[480],
        Path::new(BIN),
        false,
        false,
        Platform::Linux,
        NOW,
    )
    .unwrap();
    let users: Vec<&str> = data.changed.iter().map(|c| c.user_id.as_str()).collect();
    assert_eq!(users, ["42", "43"]);
    assert_eq!(option_in(&first, 480), Some(WRAP.to_string()));
    assert_eq!(option_in(&second, 480), Some(WRAP.to_string()));
    assert_eq!(data.backups.len(), 2);
}

#[test]
fn install_without_a_steam_user_says_so() {
    let root = temp_root();
    let data = install(
        &root,
        &[480],
        Path::new(BIN),
        false,
        false,
        Platform::Linux,
        NOW,
    )
    .unwrap();
    assert!(data.changed.is_empty());
    assert!(data.backups.is_empty());
    assert!(
        data.message.contains("sign in to Steam once"),
        "{}",
        data.message
    );
}

#[test]
fn install_quotes_the_windows_program_in_the_file() {
    let root = temp_root();
    let file = add_user(&root, "42", LOCALCONFIG);
    let program = r"C:\Program Files\save-relay\save-relay.exe";
    install(
        &root,
        &[480],
        Path::new(program),
        false,
        false,
        Platform::Windows,
        NOW,
    )
    .unwrap();
    assert_eq!(
        option_in(&file, 480),
        Some(format!(r#""{program}" wrap -- %command%"#))
    );
}

#[test]
fn install_refuses_a_file_it_cannot_read_and_writes_nothing() {
    let root = temp_root();
    let broken = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n";
    let file = add_user(&root, "42", broken);
    let result = install(
        &root,
        &[480],
        Path::new(BIN),
        false,
        false,
        Platform::Linux,
        NOW,
    );
    assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
    assert_eq!(fs::read(&file).unwrap(), broken.as_bytes());
    assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn install_reads_every_file_before_it_writes_any() {
    let root = temp_root();
    let first = add_user(&root, "42", LOCALCONFIG);
    let broken = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n";
    let second = add_user(&root, "43", broken);
    let result = install(
        &root,
        &[480],
        Path::new(BIN),
        false,
        false,
        Platform::Linux,
        NOW,
    );
    assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
    assert_eq!(fs::read_to_string(&first).unwrap(), LOCALCONFIG);
    assert_eq!(fs::read(&second).unwrap(), broken.as_bytes());
    assert_eq!(fs::read_dir(first.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn steam_running_is_false_without_a_pid_file() {
    let home = temp_root();
    assert!(!steam_running(Platform::Linux, &home).unwrap());
    assert!(!steam_running(Platform::Android, &home).unwrap());
}

#[test]
fn steam_running_refuses_a_pid_file_that_is_not_a_number() {
    let home = temp_root();
    fs::create_dir_all(home.join(".steam")).unwrap();
    fs::write(home.join(".steam/steam.pid"), "not a pid").unwrap();
    assert!(matches!(
        steam_running(Platform::Linux, &home),
        Err(Error::Io(_))
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn steam_running_sees_a_live_pid_on_linux() {
    let home = temp_root();
    fs::create_dir_all(home.join(".steam")).unwrap();
    fs::write(
        home.join(".steam/steam.pid"),
        std::process::id().to_string(),
    )
    .unwrap();
    assert!(steam_running(Platform::Linux, &home).unwrap());
}

#[cfg(target_os = "linux")]
#[test]
fn steam_running_is_false_for_a_dead_pid_on_linux() {
    let home = temp_root();
    fs::create_dir_all(home.join(".steam")).unwrap();
    // A child that has exited and been reaped: its pid names no process any more.
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    fs::write(home.join(".steam/steam.pid"), pid.to_string()).unwrap();
    assert!(!steam_running(Platform::Linux, &home).unwrap());
}
