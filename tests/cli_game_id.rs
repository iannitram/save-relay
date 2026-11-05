//! The `<game>` argument is checked against the manifest id rule before any file is read or any
//! server call is made (#147, part 3). The rule itself is `manifest::check_id`.

use save_relay::cli::run_with;
use save_relay::creds::MemoryStore;
use save_relay::local::LocalStore;
use save_relay::manifest::{check_id, Manifest};
use save_relay::platform::{AppDirs, PathVars};
use save_relay::remote::{MemoryRemote, Remote};
use save_relay::Error;
use serde_json::Value;
use std::collections::BTreeMap;
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
            "save-relay-cli-game-id-{}-{}",
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

/// Every file under `dir` with its bytes, so a test can show that nothing changed.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = Vec::new();
    files_under(dir, &mut found);
    found
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
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
fn pull_and_status_refuse_a_game_id_outside_the_rule() {
    let remote = Arc::new(MemoryRemote::new());
    let machine = pushed_machine(&remote, "tablet");
    let before = snapshot(&machine.root);
    let server_before = remote.file_paths();

    for line in [
        "pull ../../x",
        "pull DEMO-GAME",
        "status ../../x",
        "status DEMO-GAME",
        "pull Demo-Game",
    ] {
        let (code, envelope) = machine.json(line, T0 + 60);
        assert_eq!(code, 2, "{line}: {envelope}");
        assert_eq!(envelope["error"]["code"], "usage", "{line}: {envelope}");
        assert!(
            envelope["error"]["message"]
                .as_str()
                .unwrap()
                .contains("<game> must be 1-64 lowercase letters"),
            "{line}: {envelope}"
        );
    }
    // "-demo" starts with '-', so the argument parser refuses it as a flag before the game check.
    let (code, envelope) = machine.json("status -demo", T0 + 60);
    assert_eq!(code, 2, "{envelope}");
    assert_eq!(envelope["error"]["code"], "usage", "{envelope}");

    assert_eq!(
        snapshot(&machine.root),
        before,
        "no file was read or written"
    );
    assert_eq!(remote.file_paths(), server_before, "no server change");
}

#[test]
fn a_game_id_inside_the_rule_still_works() {
    let remote = Arc::new(MemoryRemote::new());
    let machine = pushed_machine(&remote, "tablet");
    // demo-game is a launcher game, so status needs --saves-dir too.
    let saves = machine.root.join("saves");
    let (code, envelope) = machine.json(
        &format!("status {GAME} --saves-dir {}", saves.display()),
        T0 + 60,
    );
    assert_eq!(code, 0, "{envelope}");
    // A well-formed id this machine has never seen is still game_unknown, not usage.
    let (code, envelope) = machine.json("pull never-seen", T0 + 60);
    assert_eq!(code, 2, "{envelope}");
    assert_eq!(envelope["error"]["code"], "game_unknown");
}

#[test]
fn manifest_validate_messages_are_unchanged() {
    let with_id = |id: &str| {
        format!(
            r#"{{"format":1,"id":"{id}","name":"x","saves":{{"linux":"~/x"}},"include":["*"]}}"#
        )
    };
    match Manifest::from_json(&with_id("Bad")) {
        Err(Error::ManifestInvalid(m)) => assert_eq!(
            m,
            "id: must be 1-64 lowercase letters, digits, or hyphens, starting with a letter or digit"
        ),
        other => panic!("{other:?}"),
    }
    match Manifest::from_json(&with_id("con")) {
        Err(Error::ManifestInvalid(m)) => assert_eq!(
            m,
            "id: is a name Windows reserves for devices (con, prn, aux, nul, com0-9, lpt0-9)"
        ),
        other => panic!("{other:?}"),
    }
    assert!(check_id("demo-game").is_ok());
    assert!(matches!(
        check_id("../../x"),
        Err(Error::ManifestInvalid(_))
    ));
}
