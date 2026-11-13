//! `wrap` and termination signals (F80, audit S3), Unix. A real signal goes to a real `wrap`
//! host process: this test binary run again as `signal_host`, which calls `wrap::run` with a
//! wrapped game that is this binary run again as `signal_game` (a sleep the signal must stop).
//! The engine is scripted: its exit push removes a `mark` file, as a push clears this device's
//! `playing.json`. Two signals: the first stops the wait and runs the exit push; a second one
//! during that push exits at once and leaves the mark.
#![cfg(unix)]

use save_relay::api::SyncReport;
use save_relay::error::Error;
use save_relay::local::LocalStore;
use save_relay::platform::{AppDirs, PathVars};
use save_relay::sync::Resolve;
use save_relay::wrap::{self, Engine};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

const HOST_GATE: &str = "SAVE_RELAY_SIGNAL_HOST";
const GAME_GATE: &str = "SAVE_RELAY_SIGNAL_GAME";
const DIR: &str = "SAVE_RELAY_SIGNAL_DIR";
const SLOW_PUSH: &str = "SAVE_RELAY_SIGNAL_SLOW_PUSH";
const SIGHUP: i32 = 1;
const SIGTERM: i32 = 15;

extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

fn send(pid: u32, sig: i32) {
    assert_eq!(
        unsafe { kill(pid as i32, sig) },
        0,
        "kill({pid}, {sig}) failed"
    );
}

fn clock() -> u64 {
    1_793_476_260
}

fn no_wait(_: u64) {}

fn exe() -> String {
    std::env::current_exe()
        .expect("the test binary's path")
        .to_string_lossy()
        .into_owned()
}

fn touch(path: &Path) {
    fs::write(path, "").expect("write a marker");
}

/// The scripted engine: `pull` fails (no server, so the game starts at once); the exit `push`
/// writes `pushing`, optionally waits, then removes `mark` and writes `pushed`.
struct Scripted {
    dir: PathBuf,
    slow: bool,
}

impl Engine for Scripted {
    fn game(&self) -> &str {
        "signal-game"
    }
    fn pull(&self, _resolve: Option<Resolve>, _anyway: bool) -> Result<SyncReport, Error> {
        Err(Error::Io("no server in this test".into()))
    }
    fn push(&self, _resolve: Option<Resolve>) -> Result<SyncReport, Error> {
        touch(&self.dir.join("pushing"));
        if self.slow {
            thread::sleep(Duration::from_secs(60));
        }
        let _ = fs::remove_file(self.dir.join("mark"));
        touch(&self.dir.join("pushed"));
        Err(Error::Io("no server in this test".into()))
    }
}

// The helper processes. Each is ignored, so only the tests below run them, and each does
// nothing unless its environment gate is set.

#[test]
#[ignore = "run as the wrap host by the tests in this file"]
fn signal_host() {
    if std::env::var_os(HOST_GATE).is_none() {
        return;
    }
    let dir = PathBuf::from(std::env::var_os(DIR).expect("the scratch folder"));
    let vars = PathVars {
        home: dir.join("home"),
        data: dir.join("data"),
        config: dir.join("config"),
        documents: None,
    };
    let store = LocalStore::new(AppDirs::from_vars(&vars));
    touch(&dir.join("mark"));
    std::env::set_var(GAME_GATE, "1");
    let command = vec![
        exe(),
        "--exact".into(),
        "signal_game".into(),
        "--ignored".into(),
    ];
    let engine = Scripted {
        slow: std::env::var_os(SLOW_PUSH).is_some(),
        dir,
    };
    let code = wrap::run(&engine, &store, &command, clock, &no_wait);
    std::process::exit(code);
}

#[test]
#[ignore = "run as the wrapped game by the wrap host"]
fn signal_game() {
    if std::env::var_os(GAME_GATE).is_none() {
        return;
    }
    let dir = PathBuf::from(std::env::var_os(DIR).expect("the scratch folder"));
    // The game's pid, written whole before `started` (no /proc on macOS to find it by).
    fs::write(dir.join("game.tmp"), std::process::id().to_string()).expect("write the pid");
    fs::rename(dir.join("game.tmp"), dir.join("game.pid")).expect("publish the pid");
    touch(&dir.join("started"));
    // Bounded, so a game the signal never reached does not outlive the test.
    let until = Instant::now() + Duration::from_secs(60);
    while Instant::now() < until {
        thread::sleep(Duration::from_millis(50));
    }
}

// The tests.

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("save-relay-signal-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the scratch folder");
    dir
}

fn spawn_host(dir: &Path, slow: bool) -> Child {
    let mut host = Command::new(exe());
    host.args(["--exact", "signal_host", "--ignored"])
        .env(HOST_GATE, "1")
        .env(DIR, dir);
    if slow {
        host.env(SLOW_PUSH, "1");
    }
    host.spawn().expect("start the wrap host")
}

/// Wait until `path` exists, for at most `secs`.
fn wait_for(path: &Path, secs: u64) -> bool {
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        if path.exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    path.exists()
}

/// The host's exit status; a host still running after `secs` is killed and the test fails.
fn wait_exit(host: &mut Child, secs: u64) -> ExitStatus {
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        if let Some(status) = host.try_wait().expect("poll the host") {
            return status;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = host.kill();
    panic!("the wrap host did not exit within {secs} s");
}

/// True once the process is gone (`kill(pid, 0)` fails) or, where `/proc` exists, only a zombie.
fn process_gone(pid: u32) -> bool {
    if unsafe { kill(pid as i32, 0) } != 0 {
        return true;
    }
    fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit(')')
            .next()
            .and_then(|rest| rest.trim_start().chars().next())
            == Some('Z')
    })
}

#[test]
fn a_termination_signal_stops_the_game_runs_the_exit_push_and_clears_the_mark() {
    let dir = scratch("term");
    let mut host = spawn_host(&dir, false);
    assert!(wait_for(&dir.join("started"), 20), "the game did not start");
    send(host.id(), SIGTERM);
    let status = wait_exit(&mut host, 20);
    assert_eq!(
        status.code(),
        Some(128 + SIGTERM),
        "the exit code after SIGTERM"
    );
    assert!(dir.join("pushed").exists(), "the exit push did not run");
    assert!(!dir.join("mark").exists(), "playing.json mark is still set");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn sighup_is_handled_the_same_way() {
    let dir = scratch("hup");
    let mut host = spawn_host(&dir, false);
    assert!(wait_for(&dir.join("started"), 20), "the game did not start");
    send(host.id(), SIGHUP);
    let status = wait_exit(&mut host, 20);
    assert_eq!(
        status.code(),
        Some(128 + SIGHUP),
        "the exit code after SIGHUP"
    );
    assert!(dir.join("pushed").exists(), "the exit push did not run");
    assert!(!dir.join("mark").exists(), "playing.json mark is still set");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_first_signal_reaches_the_game() {
    let dir = scratch("forward");
    let mut host = spawn_host(&dir, false);
    assert!(wait_for(&dir.join("started"), 20), "the game did not start");
    let pid: u32 = fs::read_to_string(dir.join("game.pid"))
        .expect("the game's pid")
        .parse()
        .expect("a pid");
    send(host.id(), SIGTERM);
    let _ = wait_exit(&mut host, 20);
    let until = Instant::now() + Duration::from_secs(10);
    while !process_gone(pid) && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        process_gone(pid),
        "the game {pid} is still running after the signal"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_second_signal_during_the_exit_push_exits_at_once_and_leaves_the_mark() {
    let dir = scratch("second");
    let mut host = spawn_host(&dir, true);
    assert!(wait_for(&dir.join("started"), 20), "the game did not start");
    send(host.id(), SIGTERM);
    assert!(
        wait_for(&dir.join("pushing"), 20),
        "the exit push did not start"
    );
    send(host.id(), SIGTERM);
    let status = wait_exit(&mut host, 5);
    assert_eq!(
        status.code(),
        Some(128 + SIGTERM),
        "the exit code after two signals"
    );
    assert!(
        !dir.join("pushed").exists(),
        "the push finished after the second signal"
    );
    assert!(
        dir.join("mark").exists(),
        "the mark must stay for the next launch"
    );
    let _ = fs::remove_dir_all(&dir);
}
