//! `wrap` (F58): the flow with a scripted engine (no server, no game), `run_child` with a real
//! child process, and the command with a set-up machine against `MemoryRemote`.
//!
//! The real child is this test binary run again: `child("child_exits_7")` is the command
//! `<this exe> --exact child_exits_7 --ignored`, and that ignored test exits with its code. No
//! fixture program is needed, and the helper tests never run in a plain `cargo test`.

use save_relay::api::{Action, Held, Side, SyncData, SyncReport};
use save_relay::cli::run_with;
use save_relay::creds::{self, CredentialStore, MemoryStore, Secret};
use save_relay::local::{Config, LocalStore};
use save_relay::pending::{self, Answer, ChoiceKind, ANSWER_WAIT_SECS};
use save_relay::platform::{AppDirs, PathVars};
use save_relay::remote::MemoryRemote;
use save_relay::sync::Resolve;
use save_relay::transfer::Counts;
use save_relay::wrap::{self, Engine};
use save_relay::{stamp, Error};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

const GAME: &str = "demo-game";
const T0: u64 = 1_793_476_260; // 2026-10-31T19:51:00Z
const PASSWORD: &str = "xK9p-Q2mR-7vLd-W4tY-hN8z";
const SERVER: &str = "https://cloud.example.org";
const USER: &str = "alice";

// The clock is a plain function (`stamp::Clock`), so its time lives in a static. Tests that
// use it take `serial()`, which also puts the clock back to T0.
static CLOCK: AtomicU64 = AtomicU64::new(T0);
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    let guard = SERIAL.lock().unwrap_or_else(|poison| poison.into_inner());
    CLOCK.store(T0, Ordering::SeqCst);
    guard
}

fn clock() -> u64 {
    CLOCK.load(Ordering::SeqCst)
}

fn tick(secs: u64) {
    CLOCK.fetch_add(secs, Ordering::SeqCst);
}

/// A `sleep` that must never be asked: the flow here does not wait.
fn no_wait(_: u64) {
    panic!("the flow must not wait here");
}

/// A `sleep` that moves the clock on, and answers the pending choice the first time it is asked.
fn answer_then_tick(store: &LocalStore, answer: Answer) -> impl Fn(u64) + '_ {
    let answered = Cell::new(false);
    move |secs| {
        if !answered.get() {
            answered.set(true);
            pending::answer(store, GAME, answer).expect("the choice is pending");
        }
        tick(secs);
    }
}

/// The child command: this test binary, running only the ignored test `name`.
fn child(name: &str) -> Vec<String> {
    let exe = std::env::current_exe().expect("the test binary's path");
    vec![
        exe.to_string_lossy().into_owned(),
        "--exact".into(),
        name.into(),
        "--ignored".into(),
    ]
}

// The helper children. Each exits with the code in its name; they are ignored, so only the
// tests above run them, as child processes.

#[test]
#[ignore = "run as a child process by the tests in this file"]
fn child_exits_0() {
    std::process::exit(0);
}

#[test]
#[ignore = "run as a child process by the tests in this file"]
fn child_exits_7() {
    std::process::exit(7);
}

// ---- a scripted engine ----------------------------------------------------------------------

struct Fake {
    pulls: RefCell<VecDeque<Result<SyncReport, Error>>>,
    pushes: RefCell<VecDeque<Result<SyncReport, Error>>>,
    /// What `pull` says once its script is used up.
    pull_forever: SyncReport,
    calls: RefCell<Vec<String>>,
}

impl Fake {
    fn new(pulls: Vec<Result<SyncReport, Error>>, pushes: Vec<Result<SyncReport, Error>>) -> Fake {
        Fake {
            pulls: RefCell::new(pulls.into()),
            pushes: RefCell::new(pushes.into()),
            pull_forever: report(Action::Noop, ""),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn holding_forever(mut self) -> Fake {
        self.pull_forever = held();
        self
    }

    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

fn resolve_name(resolve: Option<Resolve>) -> &'static str {
    match resolve {
        None => "none",
        Some(Resolve::KeepServer) => "keep-server",
        Some(Resolve::KeepLocal) => "keep-local",
    }
}

impl Engine for Fake {
    fn game(&self) -> &str {
        GAME
    }
    fn pull(&self, resolve: Option<Resolve>, anyway: bool) -> Result<SyncReport, Error> {
        self.calls
            .borrow_mut()
            .push(format!("pull {} {anyway}", resolve_name(resolve)));
        self.pulls
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Ok(self.pull_forever.clone()))
    }
    fn push(&self, resolve: Option<Resolve>) -> Result<SyncReport, Error> {
        self.calls
            .borrow_mut()
            .push(format!("push {}", resolve_name(resolve)));
        self.pushes
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Ok(report(Action::Noop, "")))
    }
}

fn report(action: Action, message: &str) -> SyncReport {
    SyncReport {
        sync: SyncData {
            game: GAME.into(),
            action,
            revision: None,
            server: None,
            local: None,
            backup: None,
            counts: Counts::default(),
            message: message.into(),
        },
        held: None,
        server_card: None,
        local_card: None,
    }
}

fn conflict() -> SyncReport {
    let mut report = report(Action::Conflict, "both sides changed");
    report.sync.server = Some(Side {
        device: "server-device".into(),
        time: "2026-11-01T19:12:00Z".into(),
        revision: Some(7),
    });
    report.sync.local = Some(Side {
        device: "local-device".into(),
        time: "2026-11-01T19:13:00Z".into(),
        revision: None,
    });
    report
}

fn held() -> SyncReport {
    let mut report = report(Action::Held, "open elsewhere");
    report.held = Some(Held {
        device: "other-device".into(),
        since: "2026-11-01T19:12:00Z".into(),
    });
    report
}

struct Fixture {
    root: PathBuf,
    store: LocalStore,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let root =
            std::env::temp_dir().join(format!("save-relay-wrap-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dirs = AppDirs {
            config: root.join("config").join("save-relay"),
            data: root.join("data").join("save-relay"),
        };
        Fixture {
            store: LocalStore::new(dirs),
            root,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// ---- the flow -------------------------------------------------------------------------------

#[test]
fn a_clean_pull_and_push_play_the_game_and_return_its_exit_code() {
    let _serial = serial();
    let fx = Fixture::new("clean");
    let fake = Fake::new(
        vec![Ok(report(Action::Pulled, "downloaded"))],
        vec![Ok(report(Action::Pushed, "uploaded"))],
    );
    let code = wrap::run(&fake, &fx.store, &child("child_exits_7"), clock, &no_wait);
    assert_eq!(code, 7, "the game's exit code is wrap's");
    assert_eq!(fake.calls(), ["pull none false", "push none"]);
    assert_eq!(fx.store.load_pending(GAME).unwrap(), None);
}

#[test]
fn the_game_starts_even_when_pull_cannot_reach_the_server() {
    let _serial = serial();
    let fx = Fixture::new("unreachable");
    let fake = Fake::new(
        vec![Err(Error::Unreachable("no route to the server".into()))],
        vec![Ok(report(Action::Deferred, "kept here"))],
    );
    let code = wrap::run(&fake, &fx.store, &child("child_exits_7"), clock, &no_wait);
    assert_eq!(code, 7);
    assert_eq!(fake.calls(), ["pull none false", "push none"]);
}

#[test]
fn a_conflict_answered_keep_server_resolves_with_it() {
    let _serial = serial();
    let fx = Fixture::new("keep-server");
    let fake = Fake::new(
        vec![
            Ok(conflict()),
            Ok(report(Action::Pulled, "from the server")),
        ],
        vec![Ok(report(Action::Noop, ""))],
    );
    let sleep = answer_then_tick(&fx.store, Answer::KeepServer);
    let code = wrap::run(&fake, &fx.store, &child("child_exits_7"), clock, &sleep);
    assert_eq!(code, 7);
    assert_eq!(
        fake.calls(),
        ["pull none false", "pull keep-server false", "push none"]
    );
    assert_eq!(fx.store.load_pending(GAME).unwrap(), None);
}

#[test]
fn a_conflict_answered_keep_local_resolves_with_it() {
    let _serial = serial();
    let fx = Fixture::new("keep-local");
    let fake = Fake::new(
        vec![Ok(conflict()), Ok(report(Action::Pushed, "uploaded"))],
        vec![Ok(report(Action::Noop, ""))],
    );
    let sleep = answer_then_tick(&fx.store, Answer::KeepLocal);
    let code = wrap::run(&fake, &fx.store, &child("child_exits_7"), clock, &sleep);
    assert_eq!(code, 7);
    assert_eq!(
        fake.calls(),
        ["pull none false", "pull keep-local false", "push none"]
    );
}

#[test]
fn a_conflict_with_no_answer_plays_the_local_saves_and_resolves_nothing() {
    let _serial = serial();
    let fx = Fixture::new("no-answer");
    let fake = Fake::new(vec![Ok(conflict())], vec![Ok(report(Action::Noop, ""))]);
    let code = wrap::run(
        &fake,
        &fx.store,
        &child("child_exits_7"),
        clock,
        &|secs: u64| tick(secs),
    );
    assert_eq!(code, 7);
    assert_eq!(fake.calls(), ["pull none false", "push none"]);
    assert!(
        clock() - T0 >= ANSWER_WAIT_SECS,
        "the wait ran its full length"
    );
    assert_eq!(fx.store.load_pending(GAME).unwrap(), None);
}

#[test]
fn a_held_game_played_anyway_pulls_past_the_mark() {
    let _serial = serial();
    let fx = Fixture::new("play-anyway");
    let fake = Fake::new(
        vec![Ok(held()), Ok(report(Action::Pulled, "downloaded"))],
        vec![Ok(report(Action::Noop, ""))],
    );
    let sleep = answer_then_tick(&fx.store, Answer::PlayAnyway);
    let code = wrap::run(&fake, &fx.store, &child("child_exits_7"), clock, &sleep);
    assert_eq!(code, 7);
    assert_eq!(
        fake.calls(),
        ["pull none false", "pull none true", "push none"]
    );
}

#[test]
fn a_held_game_answered_wait_starts_once_the_mark_is_gone() {
    let _serial = serial();
    let fx = Fixture::new("wait-free");
    let fake = Fake::new(
        vec![Ok(held()), Ok(held()), Ok(report(Action::Noop, ""))],
        vec![Ok(report(Action::Noop, ""))],
    );
    let sleep = answer_then_tick(&fx.store, Answer::Wait);
    let code = wrap::run(&fake, &fx.store, &child("child_exits_7"), clock, &sleep);
    assert_eq!(code, 7);
    assert_eq!(
        fake.calls(),
        [
            "pull none false",
            "pull none false",
            "pull none false",
            "push none"
        ]
    );
}

#[test]
fn a_held_game_that_stays_held_starts_on_the_local_saves_after_the_wait() {
    let _serial = serial();
    let fx = Fixture::new("wait-forever");
    let fake = Fake::new(vec![Ok(held())], vec![Ok(report(Action::Noop, ""))]).holding_forever();
    let sleep = answer_then_tick(&fx.store, Answer::Wait);
    let code = wrap::run(&fake, &fx.store, &child("child_exits_7"), clock, &sleep);
    assert_eq!(code, 7);
    let calls = fake.calls();
    assert_eq!(calls.first().map(String::as_str), Some("pull none false"));
    assert_eq!(calls.last().map(String::as_str), Some("push none"));
    assert!(
        !calls.iter().any(|call| call == "pull none true"),
        "waiting never plays past the mark"
    );
    assert!(
        calls
            .iter()
            .filter(|call| *call == "pull none false")
            .count()
            > 100,
        "the mark was read again for the whole wait: {calls:?}"
    );
    assert!(clock() - T0 >= ANSWER_WAIT_SECS);
}

#[test]
fn a_conflict_found_at_exit_is_left_pending_and_not_waited_for() {
    let _serial = serial();
    let fx = Fixture::new("push-conflict");
    let fake = Fake::new(vec![Ok(report(Action::Noop, ""))], vec![Ok(conflict())]);
    let code = wrap::run(&fake, &fx.store, &child("child_exits_7"), clock, &no_wait);
    assert_eq!(code, 7);
    assert_eq!(fake.calls(), ["pull none false", "push none"]);
    let choice = fx
        .store
        .load_pending(GAME)
        .unwrap()
        .expect("the choice is left for the panel");
    assert_eq!(choice.kind, ChoiceKind::Conflict);
    assert_eq!(choice.answer, None);
}

#[test]
fn a_settled_exit_clears_a_choice_written_earlier() {
    let _serial = serial();
    let fx = Fixture::new("settled");
    let earlier = wrap::choice_for(GAME, &conflict(), T0).expect("a conflict is a choice");
    fx.store.save_pending(&earlier).unwrap();
    let fake = Fake::new(
        vec![Ok(report(Action::Noop, ""))],
        vec![Ok(report(Action::Pushed, "uploaded"))],
    );
    let code = wrap::run(&fake, &fx.store, &child("child_exits_7"), clock, &no_wait);
    assert_eq!(code, 7);
    assert_eq!(fx.store.load_pending(GAME).unwrap(), None);
}

#[test]
fn a_program_that_cannot_start_still_gets_its_push_and_exits_1() {
    let _serial = serial();
    let fx = Fixture::new("missing-program");
    let fake = Fake::new(
        vec![Ok(report(Action::Noop, ""))],
        vec![Ok(report(Action::Noop, ""))],
    );
    let command = vec!["save-relay-no-such-program-4f2a".to_string()];
    let code = wrap::run(&fake, &fx.store, &command, clock, &no_wait);
    assert_eq!(code, 1);
    assert_eq!(fake.calls(), ["pull none false", "push none"]);
}

// ---- the pure parts -------------------------------------------------------------------------

#[test]
fn choice_for_names_a_conflict_and_a_hold_and_nothing_else() {
    let choice = wrap::choice_for(GAME, &conflict(), T0).expect("a conflict is a choice");
    assert_eq!(choice.kind, ChoiceKind::Conflict);
    assert_eq!(choice.game, GAME);
    assert_eq!(choice.server, conflict().sync.server);
    assert_eq!(choice.local, conflict().sync.local);
    assert_eq!(choice.held, None);
    assert_eq!(choice.time, stamp::rfc3339(T0));
    assert_eq!(choice.answer, None);

    let choice = wrap::choice_for(GAME, &held(), T0).expect("a hold is a choice");
    assert_eq!(choice.kind, ChoiceKind::Held);
    assert_eq!(choice.server, None);
    assert_eq!(choice.held, held().held);

    for action in [
        Action::Noop,
        Action::Adopted,
        Action::Pulled,
        Action::Pushed,
        Action::Offline,
        Action::Deferred,
    ] {
        assert_eq!(
            wrap::choice_for(GAME, &report(action, ""), T0),
            None,
            "{action:?} is not a choice"
        );
    }
}

#[test]
fn resolve_for_maps_each_answer_to_the_engine_call_it_needs() {
    assert_eq!(
        wrap::resolve_for(ChoiceKind::Conflict, Answer::KeepServer),
        (Some(Resolve::KeepServer), false)
    );
    assert_eq!(
        wrap::resolve_for(ChoiceKind::Conflict, Answer::KeepLocal),
        (Some(Resolve::KeepLocal), false)
    );
    assert_eq!(
        wrap::resolve_for(ChoiceKind::Held, Answer::PlayAnyway),
        (None, true)
    );
    assert_eq!(
        wrap::resolve_for(ChoiceKind::Held, Answer::Wait),
        (None, false)
    );
}

#[test]
fn game_id_is_the_explicit_one_else_the_steam_app_id() {
    let env = |vars: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    };
    let steam = env(&[("SteamAppId", "1245620")]);
    assert_eq!(
        wrap::game_id(Some("demo-game"), &steam).unwrap(),
        "demo-game"
    );
    assert_eq!(wrap::game_id(None, &steam).unwrap(), "steam-1245620");

    // Steam's 64-bit shortcut ids do not fit: the next variable is tried, then nothing.
    let shortcut = env(&[("SteamGameId", "4294967297"), ("STEAM_COMPAT_APP_ID", "99")]);
    assert_eq!(wrap::game_id(None, &shortcut).unwrap(), "steam-99");

    let nothing = env(&[]);
    assert!(matches!(
        wrap::game_id(None, &nothing),
        Err(Error::Usage(_))
    ));
    assert!(matches!(
        wrap::game_id(Some(""), &steam),
        Err(Error::Usage(_))
    ));
}

// ---- the child process ----------------------------------------------------------------------

#[test]
fn run_child_returns_the_childs_own_exit_code() {
    assert_eq!(wrap::run_child(&child("child_exits_0")).unwrap(), 0);
    assert_eq!(wrap::run_child(&child("child_exits_7")).unwrap(), 7);
}

#[test]
fn run_child_names_a_program_that_cannot_start() {
    let error = wrap::run_child(&["save-relay-no-such-program-4f2a".to_string()])
        .expect_err("no such program");
    assert!(
        matches!(&error, Error::Io(text) if text.contains("save-relay-no-such-program-4f2a")),
        "{error:?}"
    );
}

#[test]
fn run_child_refuses_an_empty_command() {
    assert!(matches!(wrap::run_child(&[]), Err(Error::Usage(_))));
}

// ---- the command, with a set-up machine -----------------------------------------------------

/// `wrap` arguments: the head (`--game ...`), then `--`, then the command.
fn wrap_args(head: &[&str], command: Vec<String>) -> Vec<String> {
    let mut args = vec!["wrap".to_string()];
    args.extend(head.iter().map(|arg| arg.to_string()));
    args.push("--".to_string());
    args.extend(command);
    args
}

#[test]
fn wrap_on_a_machine_not_set_up_starts_the_game_and_prints_nothing() {
    let fx = Fixture::new("not-set-up");
    let vars = PathVars {
        home: fx.root.join("home"),
        data: fx.root.join("data"),
        config: fx.root.join("config"),
        documents: None,
    };
    let store = LocalStore::new(AppDirs::from_vars(&vars));
    let remote = MemoryRemote::new();
    let creds = MemoryStore::new();
    let args = wrap_args(&["--game", GAME], child("child_exits_7"));
    let mut stdin = Cursor::new(Vec::<u8>::new());
    let mut stdout = Vec::<u8>::new();
    let code = run_with(
        &args,
        &mut stdin,
        &mut stdout,
        &remote,
        &creds,
        &store,
        &vars,
        T0,
    );
    assert_eq!(code, 7, "the game ran and its code is wrap's");
    assert!(
        stdout.is_empty(),
        "wrap prints nothing itself: {}",
        String::from_utf8_lossy(&stdout)
    );
}

#[test]
fn wrap_on_a_set_up_machine_syncs_the_saves_and_exits_with_the_game_code() {
    let fx = Fixture::new("synced");
    let vars = PathVars {
        home: fx.root.join("home"),
        data: fx.root.join("data"),
        config: fx.root.join("config"),
        documents: None,
    };
    let store = LocalStore::new(AppDirs::from_vars(&vars));
    store
        .save_config(&Config {
            format: 1,
            server: SERVER.into(),
            user: USER.into(),
            root: "save-relay".into(),
            device: "this-pc".into(),
        })
        .unwrap();
    let creds = MemoryStore::new();
    creds
        .set(
            &creds::account(SERVER, USER).unwrap(),
            &Secret::new(PASSWORD),
        )
        .unwrap();

    let manifest = fx.root.join("demo.json");
    fs::write(
        &manifest,
        r#"{"format":1,"id":"demo-game","name":"Demo Game","saves_from":"launcher","saves":{},"include":["**/*.sav"]}"#,
    )
    .unwrap();
    let saves = fx.root.join("saves");
    fs::create_dir_all(&saves).unwrap();
    fs::write(saves.join("slot1.sav"), "save data").unwrap();

    let remote = MemoryRemote::new();
    let manifest_arg = manifest.to_string_lossy().into_owned();
    let saves_arg = saves.to_string_lossy().into_owned();
    let args = wrap_args(
        &[
            "--game",
            GAME,
            "--manifest",
            &manifest_arg,
            "--saves-dir",
            &saves_arg,
        ],
        child("child_exits_7"),
    );
    let mut stdin = Cursor::new(Vec::<u8>::new());
    let mut stdout = Vec::<u8>::new();
    let code = run_with(
        &args,
        &mut stdin,
        &mut stdout,
        &remote,
        &creds,
        &store,
        &vars,
        T0,
    );
    assert_eq!(code, 7);
    assert!(stdout.is_empty(), "{}", String::from_utf8_lossy(&stdout));
    assert!(
        remote
            .file_paths()
            .iter()
            .any(|path| path.ends_with("slot1.sav")),
        "the save reached the server: {:?}",
        remote.file_paths()
    );
}
