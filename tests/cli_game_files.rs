//! `save-relay game-files plan|download|cleanup` (#310) end to end through
//! `cli::run_with_net`: the real `Dropbox` source over a local server that speaks ranged
//! downloads (`support_gamefiles`), `MemoryStore` credentials and a temp-folder machine.
//! Envelopes, text progress, exit codes, the folder refusals, busy, resume after an
//! interruption and after Ctrl-C, and the explicit legacy root.
#![cfg(feature = "http")]

mod support_gamefiles;

use save_relay::cli::{request_interrupt, run_with_net, Net, EXIT_CANCELLED};
use save_relay::clients::{Client, ClientSet};
use save_relay::creds::{CredentialStore, MemoryStore, Secret};
use save_relay::fake::REFRESH_TOKEN;
use save_relay::http::{Request, Response, Transport};
use save_relay::local::{Config, GameRecord, LocalStore};
use save_relay::manifest::Manifest;
use save_relay::platform::{AppDirs, PathVars};
use save_relay::remote::MemoryRemote;
use save_relay::service::{token_account, Service};
use save_relay::{Error, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use support_gamefiles::*;

const T0: u64 = 1_793_476_260;
const ACCOUNT: &str = "fake@example.com";
const APP: &str = "ios-one";
/// 64 KiB: a 300 kB file takes five requests.
const CHUNK: &str = "65536";

static NEXT: AtomicUsize = AtomicUsize::new(0);
/// The Ctrl-C flag is one per process, so the tests that download take turns.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn clock() -> u64 {
    T0
}

fn no_browser(_: &str) -> Result<()> {
    Err(Error::SignInFailed("no browser".into()))
}

fn clients() -> ClientSet {
    ClientSet {
        google_desktop: Client {
            id: "g",
            secret: Some("s"),
        },
        google_device: Client {
            id: "g",
            secret: Some("s"),
        },
        dropbox: Client {
            id: "test-dropbox-app-key",
            secret: None,
        },
    }
}

/// Passes requests through, counting the ones that read file content (the manifest and every
/// chunk). Once `left` runs out it answers `Unreachable`; `interrupt_after` asks the download to
/// stop (as Ctrl-C does) when that many have been served.
struct Gate {
    inner: Arc<LocalTransport>,
    left: AtomicI64,
    interrupt_after: AtomicI64,
}

impl Gate {
    fn new(inner: Arc<LocalTransport>) -> Arc<Gate> {
        Arc::new(Gate {
            inner,
            left: AtomicI64::new(i64::MAX),
            interrupt_after: AtomicI64::new(i64::MAX),
        })
    }
}

impl Transport for Gate {
    fn send(&self, request: &Request) -> Result<Response> {
        if request.url.contains("/files/download") {
            if self.left.fetch_sub(1, Ordering::SeqCst) <= 0 {
                return Err(Error::Unreachable("the line went dead".into()));
            }
            if self.interrupt_after.fetch_sub(1, Ordering::SeqCst) == 1 {
                request_interrupt();
            }
        }
        self.inner.send(request)
    }
}

struct Machine {
    root: PathBuf,
    store: LocalStore,
    vars: PathVars,
    creds: MemoryStore,
    server: RangeServer,
    gate: Arc<Gate>,
}

impl Machine {
    /// A set-up Dropbox machine with `APP` on the server.
    fn new(files: &[(&str, &[u8])]) -> Machine {
        let machine = Machine::bare();
        machine.server.put_game(APP, files);
        machine
    }

    fn bare() -> Machine {
        let root = std::env::temp_dir().join(format!(
            "sr-cli-gf-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
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
        let server = RangeServer::start();
        let gate = Gate::new(server.transport());
        Machine {
            root,
            store,
            vars,
            creds: MemoryStore::new(),
            server,
            gate,
        }
    }

    fn sign_in(&self) {
        self.store
            .save_config(&Config {
                format: 1,
                server: "dropbox".into(),
                user: ACCOUNT.into(),
                root: "save-relay".into(),
                device: "desk".into(),
            })
            .unwrap();
        self.creds
            .set(
                &token_account(Service::Dropbox, ACCOUNT).unwrap(),
                &Secret::new(REFRESH_TOKEN.to_string()),
            )
            .unwrap();
    }

    fn signed_in(files: &[(&str, &[u8])]) -> Machine {
        let machine = Machine::new(files);
        machine.sign_in();
        machine
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Remember a game whose saves live in `saves`.
    fn game_with_saves(&self, saves: &Path) {
        let manifest = Manifest::from_json(
            r#"{"format":1,"id":"demo-game","name":"Demo Game","saves_from":"launcher","saves":{},"include":["**/*.sav"]}"#,
        )
        .unwrap();
        self.store
            .save_game(&GameRecord {
                manifest,
                saves_dir: Some(saves.to_path_buf()),
            })
            .unwrap();
    }

    fn line(&self, line: &str) -> (i32, String) {
        let args: Vec<String> = line.split_whitespace().map(String::from).collect();
        self.args(&args)
    }

    fn args(&self, args: &[String]) -> (i32, String) {
        let transport: Arc<dyn Transport> = self.gate.clone();
        let net = Net {
            transport,
            open_browser: &no_browser,
            clients: clients(),
            game_mode: false,
            clock,
        };
        let mut output = Vec::new();
        let code = run_with_net(
            args,
            &mut std::io::empty(),
            &mut output,
            &MemoryRemote::new(),
            &self.creds,
            &self.store,
            &self.vars,
            T0,
            &net,
        );
        (code, String::from_utf8(output).unwrap())
    }

    /// `line --json`, out of the one envelope line.
    fn json(&self, line: &str) -> (i32, Value) {
        let (code, out) = self.line(&format!("{line} --json"));
        assert_eq!(
            out.trim_end().lines().count(),
            1,
            "one envelope line: {out}"
        );
        let envelope: Value = serde_json::from_str(out.trim_end()).unwrap();
        assert!(
            !out.contains(REFRESH_TOKEN),
            "no token in the output: {out}"
        );
        (code, envelope)
    }

    fn with(&self, sub: &str, extra: &str) -> String {
        format!(
            "game-files {sub} {APP} --dest {} --state {} {extra}",
            self.dir("game").display(),
            self.dir("state").display()
        )
    }
}

fn two_files() -> (Vec<u8>, Vec<u8>) {
    (body(1, 300_000), body(2, 30_000))
}

#[test]
fn plan_reports_what_a_download_would_do_and_changes_nothing() {
    let (big, small) = two_files();
    let m = Machine::signed_in(&[("data/big.bin", &big), ("small.bin", &small)]);
    let (code, e) = m.json(&m.with("plan", ""));
    assert_eq!(code, 0, "{e}");
    assert_eq!(e["ok"], true);
    assert_eq!(e["command"], "game-files-plan");
    assert_eq!(e["data"]["app_id"], APP);
    assert_eq!(e["data"]["files_total"], 2);
    assert_eq!(e["data"]["bytes_to_fetch"], 330_000);
    assert_eq!(e["data"]["files_missing"], 2);
    assert!(!m.dir("game").exists(), "plan wrote the destination");
    assert!(!m.dir("state").exists(), "plan wrote the state folder");

    let (code, text) = m.line(&m.with("plan", ""));
    assert_eq!(code, 0);
    assert!(text.contains("2 files"), "{text}");
}

#[test]
fn download_installs_the_files_with_progress_lines_in_text_mode() {
    let _turn = serial();
    let (big, small) = two_files();
    let m = Machine::signed_in(&[("data/big.bin", &big), ("small.bin", &small)]);
    let (code, text) = m.line(&m.with("download", &format!("--chunk {CHUNK}")));
    assert_eq!(code, 0, "{text}");
    assert_eq!(fs::read(m.dir("game/data/big.bin")).unwrap(), big);
    assert_eq!(fs::read(m.dir("game/small.bin")).unwrap(), small);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= 3, "progress lines then a result: {text}");
    assert!(lines[0].starts_with(APP), "{text}");
    assert!(
        lines.last().unwrap().contains("complete"),
        "last line says complete: {text}"
    );
    assert!(text.contains("data/big.bin"), "{text}");
}

#[test]
fn download_json_is_one_envelope_with_the_final_progress() {
    let _turn = serial();
    let (big, small) = two_files();
    let m = Machine::signed_in(&[("data/big.bin", &big), ("small.bin", &small)]);
    let (code, e) = m.json(&m.with("download", &format!("--chunk {CHUNK}")));
    assert_eq!(code, 0, "{e}");
    assert_eq!(e["command"], "game-files-download");
    let data = &e["data"];
    assert_eq!(data["state"], "complete");
    assert_eq!(data["files_done"], 2);
    assert_eq!(data["bytes_done"], 330_000);
    assert_eq!(data["downloaded_files"], 2);
    assert_eq!(data["skipped_files"], 0);
    assert_eq!(data["bytes_fetched"], 330_000);

    // A second run finds everything in place and fetches no payload.
    let (code, e) = m.json(&m.with("download", &format!("--chunk {CHUNK}")));
    assert_eq!(code, 0, "{e}");
    assert_eq!(e["data"]["downloaded_files"], 0);
    assert_eq!(e["data"]["skipped_files"], 2);
    assert_eq!(e["data"]["bytes_fetched"], 0);
}

// ---------------------------------------------------------------------------------------
// Errors and exit codes
// ---------------------------------------------------------------------------------------

#[test]
fn usage_errors_are_exit_2_envelopes_named_for_the_subcommand() {
    let m = Machine::signed_in(&[("a.bin", b"abc")]);
    let d = m.dir("game");
    let s = m.dir("state");
    let (d, s) = (d.display(), s.display());
    let cases: Vec<(String, &str)> = vec![
        ("game-files".into(), "game-files"),
        ("game-files frobnicate".into(), "game-files"),
        ("game-files plan".into(), "game-files-plan"),
        (
            format!("game-files plan {APP} --state {s}"),
            "game-files-plan",
        ),
        (
            format!("game-files plan {APP} --dest {d}"),
            "game-files-plan",
        ),
        (
            format!("game-files plan Bad_ID --dest {d} --state {s}"),
            "game-files-plan",
        ),
        (
            format!("game-files download {APP} --dest {d} --state {s} --chunk 0"),
            "game-files-download",
        ),
        (
            format!("game-files download {APP} --dest {d} --state {s} --chunk x"),
            "game-files-download",
        ),
        (
            format!("game-files download {APP} --dest {d} --state {s} --retries 99"),
            "game-files-download",
        ),
        (
            format!("game-files download {APP} --dest {d} --state {s} --bogus"),
            "game-files-download",
        ),
        (
            format!("game-files download {APP} extra --dest {d} --state {s}"),
            "game-files-download",
        ),
        ("game-files cleanup".into(), "game-files-cleanup"),
        (
            format!("game-files cleanup --dest {d} --state {s}"),
            "game-files-cleanup",
        ),
        // `--legacy-root` must be relative, clean and explicit.
        (
            format!("game-files plan {APP} --dest {d} --state {s} --legacy-root /abs/root"),
            "game-files-plan",
        ),
        (
            format!("game-files plan {APP} --dest {d} --state {s} --legacy-root a/../b"),
            "game-files-plan",
        ),
        (
            format!("game-files plan {APP} --dest {d} --state {s} --legacy-root C:/x"),
            "game-files-plan",
        ),
        (
            format!("game-files plan {APP} --dest {d} --state {s} --legacy-root a//b"),
            "game-files-plan",
        ),
    ];
    for (line, name) in cases {
        let (code, e) = m.json(&line);
        assert_eq!(code, 2, "{line}: {e}");
        assert_eq!(e["ok"], false, "{line}");
        assert_eq!(e["command"], name, "{line}");
        assert_eq!(e["error"]["code"], "usage", "{line}: {e}");
        assert!(e["error"]["class"].is_string(), "{line}: class: {e}");
    }
    assert!(!m.dir("game").exists());
}

#[test]
fn a_machine_that_is_not_set_up_gets_not_set_up_exit_4() {
    let m = Machine::new(&[("a.bin", b"abc")]);
    for sub in ["plan", "download"] {
        let (code, e) = m.json(&m.with(sub, ""));
        assert_eq!(code, 4, "{e}");
        assert_eq!(e["error"]["code"], "not_set_up");
        assert_eq!(e["command"], format!("game-files-{sub}"));
    }
    // Set up, but the sign-in is gone from the credential store.
    m.sign_in();
    m.creds
        .delete(&token_account(Service::Dropbox, ACCOUNT).unwrap())
        .unwrap();
    let (code, e) = m.json(&m.with("download", ""));
    assert_eq!(code, 4, "{e}");
    assert_eq!(e["error"]["code"], "not_set_up");
    assert!(!m.dir("game").exists());
    let (code, text) = m.line(&m.with("plan", ""));
    assert_eq!(code, 4);
    assert!(text.starts_with("save-relay:"), "{text}");
}

/// Before the merge with the provider lane, Nextcloud had no game source and answered
/// `range_unsupported` (exit 5). Every service has one now (`service::open_game_source`), so a
/// Nextcloud setup without its stored app password is `not_set_up` (exit 4), like Dropbox
/// without its token above.
#[test]
fn a_nextcloud_setup_without_its_app_password_is_not_set_up() {
    let m = Machine::new(&[("a.bin", b"abc")]);
    m.store
        .save_config(&Config {
            format: 1,
            server: "https://cloud.example.org".into(),
            user: "alice".into(),
            root: "save-relay".into(),
            device: "desk".into(),
        })
        .unwrap();
    let (code, e) = m.json(&m.with("plan", ""));
    assert_eq!(code, 4, "{e}");
    assert_eq!(e["error"]["code"], "not_set_up");
}

#[test]
fn a_manifest_that_names_a_path_outside_the_folder_is_manifest_invalid_exit_2() {
    let _turn = serial();
    let m = Machine::signed_in(&[]);
    m.server.put(
        &format!("game-files/{APP}/manifest.json"),
        manifest(APP, &[("../escape.bin", b"abc")]),
    );
    for sub in ["plan", "download"] {
        let (code, e) = m.json(&m.with(sub, ""));
        assert_eq!(code, 2, "{e}");
        assert_eq!(e["error"]["code"], "manifest_invalid");
    }
    assert!(!m.dir("escape.bin").exists());
}

#[test]
fn a_manifest_missing_on_the_server_is_not_yet_uploaded_exit_5() {
    let m = Machine::signed_in(&[("a.bin", b"abc")]);
    let (code, e) = m.json(&format!(
        "game-files plan nothing-here --dest {} --state {}",
        m.dir("game").display(),
        m.dir("state").display()
    ));
    assert_eq!(code, 5, "{e}");
    assert_eq!(e["error"]["code"], "not_yet_uploaded");
}

#[test]
fn a_different_file_in_the_way_is_destination_conflict_exit_1_and_is_kept() {
    let _turn = serial();
    let m = Machine::signed_in(&[("a.bin", b"abc"), ("b.bin", b"def")]);
    fs::create_dir_all(m.dir("game")).unwrap();
    fs::write(m.dir("game/a.bin"), b"player data").unwrap();
    let (code, e) = m.json(&m.with("plan", ""));
    assert_eq!(code, 0, "{e}");
    assert_eq!(e["data"]["conflict_count"], 1);
    assert_eq!(e["data"]["conflicts"][0], "a.bin");
    let (code, e) = m.json(&m.with("download", ""));
    assert_eq!(code, 1, "{e}");
    assert_eq!(e["error"]["code"], "destination_conflict");
    assert_eq!(fs::read(m.dir("game/a.bin")).unwrap(), b"player data");
}

#[test]
fn another_download_on_the_same_state_folder_is_busy_exit_1() {
    let _turn = serial();
    let m = Machine::signed_in(&[("a.bin", b"abc")]);
    fs::create_dir_all(m.dir("state")).unwrap();
    let held = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(m.dir("state/lock"))
        .unwrap();
    held.lock().unwrap();
    let (code, e) = m.json(&m.with("download", ""));
    assert_eq!(code, 1, "{e}");
    assert_eq!(e["error"]["code"], "busy");
    assert_eq!(e["error"]["class"], "retryable");
    drop(held);
    let (code, e) = m.json(&m.with("download", ""));
    assert_eq!(code, 0, "{e}");
}

// ---------------------------------------------------------------------------------------
// Folder refusals (the saves folder, each other, foreign state)
// ---------------------------------------------------------------------------------------

#[test]
fn dest_and_state_may_not_touch_the_saves_folder_or_each_other() {
    let _turn = serial();
    let m = Machine::signed_in(&[("a.bin", b"abc")]);
    let saves = m.dir("games/demo/saves");
    fs::create_dir_all(&saves).unwrap();
    m.game_with_saves(&saves);
    let game = m.dir("games/demo");
    let other = m.dir("elsewhere");
    // (A game folder that holds a saves folder is allowed, if the manifest keeps clear of it:
    // see `a_game_folder_may_hold_the_saves_folder_unless_the_manifest_reaches_it`.)
    let cases: Vec<(PathBuf, PathBuf)> = vec![
        (saves.clone(), other.join("s")),
        (saves.join("inside"), other.join("s")),
        (other.join("g"), saves.clone()),
        (other.join("g"), saves.join("state")),
        (other.join("g"), game.clone()),
        // A `..` path to the same folder.
        (other.join("../games/demo/saves"), other.join("s")),
        // dest and state inside each other, or the same.
        (other.join("g"), other.join("g/state")),
        (other.join("g/dest"), other.join("g")),
        (other.join("g"), other.join("g")),
        // This program's own folders.
        (m.root.join("config/save-relay"), other.join("s")),
        (other.join("g"), m.root.join("data/save-relay/games")),
    ];
    for (dest, state) in cases {
        for sub in ["plan", "download"] {
            let line = format!(
                "game-files {sub} {APP} --dest {} --state {}",
                dest.display(),
                state.display()
            );
            let (code, e) = m.json(&line);
            assert_eq!(code, 1, "{line}: {e}");
            assert_eq!(e["error"]["code"], "destination_conflict", "{line}: {e}");
        }
    }
    // Nothing was created by any refusal.
    assert!(!other.exists());
    assert_eq!(fs::read_dir(&saves).unwrap().count(), 0);
    // And folders of their own are fine.
    let (code, e) = m.json(&m.with("download", ""));
    assert_eq!(code, 0, "{e}");
}

#[test]
fn a_game_folder_may_hold_the_saves_folder_unless_the_manifest_reaches_it() {
    let _turn = serial();
    let m = Machine::signed_in(&[("a.bin", b"abc"), ("saves/b.bin", b"def")]);
    let game = m.dir("games/demo");
    let saves = game.join("user");
    fs::create_dir_all(&saves).unwrap();
    fs::write(saves.join("slot1.sav"), b"precious").unwrap();
    m.game_with_saves(&saves);
    let state = m.dir("elsewhere/state");
    let line = |sub: &str| {
        format!(
            "game-files {sub} {APP} --dest {} --state {}",
            game.display(),
            state.display()
        )
    };
    // `a.bin` and `saves/b.bin` are clear of `user/`: plan and download work.
    for sub in ["plan", "download"] {
        let (code, e) = m.json(&line(sub));
        assert_eq!(code, 0, "{sub}: {e}");
    }
    assert_eq!(fs::read(game.join("a.bin")).unwrap(), b"abc");
    assert_eq!(fs::read(saves.join("slot1.sav")).unwrap(), b"precious");
    // A manifest that reaches the saves folder is refused, before anything is written.
    let m = Machine::signed_in(&[("a.bin", b"abc"), ("saves/x.bin", b"def")]);
    let game = m.dir("games/demo");
    let saves = game.join("saves");
    fs::create_dir_all(&saves).unwrap();
    m.game_with_saves(&saves);
    for sub in ["plan", "download"] {
        let line = format!(
            "game-files {sub} {APP} --dest {} --state {}",
            game.display(),
            m.dir("elsewhere/state").display()
        );
        let (code, e) = m.json(&line);
        assert_eq!(code, 1, "{sub}: {e}");
        assert_eq!(e["error"]["code"], "destination_conflict", "{sub}: {e}");
    }
    assert!(!game.join("a.bin").exists());
    assert_eq!(fs::read_dir(&saves).unwrap().count(), 0);
}

#[test]
fn cleanup_refuses_the_saves_folder() {
    let m = Machine::signed_in(&[("a.bin", b"abc")]);
    let saves = m.dir("saves");
    fs::create_dir_all(saves.join("parts")).unwrap();
    fs::write(saves.join("parts/slot1.sav"), b"precious").unwrap();
    m.game_with_saves(&saves);
    for state in [saves.clone(), saves.join("parts"), m.root.clone()] {
        let (code, e) = m.json(&format!("game-files cleanup --state {}", state.display()));
        assert_eq!(code, 1, "{e}");
        assert_eq!(e["error"]["code"], "destination_conflict");
    }
    assert_eq!(
        fs::read(saves.join("parts/slot1.sav")).unwrap(),
        b"precious"
    );
}

#[test]
fn a_folder_with_other_files_is_never_used_as_state() {
    let _turn = serial();
    let m = Machine::signed_in(&[("a.bin", b"abc")]);
    let state = m.dir("state");
    fs::create_dir_all(state.join("parts")).unwrap();
    fs::write(state.join("parts/notes.txt"), b"mine").unwrap();
    fs::write(state.join("notes.txt"), b"mine").unwrap();
    let (code, e) = m.json(&m.with("download", ""));
    assert_eq!(code, 1, "{e}");
    assert_eq!(e["error"]["code"], "destination_conflict");
    let (code, e) = m.json(&format!("game-files cleanup --state {}", state.display()));
    assert_eq!(code, 1, "{e}");
    assert_eq!(e["error"]["code"], "destination_conflict");
    assert_eq!(fs::read(state.join("notes.txt")).unwrap(), b"mine");
    assert_eq!(fs::read(state.join("parts/notes.txt")).unwrap(), b"mine");
}

/// A `parts` folder of someone else's files is not ours, even with nothing else beside it.
#[test]
fn a_state_holding_only_a_foreign_parts_folder_is_never_swept_or_deleted() {
    let _turn = serial();
    let m = Machine::signed_in(&[("a.bin", b"abc")]);
    let state = m.dir("state");
    fs::create_dir_all(state.join("parts")).unwrap();
    fs::write(state.join("parts/notes.txt"), b"mine").unwrap();
    fs::write(
        state.join("parts/ffffffffffffffffffffffffffffffff.part"),
        b"x",
    )
    .unwrap();
    for line in [
        m.with("download", ""),
        format!("game-files cleanup --state {}", state.display()),
    ] {
        let (code, e) = m.json(&line);
        assert_eq!(code, 1, "{e}");
        assert_eq!(e["error"]["code"], "destination_conflict", "{e}");
    }
    assert_eq!(fs::read(state.join("parts/notes.txt")).unwrap(), b"mine");
    // A subfolder or a stray `lock` folder is foreign too.
    fs::remove_file(state.join("parts/notes.txt")).unwrap();
    fs::remove_file(state.join("parts/ffffffffffffffffffffffffffffffff.part")).unwrap();
    fs::create_dir_all(state.join("parts/sub")).unwrap();
    let (code, e) = m.json(&format!("game-files cleanup --state {}", state.display()));
    assert_eq!(code, 1, "{e}");
    fs::remove_dir_all(state.join("parts/sub")).unwrap();
    fs::create_dir_all(state.join("lock")).unwrap();
    let (code, e) = m.json(&format!("game-files cleanup --state {}", state.display()));
    assert_eq!(code, 1, "{e}");
}

/// Leftovers of an interrupted run (a crashed atomic write) do not lock the user out. The
/// engine's atomic write (`fsx::Dir::write_atomic`) leaves `<name>.tmp`, so those are the
/// leftovers here (the lane's `craft_tmp` names came from the engine before the merge, which
/// no longer writes them).
#[test]
fn a_crashed_runs_leftovers_are_still_ours() {
    let _turn = serial();
    let m = Machine::signed_in(&[("a.bin", b"abc")]);
    let state = m.dir("state");
    fs::create_dir_all(state.join("parts")).unwrap();
    // A run claims the folder (writes the ownership marker) before it writes anything else, so
    // a crashed run's leftovers always sit beside the marker. A markerless folder holding a
    // `session.json.tmp` is not adopted any more (a foreign file with that name must survive):
    // see `a_state_folder_holding_only_a_foreign_session_file_is_not_adopted`.
    fs::write(
        state.join(".save-relay-state"),
        b"save-relay game-files state folder; the library deletes only its own staging here
",
    )
    .unwrap();
    fs::write(state.join("session.json.tmp"), b"x").unwrap();
    fs::write(
        state.join("parts/0123456789abcdef0123456789abcdef.json.tmp"),
        b"x",
    )
    .unwrap();
    let (code, e) = m.json(&m.with("download", ""));
    assert_eq!(code, 0, "{e}");
}

/// A `parts` link (or Windows junction) into another folder is never swept through.
#[test]
fn a_parts_link_in_the_state_folder_is_refused_and_its_target_kept() {
    let _turn = serial();
    let m = Machine::signed_in(&[("a.bin", b"abc")]);
    let target = m.dir("precious");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("keep.txt"), b"mine").unwrap();
    let state = m.dir("state");
    fs::create_dir_all(&state).unwrap();
    let link = state.join("parts");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(windows)]
    {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap();
        if !made.status.success() {
            eprintln!("cannot make a junction here; skipped");
            return;
        }
    }
    for line in [
        m.with("download", ""),
        format!("game-files cleanup --state {}", state.display()),
    ] {
        let (code, e) = m.json(&line);
        assert_eq!(code, 1, "{e}");
        assert_eq!(e["error"]["code"], "destination_conflict", "{e}");
    }
    assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"mine");
}

/// `link/..` is the parent of the link's target, so it cannot slip a dest into a saves folder.
#[cfg(unix)]
#[test]
fn a_dotdot_after_a_link_is_read_as_the_file_system_reads_it() {
    let _turn = serial();
    let m = Machine::signed_in(&[("a.bin", b"abc")]);
    let saves = m.dir("srv/saves");
    fs::create_dir_all(saves.join("sub")).unwrap();
    m.game_with_saves(&saves);
    let other = m.dir("elsewhere");
    fs::create_dir_all(&other).unwrap();
    std::os::unix::fs::symlink(saves.join("sub"), other.join("l")).unwrap();
    // `elsewhere/l/../x` is `srv/saves/x`, not `elsewhere/x`.
    let line = format!(
        "game-files download {APP} --dest {} --state {}",
        other.join("l/../x").display(),
        other.join("s").display()
    );
    let (code, e) = m.json(&line);
    assert_eq!(code, 1, "{e}");
    assert_eq!(e["error"]["code"], "destination_conflict", "{e}");
    assert!(!saves.join("x").exists());
}

/// A check that cannot read the remembered games must not pass.
#[test]
fn an_unreadable_game_record_refuses_instead_of_skipping_its_saves_folder() {
    let _turn = serial();
    let m = Machine::signed_in(&[("a.bin", b"abc")]);
    let saves = m.dir("saves");
    fs::create_dir_all(&saves).unwrap();
    m.game_with_saves(&saves);
    let record = m.dir("data/save-relay/games/demo-game.json");
    assert!(record.exists(), "the record is where the test expects");
    fs::write(&record, b"{ not json").unwrap();
    for line in [
        m.with("plan", ""),
        m.with("download", ""),
        format!("game-files cleanup --state {}", m.dir("state").display()),
    ] {
        let (code, e) = m.json(&line);
        assert_eq!(code, 1, "{line}: {e}");
        assert_eq!(e["error"]["code"], "destination_conflict", "{line}: {e}");
    }
    assert!(!m.dir("game").exists());
}

// ---------------------------------------------------------------------------------------
// Resume, Ctrl-C, cleanup
// ---------------------------------------------------------------------------------------

#[test]
fn a_dropped_line_keeps_the_checkpoint_and_the_next_run_resumes_mid_file() {
    let _turn = serial();
    let (big, small) = two_files();
    let m = Machine::signed_in(&[("a-big.bin", &big), ("b-small.bin", &small)]);
    // The manifest, then two chunks of the big file, then the line goes dead.
    m.gate.left.store(3, Ordering::SeqCst);
    let (code, e) = m.json(&m.with("download", &format!("--chunk {CHUNK} --retries 0")));
    assert_eq!(code, 5, "{e}");
    assert_eq!(e["error"]["code"], "server_unreachable");
    assert_eq!(e["error"]["class"], "retryable");
    assert!(!m.dir("game/a-big.bin").exists(), "nothing half-installed");
    assert!(m.dir("state/session.json").exists(), "the checkpoint stays");

    m.gate.left.store(i64::MAX, Ordering::SeqCst);
    m.server.clear_log();
    let (code, e) = m.json(&m.with("download", &format!("--chunk {CHUNK} --retries 0")));
    assert_eq!(code, 0, "{e}");
    assert_eq!(fs::read(m.dir("game/a-big.bin")).unwrap(), big);
    assert_eq!(fs::read(m.dir("game/b-small.bin")).unwrap(), small);
    let fetched = e["data"]["bytes_fetched"].as_u64().unwrap();
    assert!(
        fetched <= 330_000 - 65_536,
        "the durable chunks were not fetched again: {fetched}"
    );
    let starts: Vec<String> = m
        .server
        .log()
        .into_iter()
        .filter(|(path, _)| path.ends_with("a-big.bin"))
        .filter_map(|(_, range)| range)
        .collect();
    assert!(
        starts.first().is_some_and(|r| !r.starts_with("bytes=0-")),
        "resumed past the start: {starts:?}"
    );
}

#[test]
fn ctrl_c_stops_after_a_durable_chunk_with_exit_7_and_a_later_run_continues() {
    let _turn = serial();
    let (big, small) = two_files();
    let m = Machine::signed_in(&[("a-big.bin", &big), ("b-small.bin", &small)]);
    // Ctrl-C arrives while the second chunk of the big file is being read.
    m.gate.interrupt_after.store(3, Ordering::SeqCst);
    let (code, e) = m.json(&m.with("download", &format!("--chunk {CHUNK}")));
    assert_eq!(code, EXIT_CANCELLED, "{e}");
    assert_eq!(e["ok"], true);
    assert_eq!(e["data"]["state"], "cancelled");
    assert!(e["data"]["bytes_done"].as_u64().unwrap() >= 65_536);
    assert!(!m.dir("game/a-big.bin").exists());
    assert!(m.dir("state/session.json").exists());

    m.gate.interrupt_after.store(i64::MAX, Ordering::SeqCst);
    let (code, e) = m.json(&m.with("download", &format!("--chunk {CHUNK}")));
    assert_eq!(code, 0, "{e}");
    assert_eq!(e["data"]["state"], "complete");
    assert_eq!(fs::read(m.dir("game/a-big.bin")).unwrap(), big);
    assert!(e["data"]["bytes_fetched"].as_u64().unwrap() < 330_000);

    // In text mode the stop is a sentence, not a failure.
    let m = Machine::signed_in(&[("a-big.bin", &big)]);
    m.gate.interrupt_after.store(2, Ordering::SeqCst);
    let (code, text) = m.line(&m.with("download", &format!("--chunk {CHUNK}")));
    assert_eq!(code, EXIT_CANCELLED, "{text}");
    assert!(text.contains("Run the same command again"), "{text}");
}

#[test]
fn cleanup_removes_staging_and_keeps_the_installed_files() {
    let _turn = serial();
    let (big, small) = two_files();
    let m = Machine::signed_in(&[("a-big.bin", &big), ("b-small.bin", &small)]);
    m.gate.left.store(3, Ordering::SeqCst);
    let (code, _) = m.json(&m.with("download", &format!("--chunk {CHUNK} --retries 0")));
    assert_eq!(code, 5);
    m.gate.left.store(i64::MAX, Ordering::SeqCst);
    let installed = m.dir("game/keep.bin");
    fs::create_dir_all(m.dir("game")).unwrap();
    fs::write(&installed, b"already here").unwrap();

    let state = m.dir("state");
    let (code, e) = m.json(&format!("game-files cleanup --state {}", state.display()));
    assert_eq!(code, 0, "{e}");
    assert_eq!(e["command"], "game-files-cleanup");
    assert_eq!(e["data"]["cleaned"], true);
    assert!(!state.join("session.json").exists());
    assert!(!state.join("parts").exists());
    assert_eq!(fs::read(&installed).unwrap(), b"already here");

    // A state folder that does not exist yet is nothing to clean.
    let (code, e) = m.json(&format!(
        "game-files cleanup --state {}",
        m.dir("nope").display()
    ));
    assert_eq!(code, 0, "{e}");
    let (code, text) = m.line(&format!("game-files cleanup --state {}", state.display()));
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("Removed"), "{text}");
}

// ---------------------------------------------------------------------------------------
// The explicit legacy root
// ---------------------------------------------------------------------------------------

#[test]
fn the_legacy_root_is_the_operators_and_the_manifest_may_omit_its_app_id() {
    let _turn = serial();
    let m = Machine::bare();
    m.sign_in();
    let data = body(7, 5_000);
    let manifest = format!(
        r#"{{"version":1,"files":[{{"path":"lib/game.bin","size":{},"sha256":"{}"}}]}}"#,
        data.len(),
        sha(&data)
    );
    let root = "acnh-universal-game/android";
    m.server
        .put(&format!("{root}/manifest.json"), manifest.into_bytes());
    m.server.put(&format!("{root}/lib/game.bin"), data.clone());

    // Without the option the standard layout is read, and it has nothing for this app.
    let (code, e) = m.json(&m.with("plan", ""));
    assert_eq!(code, 5, "{e}");
    assert_eq!(e["error"]["code"], "not_yet_uploaded");

    let (code, e) = m.json(&m.with("plan", &format!("--legacy-root {root}")));
    assert_eq!(code, 0, "{e}");
    assert_eq!(e["data"]["files_total"], 1);
    let (code, e) = m.json(&m.with("download", &format!("--legacy-root {root}")));
    assert_eq!(code, 0, "{e}");
    assert_eq!(fs::read(m.dir("game/lib/game.bin")).unwrap(), data);

    // The player's own files are never a legacy manifest's to name.
    let evil = format!(
        r#"{{"version":1,"files":[{{"path":"user/save.dat","size":3,"sha256":"{}"}}]}}"#,
        sha(b"abc")
    );
    m.server
        .put(&format!("{root}/manifest.json"), evil.into_bytes());
    let (code, e) = m.json(&m.with("plan", &format!("--legacy-root {root}")));
    assert_eq!(code, 2, "{e}");
    assert_eq!(e["error"]["code"], "manifest_invalid");
}

#[test]
fn a_standard_manifest_cannot_choose_a_root_for_itself() {
    let m = Machine::signed_in(&[]);
    // A manifest that tries to point elsewhere has an unknown field and is refused.
    let sneaky = format!(
        r#"{{"version":1,"app_id":"{APP}","root":"other/place","files":[{{"path":"a.bin","size":3,"sha256":"{}"}}]}}"#,
        sha(b"abc")
    );
    m.server.put(
        &format!("game-files/{APP}/manifest.json"),
        sneaky.into_bytes(),
    );
    let (code, e) = m.json(&m.with("plan", ""));
    assert_eq!(code, 2, "{e}");
    assert_eq!(e["error"]["code"], "manifest_invalid");
}

#[test]
fn help_lists_the_game_files_commands() {
    let m = Machine::bare();
    let (code, text) = m.line("help");
    assert_eq!(code, 0);
    for needle in [
        "game-files plan",
        "game-files download",
        "game-files cleanup",
    ] {
        assert!(text.contains(needle), "{needle}: {text}");
    }
}

#[cfg(unix)]
#[test]
fn a_link_into_the_saves_folder_is_seen_through() {
    let m = Machine::signed_in(&[("a.bin", b"abc")]);
    let saves = m.dir("saves");
    fs::create_dir_all(&saves).unwrap();
    m.game_with_saves(&saves);
    std::os::unix::fs::symlink(&saves, m.dir("innocent")).unwrap();
    let line = format!(
        "game-files plan {APP} --dest {} --state {}",
        m.dir("innocent/game").display(),
        m.dir("state").display()
    );
    let (code, e) = m.json(&line);
    assert_eq!(code, 1, "{e}");
    assert_eq!(e["error"]["code"], "destination_conflict");
    assert_eq!(fs::read_dir(&saves).unwrap().count(), 0);
}
