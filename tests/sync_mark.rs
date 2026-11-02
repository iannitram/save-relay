//! M11 (#55): `pull` reads, honours and writes `playing.json`; `push` clears its own; `status`
//! reports another machine's mark. Two machines share one `MemoryRemote`, each with its own
//! saves folder and local store.

use save_relay::api::{exit_code_for, Action, Held, SyncReport};
use save_relay::layout::Layout;
use save_relay::local::LocalStore;
use save_relay::manifest::Manifest;
use save_relay::plan::Decision;
use save_relay::platform::AppDirs;
use save_relay::presence::Mark;
use save_relay::remote::{MemoryRemote, Precondition, Remote};
use save_relay::stamp;
use save_relay::sync::{pull, push, status, Game, Resolve, Session};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const NOW: u64 = 1_793_476_260;
const GAME: &str = "fixture";
const AMD: &str = "amd-handheld";
const TABLET: &str = "strix-halo-tablet";
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// The temp folder every machine of one test lives in, and the server they share.
struct World {
    root: PathBuf,
    remote: MemoryRemote,
}

impl World {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "save-relay-mark-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self {
            root,
            remote: MemoryRemote::new(),
        }
    }

    fn machine(&self, device: &str) -> Machine {
        let dir = self.root.join(device);
        let manifest = Manifest::from_json(
            r#"{"format":1,"id":"fixture","name":"Fixture",
                "saves_from":"launcher","saves":{},
                "include":["**/*.sav"],"exclude":["skip/**"]}"#,
        )
        .unwrap();
        Machine {
            device: device.into(),
            game: Game {
                manifest,
                saves_dir: dir.join("saves"),
            },
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
    game: Game,
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

    /// Write the one save file `a.sav` with `content`; an empty string removes it.
    fn save(&self, content: &str) {
        let dir = &self.game.saves_dir;
        fs::create_dir_all(dir).unwrap();
        let path = dir.join("a.sav");
        if content.is_empty() {
            let _ = fs::remove_file(path);
        } else {
            fs::write(path, content).unwrap();
        }
    }

    fn saves(&self) -> Option<Vec<u8>> {
        fs::read(self.game.saves_dir.join("a.sav")).ok()
    }

    fn pull(
        &self,
        remote: &MemoryRemote,
        now: u64,
        resolve: Option<Resolve>,
        anyway: bool,
    ) -> SyncReport {
        pull(
            &self.session(remote, now),
            &self.game,
            &self.store,
            resolve,
            anyway,
        )
        .unwrap()
    }

    fn push(&self, remote: &MemoryRemote, now: u64) -> SyncReport {
        push(&self.session(remote, now), &self.game, &self.store, None).unwrap()
    }
}

fn layout() -> Layout {
    Layout::new("save-relay").unwrap()
}

/// The mark on the server, parsed; `None` when there is no `playing.json`.
fn mark(remote: &MemoryRemote) -> Option<Mark> {
    remote
        .read(&layout().playing_path(GAME))
        .map(|bytes| Mark::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap())
}

fn mark_for(device: &str, since: u64, revision_read: Option<u64>) -> Mark {
    Mark {
        format: 1,
        device: device.into(),
        since: stamp::rfc3339(since),
        revision_read,
    }
}

fn snapshot(remote: &MemoryRemote) -> Vec<(String, Vec<u8>)> {
    remote
        .file_paths()
        .into_iter()
        .map(|path| {
            let bytes = remote.read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

#[test]
fn a_launch_writes_the_mark_and_a_second_machine_is_held_until_anyway() {
    let w = World::new();
    let amd = w.machine(AMD);
    let tablet = w.machine(TABLET);
    amd.save("amd saves");
    amd.push(&w.remote, NOW);

    let data = amd.pull(&w.remote, NOW + 60, None, false);
    assert_eq!(data.sync.action, Action::Noop);
    assert_eq!(data.held, None);
    assert_eq!(mark(&w.remote), Some(mark_for(AMD, NOW + 60, Some(1))));

    let before = snapshot(&w.remote);
    let data = tablet.pull(&w.remote, NOW + 120, None, false);
    assert_eq!(data.sync.action, Action::Held);
    assert_eq!(data.sync.revision, None);
    assert_eq!(
        data.held,
        Some(Held {
            device: AMD.into(),
            since: stamp::rfc3339(NOW + 60),
        })
    );
    assert!(data.sync.message.contains(AMD));
    assert!(data.sync.message.contains(&stamp::rfc3339(NOW + 60)));
    assert_eq!(exit_code_for(data.sync.action), 6);
    assert_eq!(tablet.saves(), None);
    assert_eq!(tablet.store.load_last(GAME).unwrap(), None);
    assert_eq!(snapshot(&w.remote), before);

    let data = tablet.pull(&w.remote, NOW + 180, None, true);
    assert_eq!(data.sync.action, Action::Pulled);
    assert_eq!(data.held, None);
    assert_eq!(tablet.saves(), Some(b"amd saves".to_vec()));
    assert_eq!(mark(&w.remote), Some(mark_for(TABLET, NOW + 180, Some(1))));
}

#[test]
fn a_launch_with_nothing_on_the_server_writes_a_mark_with_no_revision() {
    let w = World::new();
    let amd = w.machine(AMD);

    let data = amd.pull(&w.remote, NOW, None, false);

    assert_eq!(data.sync.action, Action::Noop);
    assert_eq!(data.sync.revision, None);
    assert_eq!(mark(&w.remote), Some(mark_for(AMD, NOW, None)));
    assert_eq!(w.remote.read(&layout().state_path(GAME)), None);
}

#[test]
fn offline_and_unresolved_conflict_launches_write_no_mark() {
    let w = World::new();
    let amd = w.machine(AMD);
    let tablet = w.machine(TABLET);

    w.remote.set_reachable(false);
    let data = amd.pull(&w.remote, NOW, None, false);
    assert_eq!(data.sync.action, Action::Offline);
    w.remote.set_reachable(true);
    assert!(w.remote.file_paths().is_empty());

    // The tablet's saves become revision 1; amd has different saves and no sync record yet.
    tablet.save("tablet saves");
    tablet.push(&w.remote, NOW);
    amd.save("amd saves");
    let data = amd.pull(&w.remote, NOW + 60, None, false);
    assert_eq!(data.sync.action, Action::Conflict);
    assert_eq!(mark(&w.remote), None);

    let data = amd.pull(&w.remote, NOW + 120, Some(Resolve::KeepServer), false);
    assert_eq!(data.sync.action, Action::Pulled);
    assert_eq!(mark(&w.remote), Some(mark_for(AMD, NOW + 120, Some(1))));
}

#[test]
fn push_clears_its_own_mark_after_a_push_and_after_a_noop() {
    let w = World::new();
    let amd = w.machine(AMD);
    amd.save("one");
    amd.push(&w.remote, NOW);
    amd.pull(&w.remote, NOW + 60, None, false);
    assert_eq!(mark(&w.remote).map(|mark| mark.device), Some(AMD.into()));

    amd.save("two");
    let data = amd.push(&w.remote, NOW + 120);
    assert_eq!(data.sync.action, Action::Pushed);
    assert_eq!(mark(&w.remote), None);

    amd.pull(&w.remote, NOW + 180, None, false);
    assert!(mark(&w.remote).is_some());
    let data = amd.push(&w.remote, NOW + 240);
    assert_eq!(data.sync.action, Action::Noop);
    assert_eq!(mark(&w.remote), None);
}

#[test]
fn a_push_leaves_another_machines_mark() {
    let w = World::new();
    let amd = w.machine(AMD);
    let tablet = w.machine(TABLET);
    amd.save("one");
    amd.push(&w.remote, NOW);
    amd.pull(&w.remote, NOW + 60, None, false);
    tablet.pull(&w.remote, NOW + 120, None, true);
    assert_eq!(mark(&w.remote), Some(mark_for(TABLET, NOW + 120, Some(1))));

    amd.save("two");
    let data = amd.push(&w.remote, NOW + 180);

    assert_eq!(data.sync.action, Action::Pushed);
    assert_eq!(mark(&w.remote), Some(mark_for(TABLET, NOW + 120, Some(1))));
}

#[test]
fn a_deferred_push_leaves_its_own_mark() {
    let w = World::new();
    let amd = w.machine(AMD);
    amd.save("one");
    amd.push(&w.remote, NOW);
    amd.pull(&w.remote, NOW + 60, None, false);
    amd.save("two");

    w.remote.set_reachable(false);
    let data = amd.push(&w.remote, NOW + 120);
    w.remote.set_reachable(true);

    assert_eq!(data.sync.action, Action::Deferred);
    assert_eq!(mark(&w.remote), Some(mark_for(AMD, NOW + 60, Some(1))));
}

#[test]
fn a_crashed_machine_pulls_again_as_its_own_mark_not_held() {
    let w = World::new();
    let amd = w.machine(AMD);
    amd.save("one");
    amd.push(&w.remote, NOW);
    amd.pull(&w.remote, NOW + 60, None, false);
    // amd crashes here: no push, so its mark stays.

    let data = amd.pull(&w.remote, NOW + 300, None, false);

    assert_eq!(data.sync.action, Action::Noop);
    assert_eq!(data.held, None);
    assert_eq!(mark(&w.remote), Some(mark_for(AMD, NOW + 300, Some(1))));
}

#[test]
fn status_reports_another_machines_mark_and_writes_nothing() {
    let w = World::new();
    let amd = w.machine(AMD);
    let tablet = w.machine(TABLET);
    amd.save("one");
    amd.push(&w.remote, NOW);
    amd.pull(&w.remote, NOW + 60, None, false);
    let before = snapshot(&w.remote);

    let data = status(
        &tablet.session(&w.remote, NOW + 120),
        &tablet.game,
        &tablet.store,
    )
    .unwrap();
    assert_eq!(
        data.held,
        Some(Held {
            device: AMD.into(),
            since: stamp::rfc3339(NOW + 60),
        })
    );
    assert_eq!(data.would, Decision::Held);
    assert!(data.message.contains(AMD));
    assert_eq!(snapshot(&w.remote), before);

    // amd's own mark is not another machine's.
    let own = status(&amd.session(&w.remote, NOW + 120), &amd.game, &amd.store).unwrap();
    assert_eq!(own.held, None);
    assert_eq!(own.would, Decision::Noop);

    w.remote.set_reachable(false);
    let data = status(
        &tablet.session(&w.remote, NOW + 120),
        &tablet.game,
        &tablet.store,
    )
    .unwrap();
    assert_eq!(data.held, None);
    assert_eq!(data.would, Decision::Offline);
}

#[test]
fn a_broken_mark_is_no_mark_for_pull_and_push_leaves_it() {
    let w = World::new();
    let amd = w.machine(AMD);
    let tablet = w.machine(TABLET);
    amd.save("one");
    amd.push(&w.remote, NOW);
    w.remote
        .put(
            &layout().playing_path(GAME),
            b"not json",
            &Precondition::Any,
        )
        .unwrap();

    let data = tablet.pull(&w.remote, NOW + 60, None, false);
    assert_eq!(data.sync.action, Action::Pulled);
    assert_eq!(data.held, None);
    assert_eq!(mark(&w.remote), Some(mark_for(TABLET, NOW + 60, Some(1))));

    w.remote
        .put(
            &layout().playing_path(GAME),
            b"not json",
            &Precondition::Any,
        )
        .unwrap();
    let data = amd.push(&w.remote, NOW + 120);
    assert_eq!(data.sync.action, Action::Noop);
    assert_eq!(
        w.remote.read(&layout().playing_path(GAME)),
        Some(b"not json".to_vec())
    );
}
