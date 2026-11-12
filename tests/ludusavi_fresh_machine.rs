//! A wrapped game on a machine that has never run it: the ludusavi data names its saves folder,
//! and the folder does not exist yet. The server's saves arrive there, and the first pull is not
//! a conflict.

use save_relay::api::Action;
use save_relay::layout::Layout;
use save_relay::local::LocalStore;
use save_relay::ludusavi::{translate, Entry, Facts};
use save_relay::platform::{AppDirs, PathVars, Platform};
use save_relay::remote::MemoryRemote;
use save_relay::sync::{pull, push, Game, Session};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

const NOW: u64 = 1_793_476_260;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("save-relay-fresh-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn facts(root: &Path) -> Facts {
    Facts {
        platform: Platform::Linux,
        vars: PathVars {
            home: root.join("home"),
            data: root.join("data"),
            config: root.join("config"),
            documents: None,
        },
        app_id: 1145360,
        steam_root: root.join("steam"),
        library: root.join("library"),
        install_dir: Some(root.join("library/steamapps/common/Hades")),
        user_id: Some("123".into()),
        os_user: "player".into(),
        proton: false,
    }
}

fn session<'a>(remote: &'a MemoryRemote, device: &str) -> Session<'a> {
    Session {
        remote,
        layout: Layout::new("save-relay").unwrap(),
        device: device.into(),
        now: NOW,
    }
}

#[test]
fn saves_arrive_on_a_machine_that_has_never_run_the_game() {
    let root = scratch("never-run");
    let entry: Entry = serde_json::from_value(json!({
        "files": {
            "<xdgData>/Hades/*.sav": {"tags": ["save"], "when": [{"os": "linux", "store": "steam"}]}
        }
    }))
    .unwrap();
    let located = translate(
        "Hades",
        &entry,
        &facts(&root),
        &|path: &Path| path.exists(),
        &|path: &Path| path.is_dir(),
    )
    .unwrap();
    assert!(!located.saves_dir.exists(), "the folder is not made yet");

    // Another machine has played: its saves are on the server.
    let remote = MemoryRemote::new();
    let played = Game {
        manifest: located.manifest.clone(),
        saves_dir: root.join("played"),
    };
    fs::create_dir_all(&played.saves_dir).unwrap();
    fs::write(
        played.saves_dir.join("slot1.sav"),
        b"from the other machine",
    )
    .unwrap();
    let other_store = LocalStore::new(AppDirs {
        config: root.join("other/config"),
        data: root.join("other/data"),
    });
    push(
        &session(&remote, "other-machine"),
        &played,
        &other_store,
        None,
    )
    .unwrap();

    // This machine has no folder and no record: the first pull brings the saves down.
    let game = Game {
        manifest: located.manifest,
        saves_dir: located.saves_dir.clone(),
    };
    let store = LocalStore::new(AppDirs {
        config: root.join("config"),
        data: root.join("data"),
    });
    let report = pull(
        &session(&remote, "this-machine"),
        &game,
        &store,
        None,
        false,
    )
    .unwrap();
    assert_eq!(report.sync.action, Action::Pulled);
    assert_eq!(
        fs::read(game.saves_dir.join("slot1.sav")).unwrap(),
        b"from the other machine"
    );
}
