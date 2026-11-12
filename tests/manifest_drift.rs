//! S12: two machines share one game, and their manifests differ (here one include is narrower).
//! The narrow machine pulls a file its manifest does not scan; its next push must still carry that
//! file, or refuse to push. Runs on the in-memory remote only.

use save_relay::layout::Layout;
use save_relay::local::LocalStore;
use save_relay::manifest::Manifest;
use save_relay::platform::AppDirs;
use save_relay::remote::{MemoryRemote, Remote};
use save_relay::state::ServerState;
use save_relay::sync::{self, Game, Session};
use std::fs;
use std::path::PathBuf;
use std::process;

const GAME: &str = "fixture";

/// One machine: its own saves folder, its own local records and its manifest's include list.
struct Machine {
    game: Game,
    store: LocalStore,
    device: &'static str,
}

impl Machine {
    fn new(test: &str, name: &'static str, include: &str) -> Machine {
        let root =
            std::env::temp_dir().join(format!("save-relay-drift-{}-{test}-{name}", process::id()));
        let _ = fs::remove_dir_all(&root);
        let manifest = Manifest::from_json(&format!(
            r#"{{"format":1,"id":"{GAME}","name":"Fixture","saves_from":"launcher",
                "saves":{{}},"include":[{include}],"exclude":[]}}"#
        ))
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
            device: name,
        }
    }

    fn session<'a>(&self, remote: &'a dyn Remote, now: u64) -> Session<'a> {
        Session {
            remote,
            layout: Layout::new("save-relay").unwrap(),
            device: self.device.into(),
            now,
        }
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let full = self.game.saves_dir.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, bytes).unwrap();
    }

    fn file(&self, path: &str) -> PathBuf {
        self.game.saves_dir.join(path)
    }
}

fn server_paths(remote: &dyn Remote) -> Vec<String> {
    let (bytes, _) = remote
        .get(&Layout::new("save-relay").unwrap().state_path(GAME))
        .unwrap();
    let state = ServerState::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap();
    state.save_set().files.into_iter().map(|f| f.path).collect()
}

/// The wide machine pushes `a/one.sav` and `b/extra.sav`; the narrow one (only `a/*.sav`) pulls
/// both, edits `a/one.sav` and pushes.
fn wide_then_narrow_edit(test: &str, remote: &MemoryRemote) -> (Machine, Machine) {
    let wide = Machine::new(test, "wide", r#""**/*.sav""#);
    let narrow = Machine::new(test, "narrow", r#""a/*.sav""#);
    wide.write("a/one.sav", b"1");
    wide.write("b/extra.sav", b"x");
    sync::push(&wide.session(remote, 1_000), &wide.game, &wide.store, None).unwrap();
    sync::pull(
        &narrow.session(remote, 2_000),
        &narrow.game,
        &narrow.store,
        None,
        false,
    )
    .unwrap();
    assert_eq!(fs::read(narrow.file("b/extra.sav")).unwrap(), b"x");
    narrow.write("a/one.sav", b"2");
    (wide, narrow)
}

#[test]
fn narrow_push_keeps_the_file_its_manifest_does_not_scan() {
    let remote = MemoryRemote::new();
    let (_wide, narrow) = wide_then_narrow_edit("keeps", &remote);
    sync::push(
        &narrow.session(&remote, 3_000),
        &narrow.game,
        &narrow.store,
        None,
    )
    .unwrap();
    assert_eq!(server_paths(&remote), ["a/one.sav", "b/extra.sav"]);
}

#[test]
fn narrow_push_refuses_when_the_left_out_file_is_gone() {
    let remote = MemoryRemote::new();
    let (_wide, narrow) = wide_then_narrow_edit("refuses", &remote);
    fs::remove_file(narrow.file("b/extra.sav")).unwrap();
    let before = server_paths(&remote);
    let result = sync::push(
        &narrow.session(&remote, 3_000),
        &narrow.game,
        &narrow.store,
        None,
    );
    assert!(
        result.is_err(),
        "a push that would drop b/extra.sav must not go ahead"
    );
    assert_eq!(server_paths(&remote), before);
}
