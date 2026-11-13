//! `wrap` and the ludusavi path end to end (M12, M13, F65): a fixture Steam folder
//! (`tests/fixtures/steam/`), the fixture ludusavi data (`tests/fixtures/ludusavi/wrap.yaml`)
//! served by a fake transport, `SteamAppId` set, and a wrapped program that is this test binary
//! run again (`child_game`: it notes what it found, writes a save, and exits 7). Each machine has
//! its own temp folders and Steam folder; both talk to one `MemoryRemote`.
//!
//! The environment and the clock are process-wide, so every test takes `serial()`. The clock
//! moves on `STEP` seconds at each reading, so a wait ends without anyone waiting for long.

use save_relay::api::{AnswerData, InstallWrapData, WhichData, WhichSource};
use save_relay::cli::{run_with, run_with_net, Net};
use save_relay::clients::ClientSet;
use save_relay::creds::{self, CredentialStore, MemoryStore, Secret};
use save_relay::http::{Request, Response, Transport};
use save_relay::layout::Layout;
use save_relay::local::{Config, GameRecord, LocalStore};
use save_relay::ludusavi;
use save_relay::manifest::Manifest;
use save_relay::pending::{Answer, ChoiceKind};
use save_relay::platform::{AppDirs, PathVars, Platform};
use save_relay::presence::{self, Mark};
use save_relay::remote::{MemoryRemote, Precondition, Remote};
use save_relay::state::ServerState;
use save_relay::steam::launch::steam_running;
use save_relay::{stamp, Error};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const APP_ID: &str = "1000101";
const GAME: &str = "steam-1000101";
const T0: u64 = 1_793_476_260; // 2026-10-31T19:51:00Z
const SERVER: &str = "https://cloud.example.org";
const USER: &str = "alice";
const PASSWORD: &str = "xK9p-Q2mR-7vLd-W4tY-hN8z";
const DATA: &[u8] = include_bytes!("fixtures/ludusavi/wrap.yaml");
static NEXT: AtomicUsize = AtomicUsize::new(0);

// ---- the clock and the process environment ---------------------------------------------------

static NOW: AtomicU64 = AtomicU64::new(T0);
static STEP: AtomicU64 = AtomicU64::new(10);
static SERIAL: Mutex<()> = Mutex::new(());

/// Takes the lock on the process-wide state and puts the clock back to T0, 10 seconds a reading.
fn serial() -> Environment {
    let guard = SERIAL.lock().unwrap_or_else(|poison| poison.into_inner());
    NOW.store(T0, Ordering::SeqCst);
    STEP.store(10, Ordering::SeqCst);
    let saved = [
        "SAVE_RELAY_STEAM_DIR",
        "SteamAppId",
        CHILD_GATE,
        CHILD_SAVE,
        CHILD_SEEN,
        CHILD_WRITE,
    ]
    .map(|key| (key, std::env::var_os(key)));
    Environment {
        saved,
        _guard: guard,
    }
}

struct Environment {
    saved: [(&'static str, Option<std::ffi::OsString>); 6],
    _guard: MutexGuard<'static, ()>,
}

impl Drop for Environment {
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

// Kill and reap even when a timeout or an assertion unwinds the test.
struct RunningChild(Option<Child>);

impl Drop for RunningChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn bounded_output(command: &mut Command) -> Output {
    let mut child = RunningChild(Some(command.spawn().expect("start fixture process")));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            return child.0.take().unwrap().wait_with_output().unwrap();
        }
        assert!(Instant::now() < deadline, "fixture process timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn clock() -> u64 {
    NOW.fetch_add(STEP.load(Ordering::SeqCst), Ordering::SeqCst)
}

// ---- the wrapped program ---------------------------------------------------------------------

const CHILD_GATE: &str = "SAVE_RELAY_WRAP_TEST_CHILD";
const CHILD_SAVE: &str = "SAVE_RELAY_WRAP_TEST_SAVE";
const CHILD_SEEN: &str = "SAVE_RELAY_WRAP_TEST_SEEN";
const CHILD_WRITE: &str = "SAVE_RELAY_WRAP_TEST_WRITE";
const NOTHING: &str = "<nothing>";

/// The game: it notes what its save held when it started, writes a new one when told to, and
/// exits 7. Ignored and gated, so only `wrap` starts it.
#[test]
#[ignore = "run as a child process by the tests in this file"]
fn child_game() {
    match std::env::var(CHILD_GATE).as_deref() {
        Ok("worker") => {}
        Ok("1") => {
            let output = bounded_output(
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "child_game", "--ignored"])
                    .env(CHILD_GATE, "worker"),
            );
            std::process::exit(output.status.code().unwrap_or(1));
        }
        _ => return,
    }
    let save = std::env::var(CHILD_SAVE).expect("the save path");
    let seen = std::env::var(CHILD_SEEN).expect("the notes path");
    let found = fs::read_to_string(&save).unwrap_or_else(|_| NOTHING.to_string());
    fs::write(seen, found).expect("write the notes");
    if let Ok(bytes) = std::env::var(CHILD_WRITE) {
        fs::write(&save, bytes).expect("write the save");
    }
    std::process::exit(7);
}

/// This test binary, running only `child_game`.
fn child_command() -> Vec<String> {
    std::env::set_var(CHILD_GATE, "1");
    let exe = std::env::current_exe().expect("the test binary's path");
    vec![
        exe.to_string_lossy().into_owned(),
        "--exact".into(),
        "child_game".into(),
        "--ignored".into(),
    ]
}

// ---- the ludusavi data over a fake transport -------------------------------------------------

/// Serves the fixture ludusavi data at its address and counts the requests.
struct Served {
    requests: Arc<AtomicUsize>,
}

impl Transport for Served {
    fn send(&self, request: &Request) -> Result<Response, Error> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.url, ludusavi::MANIFEST_URL);
        Ok(Response::new(200, DATA).header("ETag", "\"wrap-fixture\""))
    }
}

// ---- a machine -------------------------------------------------------------------------------

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// One machine: its folders, its set-up config, a copy of the fixture Steam folder, and the
/// saves folder the ludusavi data names for the game on this OS (Linux: the Proton prefix's
/// Documents; elsewhere: the install folder's `Saves`).
struct Machine {
    root: PathBuf,
    vars: PathVars,
    store: LocalStore,
    creds: MemoryStore,
    steam: PathBuf,
    requests: Arc<AtomicUsize>,
}

impl Machine {
    fn new(base: &Path, device: &str) -> Machine {
        let root = base.join(device);
        let vars = PathVars {
            home: root.join("home"),
            data: root.join("data"),
            config: root.join("config"),
            documents: Some(root.join("home").join("Documents")),
        };
        let store = LocalStore::new(AppDirs::from_vars(&vars));
        store
            .save_config(&Config {
                format: 1,
                server: SERVER.into(),
                user: USER.into(),
                root: "save-relay".into(),
                device: device.into(),
            })
            .unwrap();
        let creds = MemoryStore::new();
        creds
            .set(
                &creds::account(SERVER, USER).unwrap(),
                &Secret::new(PASSWORD),
            )
            .unwrap();
        let steam = root.join("steam");
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/steam"),
            &steam,
        );
        let libraries = steam.join("steamapps/libraryfolders.vdf");
        let text = fs::read_to_string(&libraries)
            .unwrap()
            .replace("<library>", &steam.to_string_lossy().replace('\\', "/"));
        fs::write(libraries, text).unwrap();
        let machine = Machine {
            root,
            vars,
            store,
            creds,
            steam,
            requests: Arc::new(AtomicUsize::new(0)),
        };
        fs::create_dir_all(machine.saves_dir()).unwrap();
        machine
    }

    fn saves_dir(&self) -> PathBuf {
        if cfg!(target_os = "linux") {
            self.steam.join(
                "steamapps/compatdata/1000101/pfx/drive_c/users/steamuser/Documents/WrapQuest",
            )
        } else {
            self.steam.join("steamapps/common/WrapQuest/Saves")
        }
    }

    /// A second folder the data names for the game on this OS, where one exists (not on a Mac).
    #[cfg(not(target_os = "macos"))]
    fn second_saves_dir(&self) -> PathBuf {
        if cfg!(target_os = "linux") {
            self.steam.join("steamapps/common/WrapQuest/Saves")
        } else {
            self.root.join("home/Documents/WrapQuest")
        }
    }

    /// The saves folder as the JSON says it: `/`-separated.
    fn saves_text(&self) -> String {
        self.saves_dir().to_string_lossy().replace('\\', "/")
    }

    /// One command, as the process would run it with this machine's Steam folder and the app id
    /// Steam sets: the exit code and what went to standard output.
    fn run(&self, remote: &MemoryRemote, args: Vec<String>) -> (i32, String) {
        std::env::set_var("SAVE_RELAY_STEAM_DIR", &self.steam);
        std::env::set_var("SteamAppId", APP_ID);
        // On Linux the game runs under Proton, as Steam says with this variable at launch.
        if cfg!(target_os = "linux") {
            let prefix = self.steam.join("steamapps/compatdata").join(APP_ID);
            std::env::set_var("STEAM_COMPAT_DATA_PATH", prefix);
        }
        let refuse: &dyn Fn(&str) -> save_relay::Result<()> =
            &|_: &str| Err(Error::Unreachable("no browser in this test".into()));
        let net = Net {
            transport: Arc::new(Served {
                requests: self.requests.clone(),
            }),
            open_browser: refuse,
            clients: ClientSet::shipped(),
            game_mode: false,
            clock,
        };
        let mut stdin = Cursor::new(Vec::<u8>::new());
        let mut stdout = Vec::<u8>::new();
        let code = run_with_net(
            &args,
            &mut stdin,
            &mut stdout,
            remote,
            &self.creds,
            &self.store,
            &self.vars,
            T0,
            &net,
        );
        (code, String::from_utf8(stdout).unwrap())
    }

    /// `wrap -- <the game>` with no manifest of the person's own. The game writes `write` as its
    /// save when given one. Returns the exit code, what wrap printed, and what the game's save
    /// held when it started.
    fn wrap(&self, remote: &MemoryRemote, write: Option<&str>) -> (i32, String, String) {
        let save = self.saves_dir().join("slot1.sav");
        let seen = self.root.join("seen.txt");
        let _ = fs::remove_file(&seen);
        let mut args = vec!["wrap".to_string(), "--".to_string()];
        args.extend(child_command());
        std::env::set_var(CHILD_SAVE, &save);
        std::env::set_var(CHILD_SEEN, &seen);
        match write {
            Some(bytes) => std::env::set_var(CHILD_WRITE, bytes),
            None => std::env::remove_var(CHILD_WRITE),
        }
        let (code, stdout) = self.run(remote, args);
        let seen = fs::read_to_string(&seen).expect("the game ran and left its notes");
        (code, stdout, seen)
    }
}

/// Temp folders for a test's machines, removed afterwards.
struct Base(PathBuf);

impl Base {
    fn new(name: &str) -> Base {
        let root = std::env::temp_dir().join(format!(
            "save-relay-wrapit-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Base(root)
    }
}

impl Drop for Base {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn layout() -> Layout {
    Layout::new("save-relay").unwrap()
}

fn data<T: DeserializeOwned>(text: &str) -> T {
    let value: Value = serde_json::from_str(text.trim())
        .unwrap_or_else(|error| panic!("not one JSON envelope ({error}): {text}"));
    assert_eq!(value["ok"], true, "{text}");
    serde_json::from_value(value["data"].clone())
        .unwrap_or_else(|error| panic!("data does not fit ({error}): {text}"))
}

fn error_code(text: &str) -> String {
    let value: Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(value["ok"], false, "{text}");
    value["error"]["code"].as_str().unwrap().to_string()
}

fn server_has_save(remote: &MemoryRemote, bytes: &str) -> bool {
    let layout = layout();
    let state = remote
        .read(&layout.state_path(GAME))
        .expect("current revision");
    let state = ServerState::from_json(std::str::from_utf8(&state).unwrap()).unwrap();
    let path = format!("{}/slot1.sav", layout.entry_dir(GAME, &state.entry));
    remote.read(&path).as_deref() == Some(bytes.as_bytes())
}

fn mark_on_server(remote: &MemoryRemote) -> Option<Mark> {
    let bytes = remote.read(&layout().playing_path(GAME))?;
    Some(Mark::from_json(std::str::from_utf8(&bytes).unwrap()).expect("valid server mark"))
}

/// The handheld wraps the game once on an empty server: its save is on the server afterwards.
fn handheld_wraps_first(base: &Base, remote: &MemoryRemote) -> Machine {
    let handheld = Machine::new(&base.0, "handheld");
    let (code, _, _) = handheld.wrap(remote, Some("handheld save"));
    assert_eq!(code, 7);
    handheld
}

// ---- M12: wrap -------------------------------------------------------------------------------

#[test]
fn m12_wrap_pulls_starts_the_game_pushes_when_it_quits_and_exits_with_its_code() {
    let _serial = serial();
    let base = Base::new("push");
    let remote = MemoryRemote::new();
    let seed = Machine::new(&base.0, "seed");
    assert_eq!(seed.wrap(&remote, Some("server save")).0, 7);
    let handheld = Machine::new(&base.0, "handheld");

    let (code, stdout, seen) = handheld.wrap(&remote, Some("handheld save"));

    assert_eq!(code, 7, "the game's exit code is wrap's");
    assert_eq!(
        stdout, "",
        "wrap prints nothing: the game's output is the only output"
    );
    assert_eq!(seen, "server save", "pull finished before the game started");
    assert!(
        server_has_save(&remote, "handheld save"),
        "the save appeared on the server: {:?}",
        remote.file_paths()
    );
    assert!(
        mark_on_server(&remote).is_none(),
        "the push cleared the mark the pull left"
    );
    // The folder came from the ludusavi data, which is now cached on the machine.
    assert_eq!(handheld.requests.load(Ordering::SeqCst), 1);
    let cache = ludusavi::Cache::new(&AppDirs::from_vars(&handheld.vars));
    assert!(cache.index().unwrap().is_some());
}

#[test]
fn m12_a_second_machines_wrap_gets_the_save_before_the_game_starts() {
    let _serial = serial();
    let base = Base::new("second");
    let remote = MemoryRemote::new();
    handheld_wraps_first(&base, &remote);
    let desktop = Machine::new(&base.0, "desktop");

    let (code, _, seen) = desktop.wrap(&remote, None);

    assert_eq!(code, 7);
    assert_eq!(seen, "handheld save", "the game started on the pulled save");
}

#[test]
fn m12_a_held_game_waits_for_an_answer_and_play_anyway_pulls_past_the_mark() {
    let _serial = serial();
    let base = Base::new("anyway");
    let remote = MemoryRemote::new();
    handheld_wraps_first(&base, &remote);
    // The handheld is suspended with the game open: its mark is on the server.
    let mark = Mark {
        format: presence::FORMAT,
        device: "handheld".into(),
        since: stamp::rfc3339(T0),
        revision_read: Some(1),
    };
    remote
        .put(
            &layout().playing_path(GAME),
            mark.to_json().as_bytes(),
            &Precondition::Any,
        )
        .unwrap();
    let desktop = Machine::new(&base.0, "desktop");
    // One fake second per reading, and the answer comes at once: the wait stays short.
    STEP.store(1, Ordering::SeqCst);

    // The panel's part: when the choice is pending, run `answer play-anyway --json`.
    let vars = desktop.vars.clone();
    let (code, seen, choice) = std::thread::scope(|scope| {
        let answerer = scope.spawn(move || {
            let store = LocalStore::new(AppDirs::from_vars(&vars));
            for _ in 0..3000 {
                if store.load_pending(GAME).unwrap().is_some() {
                    let mut stdout = Vec::new();
                    let code = run_with(
                        &["answer", GAME, "play-anyway", "--json"].map(String::from),
                        &mut Cursor::new(Vec::<u8>::new()),
                        &mut stdout,
                        &MemoryRemote::new(),
                        &MemoryStore::new(),
                        &store,
                        &vars,
                        T0,
                    );
                    let stdout = String::from_utf8(stdout).unwrap();
                    assert_eq!(code, 0, "{stdout}");
                    let report: AnswerData = data(&stdout);
                    assert_eq!(report.game, GAME);
                    assert_eq!(report.answer, Answer::PlayAnyway);
                    return report;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("wrap never wrote a pending choice");
        });
        let (code, _, seen) = desktop.wrap(&remote, Some("desktop save"));
        let choice = answerer.join().unwrap();
        (code, seen, choice)
    });

    assert_eq!(
        choice.kind,
        ChoiceKind::Held,
        "the held game was the choice"
    );
    assert_eq!(code, 7);
    assert_eq!(seen, "handheld save", "play anyway pulled past the mark");
    assert!(server_has_save(&remote, "desktop save"), "and pushed after");
    assert!(
        mark_on_server(&remote).is_none(),
        "the desktop's push cleared its mark"
    );
    assert_eq!(desktop.store.load_pending(GAME).unwrap(), None);
}

#[test]
fn m12_a_held_game_with_no_answer_in_the_wait_starts_on_the_local_saves() {
    let _serial = serial();
    let base = Base::new("no-answer");
    let remote = MemoryRemote::new();
    let handheld = handheld_wraps_first(&base, &remote);
    let desktop = Machine::new(&base.0, "desktop");
    assert_eq!(desktop.wrap(&remote, None).2, "handheld save");
    assert_eq!(handheld.wrap(&remote, Some("server newer save")).0, 7);
    fs::write(desktop.saves_dir().join("slot1.sav"), "desktop local save").unwrap();
    let mark = Mark {
        format: presence::FORMAT,
        device: "handheld".into(),
        since: stamp::rfc3339(T0),
        revision_read: Some(1),
    };
    remote
        .put(
            &layout().playing_path(GAME),
            mark.to_json().as_bytes(),
            &Precondition::Any,
        )
        .unwrap();
    let before: Vec<_> = remote
        .file_paths()
        .into_iter()
        .map(|path| {
            let bytes = remote.read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    // Each reading of the clock is a minute: the 120 second wait is over after two.
    STEP.store(60, Ordering::SeqCst);

    let (code, _, seen) = desktop.wrap(&remote, Some("desktop save after timeout"));

    assert_eq!(code, 7, "the game always starts");
    assert_eq!(
        seen, "desktop local save",
        "local progress survived the wait"
    );
    assert_eq!(remote.file_paths().len(), before.len());
    for (path, bytes) in before {
        assert_eq!(remote.read(&path), Some(bytes), "server changed: {path}");
    }
    assert_eq!(
        fs::read_to_string(desktop.saves_dir().join("slot1.sav")).unwrap(),
        "desktop save after timeout"
    );
    let still = mark_on_server(&remote).expect("the handheld's mark is still there");
    assert_eq!(
        still.device, "handheld",
        "the mark never expires or is taken"
    );
    let choice = desktop
        .store
        .load_pending(GAME)
        .unwrap()
        .expect("exit conflict is pending");
    assert_eq!(choice.kind, ChoiceKind::Conflict);
    assert_eq!(choice.answer, None);
}

// ---- M13: the ludusavi data, which and install-wrap -----------------------------------------

#[test]
fn m13_which_names_the_saves_folder_from_the_ludusavi_data() {
    let _serial = serial();
    let base = Base::new("which");
    let remote = MemoryRemote::new();
    let machine = Machine::new(&base.0, "handheld");

    let args = ["which", APP_ID, "--json"].map(String::from).to_vec();
    let (code, stdout) = machine.run(&remote, args);

    assert_eq!(code, 0, "{stdout}");
    let found: WhichData = data(&stdout);
    assert_eq!(found.app_id, 1_000_101);
    assert_eq!(found.game, GAME);
    assert_eq!(found.name, "Wrap Quest");
    assert_eq!(found.saves_dir, machine.saves_text());
    assert_eq!(found.include, ["*.sav"]);
    assert_eq!(found.source, WhichSource::Ludusavi);
}

#[test]
fn m13_which_refuses_a_game_the_data_does_not_know() {
    let _serial = serial();
    let base = Base::new("unknown");
    let remote = MemoryRemote::new();
    let machine = Machine::new(&base.0, "handheld");

    let args = ["which", "999999", "--json"].map(String::from).to_vec();
    let (code, stdout) = machine.run(&remote, args);

    assert_eq!(code, 2, "{stdout}");
    assert_eq!(error_code(&stdout), "save_location_unclear");
}

#[cfg(not(target_os = "macos"))]
#[test]
fn m13_which_stops_when_the_data_names_more_than_one_existing_folder() {
    let _serial = serial();
    let base = Base::new("two-folders");
    let remote = MemoryRemote::new();
    let machine = Machine::new(&base.0, "handheld");
    fs::create_dir_all(machine.second_saves_dir()).unwrap();

    let args = ["which", APP_ID, "--json"].map(String::from).to_vec();
    let (code, stdout) = machine.run(&remote, args);

    assert_eq!(code, 2, "it did not guess: {stdout}");
    assert_eq!(error_code(&stdout), "save_location_unclear");
}

#[test]
fn m13_a_manifest_of_my_own_always_wins_over_the_ludusavi_data() {
    let _serial = serial();
    let base = Base::new("own-manifest");
    let remote = MemoryRemote::new();
    let machine = Machine::new(&base.0, "handheld");
    let own = base.0.join("my-saves");
    fs::create_dir_all(&own).unwrap();
    let manifest = Manifest::from_json(
        r#"{"format":1,"id":"steam-1000101","name":"My Quest","saves_from":"launcher","saves":{},"include":["**/*.dat"]}"#,
    )
    .unwrap();
    machine
        .store
        .save_game(&GameRecord {
            manifest,
            saves_dir: Some(own.clone()),
        })
        .unwrap();

    let args = ["which", APP_ID, "--json"].map(String::from).to_vec();
    let (code, stdout) = machine.run(&remote, args);

    assert_eq!(code, 0, "{stdout}");
    let found: WhichData = data(&stdout);
    assert_eq!(found.source, WhichSource::Manifest);
    assert_eq!(found.name, "My Quest");
    assert_eq!(found.saves_dir, own.to_string_lossy().replace('\\', "/"));
    assert_eq!(found.include, ["**/*.dat"]);
}

#[test]
fn m13_install_wrap_dry_run_reports_the_launch_option_and_changes_nothing() {
    let _serial = serial();
    let base = Base::new("install");
    let machine = Machine::new(&base.0, "handheld");
    let config = machine.steam.join("userdata/12345/config/localconfig.vdf");
    let before = fs::read(&config).unwrap();
    // On Windows the check is `tasklist`, which sees the host's real Steam (on Linux and macOS
    // it reads pid files under the machine's own home, so it is false here). A dev PC with
    // Steam open gets the refusal instead, which must also change nothing.
    let steam_open = steam_running(Platform::current(), &machine.vars.home).unwrap();

    // The test binary is not named save-relay, which install-wrap requires, so this runs the
    // real program.
    let output = bounded_output(
        Command::new(env!("CARGO_BIN_EXE_save-relay"))
            .args(["install-wrap", "--dry-run", "--json", APP_ID])
            .env("SAVE_RELAY_STEAM_DIR", &machine.steam)
            .env("HOME", &machine.vars.home)
            .env("USERPROFILE", &machine.vars.home)
            .env("APPDATA", &machine.vars.config)
            .env("LOCALAPPDATA", &machine.vars.data)
            .env_remove("XDG_DATA_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    let stdout = String::from_utf8(output.stdout).unwrap();

    if steam_open {
        assert_eq!(output.status.code(), Some(2), "{stdout}");
        assert_eq!(error_code(&stdout), "steam_running");
        assert_eq!(fs::read(&config).unwrap(), before, "nothing was written");
        eprintln!("Steam is running on this host: checked the refusal, not the dry run");
        return;
    }
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    let report: InstallWrapData = data(&stdout);
    assert!(report.dry_run);
    assert_eq!(report.changed.len(), 1, "{report:?}");
    let change = &report.changed[0];
    assert_eq!(change.app_id, 1_000_101);
    assert_eq!(change.user_id, "12345");
    assert_eq!(change.before.as_deref(), Some("-windowed"));
    let after = change.after.as_deref().expect("the new launch option");
    assert!(
        after.ends_with("wrap -- %command% -windowed"),
        "the game's own option is kept: {after}"
    );
    assert!(report.backups.is_empty(), "a dry run writes no backup");
    assert_eq!(fs::read(&config).unwrap(), before, "nothing was written");
}
