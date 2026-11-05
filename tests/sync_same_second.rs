//! Locked tests for #148: two history entries from one device in the same second both land.
//! Do not edit.
use save_relay::api::Action;
use save_relay::layout::{entry_name, Layout};
use save_relay::local::LocalStore;
use save_relay::manifest::Manifest;
use save_relay::platform::AppDirs;
use save_relay::remote::{MemoryRemote, Precondition, Remote};
use save_relay::scan::{scan, SaveSet};
use save_relay::stamp;
use save_relay::state::ServerState;
use save_relay::sync::{pull, push, Game, Resolve, Session};
use save_relay::transfer::{read_record, upload_entry};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const NOW: u64 = 1_793_476_260;
const GAME: &str = "fixture";
const DEVICE: &str = "test-device";
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    game: Game,
    store: LocalStore,
    remote: MemoryRemote,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "save-relay-same-second-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let manifest = Manifest::from_json(
            r#"{"format":1,"id":"fixture","name":"Fixture",
                "saves_from":"launcher","saves":{},"include":["**/*.sav"]}"#,
        )
        .unwrap();
        Self {
            game: Game {
                manifest,
                saves_dir: root.join("saves"),
            },
            store: LocalStore::new(AppDirs {
                config: root.join("config"),
                data: root.join("data"),
            }),
            root,
            remote: MemoryRemote::new(),
        }
    }

    fn session(&self) -> Session<'_> {
        Session {
            remote: &self.remote,
            layout: layout(),
            device: DEVICE.into(),
            now: NOW,
        }
    }

    fn history(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .remote
            .list(&layout().history_dir(GAME))
            .unwrap()
            .into_iter()
            .filter(|entry| entry.is_dir)
            .map(|entry| entry.name)
            .collect();
        names.sort();
        names
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn layout() -> Layout {
    Layout::new("save-relay").unwrap()
}

fn write_set(dir: &Path, content: &str) -> SaveSet {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("a.sav"), content).unwrap();
    scan(dir, &["**/*.sav".into()], &[]).unwrap()
}

fn server_state(remote: &MemoryRemote) -> ServerState {
    let bytes = remote.read(&layout().state_path(GAME)).unwrap();
    ServerState::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap()
}

fn mine(at: u64) -> String {
    entry_name(&stamp::entry_stamp(at), DEVICE)
}

#[test]
fn two_pushes_in_one_second_both_succeed_and_history_keeps_both() {
    let f = Fixture::new();
    write_set(&f.game.saves_dir, "first");
    let one = push(&f.session(), &f.game, &f.store, None).unwrap().sync;
    assert_eq!(one.action, Action::Pushed);
    assert_eq!(server_state(&f.remote).entry, mine(NOW));

    let second = write_set(&f.game.saves_dir, "second");
    let two = push(&f.session(), &f.game, &f.store, None)
        .expect("a second push in the same second succeeds")
        .sync;
    assert_eq!(two.action, Action::Pushed);
    assert_eq!(two.revision, Some(2));

    let state = server_state(&f.remote);
    assert_eq!(
        state.entry,
        mine(NOW + 1),
        "the second entry moves one second on, so time order still holds"
    );
    assert_eq!(f.history(), vec![mine(NOW), mine(NOW + 1)]);
    assert_eq!(
        read_record(&f.remote, &layout(), GAME, &state.entry).unwrap(),
        second
    );
}

#[test]
fn keep_server_backup_in_the_same_second_as_this_devices_push_succeeds() {
    let f = Fixture::new();
    write_set(&f.game.saves_dir, "mine");
    push(&f.session(), &f.game, &f.store, None).unwrap();
    assert_eq!(f.history(), vec![mine(NOW)]);

    // Another device pushes revision 2, and this machine changes its saves too: a conflict.
    let source = f.root.join("source");
    let theirs = write_set(&source, "theirs");
    let other = entry_name(&stamp::entry_stamp(NOW - 50), "other-device");
    upload_entry(&f.remote, &layout(), GAME, &other, &source, &theirs).unwrap();
    let state = ServerState {
        format: 1,
        game: GAME.into(),
        revision: 2,
        device: "other-device".into(),
        time: stamp::rfc3339(NOW - 50),
        entry: other.clone(),
        files: theirs.files.clone(),
    };
    f.remote
        .put(
            &layout().state_path(GAME),
            state.to_json().as_bytes(),
            &Precondition::Any,
        )
        .unwrap();
    write_set(&f.game.saves_dir, "mine, changed");

    let data = pull(
        &f.session(),
        &f.game,
        &f.store,
        Some(Resolve::KeepServer),
        false,
    )
    .expect("the keep-server backup in the same second succeeds")
    .sync;
    assert_eq!(data.action, Action::Pulled);
    assert_eq!(data.backup, Some(mine(NOW + 1)));
    assert_eq!(f.history(), vec![other, mine(NOW), mine(NOW + 1)]);
    assert_eq!(
        fs::read_to_string(f.game.saves_dir.join("a.sav")).unwrap(),
        "theirs"
    );
}
