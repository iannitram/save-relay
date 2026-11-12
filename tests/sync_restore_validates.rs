//! #250 item 3: `restore` checks the entry's record and file hashes before it publishes
//! anything, so a bad entry changes nothing on the server.
use save_relay::layout::{entry_name, Layout};
use save_relay::local::LocalStore;
use save_relay::manifest::Manifest;
use save_relay::platform::AppDirs;
use save_relay::remote::{MemoryRemote, Precondition, Remote};
use save_relay::sync::{push, restore, Game, Session};
use save_relay::transfer::RECORD;
use std::fs;
use std::path::PathBuf;

const NOW: u64 = 1_793_476_260;
const DEVICE: &str = "test-device";

struct World {
    root: PathBuf,
    game: Game,
    store: LocalStore,
    remote: MemoryRemote,
    layout: Layout,
}

fn world(tag: &str) -> World {
    let root = std::env::temp_dir().join(format!("save-relay-rv-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("saves")).unwrap();
    let manifest = Manifest::from_json(
        r#"{"format":1,"id":"fixture","name":"Fixture","saves_from":"launcher","saves":{},
            "include":["**/*.sav"],"exclude":[]}"#,
    )
    .unwrap();
    World {
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
        layout: Layout::new("save-relay").unwrap(),
    }
}

impl World {
    fn session(&self, now: u64) -> Session<'_> {
        Session {
            remote: &self.remote,
            layout: self.layout.clone(),
            device: DEVICE.into(),
            now,
        }
    }
    fn push(&self, content: &str, now: u64) {
        fs::write(self.root.join("saves/a.sav"), content).unwrap();
        push(&self.session(now), &self.game, &self.store, None).unwrap();
    }
    fn files(&self) -> Vec<String> {
        self.remote.file_paths()
    }
}

fn old_entry() -> String {
    entry_name(&save_relay::stamp::entry_stamp(NOW), DEVICE)
}

#[test]
fn a_corrupt_file_in_the_entry_stops_restore_before_anything_is_published() {
    let w = world("hash");
    w.push("one", NOW);
    w.push("two", NOW + 100);
    let path = format!("{}/a.sav", w.layout.entry_dir("fixture", &old_entry()));
    w.remote
        .put(&path, b"tampered", &Precondition::Any)
        .unwrap();
    let before = w.files();
    let state_before = w.remote.read(&w.layout.state_path("fixture"));

    let result = restore(&w.session(NOW + 200), &w.game, &w.store, &old_entry());

    assert!(result.is_err(), "{result:?}");
    assert_eq!(
        w.files(),
        before,
        "no copy of the entry was left on the server"
    );
    assert_eq!(w.remote.read(&w.layout.state_path("fixture")), state_before);
    assert_eq!(fs::read(w.root.join("saves/a.sav")).unwrap(), b"two");
}

#[test]
fn a_record_that_names_a_path_outside_the_saves_folder_stops_restore_first() {
    let w = world("path");
    w.push("one", NOW);
    w.push("two", NOW + 100);
    let record = format!("{}/{RECORD}", w.layout.entry_dir("fixture", &old_entry()));
    let bad = r#"{"files":[{"path":"../evil.sav","sha256":"0000000000000000000000000000000000000000000000000000000000000000","size":1,"mtime":1}]}"#;
    w.remote
        .put(&record, bad.as_bytes(), &Precondition::Any)
        .unwrap();
    let before = w.files();
    let state_before = w.remote.read(&w.layout.state_path("fixture"));

    let result = restore(&w.session(NOW + 200), &w.game, &w.store, &old_entry());

    assert!(result.is_err(), "{result:?}");
    assert_eq!(w.files(), before);
    assert_eq!(w.remote.read(&w.layout.state_path("fixture")), state_before);
}
