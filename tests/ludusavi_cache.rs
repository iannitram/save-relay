//! The ludusavi data (M13): the YAML reader, the Steam index, the weekly refresh with its ETag
//! against a fake `http::Transport` (no network), the cache files, `lookup`, and
//! `save-relay which <appid>` end to end on a fixture Steam folder.

use save_relay::cli::{run_with_net, Net};
use save_relay::clients::ClientSet;
use save_relay::creds::CredentialStore;
use save_relay::creds::MemoryStore;
use save_relay::http::{Request, Response, Transport};
use save_relay::local::{GameRecord, LocalStore};
use save_relay::ludusavi::{self, Cache, MANIFEST_URL, REFRESH_SECS};
use save_relay::manifest::Manifest;
use save_relay::platform::{AppDirs, PathVars};
use save_relay::remote::{MemoryRemote, Remote};
use save_relay::stamp;
use save_relay::{Error, Result};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/ludusavi/manifest.yaml"
);
/// 2026-10-31T19:51:00Z.
const NOW: u64 = 1_793_476_260;

/// Answers the scripted replies in order and records every request; a request with no reply
/// left panics, so a test also proves that nothing was fetched.
struct Fake {
    replies: RefCell<VecDeque<Result<Response>>>,
    sent: RefCell<Vec<Request>>,
}

impl Fake {
    fn new(replies: Vec<Result<Response>>) -> Fake {
        Fake {
            replies: RefCell::new(replies.into()),
            sent: RefCell::new(Vec::new()),
        }
    }

    fn sent(&self) -> Vec<Request> {
        self.sent.borrow().clone()
    }
}

impl Transport for Fake {
    fn send(&self, request: &Request) -> Result<Response> {
        self.sent.borrow_mut().push(request.clone());
        self.replies
            .borrow_mut()
            .pop_front()
            .expect("a reply was scripted for this request")
    }
}

fn body(text: &str, etag: &str) -> Result<Response> {
    Ok(Response::new(200, text.as_bytes().to_vec()).header("ETag", etag))
}

fn not_modified() -> Result<Response> {
    Ok(Response::new(304, Vec::new()))
}

fn offline() -> Result<Response> {
    Err(Error::Unreachable(
        "raw.githubusercontent.com: Timeout".into(),
    ))
}

fn fixture() -> String {
    fs::read_to_string(FIXTURE).expect("the fixture is in the repository")
}

/// A newer copy: the fixture plus one more game, with a Steam id of its own.
fn newer() -> String {
    format!(
        "{}New Game:\n  files:\n    <xdgData>/new-game:\n      tags: [save]\n  steam:\n    id: 1000009\n",
        fixture()
    )
}

/// An empty folder under the system temp folder, named for this test.
fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("save-relay-ludusavi-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cache_in(root: &Path) -> Cache {
    Cache::new(&AppDirs {
        config: root.join("config"),
        data: root.join("data"),
    })
}

fn has_temp_file(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                let path = entry.path();
                path.extension().is_some_and(|ext| ext == "tmp")
                    || (path.is_dir() && has_temp_file(&path))
            })
        })
        .unwrap_or(false)
}

fn index_etag(cache: &Cache) -> Option<String> {
    cache.index().unwrap().and_then(|index| index.etag)
}

// ---- the reader ------------------------------------------------------------------------------

#[test]
fn parse_reads_the_games_and_skips_the_fields_save_relay_does_not_use() {
    let games = ludusavi::parse(&fixture()).unwrap();
    let names: Vec<&str> = games.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "Another Title",
            "Duplicate Title",
            "Example Quest",
            "Gog Only Tale"
        ]
    );
    let quest = &games["Example Quest"];
    // Windows, Linux, config and the every-OS save folder; the registry block is skipped.
    assert_eq!(quest.files.len(), 4);
    assert_eq!(quest.install_dir.len(), 1);
    assert_eq!(
        quest.steam.as_ref().and_then(|steam| steam.id),
        Some(1000001)
    );
    assert_eq!(
        quest.files["<xdgData>/example-quest/saves"].tags,
        ["save".to_string()]
    );
}

#[test]
fn parse_rejects_text_that_is_not_a_map_of_games() {
    for text in ["- just\n- a list\n", "key: [unclosed\n"] {
        match ludusavi::parse(text) {
            Err(Error::StateInvalid(message)) => {
                assert!(message.starts_with("ludusavi data: "), "{message}");
            }
            other => panic!("expected state_invalid for {text:?}, got {other:?}"),
        }
    }
}

#[test]
fn parse_budgets_are_raised_for_the_real_file_but_still_bounded() {
    // The crate's default is 250,000 nodes; the real file is far above that. 40,000 games of
    // about ten nodes each pass only because the budget was raised.
    let mut big = String::new();
    for n in 0..40_000 {
        big.push_str(&format!(
            "Game {n}:\n  files:\n    <home>/g{n}:\n      tags:\n      - save\n      - config\n"
        ));
    }
    let games = ludusavi::parse(&big).unwrap();
    assert_eq!(games.len(), 40_000);

    // Nesting keeps the crate's depth limit: a field save-relay skips cannot be used to build
    // a tower that exhausts the stack.
    let deep = format!("Tower:\n  alias: {}{}\n", "[".repeat(500), "]".repeat(500));
    match ludusavi::parse(&deep) {
        Err(Error::StateInvalid(message)) => assert!(message.starts_with("ludusavi data: ")),
        other => panic!("expected state_invalid for deep nesting, got {other:?}"),
    }
}

#[test]
fn steam_index_keeps_the_first_name_for_a_shared_id_and_skips_other_stores() {
    let index = ludusavi::steam_index(&ludusavi::parse(&fixture()).unwrap());
    assert_eq!(index.len(), 2);
    assert_eq!(index[&1000001], "Example Quest");
    // "Another Title" sorts before "Duplicate Title".
    assert_eq!(index[&1000003], "Another Title");
    assert!(!index.contains_key(&2000001));
}

#[test]
fn needs_refresh_turns_over_exactly_a_week_after_the_last_check() {
    assert!(ludusavi::needs_refresh(None, NOW));
    assert!(!ludusavi::needs_refresh(Some(NOW), NOW));
    assert!(!ludusavi::needs_refresh(Some(NOW - REFRESH_SECS + 1), NOW));
    assert!(ludusavi::needs_refresh(Some(NOW - REFRESH_SECS), NOW));
}

// ---- the cache -------------------------------------------------------------------------------

#[test]
fn first_refresh_downloads_the_data_and_writes_the_index() {
    let root = scratch("first");
    let cache = cache_in(&root);
    let fake = Fake::new(vec![body(&fixture(), "\"v1\"")]);

    assert!(cache.refresh(&fake, NOW, false).unwrap());

    let sent = fake.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "GET");
    assert_eq!(sent[0].url, MANIFEST_URL);
    assert_eq!(sent[0].header_value("If-None-Match"), None);
    let index = cache.index().unwrap().expect("the index is written");
    assert_eq!(index.etag.as_deref(), Some("\"v1\""));
    assert_eq!(index.checked, NOW);
    assert_eq!(index.apps[&1000001], "Example Quest");
    assert!(!has_temp_file(&root), "no temp file is left behind");
}

#[test]
fn inside_a_week_nothing_is_fetched() {
    let root = scratch("week");
    let cache = cache_in(&root);
    // One reply only: a second request would panic in the fake.
    let fake = Fake::new(vec![body(&fixture(), "\"v1\"")]);
    cache.refresh(&fake, NOW, false).unwrap();

    assert!(!cache.refresh(&fake, NOW + REFRESH_SECS - 1, false).unwrap());
    assert_eq!(fake.sent().len(), 1);
}

#[test]
fn a_week_later_the_etag_gets_a_304_and_the_copy_is_kept() {
    let root = scratch("304");
    let cache = cache_in(&root);
    let fake = Fake::new(vec![body(&fixture(), "\"v1\""), not_modified()]);
    cache.refresh(&fake, NOW, false).unwrap();

    let later = NOW + REFRESH_SECS;
    assert!(!cache.refresh(&fake, later, false).unwrap());

    let sent = fake.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].header_value("If-None-Match"), Some("\"v1\""));
    let index = cache.index().unwrap().unwrap();
    assert_eq!(index.checked, later);
    assert_eq!(index.etag.as_deref(), Some("\"v1\""));
    assert_eq!(
        fs::read_to_string(root.join("data/ludusavi/manifest.yaml")).unwrap(),
        fixture()
    );
    assert_eq!(cache.lookup(1000001).unwrap().0, "Example Quest");
}

#[test]
fn a_newer_copy_replaces_the_cached_one() {
    let root = scratch("newer");
    let cache = cache_in(&root);
    let fake = Fake::new(vec![body(&fixture(), "\"v1\""), body(&newer(), "\"v2\"")]);
    cache.refresh(&fake, NOW, false).unwrap();

    assert!(cache.refresh(&fake, NOW + REFRESH_SECS, false).unwrap());
    assert_eq!(index_etag(&cache).as_deref(), Some("\"v2\""));
    assert_eq!(cache.lookup(1000009).unwrap().0, "New Game");
}

#[test]
fn a_failed_refresh_keeps_the_cached_copy_and_is_tried_again_later() {
    let root = scratch("offline");
    let cache = cache_in(&root);
    let fake = Fake::new(vec![
        body(&fixture(), "\"v1\""),
        offline(),
        Ok(Response::new(503, "busy")),
    ]);
    cache.refresh(&fake, NOW, false).unwrap();
    let later = NOW + REFRESH_SECS;

    assert!(!cache.refresh(&fake, later, false).unwrap());
    assert!(!cache.refresh(&fake, later, false).unwrap());

    // The check time did not move, so the next call asks again.
    assert_eq!(cache.index().unwrap().unwrap().checked, NOW);
    assert_eq!(cache.lookup(1000001).unwrap().0, "Example Quest");
}

#[test]
fn with_no_copy_an_unreachable_server_is_an_error() {
    let root = scratch("none");
    let cache = cache_in(&root);
    let fake = Fake::new(vec![offline()]);

    match cache.refresh(&fake, NOW, false) {
        Err(Error::Unreachable(_)) => {}
        other => panic!("expected server_unreachable, got {other:?}"),
    }
    assert!(cache.index().unwrap().is_none());
}

#[test]
fn a_download_that_does_not_parse_never_replaces_the_copy() {
    let root = scratch("bad");
    let cache = cache_in(&root);
    let fake = Fake::new(vec![
        body(&fixture(), "\"v1\""),
        body("key: [unclosed\n", "\"v2\""),
    ]);
    cache.refresh(&fake, NOW, false).unwrap();

    match cache.refresh(&fake, NOW + REFRESH_SECS, false) {
        Err(Error::StateInvalid(_)) => {}
        other => panic!("expected state_invalid, got {other:?}"),
    }
    assert_eq!(index_etag(&cache).as_deref(), Some("\"v1\""));
    assert_eq!(cache.lookup(1000001).unwrap().0, "Example Quest");
}

#[test]
fn force_fetches_inside_the_week() {
    let root = scratch("force");
    let cache = cache_in(&root);
    let fake = Fake::new(vec![body(&fixture(), "\"v1\""), not_modified()]);
    cache.refresh(&fake, NOW, false).unwrap();

    assert!(!cache.refresh(&fake, NOW + 60, true).unwrap());
    assert_eq!(fake.sent()[1].header_value("If-None-Match"), Some("\"v1\""));
}

#[test]
fn lookup_finds_the_entry_and_names_a_miss() {
    let root = scratch("lookup");
    let cache = cache_in(&root);
    let fake = Fake::new(vec![body(&fixture(), "\"v1\"")]);
    cache.refresh(&fake, NOW, false).unwrap();

    let (name, entry) = cache.lookup(1000001).unwrap();
    assert_eq!(name, "Example Quest");
    assert!(entry.files.contains_key("<xdgData>/example-quest/saves"));

    for app_id in [4242, 2000001] {
        match cache.lookup(app_id) {
            Err(Error::SaveLocationUnclear(message)) => assert_eq!(
                message,
                format!("no entry for Steam app {app_id} in the ludusavi data")
            ),
            other => panic!("expected save_location_unclear, got {other:?}"),
        }
    }
}

#[test]
fn lookup_before_any_download_is_unclear() {
    let root = scratch("empty");
    match cache_in(&root).lookup(1000001) {
        Err(Error::SaveLocationUnclear(_)) => {}
        other => panic!("expected save_location_unclear, got {other:?}"),
    }
}

// ---- which -----------------------------------------------------------------------------------

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| word.to_string()).collect()
}

fn no_browser(_url: &str) -> Result<()> {
    panic!("which never opens a browser")
}

/// `run_with_net` with `fake` as the network and nothing else reachable.
/// The fake is used on this thread only.
#[allow(clippy::too_many_arguments, clippy::arc_with_non_send_sync)]
fn run_which(
    args: &[String],
    stdin: &mut dyn std::io::Read,
    stdout: &mut dyn std::io::Write,
    remote: &dyn Remote,
    creds: &dyn CredentialStore,
    store: &LocalStore,
    vars: &PathVars,
    now: u64,
    fake: Fake,
) -> i32 {
    let net = Net {
        transport: Arc::new(fake),
        open_browser: &no_browser,
        clients: ClientSet::shipped(),
        game_mode: false,
        clock: stamp::now,
    };
    run_with_net(args, stdin, stdout, remote, creds, store, vars, now, &net)
}

/// `which` finds the Steam folder with the platform's own rule and the libraries from its
/// `libraryfolders.vdf`; the test builds that folder under its own home. The saves folder is the
/// every-OS entry's `<home>` folder, so the same test runs on Linux and macOS.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn which_prints_the_one_saves_folder_from_the_cached_data() {
    use save_relay::api::{WhichData, WhichSource};

    let root = scratch("which");
    let vars = PathVars {
        home: root.join("home"),
        data: root.join("data"),
        config: root.join("config"),
        documents: None,
    };
    let steam_home = if cfg!(target_os = "macos") {
        "Library/Application Support/Steam"
    } else {
        ".local/share/Steam"
    };
    let library = root.join("library");
    let steamapps = vars.home.join(steam_home).join("steamapps");
    fs::create_dir_all(&steamapps).unwrap();
    fs::write(
        steamapps.join("libraryfolders.vdf"),
        format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t\t\"apps\"\n\t\t{{\n\t\t\t\"1000001\"\t\t\"1\"\n\t\t}}\n\t}}\n}}\n",
            library.display()
        ),
    )
    .unwrap();
    fs::create_dir_all(library.join("steamapps/common/ExampleQuest")).unwrap();
    fs::write(
        library.join("steamapps/appmanifest_1000001.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"1000001\"\n\t\"installdir\"\t\t\"ExampleQuest\"\n}\n",
    )
    .unwrap();
    let saves = vars.home.join("Example Quest Saves");
    fs::create_dir_all(&saves).unwrap();

    let store = LocalStore::new(AppDirs::from_vars(&vars));
    let remote = MemoryRemote::new();
    let creds = MemoryStore::new();
    let mut stdin = std::io::empty();
    let fake = Fake::new(vec![body(&fixture(), "\"v1\"")]);
    let mut out = Vec::new();
    let code = run_which(
        &args(&["which", "1000001", "--json"]),
        &mut stdin,
        &mut out,
        &remote,
        &creds,
        &store,
        &vars,
        NOW,
        fake,
    );
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&out));
    let envelope: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["command"], "which");
    let data: WhichData = serde_json::from_value(envelope["data"].clone()).unwrap();
    assert_eq!(data.app_id, 1000001);
    assert_eq!(data.game, "steam-1000001");
    assert_eq!(data.name, "Example Quest");
    assert_eq!(data.saves_dir, saves.to_string_lossy());
    assert_eq!(data.include, ["*.sav"]);
    assert_eq!(data.source, WhichSource::Ludusavi);

    // Fresh copy: the second call is served from disk, with no request at all.
    let mut out = Vec::new();
    let code = run_which(
        &args(&["which", "1000001"]),
        &mut stdin,
        &mut out,
        &remote,
        &creds,
        &store,
        &vars,
        NOW + 60,
        Fake::new(Vec::new()),
    );
    assert_eq!(code, 0);
    assert!(String::from_utf8_lossy(&out).contains("Files: *.sav"));

    // No entry: exit code 2 and the stable error code.
    let mut out = Vec::new();
    let code = run_which(
        &args(&["which", "4242", "--json"]),
        &mut stdin,
        &mut out,
        &remote,
        &creds,
        &store,
        &vars,
        NOW + 60,
        Fake::new(Vec::new()),
    );
    assert_eq!(code, 2);
    let envelope: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(envelope["error"]["code"], "save_location_unclear");
}

#[test]
fn which_uses_the_players_own_manifest_before_the_data() {
    let root = scratch("own");
    let vars = PathVars {
        home: root.join("home"),
        data: root.join("data"),
        config: root.join("config"),
        documents: None,
    };
    let store = LocalStore::new(AppDirs::from_vars(&vars));
    let manifest = Manifest::from_json(
        r#"{"format":1,"id":"steam-1000001","name":"My Quest",
            "saves":{"linux":"{home}/my-quest"},"include":["**/*.sav"]}"#,
    )
    .unwrap();
    let saves = root.join("my-saves");
    store
        .save_game(&GameRecord {
            manifest,
            saves_dir: Some(saves.clone()),
        })
        .unwrap();

    let remote = MemoryRemote::new();
    let creds = MemoryStore::new();
    let mut stdin = std::io::empty();
    let mut out = Vec::new();
    // No reply scripted: the data is never asked for.
    let code = run_which(
        &args(&["which", "1000001", "--json"]),
        &mut stdin,
        &mut out,
        &remote,
        &creds,
        &store,
        &vars,
        NOW,
        Fake::new(Vec::new()),
    );
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&out));
    let envelope: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(envelope["data"]["source"], "manifest");
    assert_eq!(
        envelope["data"]["saves_dir"],
        saves.to_string_lossy().replace('\\', "/").as_str()
    );
    assert_eq!(envelope["data"]["include"][0], "**/*.sav");
}
