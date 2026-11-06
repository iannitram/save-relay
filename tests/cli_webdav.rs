//! The built `save-relay` binary against a real WebDAV server (issue #23): two machines, each with
//! its own folders and config, sharing one game, and a second game that joins with only its
//! manifest. The tests are named after SPEC.md's Musts: M1 to M5 and M7, plus M6 for the
//! `--password VALUE` form only (`m6_password_flags_are_refused_without_echo` lists what it does
//! not cover). A racing push checks that Nextcloud's `If-Match` refuses a write one second later
//! with a different size, on Linux only: rclone 1.73.5, the server the Windows and macOS legs run,
//! ignores `If-Match` and `If-None-Match`. A same-second, same-size write is covered by the revision
//! claim, through the engine, in `same_second_same_size_push_is_refused_by_the_revision_claim` (#149).
//! They are ignored by default, so `cargo test` needs no server. The
//! `integration` job in `.github/workflows/check.yml` starts one and runs them, except the one
//! test ignored for #148, which the job skips:
//!
//! ```text
//! SR_WEBDAV_SERVER=http://127.0.0.1:8080 SR_WEBDAV_USER=admin SR_WEBDAV_PASSWORD=<app password> \
//!   cargo test --locked --test cli_webdav -- --ignored
//! ```
//!
//! Each test works in a folder of its own on the server (`sr-it-<test>-<stamp>`), so the tests
//! share one server. The password reaches the binary through `SAVE_RELAY_TEST_PASSWORD`, which
//! only a debug build reads (`creds::test_password_from_env`).

use save_relay::creds::TEST_PASSWORD_VAR;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const BINARY: &str = env!("CARGO_BIN_EXE_save-relay");
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
/// The fixture game's saves, relative to its saves folder. The manifest's include matches
/// `cache.tmp` and its exclude removes it, so only the exclude keeps it out of a save.
const FIXTURE_SAVES: [&str; 3] = ["slot1.sav", "slot2.sav", "profile/progress.sav"];

/// A game under test: its id, and its manifest under `tests/fixtures/games/`.
#[derive(Clone, Copy)]
struct Game {
    id: &'static str,
    manifest: &'static str,
}

const FIXTURE: Game = Game {
    id: "fixture-game",
    manifest: "games/fixture-game.json",
};
const SECOND: Game = Game {
    id: "second-game",
    manifest: "games/second-game.json",
};

/// The server under test, from the environment the `integration` job sets.
struct Server {
    url: String,
    user: String,
    password: String,
}

impl Server {
    fn from_env() -> Server {
        let get = |name: &str| {
            std::env::var(name).unwrap_or_else(|_| panic!("set {name}: see the file comment"))
        };
        Server {
            url: get("SR_WEBDAV_SERVER"),
            user: get("SR_WEBDAV_USER"),
            password: get("SR_WEBDAV_PASSWORD"),
        }
    }
}

/// A folder on the server that no other test or run uses.
fn server_folder(test: &str) -> String {
    format!("sr-it-{test}-{}", stamp())
}

fn stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

/// An address with nothing listening on it: a connection to it is refused at once.
fn unused_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

fn fixture_path(relative: &str) -> PathBuf {
    Path::new(FIXTURES).join(relative)
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Every file under `dir` whose bytes contain `needle`.
fn files_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files_containing(&path, needle));
        } else if fs::read(&path)
            .unwrap()
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
        {
            found.push(path);
        }
    }
    found
}

/// The `action` of a sync envelope, or "" when there is none.
fn action(envelope: &Value) -> &str {
    envelope["data"]["action"].as_str().unwrap_or("")
}

/// Asserts a command's exit code and action, and returns its envelope.
fn expect(result: (i32, Value), code: i32, action_name: &str) -> Value {
    let (got, envelope) = result;
    assert_eq!((got, action(&envelope)), (code, action_name), "{envelope}");
    envelope
}

/// What one run of the binary printed and returned.
#[derive(Debug)]
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// One machine: its own home, config and data folders. The binary takes its platform folders
/// from them, so two machines share nothing but the server.
struct Machine {
    root: PathBuf,
    password: String,
}

impl Machine {
    fn new(test: &str, name: &str, server: &Server) -> Machine {
        let root =
            std::env::temp_dir().join(format!("save-relay-webdav-{test}-{name}-{}", stamp()));
        for folder in ["home", "config", "data"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        Machine {
            root,
            password: server.password.clone(),
        }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    /// Where `game`'s saves are, as its manifest's template resolves on this platform (see
    /// `tests/fixtures/games/` and `src/platform.rs`). `{data}` is `data` on Linux and Windows,
    /// and on a Mac `{data}` and `{config}` are both `Library/Application Support`.
    fn saves(&self, game: Game) -> PathBuf {
        let data = if cfg!(target_os = "macos") {
            self.home().join("Library/Application Support")
        } else {
            self.root.join("data")
        };
        match game.id {
            "fixture-game" => data.join("fixture-game/saves"),
            "second-game" => self.home().join("Documents").join("second-game"),
            other => panic!("no saves folder for {other}"),
        }
    }

    fn save(&self, game: Game, name: &str) -> PathBuf {
        self.saves(game).join(name)
    }

    fn write(&self, game: Game, name: &str, text: &str) {
        let path = self.save(game, name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
    }

    fn read(&self, game: Game, name: &str) -> String {
        fs::read_to_string(self.save(game, name)).unwrap()
    }

    /// The fixture game's saves, as a player has them before the first push.
    fn load_fixture(&self, game: Game) {
        copy_tree(
            &fixture_path(&format!("saves/{}", game.id)),
            &self.saves(game),
        );
    }

    /// Where `config.json` is, as the binary's `AppDirs` names it on each platform.
    fn config_file(&self) -> PathBuf {
        let config = if cfg!(target_os = "macos") {
            self.home().join("Library/Application Support")
        } else {
            self.root.join("config")
        };
        config.join("save-relay").join("config.json")
    }

    /// Points this machine at `url`. A URL with nothing listening is a server that is down.
    fn point_at(&self, url: &str) {
        let path = self.config_file();
        let mut config: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        config["server"] = Value::String(url.to_string());
        fs::write(&path, config.to_string()).unwrap();
    }

    /// The binary with `args` and `stdin`, on this machine's folders and the test password.
    fn run(&self, args: &[&str], stdin: &str) -> Run {
        let mut child = Command::new(BINARY)
            .args(args)
            .env("HOME", self.home())
            .env("USERPROFILE", self.home())
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("APPDATA", self.root.join("config"))
            .env("LOCALAPPDATA", self.root.join("data"))
            .env(TEST_PASSWORD_VAR, &self.password)
            .env_remove("SteamGamepadUI")
            .env_remove("XDG_CURRENT_DESKTOP")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start the save-relay binary");
        if let Some(mut pipe) = child.stdin.take() {
            // Dropped at the end of this statement, so the binary reads end of input after `stdin`.
            let _ = pipe.write_all(stdin.as_bytes());
        }
        let out = child.wait_with_output().expect("wait for save-relay");
        Run {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// `args` with `--json` added: the exit code and the one-line envelope.
    fn json(&self, args: &[&str], stdin: &str) -> (i32, Value) {
        let mut with_json = args.to_vec();
        with_json.push("--json");
        let run = self.run(&with_json, stdin);
        let envelope = serde_json::from_str(run.stdout.trim())
            .unwrap_or_else(|error| panic!("not one JSON envelope ({error}): {run:?}"));
        (run.code, envelope)
    }

    /// The first run on this machine: `setup` against `server`, in the server folder `folder`.
    fn setup(&self, server: &Server, device: &str, folder: &str) {
        let (code, envelope) = self.json(
            &[
                "setup",
                "--service",
                "nextcloud",
                "--server",
                server.url.as_str(),
                "--user",
                server.user.as_str(),
                "--device",
                device,
                "--root",
                folder,
            ],
            &format!("{}\n", server.password),
        );
        assert_eq!(code, 0, "setup: {envelope}");
        assert_eq!(envelope["data"]["service"], "nextcloud");
    }

    /// A game command (`pull`, `push`, `status`, `restore`) for `game`, with its manifest and
    /// saves folder as a launcher passes them.
    fn sync(&self, verb: &str, game: Game, extra: &[&str]) -> (i32, Value) {
        self.sync_now(verb, game, extra)
    }

    /// `sync`, called by this name where two writes land in one second (#148).
    fn sync_now(&self, verb: &str, game: Game, extra: &[&str]) -> (i32, Value) {
        let manifest = fixture_path(game.manifest);
        let saves = self.saves(game);
        fs::create_dir_all(&saves).unwrap();
        let mut args = vec![
            verb,
            game.id,
            "--manifest",
            manifest.to_str().unwrap(),
            "--saves-dir",
            saves.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        self.json(&args, "")
    }

    /// A game command with no `--saves-dir`: the saves folder comes from the manifest's template,
    /// as a launch with no override does. Nothing creates that folder first.
    fn sync_by_manifest(&self, verb: &str, game: Game) -> (i32, Value) {
        let manifest = fixture_path(game.manifest);
        self.json(
            &[verb, game.id, "--manifest", manifest.to_str().unwrap()],
            "",
        )
    }

    /// `history <game>`: the server's entries for the game, newest first.
    fn history(&self, game: Game) -> Vec<Value> {
        let (code, envelope) = self.json(&["history", game.id], "");
        assert_eq!(code, 0, "history: {envelope}");
        envelope["data"]["entries"].as_array().unwrap().clone()
    }

    /// The device that made the history entry `entry`.
    fn device_of(&self, game: Game, entry: &str) -> String {
        self.history(game)
            .iter()
            .find(|found| found["entry"] == entry)
            .map(|found| found["device"].as_str().unwrap().to_string())
            .unwrap_or_else(|| panic!("no history entry {entry}"))
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Two machines in a conflict: A pushed the fixture game, B pulled it and pushed a change, and A
/// changed the same save without pulling. Returns A and B.
fn conflicting_machines(test: &str, server: &Server, folder: &str) -> (Machine, Machine) {
    let a = Machine::new(test, "a", server);
    a.setup(server, "device-a", folder);
    a.load_fixture(FIXTURE);
    expect(a.sync("push", FIXTURE, &[]), 0, "pushed");

    let b = Machine::new(test, "b", server);
    b.setup(server, "device-b", folder);
    expect(b.sync("pull", FIXTURE, &[]), 0, "pulled");
    b.write(FIXTURE, "slot1.sav", "chapter two, tablet");
    expect(b.sync("push", FIXTURE, &[]), 0, "pushed");

    a.write(FIXTURE, "slot1.sav", "chapter two, handheld");
    (a, b)
}

#[test]
#[ignore = "needs a WebDAV server: see the file comment"]
fn m1_pull_brings_the_newest_saves_from_another_machine() {
    let server = Server::from_env();
    let folder = server_folder("m1");
    let a = Machine::new("m1", "a", &server);
    a.setup(&server, "device-a", &folder);
    a.load_fixture(FIXTURE);
    expect(a.sync_by_manifest("push", FIXTURE), 0, "pushed");

    let b = Machine::new("m1", "b", &server);
    b.setup(&server, "device-b", &folder);
    expect(b.sync_by_manifest("pull", FIXTURE), 0, "pulled");
    for name in FIXTURE_SAVES {
        assert_eq!(b.read(FIXTURE, name), a.read(FIXTURE, name), "{name}");
    }
    assert!(
        !b.save(FIXTURE, "cache.tmp").exists(),
        "a file the manifest excludes is not a save"
    );
    // B quits without changing anything: the push clears B's open mark.
    expect(b.sync_by_manifest("push", FIXTURE), 0, "noop");

    // A plays again and quits; B's next launch brings the newest save.
    a.write(FIXTURE, "slot1.sav", "chapter two");
    expect(a.sync_by_manifest("push", FIXTURE), 0, "pushed");
    expect(b.sync_by_manifest("pull", FIXTURE), 0, "pulled");
    assert_eq!(b.read(FIXTURE, "slot1.sav"), "chapter two");
}

#[test]
#[ignore = "needs a WebDAV server: see the file comment"]
fn m2_push_puts_the_save_on_the_server_with_device_and_time() {
    let server = Server::from_env();
    let folder = server_folder("m2");
    let a = Machine::new("m2", "a", &server);
    a.setup(&server, "device-a", &folder);
    a.load_fixture(FIXTURE);

    let pushed = expect(a.sync("push", FIXTURE, &[]), 0, "pushed");
    assert_eq!(pushed["data"]["counts"]["uploaded"], FIXTURE_SAVES.len());
    assert_eq!(pushed["data"]["server"]["device"], "device-a");
    assert!(pushed["data"]["server"]["time"].is_string());

    let entries = a.history(FIXTURE);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["device"], "device-a");
    assert!(entries[0]["time"].is_string());

    // Another machine sees the same record in its status, before it has pulled anything.
    let b = Machine::new("m2", "b", &server);
    b.setup(&server, "device-b", &folder);
    let (code, status) = b.sync("status", FIXTURE, &[]);
    assert_eq!(code, 0, "{status}");
    assert_eq!(status["data"]["server"]["device"], "device-a");
}

#[test]
#[ignore = "needs a WebDAV server: see the file comment"]
fn m3_conflict_exits_3_then_keep_local_keeps_the_other_side_as_a_backup() {
    let server = Server::from_env();
    let folder = server_folder("m3-local");
    let (a, _b) = conflicting_machines("m3-local", &server, &folder);

    let conflict = expect(a.sync("pull", FIXTURE, &[]), 3, "conflict");
    assert_eq!(conflict["data"]["server"]["device"], "device-b");
    assert_eq!(conflict["data"]["local"]["device"], "device-a");
    assert!(conflict["data"]["server"]["time"].is_string());
    assert!(conflict["data"]["local"]["time"].is_string());
    assert_eq!(
        a.read(FIXTURE, "slot1.sav"),
        "chapter two, handheld",
        "a conflict touches nothing"
    );

    let pushed = expect(
        a.sync("push", FIXTURE, &["--resolve", "keep-local"]),
        0,
        "pushed",
    );
    let backup = pushed["data"]["backup"]
        .as_str()
        .expect("the side not chosen is kept as a dated backup");
    assert_eq!(a.device_of(FIXTURE, backup), "device-b");

    // The loser is restorable.
    let (code, envelope) = a.sync("restore", FIXTURE, &[backup]);
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(a.read(FIXTURE, "slot1.sav"), "chapter two, tablet");
}

#[test]
#[ignore = "needs a WebDAV server: see the file comment"]
fn m3_conflict_exits_3_then_keep_server_keeps_the_local_side_as_a_backup() {
    let server = Server::from_env();
    let folder = server_folder("m3-server");
    let (a, _b) = conflicting_machines("m3-server", &server, &folder);

    expect(a.sync("pull", FIXTURE, &[]), 3, "conflict");
    let pulled = expect(
        a.sync("pull", FIXTURE, &["--resolve", "keep-server"]),
        0,
        "pulled",
    );
    let backup = pulled["data"]["backup"]
        .as_str()
        .expect("the side not chosen is kept as a dated backup");
    assert_eq!(a.device_of(FIXTURE, backup), "device-a");
    assert_eq!(a.read(FIXTURE, "slot1.sav"), "chapter two, tablet");

    // The launch wrote A's open mark; A quits, which clears it.
    let (code, envelope) = a.sync("push", FIXTURE, &[]);
    assert_eq!(code, 0, "{envelope}");

    // The handheld's save is restorable from the backup.
    let (code, envelope) = a.sync("restore", FIXTURE, &[backup]);
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(a.read(FIXTURE, "slot1.sav"), "chapter two, handheld");
}

#[test]
#[ignore = "needs a WebDAV server: see the file comment"]
fn m4_keeps_the_last_ten_pushes_and_restores_any_of_them() {
    let server = Server::from_env();
    let folder = server_folder("m4");
    let a = Machine::new("m4", "a", &server);
    a.setup(&server, "device-a", &folder);
    a.load_fixture(FIXTURE);

    for version in 1..=11 {
        a.write(FIXTURE, "slot1.sav", &format!("version {version}"));
        expect(a.sync("push", FIXTURE, &[]), 0, "pushed");
    }

    // Newest first: the current copy, then ten dated copies in all, so version 1 is gone.
    let entries = a.history(FIXTURE);
    assert_eq!(entries.len(), 10);
    assert_eq!(entries[0]["current"], true);
    let oldest = entries[9]["entry"].as_str().unwrap();

    let (code, envelope) = a.sync("restore", FIXTURE, &[oldest]);
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(envelope["data"]["entry"], oldest);
    assert_eq!(a.read(FIXTURE, "slot1.sav"), "version 2");
}

/// M4 says every push keeps a dated copy. Two pushes from one device in the same second must both
/// do so (#148).
#[test]
#[ignore = "needs a WebDAV server: see the file comment"]
fn two_pushes_in_one_second_both_keep_a_dated_copy() {
    let server = Server::from_env();
    let folder = server_folder("same-second");
    let a = Machine::new("same-second", "a", &server);
    a.setup(&server, "device-a", &folder);
    a.load_fixture(FIXTURE);

    a.write(FIXTURE, "slot1.sav", "version 1");
    expect(a.sync_now("push", FIXTURE, &[]), 0, "pushed");
    a.write(FIXTURE, "slot1.sav", "version 2");
    expect(a.sync_now("push", FIXTURE, &[]), 0, "pushed");

    // The current copy, and the dated copy of version 1 that the second push kept.
    let entries = a.history(FIXTURE);
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert_eq!(entries[0]["current"], true);
}

#[test]
#[ignore = "needs a WebDAV server: see the file comment"]
fn m5_offline_launch_starts_on_local_saves_and_checks_before_the_deferred_push() {
    let server = Server::from_env();
    let folder = server_folder("m5");
    let a = Machine::new("m5", "a", &server);
    a.setup(&server, "device-a", &folder);
    a.load_fixture(FIXTURE);
    expect(a.sync("push", FIXTURE, &[]), 0, "pushed");

    let b = Machine::new("m5", "b", &server);
    b.setup(&server, "device-b", &folder);
    expect(b.sync("pull", FIXTURE, &[]), 0, "pulled");
    expect(b.sync("push", FIXTURE, &[]), 0, "noop");

    // The server goes away: the game starts on the local saves, and the quit cannot push.
    b.point_at(&unused_url());
    let offline = expect(b.sync("pull", FIXTURE, &[]), 0, "offline");
    assert!(
        offline["data"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("offline"),
        "{offline}"
    );
    // The fixture's own bytes: a Windows checkout may write CRLF, and the pulled copy matches it.
    assert_eq!(
        b.read(FIXTURE, "slot1.sav"),
        fs::read_to_string(fixture_path("saves/fixture-game/slot1.sav")).unwrap()
    );
    b.write(FIXTURE, "slot1.sav", "chapter two, made offline");
    expect(b.sync("push", FIXTURE, &[]), 0, "deferred");

    // While B is away, A plays on and pushes a different save.
    a.write(FIXTURE, "slot1.sav", "chapter two, handheld");
    expect(a.sync("push", FIXTURE, &[]), 0, "pushed");

    // The server is back: B's next launch checks first (M3). The server has a newer entry than
    // B's last pull and B's deferred save differs from it, so the launch is a conflict. Nothing is
    // written, the server stays at A's entry, and B's save stays local.
    b.point_at(&server.url);
    let conflict = expect(b.sync("pull", FIXTURE, &[]), 3, "conflict");
    assert_eq!(conflict["data"]["server"]["device"], "device-a");
    assert_eq!(
        b.read(FIXTURE, "slot1.sav"),
        "chapter two, made offline",
        "a conflict touches nothing"
    );
    let entries = b.history(FIXTURE);
    assert_eq!(entries[0]["device"], "device-a");
    assert_eq!(entries[0]["current"], true);

    // B keeps its save: the push sends it, and A's entry is kept as a dated backup.
    let pushed = expect(
        b.sync("push", FIXTURE, &["--resolve", "keep-local"]),
        0,
        "pushed",
    );
    let backup = pushed["data"]["backup"]
        .as_str()
        .expect("the side not chosen is kept as a dated backup");
    assert_eq!(b.device_of(FIXTURE, backup), "device-a");
    expect(b.sync("push", FIXTURE, &[]), 0, "noop");

    expect(a.sync("pull", FIXTURE, &[]), 0, "pulled");
    assert_eq!(a.read(FIXTURE, "slot1.sav"), "chapter two, made offline");
}

#[test]
#[ignore = "needs a WebDAV server: see the file comment"]
fn m5_a_deferred_save_goes_up_at_the_next_launch_when_the_server_has_not_moved() {
    let server = Server::from_env();
    let folder = server_folder("m5-clean");
    let a = Machine::new("m5-clean", "a", &server);
    a.setup(&server, "device-a", &folder);
    a.load_fixture(FIXTURE);
    expect(a.sync("push", FIXTURE, &[]), 0, "pushed");

    let b = Machine::new("m5-clean", "b", &server);
    b.setup(&server, "device-b", &folder);
    expect(b.sync("pull", FIXTURE, &[]), 0, "pulled");
    expect(b.sync("push", FIXTURE, &[]), 0, "noop");

    // The server goes away: the game starts on the local saves, and the quit cannot push.
    b.point_at(&unused_url());
    expect(b.sync("pull", FIXTURE, &[]), 0, "offline");
    b.write(FIXTURE, "slot1.sav", "chapter two, made offline");
    expect(b.sync("push", FIXTURE, &[]), 0, "deferred");

    // The server is back and nobody has written since B's pull. B's next launch is no conflict:
    // its deferred save goes up at that launch, so the server's current entry is B's.
    b.point_at(&server.url);
    expect(b.sync("pull", FIXTURE, &[]), 0, "pushed");
    assert_eq!(b.read(FIXTURE, "slot1.sav"), "chapter two, made offline");
    let entries = b.history(FIXTURE);
    assert_eq!(entries[0]["device"], "device-b");
    assert_eq!(entries[0]["current"], true);

    // B quits with nothing new: the launch already sent the save.
    expect(b.sync("push", FIXTURE, &[]), 0, "noop");

    // A's next launch brings the deferred save.
    expect(a.sync("pull", FIXTURE, &[]), 0, "pulled");
    assert_eq!(a.read(FIXTURE, "slot1.sav"), "chapter two, made offline");
}

/// M6, for the `--password VALUE` form. It is refused before anything is sent, and no part of the
/// password reaches its stdout or stderr. A password on standard input stays out of the setup and
/// status envelopes, and out of every file under both machines' folders.
///
/// The `--password=VALUE` form is refused the same way (`tests/cli_password_eq.rs`). Not covered,
/// and still printed by a usage error on main: a secret typed as the command word, as an unknown
/// flag or as a bare positional argument; and a secret typed as the service, answer or `--resolve`
/// value. A value typed as `--device` or `--root` is kept in `config.json`. Each of those needs a
/// change to the CLI's messages, which this test does not make.
#[test]
#[ignore = "needs a WebDAV server: see the file comment"]
fn m6_password_flags_are_refused_without_echo() {
    let server = Server::from_env();
    let folder = server_folder("m6");
    let a = Machine::new("m6", "a", &server);
    let b = Machine::new("m6", "b", &server);
    let password = server.password.as_str();

    // `--password VALUE` is refused before anything is sent. It exits 2 and prints no part of it.
    let base = [
        "setup",
        "--service",
        "nextcloud",
        "--server",
        server.url.as_str(),
        "--user",
        server.user.as_str(),
    ];
    let refused_args: Vec<&str> = [&base[..], &["--password", password][..]].concat();
    let refused = a.run(&refused_args, "");
    assert_eq!(refused.code, 2, "{refused:?}");
    assert!(!refused.stdout.contains(password), "{refused:?}");
    assert!(!refused.stderr.contains(password), "{refused:?}");

    // The password goes in on standard input, and comes back in no output.
    let (code, envelope) = a.json(
        &[
            "setup",
            "--service",
            "nextcloud",
            "--server",
            server.url.as_str(),
            "--user",
            server.user.as_str(),
            "--device",
            "device-a",
            "--root",
            folder.as_str(),
        ],
        &format!("{password}\n"),
    );
    assert_eq!(code, 0, "{envelope}");
    assert!(!envelope.to_string().contains(password));
    let (code, status) = a.sync("status", FIXTURE, &[]);
    assert_eq!(code, 0, "{status}");
    assert_eq!(status["data"]["service"], "nextcloud");
    assert!(!status.to_string().contains(password));

    // A second machine joins the same server the same way, and then no file under either
    // machine's folders holds the password.
    b.setup(&server, "device-b", &folder);
    for machine in [&a, &b] {
        let leaks = files_containing(&machine.root, password);
        assert!(leaks.is_empty(), "the password is in {leaks:?}");
    }
}

#[test]
#[ignore = "needs a WebDAV server: see the file comment"]
fn m7_a_second_game_joins_with_only_its_manifest() {
    let server = Server::from_env();
    let folder = server_folder("m7");
    let a = Machine::new("m7", "a", &server);
    a.setup(&server, "device-a", &folder);
    a.write(SECOND, "run.dat", "second game, run");
    a.write(SECOND, "profile/one.dat", "second game, profile");
    expect(a.sync_by_manifest("push", SECOND), 0, "pushed");

    let b = Machine::new("m7", "b", &server);
    b.setup(&server, "device-b", &folder);
    expect(b.sync_by_manifest("pull", SECOND), 0, "pulled");
    // The template is `{documents}/second-game`: `Documents` under the home folder on this platform.
    assert!(
        b.home().join("Documents/second-game/run.dat").is_file(),
        "the template's folder holds the pulled save"
    );
    assert_eq!(b.read(SECOND, "run.dat"), "second game, run");
    assert_eq!(b.read(SECOND, "profile/one.dat"), "second game, profile");
    assert!(
        !b.save(FIXTURE, "slot1.sav").exists(),
        "the second game brings none of the fixture game's saves"
    );
}

/// The write guard, for a write one second later with a different size: a push that read its etag
/// before another writer changed the file is refused, and the other writer's bytes stay. Linux
/// only, because rclone (the Windows and macOS legs) ignores `If-Match`. Nextcloud's etag is built
/// from the modification time in whole seconds and the size, so `If-Match` catches a write only
/// when it comes in another second or with another size. A same-second, same-size write is not
/// covered here: `same_second_same_size_push_is_refused_by_the_revision_claim` does (#149).
#[cfg(all(target_os = "linux", feature = "http"))]
#[test]
#[ignore = "needs a Nextcloud server: see the file comment"]
fn racing_push_one_second_later_is_refused_and_keeps_the_other_writers_bytes() {
    use save_relay::creds::Secret;
    use save_relay::http::UreqTransport;
    use save_relay::remote::{Precondition, Remote};
    use save_relay::webdav::{files_url, WebDav};
    use save_relay::Error;
    use std::sync::Arc;

    let server = Server::from_env();
    let folder = server_folder("race");
    let url = files_url(&server.url, &server.user).unwrap();
    let dav = WebDav::new(
        &url,
        &server.user,
        Secret::new(server.password.clone()),
        Arc::new(UreqTransport::new()),
    );
    dav.mkcol(&folder).unwrap();
    let file = format!("{folder}/slot1.sav");

    // A creates the file and reads it: the etag A holds is the one the server reports.
    let mine = dav.put(&file, b"A's copy", &Precondition::Absent).unwrap();
    let (_, read_etag) = dav.get(&file).unwrap();
    assert_eq!(read_etag, mine);

    // B writes over it while A still holds the etag it read. The pause puts B's write in a later
    // second and the longer body changes its size; either one changes the server's etag, and the
    // test changes both.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let theirs = dav
        .put(&file, b"B's longer copy", &Precondition::Any)
        .unwrap();
    assert_ne!(theirs, mine);

    // A's push with the stale etag is refused and writes nothing.
    let err = dav
        .put(&file, b"A's new copy", &Precondition::Match(read_etag))
        .unwrap_err();
    assert!(matches!(err, Error::Changed), "{err:?}");
    let (bytes, etag) = dav.get(&file).unwrap();
    assert_eq!(bytes, b"B's longer copy");
    assert_eq!(etag, theirs);
}

/// The write guard for a same-second, same-size write (#149), against Nextcloud, through the
/// engine. Two machines read one revision; the first one's push writes the next revision; the
/// second one's push, with its stale read, still passes `If-Match` when both writes share a second
/// and a size, because Nextcloud's etag is then the same. The revision claim (`rev/<n>`) must
/// refuse it and keep the first machine's bytes. The writes land in one second only by chance, so
/// each attempt starts at the top of a second and an attempt whose etags differ is retried; the
/// test fails if none of the attempts gets the same etag. Linux only, like the test above.
#[cfg(all(target_os = "linux", feature = "http"))]
#[test]
#[ignore = "needs a Nextcloud server: see the file comment"]
fn same_second_same_size_push_is_refused_by_the_revision_claim() {
    use save_relay::creds::Secret;
    use save_relay::http::UreqTransport;
    use save_relay::layout::{entry_name, Layout};
    use save_relay::local::LocalStore;
    use save_relay::manifest::Manifest;
    use save_relay::platform::AppDirs;
    use save_relay::remote::{Precondition, Remote, RemoteEntry};
    use save_relay::state::ServerState;
    use save_relay::sync::{pull, push, Game, Session};
    use save_relay::webdav::{files_url, WebDav};
    use save_relay::Error;
    use std::cell::RefCell;
    use std::sync::Arc;

    /// The server, except that the next read of `state.json` returns `stale`.
    struct StaleStateRead<'a> {
        inner: &'a WebDav,
        state: String,
        stale: RefCell<Option<(Vec<u8>, String)>>,
    }

    impl Remote for StaleStateRead<'_> {
        fn list(&self, dir: &str) -> save_relay::Result<Vec<RemoteEntry>> {
            self.inner.list(dir)
        }

        fn get(&self, path: &str) -> save_relay::Result<(Vec<u8>, String)> {
            if path == self.state {
                if let Some(stale) = self.stale.borrow_mut().take() {
                    return Ok(stale);
                }
            }
            self.inner.get(path)
        }

        fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> save_relay::Result<String> {
            self.inner.put(path, bytes, pre)
        }

        fn mkcol(&self, dir: &str) -> save_relay::Result<()> {
            self.inner.mkcol(dir)
        }

        fn copy(&self, from: &str, to: &str) -> save_relay::Result<()> {
            self.inner.copy(from, to)
        }

        fn move_to(&self, from: &str, to: &str) -> save_relay::Result<()> {
            self.inner.move_to(from, to)
        }

        fn delete(&self, path: &str) -> save_relay::Result<()> {
            self.inner.delete(path)
        }

        fn needs_rev_marker(&self) -> bool {
            self.inner.needs_rev_marker()
        }
    }

    // Clean up on success, retry and assertion failure. Each retry also starts with fresh
    // local stores, so its first push cannot inherit LastSynced from a different server folder.
    struct Cleanup<'a> {
        remote: &'a dyn Remote,
        folder: String,
        local: PathBuf,
    }

    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = self.remote.delete(&self.folder);
            let _ = fs::remove_dir_all(&self.local);
        }
    }

    fn session<'a>(remote: &'a dyn Remote, layout: &Layout, device: &str, now: u64) -> Session<'a> {
        Session {
            remote,
            layout: layout.clone(),
            device: device.into(),
            now,
        }
    }

    let server = Server::from_env();
    let url = files_url(&server.url, &server.user).unwrap();
    let dav = WebDav::new(
        &url,
        &server.user,
        Secret::new(server.password.clone()),
        Arc::new(UreqTransport::new()),
    );
    let local = std::env::temp_dir().join(format!("sr-same-second-{}", stamp()));
    let manifest = r#"{"format":1,"id":"fixture","name":"Fixture",
        "saves_from":"launcher","saves":{},"include":["**/*.sav"]}"#;
    let machine = |device: &str| -> (Game, LocalStore) {
        let dir = local.join(device);
        let saves_dir = dir.join("saves");
        fs::create_dir_all(&saves_dir).unwrap();
        (
            Game {
                manifest: Manifest::from_json(manifest).unwrap(),
                saves_dir,
            },
            LocalStore::new(AppDirs {
                config: dir.join("config"),
                data: dir.join("data"),
            }),
        )
    };

    let mut proved = false;
    for attempt in 0..10 {
        // Start at the top of a second, so the writes below can land in one.
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_millis();
        std::thread::sleep(std::time::Duration::from_millis(
            u64::from(1000 - millis) + 5,
        ));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let folder = server_folder(&format!("same{attempt}"));
        let _cleanup = Cleanup {
            remote: &dav,
            folder: folder.clone(),
            local: local.clone(),
        };
        let layout = Layout::new(&folder).unwrap();
        let state = layout.state_path("fixture");

        let (game_c, store_c) = machine("dev-c");
        let (game_a, store_a) = machine("dev-a");
        let (game_b, store_b) = machine("dev-b");
        fs::write(game_c.saves_dir.join("a.sav"), "save one").unwrap();
        push(
            &session(&dav, &layout, "dev-c", now),
            &game_c,
            &store_c,
            None,
        )
        .unwrap();
        pull(
            &session(&dav, &layout, "dev-a", now),
            &game_a,
            &store_a,
            None,
            false,
        )
        .unwrap();
        pull(
            &session(&dav, &layout, "dev-b", now),
            &game_b,
            &store_b,
            None,
            true,
        )
        .unwrap();

        // B reads revision 1 and holds that read until after A's push.
        let read_by_b = dav.get(&state).unwrap();
        fs::write(game_a.saves_dir.join("a.sav"), "save two").unwrap();
        push(
            &session(&dav, &layout, "dev-a", now),
            &game_a,
            &store_a,
            None,
        )
        .unwrap();
        if dav.get(&state).unwrap().1 != read_by_b.1 {
            // Not one second and one size: the etags differ, so this attempt proves nothing.
            continue;
        }

        let stale = StaleStateRead {
            inner: &dav,
            state: state.clone(),
            stale: RefCell::new(Some(read_by_b)),
        };
        fs::write(game_b.saves_dir.join("a.sav"), "save bee").unwrap();
        let error = push(
            &session(&stale, &layout, "dev-b", now),
            &game_b,
            &store_b,
            None,
        )
        .unwrap_err();
        assert_eq!(error, Error::Changed);

        // A's revision stays: revision 2 from dev-a, its entry kept, B's entry gone.
        let (bytes, _) = dav.get(&state).unwrap();
        let current = ServerState::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!((current.revision, current.device.as_str()), (2, "dev-a"));
        let a_entry = entry_name(&save_relay::stamp::entry_stamp(now), "dev-a");
        assert_eq!(current.entry, a_entry);
        let history: Vec<String> = dav
            .list(&layout.history_dir("fixture"))
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert!(
            !history.iter().any(|name| name.ends_with("-dev-b")),
            "{history:?}"
        );
        let claim = dav.get(&layout.rev_path("fixture", "2")).unwrap().0;
        assert_eq!(claim, a_entry.as_bytes());
        let (kept, _) = dav
            .get(&format!("{}/a.sav", layout.entry_dir("fixture", &a_entry)))
            .unwrap();
        assert_eq!(kept, b"save two");
        proved = true;
        break;
    }
    assert!(
        proved,
        "no attempt wrote two revisions in one second with one size"
    );
}
