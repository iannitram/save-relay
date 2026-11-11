use save_relay::local::{Config, DriveClaimEvidence, GameRecord, LocalStore, FORMAT};
use save_relay::manifest::Manifest;
use save_relay::platform::{AppDirs, Platform};
use save_relay::scan::FileEntry;
use save_relay::state::LastSynced;
use save_relay::Error;
use std::path::{Path, PathBuf};

const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn dirs(name: &str) -> AppDirs {
    let base = std::env::temp_dir().join(format!("save-relay-local-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    AppDirs {
        config: base.join("cfg").join("save-relay"),
        data: base.join("dat").join("save-relay"),
    }
}

fn config() -> Config {
    Config {
        format: FORMAT,
        server: "https://cloud.example.org".into(),
        user: "me".into(),
        root: "save-relay".into(),
        device: "amd-handheld".into(),
    }
}

fn last() -> LastSynced {
    let files = vec![FileEntry {
        path: "Uncharted 1/SAVEDATA00/a.bin".into(),
        size: 9,
        mtime: 1793476260,
        sha256: SHA.into(),
    }];
    LastSynced {
        format: 1,
        game: "uncharted-1".into(),
        revision: 7,
        time: "2026-10-31T19:51:00Z".into(),
        files,
    }
}

#[test]
fn drive_claim_evidence_is_local_scoped_and_removable() {
    let store = LocalStore::new(dirs("drive-claim-evidence"));
    let evidence = DriveClaimEvidence {
        format: FORMAT,
        state_path: "save-relay/game/state.json".into(),
        file_id: "drive-file-id".into(),
        version: 7,
        claim: "claim-id".into(),
    };
    assert_eq!(store.load_drive_claim_evidence("game").unwrap(), None);
    store.save_drive_claim_evidence("game", &evidence).unwrap();
    assert_eq!(
        store.load_drive_claim_evidence("game").unwrap(),
        Some(evidence)
    );
    assert_eq!(store.load_drive_claim_evidence("other-game").unwrap(), None);
    store.clear_drive_claim_evidence("game").unwrap();
    assert_eq!(store.load_drive_claim_evidence("game").unwrap(), None);
}

fn record() -> GameRecord {
    let manifest = Manifest {
        format: 1,
        id: "uncharted-1".into(),
        name: "Uncharted".into(),
        saves: [(Platform::Linux, "{data}/u".to_string())]
            .into_iter()
            .collect(),
        include: vec!["Uncharted 1/**".into()],
        exclude: vec![],
        saves_from: None,
    };
    GameRecord {
        manifest,
        saves_dir: Some(PathBuf::from("games").join("Saves")),
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    v.sort();
    v
}

#[test]
fn nothing_saved_yet_is_none() {
    let store = LocalStore::new(dirs("none"));
    assert_eq!(store.load_config().unwrap(), None);
    assert_eq!(store.load_last("uncharted-1").unwrap(), None);
    assert_eq!(store.load_game("uncharted-1").unwrap(), None);
}

#[test]
fn waiting_markers_track_only_an_active_wrap_wait() {
    let store = LocalStore::new(dirs("waiting"));
    assert!(!store.is_waiting("game").unwrap());
    store.set_waiting("game", true).unwrap();
    assert!(store.is_waiting("game").unwrap());
    store.set_waiting("game", false).unwrap();
    assert!(!store.is_waiting("game").unwrap());
}

#[test]
fn config_round_trips_at_config_json_without_a_password() {
    let d = dirs("config");
    let store = LocalStore::new(d.clone());
    store.save_config(&config()).unwrap();
    assert_eq!(store.load_config().unwrap(), Some(config()));
    let text = std::fs::read_to_string(d.config.join("config.json")).unwrap();
    assert!(text.contains("\"server\"") && text.contains("cloud.example.org"));
    assert!(!text.to_lowercase().contains("password"));
    assert_eq!(names(&d.config), vec!["config.json"]);
}

#[test]
fn saving_again_replaces_the_config() {
    let d = dirs("replace");
    let store = LocalStore::new(d.clone());
    store.save_config(&config()).unwrap();
    let mut second = config();
    second.device = "rtx-3090-ti-desktop".into();
    store.save_config(&second).unwrap();
    assert_eq!(store.load_config().unwrap(), Some(second));
    assert_eq!(names(&d.config), vec!["config.json"]);
}

#[test]
fn last_synced_round_trips_under_last_game_json() {
    let d = dirs("last");
    let store = LocalStore::new(d.clone());
    store.save_last(&last()).unwrap();
    assert_eq!(store.load_last("uncharted-1").unwrap(), Some(last()));
    assert_eq!(store.load_last("other").unwrap(), None);
    assert_eq!(names(&d.data.join("last")), vec!["uncharted-1.json"]);
}

#[test]
fn game_record_round_trips_under_games_id_json() {
    let d = dirs("game");
    let store = LocalStore::new(d.clone());
    store.save_game(&record()).unwrap();
    assert_eq!(store.load_game("uncharted-1").unwrap(), Some(record()));
    assert_eq!(names(&d.data.join("games")), vec!["uncharted-1.json"]);
}

#[test]
fn unreadable_json_is_state_invalid() {
    let d = dirs("bad");
    let store = LocalStore::new(d.clone());
    std::fs::create_dir_all(&d.config).unwrap();
    std::fs::write(d.config.join("config.json"), "{\"server\":").unwrap();
    assert!(matches!(store.load_config(), Err(Error::StateInvalid(_))));
    std::fs::create_dir_all(d.data.join("last")).unwrap();
    std::fs::write(d.data.join("last").join("g.json"), "[1,2]").unwrap();
    assert!(matches!(store.load_last("g"), Err(Error::StateInvalid(_))));
    std::fs::create_dir_all(d.data.join("games")).unwrap();
    std::fs::write(d.data.join("games").join("g.json"), "nope").unwrap();
    assert!(matches!(store.load_game("g"), Err(Error::StateInvalid(_))));
}
