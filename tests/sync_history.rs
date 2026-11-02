use save_relay::layout::{entry_name, Layout};
use save_relay::local::LocalStore;
use save_relay::manifest::Manifest;
use save_relay::plan::Decision;
use save_relay::platform::AppDirs;
use save_relay::remote::MemoryRemote;
use save_relay::stamp;
use save_relay::state::{LastSynced, ServerState};
use save_relay::sync::{history, push, restore, status, Game, Session};
use save_relay::transfer::read_record;
use save_relay::Error;
use std::fs;
use std::path::PathBuf;
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
            "save-relay-history-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let manifest = Manifest::from_json(
            r#"{"format":1,"id":"fixture","name":"Fixture",
                "saves_from":"launcher","saves":{},
                "include":["**/*.sav"],"exclude":["skip/**"]}"#,
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

    fn session(&self, now: u64) -> Session<'_> {
        Session {
            remote: &self.remote,
            layout: layout(),
            device: DEVICE.into(),
            now,
        }
    }

    /// Write the one save file `a.sav` with `content` (an empty string removes it).
    fn local(&self, content: &str) {
        fs::create_dir_all(&self.game.saves_dir).unwrap();
        let path = self.game.saves_dir.join("a.sav");
        if content.is_empty() {
            let _ = fs::remove_file(path);
        } else {
            fs::write(&path, content).unwrap();
        }
    }

    /// Save `content` as the local files and push them at `now`.
    fn push_at(&self, content: &str, now: u64) {
        self.local(content);
        push(&self.session(now), &self.game, &self.store, None).unwrap();
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

fn entry_at(unix: u64) -> String {
    entry_name(&stamp::entry_stamp(unix), DEVICE)
}

fn server_state(remote: &MemoryRemote) -> ServerState {
    let bytes = remote.read(&layout().state_path(GAME)).unwrap();
    ServerState::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap()
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

fn entry_file(remote: &MemoryRemote, entry: &str) -> Option<Vec<u8>> {
    remote.read(&format!("{}/a.sav", layout().entry_dir(GAME, entry)))
}

#[test]
fn history_lists_newest_first_with_the_current_mark() {
    let f = Fixture::new();
    f.push_at("one", NOW);
    f.push_at("two", NOW + 100);
    f.push_at("three", NOW + 200);

    let data = history(&f.session(NOW + 300), GAME).unwrap();

    assert_eq!(data.game, GAME);
    assert_eq!(data.current, Some(entry_at(NOW + 200)));
    let names: Vec<&str> = data.entries.iter().map(|row| row.entry.as_str()).collect();
    assert_eq!(
        names,
        [entry_at(NOW + 200), entry_at(NOW + 100), entry_at(NOW)]
    );
    let current: Vec<bool> = data.entries.iter().map(|row| row.current).collect();
    assert_eq!(current, [true, false, false]);
    for row in &data.entries {
        assert_eq!(row.device, DEVICE);
        assert!(row.card.is_none());
    }
    assert_eq!(data.entries[0].time, stamp::rfc3339(NOW + 200));
    assert_eq!(data.entries[2].time, stamp::rfc3339(NOW));
}

#[test]
fn history_of_a_game_never_pushed_is_empty() {
    let f = Fixture::new();

    let data = history(&f.session(NOW), GAME).unwrap();

    assert_eq!(data.current, None);
    assert!(data.entries.is_empty());
}

#[test]
fn history_is_an_error_when_the_server_is_unreachable() {
    let f = Fixture::new();
    f.push_at("one", NOW);
    f.remote.set_reachable(false);

    let error = history(&f.session(NOW + 100), GAME).unwrap_err();

    assert_eq!(error.code(), "server_unreachable");
}

#[test]
fn restore_of_an_older_entry_is_a_new_revision_with_its_files() {
    let f = Fixture::new();
    f.push_at("one", NOW);
    f.push_at("two", NOW + 100);
    let old = entry_at(NOW);
    let old_record = read_record(&f.remote, &layout(), GAME, &old).unwrap();

    let data = restore(&f.session(NOW + 200), &f.game, &f.store, &old).unwrap();

    assert_eq!(data.entry, old);
    assert_eq!(data.revision, 3);
    assert_eq!(data.backup, None);
    assert_eq!(data.new_entry, entry_at(NOW + 200));
    assert_eq!(data.counts.uploaded, 0);
    assert_eq!(data.counts.downloaded, 1);
    let current = server_state(&f.remote);
    assert_eq!(current.revision, 3);
    assert_eq!(current.entry, data.new_entry);
    assert_eq!(current.device, DEVICE);
    assert_eq!(
        read_record(&f.remote, &layout(), GAME, &data.new_entry).unwrap(),
        old_record
    );
    assert_eq!(
        entry_file(&f.remote, &data.new_entry),
        Some(b"one".to_vec())
    );
    assert_eq!(fs::read(f.game.saves_dir.join("a.sav")).unwrap(), b"one");
    let last = f.store.load_last(GAME).unwrap().unwrap();
    assert_eq!(last.revision, 3);
    assert!(last.save_set().same_content(&current.save_set()));
}

#[test]
fn restore_keeps_unsynced_local_changes_as_a_backup_entry() {
    let f = Fixture::new();
    f.push_at("one", NOW);
    f.push_at("two", NOW + 100);
    f.local("edited on this machine");
    let old = entry_at(NOW);

    let data = restore(&f.session(NOW + 200), &f.game, &f.store, &old).unwrap();

    let backup = data.backup.clone().expect("the unsynced changes are kept");
    assert_eq!(backup, entry_at(NOW + 200));
    assert_ne!(data.new_entry, backup);
    assert_eq!(data.new_entry, entry_at(NOW + 201));
    assert_eq!(data.counts.uploaded, 1);
    assert_eq!(
        entry_file(&f.remote, &backup),
        Some(b"edited on this machine".to_vec())
    );
    assert_eq!(
        entry_file(&f.remote, &data.new_entry),
        Some(b"one".to_vec())
    );
    assert_eq!(fs::read(f.game.saves_dir.join("a.sav")).unwrap(), b"one");
    assert_eq!(server_state(&f.remote).revision, 3);

    let rows = history(&f.session(NOW + 300), GAME).unwrap();
    assert_eq!(rows.current, Some(data.new_entry.clone()));
    let backup_row = rows
        .entries
        .iter()
        .find(|row| row.entry == backup)
        .expect("the backup is in the history");
    assert!(!backup_row.current);
}

#[test]
fn restore_of_a_missing_entry_is_no_such_entry_and_changes_nothing() {
    let f = Fixture::new();
    f.push_at("one", NOW);
    let before = snapshot(&f.remote);
    let missing = entry_at(NOW + 900);

    let error = restore(&f.session(NOW + 1000), &f.game, &f.store, &missing).unwrap_err();

    assert_eq!(error, Error::NoSuchEntry(missing));
    assert_eq!(snapshot(&f.remote), before);
}

#[test]
fn status_for_an_unreachable_server_is_offline_not_an_error() {
    let f = Fixture::new();
    f.push_at("one", NOW);
    f.remote.set_reachable(false);

    let data = status(&f.session(NOW + 100), &f.game, &f.store).unwrap();

    assert!(!data.reachable);
    assert_eq!(data.would, Decision::Offline);
    assert_eq!(data.server, None);
    assert_eq!(data.last_synced_revision, Some(1));
    assert!(!data.local_changed);
    assert_eq!(data.held, None);
    assert_eq!(data.pending, None);
}

#[test]
fn status_changes_nothing() {
    let f = Fixture::new();
    f.push_at("one", NOW);
    f.local("edited");
    let remote_before = snapshot(&f.remote);
    let last_before: LastSynced = f.store.load_last(GAME).unwrap().unwrap();

    let data = status(&f.session(NOW + 100), &f.game, &f.store).unwrap();

    assert!(data.reachable);
    assert_eq!(data.would, Decision::Push);
    assert_eq!(data.server.map(|side| side.revision), Some(Some(1)));
    assert_eq!(data.last_synced_revision, Some(1));
    assert!(data.local_changed);
    assert_eq!(snapshot(&f.remote), remote_before);
    assert_eq!(f.store.load_last(GAME).unwrap(), Some(last_before));
    assert_eq!(fs::read(f.game.saves_dir.join("a.sav")).unwrap(), b"edited");
}
