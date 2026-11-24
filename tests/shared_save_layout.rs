//! Shared launcher directory: three games and a separate slot share one remote without sharing files.
use save_relay::layout::Layout;
use save_relay::local::LocalStore;
use save_relay::manifest::Manifest;
use save_relay::platform::AppDirs;
use save_relay::remote::MemoryRemote;
use save_relay::sync::{history, pull, push, restore, Game, Session};
use std::fs;

#[test]
fn shared_prefixes_keep_games_slots_and_history_isolated() {
    let root =
        std::env::temp_dir().join(format!("save-relay-shared-layout-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let saves = root.join("savedata/1/title");
    fs::create_dir_all(&saves).unwrap();
    let store = LocalStore::new(AppDirs {
        config: root.join("config"),
        data: root.join("data"),
    });
    let remote = MemoryRemote::new();
    let session = |now| Session {
        remote: &remote,
        layout: Layout::new("save-relay").unwrap(),
        device: "test".into(),
        now,
    };
    let game = |id: &str, prefix: &str| {
        Game {
        manifest: Manifest::from_json(&format!(r#"{{"format":2,"id":"{id}","name":"Fixture","saves_from":"launcher","saves":{{}},"include":["{prefix}*/**"],"exclude":["**/*.tmp"],"strict_scope":true}}"#)).unwrap(),
        saves_dir: saves.clone(),
    }
    };
    let games = [
        game("game-1", "SAVEDATA01"),
        game("game-2", "SAVEDATA02"),
        game("game-3", "SAVEDATA03"),
    ];
    let write = |p: &str, b: &str| {
        let f = saves.join(p);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, b).unwrap();
    };
    let read = |p: &str| fs::read_to_string(saves.join(p)).unwrap();
    write("where.json", "local metadata");
    for n in 1..=3 {
        write(
            &format!("SAVEDATA0{n}/progress.dat"),
            &format!("game {n} v1"),
        );
    }
    write("SAVEDATA01/cache.tmp", "local temp");
    for (i, g) in games.iter().enumerate() {
        push(&session(1000 + i as u64), g, &store, None).unwrap();
    }
    let old = history(&session(2000), "game-1").unwrap().current.unwrap();
    write("SAVEDATA01/progress.dat", "game 1 v2");
    write("SAVEDATA01/new.dat", "new file");
    push(&session(3000), &games[0], &store, None).unwrap();
    // Restore removes a current file absent from the old entry, but leaves other games and local files alone.
    restore(&session(4000), &games[0], &store, &old).unwrap();
    assert_eq!(read("SAVEDATA01/progress.dat"), "game 1 v1");
    assert!(!saves.join("SAVEDATA01/new.dat").exists());
    assert_eq!(read("SAVEDATA01/cache.tmp"), "local temp");
    assert_eq!(read("where.json"), "local metadata");
    assert_eq!(read("SAVEDATA02/progress.dat"), "game 2 v1");
    assert_eq!(read("SAVEDATA03/progress.dat"), "game 3 v1");
    assert_eq!(history(&session(5000), "game-2").unwrap().entries.len(), 1);
    assert_eq!(history(&session(5000), "game-3").unwrap().entries.len(), 1);
    // A different slot has its own namespace, even with the same file prefix.
    let mut slot = game("game-1-slot-2", "SAVEDATA01");
    slot.saves_dir = root.join("slot-2");
    fs::create_dir_all(slot.saves_dir.join("SAVEDATA01")).unwrap();
    fs::write(slot.saves_dir.join("SAVEDATA01/progress.dat"), "slot 2").unwrap();
    assert!(history(&session(5000), "game-1-slot-2")
        .unwrap()
        .entries
        .is_empty());
    push(&session(5100), &slot, &store, None).unwrap();
    assert_eq!(
        history(&session(5200), "game-1-slot-2")
            .unwrap()
            .entries
            .len(),
        1
    );
    assert_eq!(read("SAVEDATA01/progress.dat"), "game 1 v1");
    // A fresh device pulling game 2 into the same folder cannot delete or replace game 1 or 3.
    let other = LocalStore::new(AppDirs {
        config: root.join("other-config"),
        data: root.join("other-data"),
    });
    fs::remove_file(saves.join("SAVEDATA02/progress.dat")).unwrap();
    pull(&session(6000), &games[1], &other, None, false).unwrap();
    assert_eq!(read("SAVEDATA02/progress.dat"), "game 2 v1");
    assert_eq!(read("SAVEDATA01/progress.dat"), "game 1 v1");
    assert_eq!(read("SAVEDATA03/progress.dat"), "game 3 v1");
    assert_eq!(read("where.json"), "local metadata");
    assert_eq!(read("SAVEDATA01/cache.tmp"), "local temp");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn strict_scope_refuses_broad_current_and_historical_records_without_writes() {
    let root =
        std::env::temp_dir().join(format!("save-relay-strict-history-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let saves = root.join("saves");
    fs::create_dir_all(saves.join("SAVEDATA01")).unwrap();
    fs::create_dir_all(saves.join("SAVEDATA02")).unwrap();
    fs::write(saves.join("SAVEDATA01/save"), "one").unwrap();
    fs::write(saves.join("SAVEDATA02/save"), "other game").unwrap();
    fs::write(saves.join("where.json"), "metadata").unwrap();
    fs::write(saves.join("SAVEDATA01/cache.tmp"), "temporary").unwrap();
    let store = LocalStore::new(AppDirs {
        config: root.join("config"),
        data: root.join("data"),
    });
    let remote = MemoryRemote::new();
    let session = |now| Session {
        remote: &remote,
        layout: Layout::new("save-relay").unwrap(),
        device: "test".into(),
        now,
    };
    let make_game = |include: &str, strict: bool| {
        let format = if strict { 2 } else { 1 };
        Game {
        manifest: Manifest::from_json(&format!(r#"{{"format":{format},"id":"shared","name":"Fixture","saves_from":"launcher","saves":{{}},"include":["{include}"],"exclude":["**/*.tmp"],"strict_scope":{strict}}}"#)).unwrap(), saves_dir: saves.clone(),
    }
    };
    let broad = make_game("**", false);
    let strict = make_game("SAVEDATA01*/**", true);
    push(&session(1000), &broad, &store, None).unwrap();
    let old = history(&session(1001), "shared").unwrap().current.unwrap();
    let snapshot = || {
        remote
            .file_paths()
            .into_iter()
            .map(|p| {
                let bytes = remote.read(&p).unwrap();
                (p, bytes)
            })
            .collect::<Vec<_>>()
    };
    let before = snapshot();
    assert!(pull(&session(2000), &strict, &store, None, false).is_err());
    assert!(push(&session(2001), &strict, &store, None).is_err());
    assert!(restore(&session(2002), &strict, &store, &old).is_err());
    assert_eq!(snapshot(), before);
    assert_eq!(
        fs::read_to_string(saves.join("SAVEDATA02/save")).unwrap(),
        "other game"
    );
    assert_eq!(
        fs::read_to_string(saves.join("where.json")).unwrap(),
        "metadata"
    );
    assert_eq!(
        fs::read_to_string(saves.join("SAVEDATA01/cache.tmp")).unwrap(),
        "temporary"
    );
    // Publish a clean current revision with the legacy broad manifest; the old entry remains.
    fs::remove_file(saves.join("SAVEDATA02/save")).unwrap();
    fs::remove_file(saves.join("where.json")).unwrap();
    push(&session(3000), &broad, &store, None).unwrap();
    fs::write(saves.join("SAVEDATA02/save"), "other game").unwrap();
    fs::write(saves.join("where.json"), "metadata").unwrap();
    let before = snapshot();
    assert!(restore(&session(4000), &strict, &store, &old).is_err());
    assert_eq!(snapshot(), before);
    assert_eq!(
        fs::read_to_string(saves.join("SAVEDATA02/save")).unwrap(),
        "other game"
    );
    assert_eq!(
        fs::read_to_string(saves.join("where.json")).unwrap(),
        "metadata"
    );
    assert_eq!(
        fs::read_to_string(saves.join("SAVEDATA01/cache.tmp")).unwrap(),
        "temporary"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn strict_scope_requires_format_two() {
    let manifest = |format, strict| {
        format!(
            r#"{{"format":{format},"id":"g","name":"G","saves_from":"launcher","saves":{{}},"include":["**"],"strict_scope":{strict}}}"#
        )
    };
    assert!(Manifest::from_json(&manifest(1, true)).is_err());
    assert!(Manifest::from_json(&manifest(2, false)).is_err());
    assert!(Manifest::from_json(&manifest(2, true)).is_ok());
    assert!(Manifest::from_json(&manifest(1, false)).is_ok());
    let strict = Manifest::from_json(&manifest(2, true)).unwrap();
    let saved = serde_json::to_string(&strict).unwrap();
    assert_eq!(Manifest::from_json(&saved).unwrap(), strict);
    #[derive(serde::Deserialize)]
    struct Legacy {
        #[allow(dead_code)]
        format: u32,
    }
    assert!(serde_json::from_str::<Legacy>(&saved).is_err());
}
