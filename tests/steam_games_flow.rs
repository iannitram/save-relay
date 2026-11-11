#[path = "common/steam_games.rs"]
mod steam_games_fixture;

use save_relay::api::{InstallWrapData, InstallWrapSkipReason, SteamGamesData};
use save_relay::cli::{run_with_net, Net};
use save_relay::clients::ClientSet;
use save_relay::creds::{CredentialStore, MemoryStore, Secret};
use save_relay::http::{Request, Response, Transport};
use save_relay::layout::Layout;
use save_relay::local::{Config, LocalStore};
use save_relay::platform::{AppDirs, PathVars, Platform};
use save_relay::remote::MemoryRemote;
use save_relay::state::ServerState;
use save_relay::{Error, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const CLOUD_APP: u32 = 504230;
const NO_CLOUD_APP: u32 = 504231;
const UNKNOWN_APP: u32 = 504232;
const GAME: &str = "steam-504231";
const SERVER: &str = "https://cloud.example.org";
const USER: &str = "f78-test";
const CHILD_GATE: &str = "SAVE_RELAY_F78_CHILD";
const CHILD_SAVE: &str = "SAVE_RELAY_F78_SAVE";
const CHILD_BYTES: &str = "SAVE_RELAY_F78_BYTES";
const SAVE_BYTES: &str = "a made-up save from the fixture game";
const LUDUSAVI_FIXTURE: &str = include_str!("fixtures/ludusavi/steam-games.yaml");
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Child game used by the wrap flow. It writes a fixture save and returns its own exit code.
#[test]
#[ignore = "run as a child process by the F78 flow test"]
fn child_game() {
    match std::env::var(CHILD_GATE).as_deref() {
        Ok("worker") => {
            let save = std::env::var(CHILD_SAVE).unwrap();
            let bytes = std::env::var(CHILD_BYTES).unwrap();
            fs::write(save, bytes).unwrap();
            std::process::exit(7);
        }
        Ok("1") => {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "child_game", "--ignored"])
                .env(CHILD_GATE, "worker")
                .output()
                .unwrap();
            std::process::exit(output.status.code().unwrap_or(1));
        }
        _ => {}
    }
}

fn child_command() -> Vec<String> {
    std::env::set_var(CHILD_GATE, "1");
    vec![
        std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        "--exact".into(),
        "child_game".into(),
        "--ignored".into(),
    ]
}

struct StoreTransport {
    sent: Mutex<Vec<Request>>,
    replies: Mutex<VecDeque<Response>>,
}

impl StoreTransport {
    fn new() -> Self {
        let store = [
            (
                CLOUD_APP,
                r#"{"504230":{"success":true,"data":{"type":"game","categories":[{"id":23}]}}}"#,
            ),
            (
                NO_CLOUD_APP,
                r#"{"504231":{"success":true,"data":{"type":"game","categories":[]}}}"#,
            ),
            (UNKNOWN_APP, r#"{"504232":{"success":false}}"#),
            (
                504233,
                r#"{"504233":{"success":true,"data":{"type":"tool"}}}"#,
            ),
        ];
        let mut replies = store
            .into_iter()
            .map(|(_, text)| Response::new(200, text.as_bytes().to_vec()))
            .collect::<VecDeque<_>>();
        replies.push_back(
            Response::new(200, LUDUSAVI_FIXTURE.as_bytes().to_vec()).header("ETag", "f78-fixture"),
        );
        Self {
            sent: Mutex::new(Vec::new()),
            replies: Mutex::new(replies),
        }
    }
}

impl Transport for StoreTransport {
    fn send(&self, request: &Request) -> Result<Response> {
        self.sent.lock().unwrap().push(request.clone());
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| Error::Unreachable(format!("unexpected request to {}", request.url)))
    }
}

struct Base(PathBuf);
impl Base {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "save-relay-f78-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Base {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Machine {
    steam: PathBuf,
    vars: PathVars,
    store: LocalStore,
    creds: MemoryStore,
    remote: MemoryRemote,
    now: u64,
}

impl Machine {
    fn new(base: &Base) -> Self {
        let root = base.0.join("machine");
        let home = root.join("home");
        let platform = Platform::current();
        let (data, config) = match platform {
            Platform::Linux => (home.join(".local/share"), home.join(".config")),
            Platform::Mac => {
                let support = home.join("Library/Application Support");
                (support.clone(), support)
            }
            Platform::Windows => (root.join("local-app-data"), root.join("app-data")),
            Platform::Android | Platform::Ios => unreachable!(),
        };
        let vars = PathVars {
            home: home.clone(),
            data,
            config,
            documents: Some(home.join("Documents")),
        };
        let store = LocalStore::new(AppDirs::from_vars(&vars));
        let creds = MemoryStore::new();
        store
            .save_config(&Config {
                format: 1,
                server: SERVER.into(),
                user: USER.into(),
                device: "f78-test".into(),
                root: "save-relay".into(),
            })
            .unwrap();
        creds
            .set(
                &save_relay::creds::account(SERVER, USER).unwrap(),
                &Secret::new("made-up test secret"),
            )
            .unwrap();
        let steam = steam_games_fixture::fixture(&root);
        let save_dir = steam.join("steamapps/common/No Cloud Game/save");
        fs::create_dir_all(save_dir).unwrap();
        Self {
            steam,
            vars,
            store,
            creds,
            remote: MemoryRemote::new(),
            now: save_relay::stamp::now(),
        }
    }

    fn run(&self, transport: Arc<dyn Transport>, args: &[&str]) -> (i32, String) {
        let args = args
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        let net = Net {
            transport,
            open_browser: &refuse_browser,
            clients: ClientSet::shipped(),
            game_mode: false,
            clock: test_clock,
        };
        let mut stdout = Vec::new();
        let mut stdin = &b""[..];
        let code = run_with_net(
            &args,
            &mut stdin,
            &mut stdout,
            &self.remote,
            &self.creds,
            &self.store,
            &self.vars,
            self.now,
            &net,
        );
        (code, String::from_utf8(stdout).unwrap())
    }
}

fn test_clock() -> u64 {
    save_relay::stamp::now()
}
fn refuse_browser(_: &str) -> Result<()> {
    Err(Error::Unreachable("no browser in this test".into()))
}

/// Restore process environment after the installer and wrapped child subprocesses.
struct Environment(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl Environment {
    fn set(values: &[(&'static str, &std::ffi::OsStr)]) -> Self {
        let keys = values.iter().map(|(key, _)| *key).collect::<Vec<_>>();
        let saved = keys
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        for (key, value) in values {
            std::env::set_var(key, value);
        }
        Self(saved)
    }
}
impl Drop for Environment {
    fn drop(&mut self) {
        for (key, value) in &self.0 {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn data<T: DeserializeOwned>(text: &str) -> T {
    let envelope: Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(envelope["ok"], true, "{text}");
    serde_json::from_value(envelope["data"].clone()).unwrap()
}

#[test]
fn m16_lists_wraps_and_syncs_only_the_no_cloud_fixture_game() {
    let base = Base::new();
    let machine = Machine::new(&base);
    let _steam_root = steam_games_fixture::set_root(&machine.steam);
    let save_path = machine
        .steam
        .join("steamapps/common/No Cloud Game/save/slot1.sav");
    let mut vars = vec![
        ("SteamAppId", std::ffi::OsStr::new("504231")),
        (CHILD_GATE, std::ffi::OsStr::new("1")),
        ("HOME", machine.vars.home.as_os_str()),
        ("USERPROFILE", machine.vars.home.as_os_str()),
        (CHILD_SAVE, save_path.as_os_str()),
        ("SAVE_RELAY_F78_BYTES", std::ffi::OsStr::new(SAVE_BYTES)),
    ];
    match Platform::current() {
        Platform::Linux => {
            vars.push(("XDG_DATA_HOME", machine.vars.data.as_os_str()));
            vars.push(("XDG_CONFIG_HOME", machine.vars.config.as_os_str()));
        }
        Platform::Windows => {
            vars.push(("LOCALAPPDATA", machine.vars.data.as_os_str()));
            vars.push(("APPDATA", machine.vars.config.as_os_str()));
        }
        Platform::Mac | Platform::Android | Platform::Ios => {}
    }
    let _environment = Environment::set(&vars);

    let transport = Arc::new(StoreTransport::new());
    let (code, output) = machine.run(transport.clone(), &["steam-games", "--json"]);
    assert_eq!(code, 0, "{output}");
    let listing: SteamGamesData = data(&output);
    assert_eq!(listing.games.len(), 3);
    assert_eq!(listing.unknown, 1);
    assert_eq!(listing.games[0].name, "Cloud Game");
    assert_eq!(
        listing.games[0].steam_cloud,
        save_relay::api::SteamCloud::Yes
    );
    assert_eq!(listing.games[1].name, "No Cloud Game");
    assert_eq!(
        listing.games[1].steam_cloud,
        save_relay::api::SteamCloud::No
    );
    assert_eq!(listing.games[2].name, "Unknown Game");
    assert_eq!(
        listing.games[2].steam_cloud,
        save_relay::api::SteamCloud::Unknown
    );
    assert!(
        listing
            .games
            .iter()
            .find(|game| game.app_id == CLOUD_APP)
            .unwrap()
            .wrapped
    );
    assert!(
        !listing
            .games
            .iter()
            .find(|game| game.app_id == NO_CLOUD_APP)
            .unwrap()
            .wrapped
    );

    // The installer must run as the shipped binary: the test executable has another basename.
    let mut install = Command::new(env!("CARGO_BIN_EXE_save-relay"));
    install
        .args(["install-wrap", "--no-steam-cloud", "--json"])
        .env("SAVE_RELAY_STEAM_DIR", &machine.steam)
        .env("HOME", &machine.vars.home)
        .env("USERPROFILE", &machine.vars.home)
        .env("APPDATA", &machine.vars.config)
        .env("LOCALAPPDATA", &machine.vars.data)
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME");
    if Platform::current() == Platform::Linux {
        install
            .env("XDG_DATA_HOME", &machine.vars.data)
            .env("XDG_CONFIG_HOME", &machine.vars.config);
    }
    let installed = install.output().unwrap();
    let install_stdout = String::from_utf8(installed.stdout).unwrap();
    assert_eq!(installed.status.code(), Some(0), "{install_stdout}");
    let report: InstallWrapData = data(&install_stdout);
    assert_eq!(
        report
            .changed
            .iter()
            .map(|item| item.app_id)
            .collect::<Vec<_>>(),
        [NO_CLOUD_APP]
    );
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].app_id, UNKNOWN_APP);
    assert_eq!(
        report.skipped[0].reason,
        InstallWrapSkipReason::SteamCloudUnknown
    );

    let (code, output) = machine.run(
        Arc::new(StoreTransport::empty()),
        &["steam-games", "--json"],
    );
    assert_eq!(code, 0, "{output}");
    let after: SteamGamesData = data(&output);
    assert!(
        after
            .games
            .iter()
            .find(|game| game.app_id == NO_CLOUD_APP)
            .unwrap()
            .wrapped
    );
    assert!(
        after
            .games
            .iter()
            .find(|game| game.app_id == CLOUD_APP)
            .unwrap()
            .wrapped
    );
    assert!(
        !after
            .games
            .iter()
            .find(|game| game.app_id == UNKNOWN_APP)
            .unwrap()
            .wrapped
    );

    let mut args = vec!["wrap".to_string(), "--json".to_string(), "--".to_string()];
    args.extend(child_command());
    let net = Net {
        transport: Arc::new(StoreTransport::empty()),
        open_browser: &refuse_browser,
        clients: ClientSet::shipped(),
        game_mode: false,
        clock: test_clock,
    };
    let mut stdout = Vec::new();
    let mut stdin = &b""[..];
    let code = run_with_net(
        &args,
        &mut stdin,
        &mut stdout,
        &machine.remote,
        &machine.creds,
        &machine.store,
        &machine.vars,
        machine.now,
        &net,
    );
    assert_eq!(code, 7, "{}", String::from_utf8_lossy(&stdout));

    let layout = Layout::new("save-relay").unwrap();
    let state_text =
        String::from_utf8(machine.remote.read(&layout.state_path(GAME)).unwrap()).unwrap();
    let state = ServerState::from_json(&state_text).unwrap();
    let remote_save = format!("{}/slot1.sav", layout.entry_dir(GAME, &state.entry));
    assert_eq!(
        machine.remote.read(&remote_save).unwrap(),
        SAVE_BYTES.as_bytes()
    );
}

impl StoreTransport {
    fn empty() -> Self {
        Self {
            sent: Mutex::new(Vec::new()),
            replies: Mutex::new(VecDeque::new()),
        }
    }
}
