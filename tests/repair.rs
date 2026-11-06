//! Tests for #159: `repair` releases a revision claim that a push stranded. The crash is the real
//! path: the push moves its complete entry into history, claims `rev/<n+1>`, and dies before it
//! writes `state.json`, so it cannot release the claim. Repair then moves the entry to
//! `set-aside/` and releases the claim, and the next push succeeds. Also the CLI: the prompt, `--yes`
//! and `--json`.
use save_relay::api::Action;
use save_relay::cli::run_with;
use save_relay::creds::MemoryStore;
use save_relay::layout::Layout;
use save_relay::local::LocalStore;
use save_relay::manifest::Manifest;
use save_relay::platform::{AppDirs, PathVars};
use save_relay::remote::{MemoryRemote, Precondition, Remote, RemoteEntry};
use save_relay::state::ServerState;
use save_relay::sync::{push, repair, Game, Session};
use save_relay::{Error, Result};
use serde_json::Value;
use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const GAME: &str = "fixture";
const SERVER: &str = "https://cloud.example.org";
const PASSWORD: &str = "xK9p-Q2mR-7vLd-W4tY-hN8z";
const NOW: u64 = 1_793_476_260;
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A WebDAV-like remote with the revision claim. While `crash` is set the writer has died: its
/// `state.json` write fails and so does every claim release, so the claim is left behind.
struct Marked {
    inner: Arc<MemoryRemote>,
    crash: Cell<bool>,
}

impl Marked {
    fn new(inner: &Arc<MemoryRemote>) -> Marked {
        Marked {
            inner: inner.clone(),
            crash: Cell::new(false),
        }
    }
}

impl Remote for Marked {
    fn list(&self, p: &str) -> Result<Vec<RemoteEntry>> {
        self.inner.list(p)
    }
    fn get(&self, p: &str) -> Result<(Vec<u8>, String)> {
        self.inner.get(p)
    }
    fn put(&self, p: &str, b: &[u8], c: &Precondition) -> Result<String> {
        if self.crash.get() && p.ends_with("/state.json") {
            return Err(Error::Unreachable("the writer died".into()));
        }
        self.inner.put(p, b, c)
    }
    fn mkcol(&self, p: &str) -> Result<()> {
        self.inner.mkcol(p)
    }
    fn copy(&self, a: &str, b: &str) -> Result<()> {
        self.inner.copy(a, b)
    }
    fn move_to(&self, a: &str, b: &str) -> Result<()> {
        self.inner.move_to(a, b)
    }
    fn delete(&self, p: &str) -> Result<()> {
        if self.crash.get() && p.contains("/rev/") {
            return Err(Error::Unreachable("the writer died".into()));
        }
        self.inner.delete(p)
    }
    fn needs_rev_marker(&self) -> bool {
        true
    }
}

fn manifest_json() -> &'static str {
    r#"{"format":1,"id":"fixture","name":"Fixture",
        "saves_from":"launcher","saves":{},"include":["**/*.sav"]}"#
}

fn layout() -> Layout {
    Layout::new("save-relay").unwrap()
}

/// One machine's folders: its saves, its manifest and its local store.
struct Root(PathBuf);

impl Root {
    fn new() -> Root {
        let path = std::env::temp_dir().join(format!(
            "save-relay-repair-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("saves")).unwrap();
        fs::write(path.join("fixture.json"), manifest_json()).unwrap();
        Root(path)
    }
    fn saves(&self) -> PathBuf {
        self.0.join("saves")
    }
    fn manifest(&self) -> PathBuf {
        self.0.join("fixture.json")
    }
    fn store(&self) -> LocalStore {
        LocalStore::new(AppDirs {
            config: self.0.join("config").join("save-relay"),
            data: self.0.join("data").join("save-relay"),
        })
    }
    fn vars(&self) -> PathVars {
        PathVars {
            home: self.0.join("home"),
            data: self.0.join("data"),
            config: self.0.join("config"),
            documents: None,
        }
    }
    fn game(&self) -> Game {
        Game {
            manifest: Manifest::from_json(manifest_json()).unwrap(),
            saves_dir: self.saves(),
        }
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn session<'a>(remote: &'a dyn Remote, device: &str, now: u64) -> Session<'a> {
    Session {
        remote,
        layout: layout(),
        device: device.into(),
        now,
    }
}

/// Pushes revision 1, then pushes revision 2 with the writer dying at its state write. Returns the
/// entry that the dead push named in its claim.
fn crash_a_push(root: &Root, remote: &Marked) -> String {
    let store = root.store();
    fs::write(root.saves().join("a.sav"), "one").unwrap();
    push(&session(remote, "dev-a", NOW), &root.game(), &store, None).unwrap();
    fs::write(root.saves().join("a.sav"), "two").unwrap();
    remote.crash.set(true);
    let died = push(
        &session(remote, "dev-a", NOW + 60),
        &root.game(),
        &store,
        None,
    );
    remote.crash.set(false);
    assert!(died.is_err(), "the crashed push must fail");
    let claim = remote.inner.get(&layout().rev_path(GAME, "2")).unwrap().0;
    let entry = String::from_utf8(claim).unwrap();
    assert!(
        remote
            .inner
            .get(&format!("{}/a.sav", layout().entry_dir(GAME, &entry)))
            .is_ok(),
        "the dead push's entry is complete in history"
    );
    entry
}

fn state_revision(server: &MemoryRemote) -> u64 {
    let (bytes, _) = server.get(&layout().state_path(GAME)).unwrap();
    ServerState::from_json(std::str::from_utf8(&bytes).unwrap())
        .unwrap()
        .revision
}

#[test]
fn a_push_that_dies_at_its_state_write_is_repaired_and_the_next_push_succeeds() {
    let root = Root::new();
    let server = Arc::new(MemoryRemote::new());
    let remote = Marked::new(&server);
    let crashed = crash_a_push(&root, &remote);

    let report = repair(&session(&remote, "repair", NOW + 90), GAME, true).unwrap();
    assert_eq!(report.markers.len(), 1);
    let marker = &report.markers[0];
    assert_eq!(marker.status, "set_aside");
    let moved = layout().set_aside_path(GAME, 2, &crashed);
    assert_eq!(marker.set_aside.as_deref(), Some(moved.as_str()));
    assert!(server.get(&layout().rev_path(GAME, "2")).is_err());
    assert!(server
        .get(&format!("{}/a.sav", layout().entry_dir(GAME, &crashed)))
        .is_err());
    assert_eq!(server.get(&format!("{moved}/a.sav")).unwrap().0, b"two");

    fs::write(root.saves().join("a.sav"), "three").unwrap();
    let pushed = push(
        &session(&remote, "dev-a", NOW + 120),
        &root.game(),
        &root.store(),
        None,
    )
    .unwrap();
    assert_eq!(pushed.sync.action, Action::Pushed);
    assert_eq!(state_revision(&server), 2);
}

/// The CLI on one machine, sharing the server with the library's writer.
struct Cli {
    root: Root,
    server: Arc<MemoryRemote>,
    creds: MemoryStore,
}

impl Cli {
    fn new() -> Cli {
        let cli = Cli {
            root: Root::new(),
            server: Arc::new(MemoryRemote::new()),
            creds: MemoryStore::new(),
        };
        let (code, out) = cli.run(
            &format!("setup --server {SERVER} --user player --device dev-a --json"),
            &format!("{PASSWORD}\n"),
        );
        assert_eq!(code, 0, "{out}");
        cli
    }

    /// Runs one command line with `stdin`; returns the exit code and stdout.
    fn run(&self, line: &str, stdin: &str) -> (i32, String) {
        let args: Vec<String> = line.split_whitespace().map(String::from).collect();
        let mut input = stdin.as_bytes();
        let mut output = Vec::new();
        let remote = Marked::new(&self.server);
        let code = run_with(
            &args,
            &mut input,
            &mut output,
            &remote,
            &self.creds,
            &self.root.store(),
            &self.root.vars(),
            NOW + 200,
        );
        (code, String::from_utf8(output).unwrap())
    }

    fn json(&self, line: &str) -> (i32, Value) {
        let (code, out) = self.run(line, "");
        let value =
            serde_json::from_str(&out).unwrap_or_else(|error| panic!("not JSON ({error}): {out}"));
        (code, value)
    }

    fn game_flags(&self) -> String {
        format!(
            "{GAME} --manifest {} --saves-dir {}",
            self.root.manifest().display(),
            self.root.saves().display()
        )
    }

    fn claim_exists(&self) -> bool {
        self.server.get(&layout().rev_path(GAME, "2")).is_ok()
    }

    fn history_entry_exists(&self, entry: &str) -> bool {
        self.server
            .get(&format!("{}/a.sav", layout().entry_dir(GAME, entry)))
            .is_ok()
    }

    /// Makes the crash: a dead push that leaves its claim behind.
    fn crash(&self) -> String {
        crash_a_push(&self.root, &Marked::new(&self.server))
    }

    /// A push of the saves folder through the CLI; it must succeed.
    fn push_ok(&self, content: &str) {
        fs::write(self.root.saves().join("a.sav"), content).unwrap();
        let (code, out) = self.run(&format!("push {} --json", self.game_flags()), "");
        assert_eq!(code, 0, "{out}");
        assert_eq!(state_revision(&self.server), 2);
    }
}

#[test]
fn repair_json_without_yes_reports_the_set_aside_and_changes_nothing() {
    let cli = Cli::new();
    let crashed = cli.crash();
    let (code, value) = cli.json(&format!("repair {GAME} --json"));
    assert_eq!(code, 0, "{value}");
    assert_eq!(value["ok"], true);
    let marker = &value["data"]["markers"][0];
    assert_eq!(marker["status"], "releasable_set_aside");
    assert!(marker["set_aside"].is_null());
    assert!(cli.claim_exists());
    assert!(cli.history_entry_exists(&crashed));
}

#[test]
fn repair_prompt_declined_changes_nothing() {
    let cli = Cli::new();
    let crashed = cli.crash();
    let (code, out) = cli.run(&format!("repair {GAME}"), "n\n");
    assert_eq!(code, 0, "{out}");
    assert!(cli.claim_exists());
    assert!(cli.history_entry_exists(&crashed));
}

#[test]
fn repair_prompt_accepted_sets_the_entry_aside_and_the_next_push_succeeds() {
    let cli = Cli::new();
    let crashed = cli.crash();
    let (code, out) = cli.run(&format!("repair {GAME}"), "y\n");
    assert_eq!(code, 0, "{out}");
    assert!(!cli.claim_exists());
    assert!(!cli.history_entry_exists(&crashed));
    assert!(cli
        .server
        .get(&format!(
            "{}/a.sav",
            layout().set_aside_path(GAME, 2, &crashed)
        ))
        .is_ok());
    cli.push_ok("three");
}

#[test]
fn repair_yes_json_sets_the_entry_aside_and_the_next_push_succeeds() {
    let cli = Cli::new();
    let crashed = cli.crash();
    let (code, value) = cli.json(&format!("repair {GAME} --yes --json"));
    assert_eq!(code, 0, "{value}");
    let marker = &value["data"]["markers"][0];
    assert_eq!(marker["status"], "set_aside");
    assert_eq!(
        marker["set_aside"],
        layout().set_aside_path(GAME, 2, &crashed)
    );
    assert!(!cli.claim_exists());
    cli.push_ok("three");
}
