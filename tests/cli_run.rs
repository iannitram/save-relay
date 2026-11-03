//! `cli::run_with` end to end against `MemoryRemote` and `MemoryStore`: every command's
//! `--json` envelope (field names exactly as docs/code-map.md § API shape), the exit codes, a
//! parse error printed as an envelope, and the app password never reaching stdout or a file.

use save_relay::cli::{connection, run_with};
use save_relay::creds::{CredentialStore, MemoryStore, Secret};
use save_relay::local::LocalStore;
use save_relay::platform::{AppDirs, PathVars};
use save_relay::remote::{MemoryRemote, Precondition, Remote, RemoteEntry};
use save_relay::{Error, Result};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

const PASSWORD: &str = "xK9p-Q2mR-7vLd-W4tY-hN8z";
const SERVER: &str = "https://cloud.example.org";
const GAME: &str = "demo-game";
const T0: u64 = 1_793_476_260; // 2026-10-31T19:51:00Z
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// One machine: its own folders and credential store, sharing the server with the others.
struct Machine {
    root: PathBuf,
    store: LocalStore,
    vars: PathVars,
    creds: MemoryStore,
    remote: Arc<MemoryRemote>,
}

impl Machine {
    fn new(remote: &Arc<MemoryRemote>) -> Machine {
        let root = std::env::temp_dir().join(format!(
            "save-relay-cli-run-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("saves")).unwrap();
        let vars = PathVars {
            home: root.join("home"),
            data: root.join("data"),
            config: root.join("config"),
            documents: None,
        };
        let store = LocalStore::new(AppDirs {
            config: root.join("config").join("save-relay"),
            data: root.join("data").join("save-relay"),
        });
        let manifest = r#"{"format":1,"id":"demo-game","name":"Demo Game",
            "saves_from":"launcher","saves":{},"include":["**/*.sav"]}"#;
        fs::write(root.join("demo.json"), manifest).unwrap();
        Machine {
            root,
            store,
            vars,
            creds: MemoryStore::new(),
            remote: remote.clone(),
        }
    }

    /// Run one command line (split on spaces) with `stdin`; returns the exit code and stdout.
    fn run(&self, line: &str, stdin: &str, now: u64) -> (i32, String) {
        let args: Vec<String> = line.split_whitespace().map(String::from).collect();
        self.run_args(&args, stdin, now)
    }

    fn run_args(&self, args: &[String], stdin: &str, now: u64) -> (i32, String) {
        self.run_on(&*self.remote, args, stdin, now)
    }

    fn run_on(&self, remote: &dyn Remote, args: &[String], stdin: &str, now: u64) -> (i32, String) {
        let mut input = stdin.as_bytes();
        let mut output = Vec::new();
        let code = run_with(
            args,
            &mut input,
            &mut output,
            remote,
            &self.creds,
            &self.store,
            &self.vars,
            now,
        );
        (code, String::from_utf8(output).unwrap())
    }

    fn setup(&self, device: &str) {
        let line = format!("setup --server {SERVER} --user player --device {device} --json");
        let (code, out) = self.run(&line, &format!("{PASSWORD}\n"), T0);
        assert_eq!(code, 0, "{out}");
    }

    /// The common flags of a game command.
    fn game_flags(&self) -> String {
        format!(
            "{GAME} --manifest {} --saves-dir {}",
            self.root.join("demo.json").display(),
            self.root.join("saves").display()
        )
    }

    fn write_save(&self, name: &str, text: &str) {
        let path = self.root.join("saves").join(name);
        fs::write(&path, text).unwrap();
        // A fixed modification time, so the local side of a report reads the same every run.
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_secs(T0 + 30))
            .unwrap();
    }

    fn json(&self, line: &str, now: u64) -> (i32, Value) {
        let (code, out) = self.run(&format!("{line} --json"), "", now);
        (code, one_envelope(&out))
    }
}

/// Exactly one line, and it is JSON.
fn one_envelope(out: &str) -> Value {
    assert_eq!(
        out.lines().count(),
        1,
        "expected one envelope line, got: {out}"
    );
    serde_json::from_str(out).unwrap_or_else(|error| panic!("not JSON ({error}): {out}"))
}

fn keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

fn set(names: &[&'static str]) -> BTreeSet<&'static str> {
    names.iter().copied().collect()
}

fn files_under(dir: &Path, found: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files_under(&path, found);
            } else {
                found.push(path);
            }
        }
    }
}

/// A machine set up as `device`, with one save file and one revision pushed.
fn pushed_machine(remote: &Arc<MemoryRemote>, device: &str) -> Machine {
    let machine = Machine::new(remote);
    machine.setup(device);
    machine.write_save("slot1.sav", "chapter one");
    let (code, out) = machine.run(&format!("push {} --json", machine.game_flags()), "", T0);
    assert_eq!(code, 0, "{out}");
    machine
}

#[test]
fn setup_envelope_has_the_exact_fields_and_never_the_password() {
    let remote = Arc::new(MemoryRemote::new());
    let machine = Machine::new(&remote);
    let (code, out) = machine.run(
        &format!("setup --server {SERVER} --user player --device Strix-Tablet --json"),
        &format!("{PASSWORD}\r\n"),
        T0,
    );
    assert_eq!(code, 0, "{out}");
    let envelope = one_envelope(&out);
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["command"], "setup");
    let data = &envelope["data"];
    assert_eq!(
        keys(data),
        set(&[
            "service",
            "server",
            "user",
            "device",
            "root",
            "credential_store"
        ])
    );
    assert_eq!(data["service"], "nextcloud");
    assert_eq!(data["server"], SERVER);
    assert_eq!(data["user"], "player");
    assert_eq!(data["device"], "strix-tablet");
    assert_eq!(data["root"], "save-relay");
    assert_eq!(data["credential_store"], "memory");

    // The password is in the credential store and nowhere else.
    let stored = machine
        .creds
        .get("player@cloud.example.org")
        .unwrap()
        .unwrap();
    assert_eq!(stored.expose(), PASSWORD);
    assert!(!out.contains(PASSWORD));
    let mut files = Vec::new();
    files_under(&machine.root, &mut files);
    assert!(files.iter().any(|f| f.ends_with("config.json")));
    for file in &files {
        let bytes = fs::read(file).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(PASSWORD),
            "password found in {}",
            file.display()
        );
    }
    for path in remote.file_paths() {
        let bytes = remote.read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(PASSWORD));
    }
    // The server folder exists.
    assert!(remote
        .list("")
        .unwrap()
        .iter()
        .any(|e| e.name == "save-relay"));
}

#[test]
fn password_stays_out_of_every_later_command() {
    let remote = Arc::new(MemoryRemote::new());
    let machine = pushed_machine(&remote, "tablet");
    for line in ["status", "pull", "push"] {
        for json in ["", " --json"] {
            let (_, out) = machine.run(
                &format!("{line} {}{json}", machine.game_flags()),
                "",
                T0 + 5,
            );
            assert!(!out.contains(PASSWORD), "{line}: {out}");
        }
    }
    let (_, out) = machine.run(&format!("history {GAME}"), "", T0 + 5);
    assert!(!out.contains(PASSWORD));
    let mut files = Vec::new();
    files_under(&machine.root, &mut files);
    for file in &files {
        let bytes = fs::read(file).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(PASSWORD));
    }
}

#[test]
fn setup_text_mode_says_what_was_set_up() {
    let remote = Arc::new(MemoryRemote::new());
    let machine = Machine::new(&remote);
    let (code, out) = machine.run(
        &format!("setup --server {SERVER} --user player --device tablet --root games/saves"),
        &format!("{PASSWORD}\n"),
        T0,
    );
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "Set up: this machine (tablet) syncs through https://cloud.example.org as player, in the folder 'games/saves'.\n\
         The app password is stored in the memory credential store.\n"
    );
}

#[test]
fn setup_refuses_an_empty_password_and_a_wrong_one_stores_nothing() {
    let remote = Arc::new(MemoryRemote::new());
    let machine = Machine::new(&remote);
    let line = format!("setup --server {SERVER} --user player --device tablet --json");

    let (code, out) = machine.run(&line, "\n", T0);
    assert_eq!(code, 2);
    assert_eq!(one_envelope(&out)["error"]["code"], "usage");

    struct Refuses;
    impl Remote for Refuses {
        fn list(&self, _: &str) -> Result<Vec<RemoteEntry>> {
            Err(Error::AuthFailed)
        }
        fn get(&self, _: &str) -> Result<(Vec<u8>, String)> {
            Err(Error::AuthFailed)
        }
        fn put(&self, _: &str, _: &[u8], _: &Precondition) -> Result<String> {
            Err(Error::AuthFailed)
        }
        fn mkcol(&self, _: &str) -> Result<()> {
            Err(Error::AuthFailed)
        }
        fn copy(&self, _: &str, _: &str) -> Result<()> {
            Err(Error::AuthFailed)
        }
        fn move_to(&self, _: &str, _: &str) -> Result<()> {
            Err(Error::AuthFailed)
        }
        fn delete(&self, _: &str) -> Result<()> {
            Err(Error::AuthFailed)
        }
    }
    let args: Vec<String> = line.split_whitespace().map(String::from).collect();
    let (code, out) = machine.run_on(&Refuses, &args, &format!("{PASSWORD}\n"), T0);
    assert_eq!(code, 4);
    let envelope = one_envelope(&out);
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["command"], "setup");
    assert_eq!(envelope["error"]["code"], "auth_failed");
    assert!(!out.contains(PASSWORD));
    assert!(machine
        .creds
        .get("player@cloud.example.org")
        .unwrap()
        .is_none());
    assert!(machine.store.load_config().unwrap().is_none());
}

#[test]
fn a_machine_that_is_not_set_up_exits_4() {
    let remote = Arc::new(MemoryRemote::new());
    let machine = Machine::new(&remote);
    let (code, envelope) = machine.json(&format!("pull {}", machine.game_flags()), T0);
    assert_eq!(code, 4);
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["command"], "pull");
    assert_eq!(envelope["error"]["code"], "not_set_up");
    assert_eq!(keys(&envelope["error"]), set(&["code", "message"]),);
    assert_eq!(keys(&envelope), set(&["ok", "command", "error"]));

    // Config present, password missing: also not set up.
    machine.setup("tablet");
    machine.creds.delete("player@cloud.example.org").unwrap();
    assert_eq!(
        connection(&machine.store, &machine.creds).unwrap_err(),
        Error::NotSetUp
    );
    machine
        .creds
        .set("player@cloud.example.org", &Secret::new(PASSWORD))
        .unwrap();
    assert!(connection(&machine.store, &machine.creds).is_ok());
}

#[test]
fn push_pull_status_history_restore_envelopes() {
    let remote = Arc::new(MemoryRemote::new());
    let machine = Machine::new(&remote);
    machine.setup("tablet");
    machine.write_save("slot1.sav", "chapter one");
    let flags = machine.game_flags();

    let sync_keys = set(&[
        "game",
        "action",
        "revision",
        "server",
        "local",
        "backup",
        "counts",
        "message",
        "held",
        "server_card",
        "local_card",
    ]);
    let side_keys = set(&["device", "time", "revision"]);
    let count_keys = set(&["uploaded", "downloaded", "deleted"]);

    // push
    let (code, envelope) = machine.json(&format!("push {flags}"), T0);
    assert_eq!(code, 0);
    assert_eq!(keys(&envelope), set(&["ok", "command", "data"]));
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["command"], "push");
    let data = &envelope["data"];
    assert_eq!(keys(data), sync_keys);
    assert_eq!(data["game"], GAME);
    assert_eq!(data["action"], "pushed");
    assert_eq!(data["revision"], 1);
    assert_eq!(keys(&data["server"]), side_keys);
    assert_eq!(data["server"]["device"], "tablet");
    assert_eq!(keys(&data["counts"]), count_keys);
    assert_eq!(data["counts"]["uploaded"], 1);
    assert!(data["message"].as_str().unwrap().len() > 3);

    // pull: nothing to do
    let (code, envelope) = machine.json(&format!("pull {flags}"), T0 + 60);
    assert_eq!(code, 0);
    assert_eq!(envelope["command"], "pull");
    assert_eq!(keys(&envelope["data"]), sync_keys);
    assert_eq!(envelope["data"]["action"], "noop");

    let (code, envelope) = machine.json(&format!("status {flags}"), T0 + 60);
    assert_eq!(code, 0);
    assert_eq!(envelope["command"], "status");
    let data = &envelope["data"];
    assert_eq!(
        keys(data),
        set(&[
            "game",
            "service",
            "reachable",
            "would",
            "server",
            "local",
            "last_synced_revision",
            "local_changed",
            "message",
            "held",
            "pending",
        ])
    );
    assert_eq!(data["service"], "nextcloud");
    assert_eq!(data["reachable"], true);
    assert_eq!(data["would"], "noop");
    assert_eq!(data["last_synced_revision"], 1);
    assert_eq!(data["local_changed"], false);

    // history
    let (code, envelope) = machine.json(&format!("history {GAME}"), T0 + 60);
    assert_eq!(code, 0);
    assert_eq!(envelope["command"], "history");
    let data = &envelope["data"];
    assert_eq!(keys(data), set(&["game", "current", "entries"]));
    assert_eq!(data["current"], "20261031T195100Z-tablet");
    let entries = data["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        keys(&entries[0]),
        set(&["entry", "device", "time", "current", "card"])
    );
    assert_eq!(entries[0]["entry"], "20261031T195100Z-tablet");
    assert_eq!(entries[0]["current"], true);

    // restore (after a local change, which is kept as a backup first)
    machine.write_save("slot1.sav", "chapter two, not pushed");
    let (code, envelope) = machine.json(
        &format!("restore {flags} 20261031T195100Z-tablet"),
        T0 + 120,
    );
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(envelope["command"], "restore");
    let data = &envelope["data"];
    assert_eq!(
        keys(data),
        set(&[
            "game",
            "entry",
            "revision",
            "new_entry",
            "backup",
            "counts",
            "message"
        ])
    );
    assert_eq!(data["entry"], "20261031T195100Z-tablet");
    assert_eq!(data["revision"], 2);
    assert!(data["backup"].is_string());
    assert_eq!(
        fs::read_to_string(machine.root.join("saves/slot1.sav")).unwrap(),
        "chapter one"
    );
}

/// Two machines, a conflict between them. Returns (a, b), A holding unpushed local changes.
fn conflicting_machines(remote: &Arc<MemoryRemote>) -> (Machine, Machine) {
    let a = pushed_machine(remote, "amd-handheld");
    let b = Machine::new(remote);
    b.setup("strix-tablet");
    let (code, out) = b.run(&format!("pull {} --json", b.game_flags()), "", T0 + 60);
    assert_eq!(code, 0, "{out}");
    b.write_save("slot1.sav", "chapter two on the tablet");
    let (code, out) = b.run(&format!("push {} --json", b.game_flags()), "", T0 + 120);
    assert_eq!(code, 0, "{out}");
    a.write_save("slot1.sav", "chapter two on the handheld");
    (a, b)
}

#[test]
fn a_conflict_exits_3_and_resolves_both_ways() {
    let remote = Arc::new(MemoryRemote::new());
    let (a, _b) = conflicting_machines(&remote);

    let (code, envelope) = a.json(&format!("pull {}", a.game_flags()), T0 + 180);
    assert_eq!(code, 3);
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["data"]["action"], "conflict");
    assert_eq!(envelope["data"]["server"]["device"], "strix-tablet");
    assert_eq!(envelope["data"]["local"]["device"], "amd-handheld");
    assert_eq!(
        fs::read_to_string(a.root.join("saves/slot1.sav")).unwrap(),
        "chapter two on the handheld",
        "a conflict touches nothing"
    );

    let (code, envelope) = a.json(
        &format!("pull {} --resolve keep-server", a.game_flags()),
        T0 + 240,
    );
    assert_eq!(code, 0);
    assert_eq!(envelope["data"]["action"], "pulled");
    assert!(envelope["data"]["backup"].is_string());
    assert_eq!(
        fs::read_to_string(a.root.join("saves/slot1.sav")).unwrap(),
        "chapter two on the tablet"
    );
}

#[test]
fn a_push_conflict_resolves_with_keep_local() {
    let remote = Arc::new(MemoryRemote::new());
    let (a, _b) = conflicting_machines(&remote);
    let (code, envelope) = a.json(&format!("push {}", a.game_flags()), T0 + 180);
    assert_eq!(code, 3);
    assert_eq!(envelope["data"]["action"], "conflict");
    let (code, envelope) = a.json(
        &format!("push {} --resolve keep-local", a.game_flags()),
        T0 + 240,
    );
    assert_eq!(code, 0);
    assert_eq!(envelope["data"]["action"], "pushed");
}

#[test]
fn a_game_open_elsewhere_exits_6() {
    let remote = Arc::new(MemoryRemote::new());
    let a = pushed_machine(&remote, "amd-handheld");
    let b = Machine::new(&remote);
    b.setup("strix-tablet");
    let (code, out) = b.run(&format!("pull {} --json", b.game_flags()), "", T0 + 60);
    assert_eq!(code, 0, "{out}");
    // B has the game open now; A asks.
    let (code, envelope) = a.json(&format!("pull {}", a.game_flags()), T0 + 120);
    assert_eq!(code, 6);
    assert_eq!(envelope["data"]["action"], "held");
    assert_eq!(envelope["data"]["held"]["device"], "strix-tablet");
}

#[test]
fn an_unreachable_server_exits_5_for_history_and_0_for_pull() {
    let remote = Arc::new(MemoryRemote::new());
    let machine = pushed_machine(&remote, "tablet");
    remote.set_reachable(false);

    let (code, envelope) = machine.json(&format!("history {GAME}"), T0 + 60);
    assert_eq!(code, 5);
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["command"], "history");
    assert_eq!(envelope["error"]["code"], "server_unreachable");

    // Playing offline is not a failure: the game must still start.
    let (code, envelope) = machine.json(&format!("pull {}", machine.game_flags()), T0 + 60);
    assert_eq!(code, 0);
    assert_eq!(envelope["data"]["action"], "offline");

    let (code, envelope) = machine.json(&format!("status {}", machine.game_flags()), T0 + 60);
    assert_eq!(code, 0);
    assert_eq!(envelope["data"]["reachable"], false);
}

#[test]
fn a_game_is_remembered_after_one_run_with_its_manifest() {
    let remote = Arc::new(MemoryRemote::new());
    let machine = Machine::new(&remote);
    machine.setup("tablet");
    let platform = save_relay::platform::Platform::current().key();
    let manifest = format!(
        r#"{{"format":1,"id":"tmpl-game","name":"Template Game",
            "saves":{{"{platform}":"{{home}}/tmpl-saves"}},"include":["**/*.sav"]}}"#
    );
    let path = machine.root.join("tmpl.json");
    fs::write(&path, manifest).unwrap();
    let saves = machine.root.join("home").join("tmpl-saves");
    fs::create_dir_all(&saves).unwrap();
    fs::write(saves.join("a.sav"), "one").unwrap();

    let (code, envelope) =
        machine.json(&format!("push tmpl-game --manifest {}", path.display()), T0);
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(envelope["data"]["action"], "pushed");

    // No flags now: the manifest and its folder are remembered.
    let (code, envelope) = machine.json("status tmpl-game", T0 + 60);
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(envelope["data"]["would"], "noop");
    let (code, envelope) = machine.json("pull tmpl-game", T0 + 60);
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(envelope["data"]["action"], "noop");
}

#[test]
fn usage_errors_exit_2() {
    let remote = Arc::new(MemoryRemote::new());
    let machine = Machine::new(&remote);
    machine.setup("tablet");

    // A game this machine has never seen, without --manifest.
    let (code, envelope) = machine.json("pull never-seen", T0);
    assert_eq!(code, 2);
    assert_eq!(envelope["error"]["code"], "game_unknown");

    // --manifest for another game.
    let manifest = machine.root.join("demo.json");
    let (code, envelope) = machine.json(
        &format!("pull other-game --manifest {}", manifest.display()),
        T0,
    );
    assert_eq!(code, 2);
    assert_eq!(envelope["error"]["code"], "usage");

    // A launcher game without --saves-dir (#48): refused before any server call or file write.
    let (code, envelope) = machine.json(
        &format!("pull {GAME} --manifest {}", manifest.display()),
        T0,
    );
    assert_eq!(code, 2);
    assert_eq!(envelope["command"], "pull");
    assert_eq!(envelope["error"]["code"], "usage");
    assert!(envelope["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--saves-dir"));
    assert!(remote.file_paths().is_empty(), "no server call was made");
    assert!(
        machine.store.load_game(GAME).unwrap().is_none(),
        "nothing was remembered"
    );
    // The same refusal in text mode.
    let (code, out) = machine.run(
        &format!("push {GAME} --manifest {}", manifest.display()),
        "",
        T0,
    );
    assert_eq!(code, 2);
    assert!(out.contains("--saves-dir"), "{out}");
}

#[test]
fn a_parse_error_under_json_is_an_envelope() {
    let remote = Arc::new(MemoryRemote::new());
    let machine = Machine::new(&remote);

    let (code, out) = machine.run("pull --json", "", T0);
    assert_eq!(code, 2);
    let envelope = one_envelope(&out);
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["command"], "pull");
    assert_eq!(envelope["error"]["code"], "usage");
    assert_eq!(envelope["error"]["message"], "missing <game>");

    let (code, out) = machine.run("--json frobnicate", "", T0);
    assert_eq!(code, 2);
    let envelope = one_envelope(&out);
    assert_eq!(envelope["command"], "unknown");
    assert_eq!(envelope["error"]["code"], "usage");

    // --password is refused, and never echoed.
    let line = format!("setup --server {SERVER} --user player --password {PASSWORD} --json");
    let (code, out) = machine.run(&line, "", T0);
    assert_eq!(code, 2);
    assert_eq!(one_envelope(&out)["command"], "setup");
    assert!(!out.contains(PASSWORD));

    // Without --json the same error is plain text.
    let (code, out) = machine.run("pull", "", T0);
    assert_eq!(code, 2);
    assert_eq!(
        out,
        "save-relay: missing <game>\nRun `save-relay help` for the commands.\n"
    );
}

#[test]
fn help_and_version_and_commands_not_built_yet() {
    let remote = Arc::new(MemoryRemote::new());
    let machine = Machine::new(&remote);

    let (code, out) = machine.run("help", "", T0);
    assert_eq!(code, 0);
    assert!(out.starts_with("save-relay <command>"));
    let (code, out) = machine.run("version --json", "", T0);
    assert_eq!(code, 0);
    let envelope = one_envelope(&out);
    assert_eq!(envelope["command"], "version");
    assert_eq!(envelope["data"]["version"], env!("CARGO_PKG_VERSION"));
    let (code, out) = machine.run("", "", T0);
    assert_eq!(code, 0);
    assert!(out.starts_with("save-relay <command>"));

    // Each of these arrives with its own issue; until then it says so, exit 1.
    for line in [
        "where",
        "which 12",
        "answer demo-game wait",
        "panel",
        "setup --service gdrive",
    ] {
        let (code, out) = machine.run(&format!("{line} --json"), "", T0);
        assert_eq!(code, 1, "{line}: {out}");
        assert_eq!(one_envelope(&out)["error"]["code"], "not_built", "{line}");
    }
}

#[test]
fn text_mode_conflict_prompt_and_history_table() {
    let remote = Arc::new(MemoryRemote::new());
    let (a, _b) = conflicting_machines(&remote);

    let (code, out) = a.run(&format!("pull {}", a.game_flags()), "", T0 + 180);
    assert_eq!(code, 3);
    assert_eq!(
        out,
        "Your saves changed on this machine and on the server since they last synced.\n\
         Server: saved on strix-tablet, 2026-10-31 19:53 UTC (revision 2).\n\
         This machine: amd-handheld, 2026-10-31 19:51 UTC.\n\
         Keep which? The other one stays in history and can be restored.\n\
         \x20 save-relay pull demo-game --resolve keep-server    keep the server's\n\
         \x20 save-relay pull demo-game --resolve keep-local     keep this machine's\n"
    );

    let (code, out) = a.run(&format!("history {GAME}"), "", T0 + 180);
    assert_eq!(code, 0);
    let table = [
        "  ENTRY                          DEVICE        TIME",
        "* 20261031T195300Z-strix-tablet  strix-tablet  2026-10-31 19:53 UTC",
        "  20261031T195100Z-amd-handheld  amd-handheld  2026-10-31 19:51 UTC",
        "* is the current entry. Bring one back with: save-relay restore demo-game <entry>",
    ];
    assert_eq!(
        out,
        format!(
            "{}
",
            table.join(
                "
"
            )
        )
    );
}
