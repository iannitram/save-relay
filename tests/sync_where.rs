//! M11 (#56): `where` lists every game this machine knows with its open-elsewhere mark and the
//! server's current side. Two machines share one `MemoryRemote`; the CLI test drives `where`
//! through `cli::run_with` as the person would.

use save_relay::api::{Held, Side, WhereData};
use save_relay::cli::run_with;
use save_relay::creds::MemoryStore;
use save_relay::layout::Layout;
use save_relay::local::{GameRecord, LocalStore};
use save_relay::manifest::Manifest;
use save_relay::platform::{AppDirs, PathVars};
use save_relay::presence::Mark;
use save_relay::remote::{MemoryRemote, Precondition, Remote};
use save_relay::stamp;
use save_relay::sync::{pull, push, where_open, Game, Session};
use save_relay::Error;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const NOW: u64 = 1_793_476_260;
const TABLET: &str = "strix-halo-tablet";
const AMD: &str = "amd-handheld";
const SERVER: &str = "https://cloud.example.org";
const PASSWORD: &str = "fixture-app-password";
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// The temp folder every machine of one test lives in.
struct World {
    root: PathBuf,
}

impl World {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "save-relay-where-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn machine(&self, device: &str) -> Machine {
        let dir = self.root.join(device);
        Machine {
            device: device.into(),
            root: dir.clone(),
            store: LocalStore::new(AppDirs {
                config: dir.join("config"),
                data: dir.join("data"),
            }),
        }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Machine {
    device: String,
    root: PathBuf,
    store: LocalStore,
}

impl Machine {
    fn session<'a>(&self, remote: &'a MemoryRemote, now: u64) -> Session<'a> {
        Session {
            remote,
            layout: layout(),
            device: self.device.clone(),
            now,
        }
    }

    fn game(&self, id: &str) -> Game {
        Game {
            manifest: manifest(id),
            saves_dir: self.root.join("saves").join(id),
        }
    }

    /// Save `text` as the game's one save file and push it at `now`.
    fn push(&self, remote: &MemoryRemote, id: &str, text: &str, now: u64) {
        let dir = self.root.join("saves").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.sav"), text).unwrap();
        push(
            &self.session(remote, now),
            &self.game(id),
            &self.store,
            None,
        )
        .unwrap();
    }

    /// Pull the game at `now`; a launch that synced leaves this machine's mark.
    fn pull(&self, remote: &MemoryRemote, id: &str, now: u64) {
        let game = self.game(id);
        fs::create_dir_all(&game.saves_dir).unwrap();
        pull(&self.session(remote, now), &game, &self.store, None, false).unwrap();
    }
}

fn layout() -> Layout {
    Layout::new("save-relay").unwrap()
}

fn manifest(id: &str) -> Manifest {
    Manifest::from_json(&format!(
        r#"{{"format":1,"id":"{id}","name":"Fixture {id}",
            "saves_from":"launcher","saves":{{}},
            "include":["**/*.sav"],"exclude":["skip/**"]}}"#
    ))
    .unwrap()
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|id| id.to_string()).collect()
}

/// The shared scene. `uncharted` was pushed by the tablet and is open on the handheld (its
/// mark, since NOW + 60). `hollow` was pushed and pulled by the tablet, so its mark is the
/// tablet's own (since NOW + 200). `saturn` is recorded but nothing was ever pushed for it.
fn scene(world: &World, remote: &MemoryRemote) -> (Machine, Machine) {
    let tablet = world.machine(TABLET);
    let amd = world.machine(AMD);
    tablet.push(remote, "uncharted", "one", NOW);
    amd.pull(remote, "uncharted", NOW + 60);
    tablet.push(remote, "hollow", "two", NOW + 100);
    tablet.pull(remote, "hollow", NOW + 200);
    (tablet, amd)
}

fn held(device: &str, since: u64) -> Option<Held> {
    Some(Held {
        device: device.into(),
        since: stamp::rfc3339(since),
    })
}

fn server(device: &str, time: u64, revision: u64) -> Option<Side> {
    Some(Side {
        device: device.into(),
        time: stamp::rfc3339(time),
        revision: Some(revision),
    })
}

#[test]
fn where_open_gives_each_game_its_mark_and_the_server_side_in_order() {
    let world = World::new();
    let remote = MemoryRemote::new();
    let (tablet, _amd) = scene(&world, &remote);

    let data = where_open(
        &tablet.session(&remote, NOW + 300),
        &ids(&["hollow", "saturn", "uncharted"]),
    )
    .unwrap();

    let games: Vec<&str> = data.games.iter().map(|row| row.game.as_str()).collect();
    assert_eq!(games, ["hollow", "saturn", "uncharted"]);

    // Open on this machine: its own mark is listed too.
    assert_eq!(data.games[0].held, held(TABLET, NOW + 200));
    assert_eq!(data.games[0].server, server(TABLET, NOW + 100, 1));

    // Recorded, never pushed: no mark, nothing on the server.
    assert_eq!(data.games[1].held, None);
    assert_eq!(data.games[1].server, None);

    // Held by another device, with the server's revision from this machine's pushed copy.
    assert_eq!(data.games[2].held, held(AMD, NOW + 60));
    assert_eq!(data.games[2].server, server(TABLET, NOW, 1));
}

#[test]
fn a_broken_mark_is_held_null_and_the_server_side_is_still_listed() {
    let world = World::new();
    let remote = MemoryRemote::new();
    scene(&world, &remote);
    remote
        .put(
            &layout().playing_path("uncharted"),
            b"{not a mark",
            &Precondition::Any,
        )
        .unwrap();
    let tablet = world.machine(TABLET);

    let data = where_open(&tablet.session(&remote, NOW + 300), &ids(&["uncharted"])).unwrap();

    assert_eq!(data.games[0].held, None);
    assert_eq!(data.games[0].server, server(TABLET, NOW, 1));
}

#[test]
fn a_mark_from_a_newer_format_is_held_null() {
    let world = World::new();
    let remote = MemoryRemote::new();
    scene(&world, &remote);
    let newer = Mark {
        format: 2,
        device: AMD.into(),
        since: stamp::rfc3339(NOW + 60),
        revision_read: Some(1),
    };
    remote
        .put(
            &layout().playing_path("uncharted"),
            newer.to_json().as_bytes(),
            &Precondition::Any,
        )
        .unwrap();
    let tablet = world.machine(TABLET);

    let data = where_open(&tablet.session(&remote, NOW + 300), &ids(&["uncharted"])).unwrap();

    assert_eq!(data.games[0].held, None);
}

#[test]
fn an_unreachable_server_is_an_error_not_a_list_of_free_games() {
    let world = World::new();
    let remote = MemoryRemote::new();
    let (tablet, _amd) = scene(&world, &remote);
    remote.set_reachable(false);

    let result = where_open(&tablet.session(&remote, NOW + 300), &ids(&["hollow"]));

    assert!(matches!(result, Err(Error::Unreachable(_))), "{result:?}");
}

#[test]
fn list_games_is_sorted_and_skips_every_file_that_is_not_a_game_record() {
    let world = World::new();
    let machine = world.machine(TABLET);
    assert_eq!(machine.store.list_games().unwrap(), Vec::<String>::new());

    for id in ["uncharted", "hollow", "saturn"] {
        machine
            .store
            .save_game(&GameRecord {
                manifest: manifest(id),
                saves_dir: None,
            })
            .unwrap();
    }
    let games = machine.root.join("data").join("games");
    fs::write(games.join("notes.txt"), "stray").unwrap();
    fs::write(games.join("saturn.json.tmp"), "{}").unwrap();
    fs::write(games.join("README"), "stray").unwrap();
    fs::create_dir_all(games.join("folder.json")).unwrap();

    assert_eq!(
        machine.store.list_games().unwrap(),
        ids(&["hollow", "saturn", "uncharted"])
    );
}

/// `where` through `cli::run_with`: the envelope, the text form, and `server_unreachable`.
#[test]
fn where_command_prints_the_envelope_the_text_and_exits_5_when_unreachable() {
    let world = World::new();
    let remote = MemoryRemote::new();
    let (tablet, _amd) = scene(&world, &remote);
    let creds = MemoryStore::new();
    let vars = PathVars {
        home: world.root.join("home"),
        data: world.root.join("data"),
        config: world.root.join("config"),
        documents: None,
    };
    let setup = args(&format!(
        "setup --server {SERVER} --user player --device {TABLET} --root save-relay --json"
    ));
    let (code, out) = run(
        &setup,
        &format!("{PASSWORD}\n"),
        &remote,
        &creds,
        &tablet.store,
        &vars,
    );
    assert_eq!(code, 0, "{out}");
    for id in ["uncharted", "hollow", "saturn"] {
        tablet
            .store
            .save_game(&GameRecord {
                manifest: manifest(id),
                saves_dir: None,
            })
            .unwrap();
    }

    let (code, out) = run(
        &args("where --json"),
        "",
        &remote,
        &creds,
        &tablet.store,
        &vars,
    );
    assert_eq!(code, 0, "{out}");
    let envelope: Value = serde_json::from_str(out.trim_end()).unwrap();
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["command"], "where");
    let data: WhereData = serde_json::from_value(envelope["data"].clone()).unwrap();
    let games: Vec<&str> = data.games.iter().map(|row| row.game.as_str()).collect();
    assert_eq!(games, ["hollow", "saturn", "uncharted"]);
    assert_eq!(envelope["data"]["games"][1]["held"], Value::Null);
    assert_eq!(envelope["data"]["games"][1]["server"], Value::Null);

    let (code, out) = run(&args("where"), "", &remote, &creds, &tablet.store, &vars);
    assert_eq!(code, 0, "{out}");
    let lines: Vec<String> = out.trim_end().lines().map(String::from).collect();
    assert_eq!(
        lines,
        [
            format!(
                "hollow: open on {TABLET} since {}; server revision 1 from {TABLET}",
                stamp::rfc3339(NOW + 200)
            ),
            "saturn: free; nothing pushed".to_string(),
            format!(
                "uncharted: open on {AMD} since {}; server revision 1 from {TABLET}",
                stamp::rfc3339(NOW + 60)
            ),
        ]
    );

    remote.set_reachable(false);
    let (code, out) = run(
        &args("where --json"),
        "",
        &remote,
        &creds,
        &tablet.store,
        &vars,
    );
    assert_eq!(code, 5, "{out}");
    let envelope: Value = serde_json::from_str(out.trim_end()).unwrap();
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["command"], "where");
    assert_eq!(envelope["error"]["code"], "server_unreachable");
}

fn args(line: &str) -> Vec<String> {
    line.split_whitespace().map(String::from).collect()
}

fn run(
    args: &[String],
    stdin: &str,
    remote: &MemoryRemote,
    creds: &MemoryStore,
    store: &LocalStore,
    vars: &PathVars,
) -> (i32, String) {
    let mut input = stdin.as_bytes();
    let mut output = Vec::new();
    let code = run_with(
        args,
        &mut input,
        &mut output,
        remote,
        creds,
        store,
        vars,
        NOW + 300,
    );
    (code, String::from_utf8(output).unwrap())
}
