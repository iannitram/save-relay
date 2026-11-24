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
        manifest: Manifest::from_json(&format!(r#"{{"format":1,"id":"{id}","name":"Fixture","saves_from":"launcher","saves":{{}},"include":["{prefix}*/**"],"exclude":["**/*.tmp"]}}"#)).unwrap(),
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
