//! The sync scenarios (SPEC M1-M5, M9) run per backend: two machines, each with its own saves
//! folder and `LocalStore`, share one remote through the engine's `pull` and `push`. Every test
//! builds a fresh backend (`common`), so no scenario sees another's files. Test names are
//! `<backend>::<scenario>`, with the backend one of `memory`, `dropbox` or `drive`.

mod common;

use save_relay::api::{Action, HistoryData, RestoreData, SyncData};
use save_relay::layout::Layout;
use save_relay::local::LocalStore;
use save_relay::manifest::Manifest;
use save_relay::platform::AppDirs;
use save_relay::remote::{Precondition, Remote, RemoteEntry};
use save_relay::state::ServerState;
use save_relay::sync::{self, Game, Resolve, Session};
use save_relay::{Error, Result};
use std::cell::{Cell, RefCell};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

const START: u64 = 1_793_476_260;
/// Seconds between one machine's operations, so every push gets its own entry time.
const STEP: u64 = 600;
const GAME: &str = "fixture";
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn layout() -> Layout {
    Layout::new("save-relay").unwrap()
}

/// One machine: its own saves folder and local records, and its own clock.
struct Machine {
    root: PathBuf,
    game: Game,
    store: LocalStore,
    device: &'static str,
    clock: Cell<u64>,
}

impl Machine {
    fn new(device: &'static str) -> Machine {
        let root = env::temp_dir().join(format!(
            "save-relay-services-{}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let manifest = Manifest::from_json(
            r#"{"format":1,"id":"fixture","name":"Fixture",
                "saves_from":"launcher","saves":{},
                "include":["**/*.sav"],"exclude":["skip/**"]}"#,
        )
        .unwrap();
        Machine {
            game: Game {
                manifest,
                saves_dir: root.join("saves"),
            },
            store: LocalStore::new(AppDirs {
                config: root.join("config"),
                data: root.join("data"),
            }),
            root,
            device,
            clock: Cell::new(START),
        }
    }

    fn tick(&self) -> u64 {
        let now = self.clock.get() + STEP;
        self.clock.set(now);
        now
    }

    fn session<'a>(&self, remote: &'a dyn Remote) -> Session<'a> {
        Session {
            remote,
            layout: layout(),
            device: self.device.into(),
            now: self.tick(),
        }
    }

    /// Replace this machine's one save file with `content`, its modification time fixed.
    fn write(&self, content: &str) {
        fs::create_dir_all(&self.game.saves_dir).unwrap();
        let path = self.game.saves_dir.join("a.sav");
        fs::write(&path, content).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(START - 500))
            .unwrap();
    }

    fn saves(&self) -> String {
        fs::read_to_string(self.game.saves_dir.join("a.sav")).unwrap()
    }

    fn pull(&self, remote: &dyn Remote, resolve: Option<Resolve>) -> Result<SyncData> {
        let session = self.session(remote);
        sync::pull(&session, &self.game, &self.store, resolve, false).map(|report| report.sync)
    }

    fn push(&self, remote: &dyn Remote, resolve: Option<Resolve>) -> Result<SyncData> {
        let session = self.session(remote);
        sync::push(&session, &self.game, &self.store, resolve).map(|report| report.sync)
    }

    fn history(&self, remote: &dyn Remote) -> HistoryData {
        sync::history(&self.session(remote), GAME).unwrap()
    }

    fn restore(&self, remote: &dyn Remote, entry: &str) -> RestoreData {
        sync::restore(&self.session(remote), &self.game, &self.store, entry).unwrap()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// The server's `state.json`.
fn state(r: &dyn Remote) -> ServerState {
    let (bytes, _) = r.get(&layout().state_path(GAME)).unwrap();
    ServerState::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap()
}

/// The save file inside history entry `entry`.
fn entry_saves(r: &dyn Remote, entry: &str) -> String {
    let path = format!("{}/a.sav", layout().entry_dir(GAME, entry));
    String::from_utf8(r.get(&path).unwrap().0).unwrap()
}

/// The history entry names, newest first, as `sync::history` lists them.
fn history_names(machine: &Machine, r: &dyn Remote) -> Vec<String> {
    machine
        .history(r)
        .entries
        .into_iter()
        .map(|entry| entry.entry)
        .collect()
}

/// M1 and M2: one machine pushes, the other pulls the same saves.
fn push_then_pull(r: &dyn Remote, _online: &dyn Fn(bool)) {
    let one = Machine::new("device-one");
    let two = Machine::new("device-two");
    one.write("first");
    assert_eq!(one.push(r, None).unwrap().action, Action::Pushed);
    assert_eq!(two.pull(r, None).unwrap().action, Action::Pulled);
    assert_eq!(two.saves(), "first");
    assert_eq!(state(r).revision, 1);
    assert_eq!(state(r).device, "device-one");
    // Machine two's launch left its mark; closing the game clears it, so machine one is not held.
    assert_eq!(two.push(r, None).unwrap().action, Action::Noop);
    assert_eq!(one.pull(r, None).unwrap().action, Action::Noop);
}

/// Machine one has pushed revision 2 ("one-edited") over the revision 1 ("base") both machines
/// held; machine two has an unpushed edit ("two-edited"). Both sides changed since the last sync.
fn conflict(r: &dyn Remote) -> (Machine, Machine) {
    let one = Machine::new("device-one");
    let two = Machine::new("device-two");
    one.write("base");
    assert_eq!(one.push(r, None).unwrap().action, Action::Pushed);
    assert_eq!(two.pull(r, None).unwrap().action, Action::Pulled);
    one.write("one-edited");
    assert_eq!(one.push(r, None).unwrap().action, Action::Pushed);
    two.write("two-edited");
    (one, two)
}

/// Both changed -> `conflict`: nothing is overwritten, on either intent.
fn both_changed_conflicts(r: &dyn Remote, _online: &dyn Fn(bool)) {
    let (_one, two) = conflict(r);
    assert_eq!(two.pull(r, None).unwrap().action, Action::Conflict);
    assert_eq!(two.push(r, None).unwrap().action, Action::Conflict);
    assert_eq!(two.saves(), "two-edited");
    assert_eq!(state(r).revision, 2);
    assert_eq!(state(r).device, "device-one");
    assert_eq!(history_names(&two, r).len(), 2);
}

/// M3: `--resolve keep-local` pushes the local saves; the server's losing side stays in history.
fn resolve_keep_local_keeps_server_in_history(r: &dyn Remote, _online: &dyn Fn(bool)) {
    let (one, two) = conflict(r);
    let losing = state(r).entry;
    let data = two.push(r, Some(Resolve::KeepLocal)).unwrap();
    assert_eq!(data.action, Action::Pushed);
    assert_eq!(data.backup, Some(losing.clone()));
    assert_eq!(entry_saves(r, &losing), "one-edited");
    assert!(history_names(&two, r).contains(&losing));
    let now = state(r);
    assert_eq!(now.revision, 3);
    assert_eq!(now.device, "device-two");
    assert_eq!(entry_saves(r, &now.entry), "two-edited");
    assert_eq!(one.pull(r, None).unwrap().action, Action::Pulled);
    assert_eq!(one.saves(), "two-edited");
}

/// M3: `--resolve keep-server` takes the server's saves; the local side is kept in history as
/// a backup entry first, and the revision does not change.
fn resolve_keep_server_keeps_local_in_history(r: &dyn Remote, _online: &dyn Fn(bool)) {
    let (_one, two) = conflict(r);
    let data = two.pull(r, Some(Resolve::KeepServer)).unwrap();
    assert_eq!(data.action, Action::Pulled);
    let backup = data
        .backup
        .expect("the local side is kept as a backup entry");
    assert_eq!(entry_saves(r, &backup), "two-edited");
    assert!(history_names(&two, r).contains(&backup));
    assert_eq!(two.saves(), "one-edited");
    assert_eq!(state(r).revision, 2);
}

/// M4: eleven pushes keep the newest ten; `restore` brings one of the kept ones back as the
/// current revision (a new copy, revision 12).
fn eleven_pushes_keep_ten_and_restore(r: &dyn Remote, _online: &dyn Fn(bool)) {
    let one = Machine::new("device-one");
    for i in 1..=11 {
        one.write(&format!("save {i}"));
        assert_eq!(one.push(r, None).unwrap().action, Action::Pushed);
    }
    assert_eq!(state(r).revision, 11);
    let before = one.history(r);
    assert_eq!(before.entries.len(), 10);
    let contents: Vec<String> = before
        .entries
        .iter()
        .map(|entry| entry_saves(r, &entry.entry))
        .collect();
    let want: Vec<String> = (2..=11).rev().map(|i| format!("save {i}")).collect();
    assert_eq!(contents, want);
    let kept = before
        .entries
        .iter()
        .find(|entry| entry_saves(r, &entry.entry) == "save 2")
        .unwrap()
        .entry
        .clone();

    let restored = one.restore(r, &kept);
    assert_eq!(restored.backup, None);
    assert_eq!(restored.revision, 12);
    assert_eq!(one.saves(), "save 2");
    assert_eq!(state(r).entry, restored.new_entry);
    assert_eq!(entry_saves(r, &restored.new_entry), "save 2");
    let after = one.history(r);
    assert_eq!(after.entries.len(), 10);
    assert_eq!(after.current.as_deref(), Some(restored.new_entry.as_str()));
}

/// M5: offline, a pull plays with the local saves and a push is deferred; once back online the
/// deferred push goes up as a conflict if the server moved on, and `keep-local` settles it.
fn offline_pull_and_deferred_push(r: &dyn Remote, online: &dyn Fn(bool)) {
    let one = Machine::new("device-one");
    let two = Machine::new("device-two");
    one.write("base");
    assert_eq!(one.push(r, None).unwrap().action, Action::Pushed);
    assert_eq!(two.pull(r, None).unwrap().action, Action::Pulled);

    online(false);
    assert_eq!(two.pull(r, None).unwrap().action, Action::Offline);
    assert_eq!(two.saves(), "base");
    two.write("offline-edit");
    assert_eq!(two.push(r, None).unwrap().action, Action::Deferred);
    assert_eq!(two.saves(), "offline-edit");

    online(true);
    one.write("one-edit");
    assert_eq!(one.push(r, None).unwrap().action, Action::Pushed);
    let one_entry = state(r).entry;
    // Catch-up: the M3 check runs before the deferred push goes up.
    assert_eq!(two.push(r, None).unwrap().action, Action::Conflict);
    assert_eq!(state(r).revision, 2);
    let caught = two.push(r, Some(Resolve::KeepLocal)).unwrap();
    assert_eq!(caught.action, Action::Pushed);
    assert_eq!(caught.backup, Some(one_entry.clone()));
    assert_eq!(entry_saves(r, &one_entry), "one-edit");
    let now = state(r);
    assert_eq!(now.revision, 3);
    assert_eq!(entry_saves(r, &now.entry), "offline-edit");
}

/// A remote that runs `before_state` once, just before the first write of `state.json` reaches
/// the inner backend: the other machine's push slips in between this machine's read and write.
struct Racing<'a> {
    inner: &'a dyn Remote,
    state: String,
    before_state: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
}

impl Remote for Racing<'_> {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        self.inner.list(dir)
    }
    fn get(&self, path: &str) -> Result<(Vec<u8>, String)> {
        self.inner.get(path)
    }
    fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> Result<String> {
        if path == self.state {
            let hook = self.before_state.borrow_mut().take();
            if let Some(hook) = hook {
                hook();
            }
        }
        self.inner.put(path, bytes, pre)
    }
    fn mkcol(&self, dir: &str) -> Result<()> {
        self.inner.mkcol(dir)
    }
    fn copy(&self, from: &str, to: &str) -> Result<()> {
        self.inner.copy(from, to)
    }
    fn move_to(&self, from: &str, to: &str) -> Result<()> {
        self.inner.move_to(from, to)
    }
    fn delete(&self, path: &str) -> Result<()> {
        self.inner.delete(path)
    }
}

/// M9: two pushes from the same revision. The first to write `state.json` wins; the second
/// gets `Changed`, removes its own entry, and keeps its saves for the conflict prompt.
fn racing_pushes_one_wins(r: &dyn Remote, _online: &dyn Fn(bool)) {
    let one = Machine::new("device-one");
    let two = Machine::new("device-two");
    one.write("base");
    assert_eq!(one.push(r, None).unwrap().action, Action::Pushed);
    assert_eq!(two.pull(r, None).unwrap().action, Action::Pulled);
    one.write("one-race");
    two.write("two-race");

    let one_outcome: RefCell<Option<Result<SyncData>>> = RefCell::new(None);
    let racing = Racing {
        inner: r,
        state: layout().state_path(GAME),
        before_state: RefCell::new(Some(Box::new(|| {
            *one_outcome.borrow_mut() = Some(one.push(r, None));
        }))),
    };
    let outcome = two.push(&racing, None);
    assert_eq!(outcome.unwrap_err(), Error::Changed);

    let won = one_outcome
        .borrow_mut()
        .take()
        .expect("the first machine pushed inside the race")
        .unwrap();
    assert_eq!(won.action, Action::Pushed);
    let now = state(r);
    assert_eq!(now.revision, 2);
    assert_eq!(now.device, "device-one");
    assert_eq!(entry_saves(r, &now.entry), "one-race");
    assert_eq!(history_names(&one, r).len(), 2);
    assert_eq!(two.saves(), "two-race");
    assert_eq!(two.pull(r, None).unwrap().action, Action::Conflict);
}

/// One `#[test]` per scenario and backend: `memory::push_then_pull`, `dropbox::...`, `drive::...`.
macro_rules! scenarios {
    ($($scenario:ident),* $(,)?) => {
        mod memory {
            use super::*;
            $(
                #[test]
                fn $scenario() {
                    common::memory(super::$scenario);
                }
            )*
        }
        mod dropbox {
            use super::*;
            $(
                #[test]
                fn $scenario() {
                    common::dropbox(super::$scenario);
                }
            )*
        }
        mod drive {
            use super::*;
            $(
                #[test]
                fn $scenario() {
                    common::drive(super::$scenario);
                }
            )*
        }
    };
}

scenarios!(
    push_then_pull,
    both_changed_conflicts,
    resolve_keep_local_keeps_server_in_history,
    resolve_keep_server_keeps_local_in_history,
    eleven_pushes_keep_ten_and_restore,
    offline_pull_and_deferred_push,
    racing_pushes_one_wins,
);
