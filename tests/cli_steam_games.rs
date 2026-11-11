#[path = "common/steam_games.rs"]
mod steam_games_fixture;

use save_relay::cli::{parse, run_with_net, Command, Net};
use save_relay::clients::ClientSet;
use save_relay::creds::MemoryStore;
use save_relay::http::{Request, Response, Transport};
use save_relay::local::{GameRecord, LocalStore};
use save_relay::manifest::Manifest;
use save_relay::platform::{AppDirs, PathVars};
use save_relay::remote::MemoryRemote;
use save_relay::{Error, Result};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const NOW: u64 = 1_793_476_260;
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct FakeTransport {
    sent: RefCell<Vec<Request>>,
    replies: RefCell<VecDeque<Response>>,
}

impl FakeTransport {
    fn new() -> Self {
        let store = [
            (
                504230,
                r#"{"504230":{"success":true,"data":{"type":"game","categories":[{"id":23}]}}}"#,
            ),
            (
                504231,
                r#"{"504231":{"success":true,"data":{"type":"game","categories":[]}}}"#,
            ),
            (504232, r#"{"504232":{"success":false}}"#),
            (
                504233,
                r#"{"504233":{"success":true,"data":{"type":"tool"}}}"#,
            ),
        ];
        let mut replies = store
            .into_iter()
            .map(|(_, body)| Response::new(200, body.as_bytes().to_vec()))
            .collect::<VecDeque<_>>();
        let manifest = "Cloud Game:\n  files:\n    <base>/save:\n      tags: [save]\n  steam:\n    id: 504230\nNo Cloud Game:\n  files:\n    <base>/save:\n      tags: [save]\n  steam:\n    id: 504231\nUnknown Game:\n  files:\n    <base>/save:\n      tags: [save]\n  steam:\n    id: 504232\nRuntime Tool:\n  files:\n    <base>/save:\n      tags: [save]\n  steam:\n    id: 504233\n";
        replies.push_back(
            Response::new(200, manifest.as_bytes().to_vec()).header("ETag", "fixture-v1"),
        );
        Self {
            sent: RefCell::new(Vec::new()),
            replies: RefCell::new(replies),
        }
    }
}

impl Transport for FakeTransport {
    fn send(&self, request: &Request) -> Result<Response> {
        self.sent.borrow_mut().push(request.clone());
        self.replies
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| Error::Server(format!("unexpected request to {}", request.url)))
    }
}

struct Offline;
impl Transport for Offline {
    fn send(&self, _: &Request) -> Result<Response> {
        Err(Error::Unreachable("offline test".into()))
    }
}

struct Machine {
    root: PathBuf,
    vars: PathVars,
    store: LocalStore,
    creds: MemoryStore,
    remote: MemoryRemote,
}

impl Machine {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "save-relay-steam-games-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let dirs = AppDirs {
            config: root.join("config/save-relay"),
            data: root.join("data/save-relay"),
        };
        let vars = PathVars {
            home: root.join("home"),
            data: root.join("data"),
            config: root.join("config"),
            documents: None,
        };
        let store = LocalStore::new(dirs);
        Self {
            root,
            vars,
            store,
            creds: MemoryStore::new(),
            remote: MemoryRemote::new(),
        }
    }
}

fn run(machine: &Machine, transport: Arc<dyn Transport>, args: &[&str]) -> (i32, String) {
    let args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    let net = Net {
        transport,
        open_browser: &|_| Ok(()),
        clients: ClientSet::shipped(),
        game_mode: false,
        clock: save_relay::stamp::now,
    };
    let mut stdout = Vec::new();
    let code = run_with_net(
        &args,
        &mut &b""[..],
        &mut stdout,
        &machine.remote,
        &machine.creds,
        &machine.store,
        &machine.vars,
        NOW,
        &net,
    );
    (code, String::from_utf8(stdout).unwrap())
}

#[test]
fn parses_steam_games_flags_and_usage_lists_the_command() {
    assert_eq!(
        parse(&[
            "steam-games".into(),
            "--no-steam-cloud".into(),
            "--refresh".into()
        ])
        .unwrap()
        .command,
        Command::SteamGames {
            no_steam_cloud: true,
            refresh: true
        }
    );
    assert!(save_relay::cli::usage().contains("steam-games [--no-steam-cloud] [--refresh]"));
}

#[test]
fn lists_games_and_uses_cached_answers_offline() {
    let machine = Machine::new();
    let steam = steam_games_fixture::fixture(&machine.root);
    let _guard = steam_games_fixture::set_root(&steam);
    let manifest = Manifest::from_json(r#"{"format":1,"id":"steam-504230","name":"Cloud Game","saves":{"linux":"/tmp/cloud"},"include":["**/*.sav"]}"#).unwrap();
    machine
        .store
        .save_game(&GameRecord {
            manifest,
            saves_dir: None,
        })
        .unwrap();

    let fake = Arc::new(FakeTransport::new());
    let (code, output) = run(
        &machine,
        fake.clone(),
        &["steam-games", "--refresh", "--json"],
    );
    assert_eq!(code, 0, "{output}");
    let envelope: Value = serde_json::from_str(&output).unwrap();
    let games = envelope["data"]["games"].as_array().unwrap();
    assert_eq!(envelope["command"], "steam-games");
    assert_eq!(envelope["data"]["unknown"], 1);
    assert_eq!(games.len(), 3, "tool app must not be listed");
    assert_eq!(games[0]["name"], "Cloud Game");
    assert_eq!(games[0]["steam_cloud"], "yes");
    assert_eq!(games[0]["saves"], "manifest");
    assert_eq!(games[0]["wrapped"], true);
    assert_eq!(games[1]["name"], "No Cloud Game");
    assert_eq!(games[1]["steam_cloud"], "no");
    assert_eq!(games[1]["saves"], "ludusavi");
    assert_eq!(games[2]["name"], "Unknown Game");
    assert_eq!(games[2]["steam_cloud"], "unknown");
    assert_eq!(games[2]["saves"], "ludusavi");
    let requests = fake.sent.borrow();
    assert_eq!(
        requests.len(),
        5,
        "the stale libraryfolders entry has no appmanifest and must not be queried"
    );
    assert!(requests[0].url.contains("appids=504230"));
    assert!(requests[4].url.contains("raw.githubusercontent.com"));

    let (code, output) = run(&machine, Arc::new(Offline), &["steam-games", "--json"]);
    assert_eq!(code, 0, "{output}");
    let cached: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(cached["data"], envelope["data"]);

    let (code, output) = run(
        &machine,
        Arc::new(Offline),
        &["steam-games", "--no-steam-cloud", "--json"],
    );
    assert_eq!(code, 0, "{output}");
    let filtered: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(filtered["data"]["games"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["data"]["games"][0]["steam_cloud"], "no");
    assert_eq!(filtered["data"]["unknown"], 0);
}

#[test]
fn no_network_on_a_fresh_install_still_lists_successfully() {
    let machine = Machine::new();
    let steam = steam_games_fixture::fixture(&machine.root);
    let _guard = steam_games_fixture::set_root(&steam);
    let (code, output) = run(&machine, Arc::new(Offline), &["steam-games", "--json"]);
    assert_eq!(code, 0, "{output}");
    let envelope: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(envelope["data"]["games"].as_array().unwrap().len(), 0);
    assert_eq!(envelope["data"]["unknown"], 0);
}

#[test]
fn missing_steam_root_keeps_the_existing_error_code() {
    let machine = Machine::new();
    let empty = machine.root.join("no-steam");
    fs::create_dir_all(&empty).unwrap();
    let _guard = steam_games_fixture::set_root(&empty);
    let (code, output) = run(&machine, Arc::new(Offline), &["steam-games", "--json"]);
    assert_eq!(
        code,
        save_relay::api::exit_code(&Error::SteamNotFound(String::new()))
    );
    let envelope: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(envelope["error"]["code"], "steam_not_found");
}
