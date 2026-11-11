//! Tests for M14 save cards (#66): the card and picture of each history entry, the sidecar's
//! place and picture, the playtime from the play session, the conflict's two cards, `history`
//! and `picture`, and reserved names kept out of the save set. Run against `MemoryRemote`.

use save_relay::api::Action;
use save_relay::card::{CARD_FILE, SIDECAR};
use save_relay::layout::{entry_name, Layout};
use save_relay::local::LocalStore;
use save_relay::manifest::Manifest;
use save_relay::platform::AppDirs;
use save_relay::remote::{MemoryRemote, Precondition, Remote};
use save_relay::scan::{hash_bytes, scan, FileEntry, SaveSet};
use save_relay::stamp;
use save_relay::state::ServerState;
use save_relay::sync::{history, picture, pull, push, restore, Game, Resolve, Session};
use save_relay::transfer::{read_card, read_record, upload_entry, RECORD};
use save_relay::Error;
use std::fs::{self, FileTimes, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

const NOW: u64 = 1_793_476_260;
const GAME: &str = "cards";
const DEVICE: &str = "test-device";
const OTHER: &str = "other-device";
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
];
const GIF: &[u8] = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;";
const PICTURE_PNG: &str = ".save-relay-picture.png";
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
            "save-relay-cards-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self {
            game: Game {
                manifest: manifest(),
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

    fn session(&self, device: &str, now: u64) -> Session<'_> {
        Session {
            remote: &self.remote,
            layout: layout(),
            device: device.into(),
            now,
        }
    }

    /// Write `bytes` to `rel` under the saves folder, making folders as needed.
    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.game.saves_dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn push_at(&self, now: u64) -> save_relay::api::SyncReport {
        push(&self.session(DEVICE, now), &self.game, &self.store, None).unwrap()
    }

    fn pull_at(&self, now: u64) -> save_relay::api::SyncReport {
        pull(
            &self.session(DEVICE, now),
            &self.game,
            &self.store,
            None,
            false,
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn manifest() -> Manifest {
    Manifest::from_json(
        r#"{"format":1,"id":"cards","name":"Cards",
            "saves_from":"launcher","saves":{},
            "include":["**/*.sav"],"exclude":["skip/**"]}"#,
    )
    .unwrap()
}

fn layout() -> Layout {
    Layout::new("save-relay").unwrap()
}

fn server_state(remote: &MemoryRemote) -> ServerState {
    let bytes = remote.read(&layout().state_path(GAME)).unwrap();
    ServerState::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap()
}

/// Every file under `dir` (reserved names included) as (`/` path, bytes), sorted.
fn listing(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path.strip_prefix(root).unwrap();
                let relative = relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push((relative, fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn a_push_with_a_sidecar_and_a_png_carries_the_card_and_the_picture() {
    let f = Fixture::new();
    f.write("a.sav", b"save");
    f.write("shots/last.png", PNG);
    f.write(
        SIDECAR,
        br#"{"place":"Chapter 7:  The\nFortress","picture":"shots/last.png"}"#,
    );
    let saves_before = listing(&f.game.saves_dir);

    let report = f.push_at(NOW);

    assert_eq!(report.sync.action, Action::Pushed);
    let entry = server_state(&f.remote).entry;
    let card = report.server_card.clone().expect("the new entry's card");
    assert_eq!(card.device, DEVICE);
    assert_eq!(card.time, stamp::rfc3339(NOW));
    assert_eq!(card.place.as_deref(), Some("Chapter 7: The Fortress"));
    assert_eq!(card.picture.as_deref(), Some(PICTURE_PNG));
    assert_eq!(card.playtime_secs, None, "no play session, no playtime");
    assert_eq!(
        read_card(&f.remote, &layout(), GAME, &entry).unwrap(),
        Some(card.clone())
    );
    let entry_dir = layout().entry_dir(GAME, &entry);
    assert_eq!(
        f.remote.read(&format!("{entry_dir}/{PICTURE_PNG}")),
        Some(PNG.to_vec())
    );
    assert!(f.remote.read(&format!("{entry_dir}/{CARD_FILE}")).is_some());
    // The local card is the same place and picture, with no playtime.
    let local = report.local_card.expect("the local saves' card");
    assert_eq!(
        (local.device.as_str(), local.picture.as_deref()),
        (DEVICE, Some(PICTURE_PNG))
    );
    assert_eq!(local.place, card.place);
    assert_eq!(local.playtime_secs, None);

    // Nothing reserved is in the save set, the record or the state, and nothing reserved went up.
    let paths: Vec<String> = server_state(&f.remote)
        .files
        .into_iter()
        .map(|file| file.path)
        .collect();
    assert_eq!(paths, ["a.sav"]);
    let record: Vec<String> = read_record(&f.remote, &layout(), GAME, &entry)
        .unwrap()
        .files
        .into_iter()
        .map(|file| file.path)
        .collect();
    assert_eq!(record, ["a.sav"]);
    for path in f.remote.file_paths() {
        let name = path.rsplit('/').next().unwrap();
        if name.starts_with(".save-relay-") {
            assert!(
                [CARD_FILE, PICTURE_PNG, ".save-relay-files.json"].contains(&name),
                "reserved name uploaded: {path}"
            );
        }
    }
    // Nothing was written into the saves folder.
    assert_eq!(listing(&f.game.saves_dir), saves_before);
}

#[test]
fn a_wrapped_game_card_uses_a_steam_screenshot_taken_during_the_session() {
    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 1, 2, 3];
    const SCREENSHOT_TIME: u64 = NOW + 10;
    let f = Fixture::new();
    let game = Game {
        manifest: Manifest::from_json(
            r#"{"format":1,"id":"steam-42","name":"Wrapped game",
                "saves_from":"launcher","saves":{},"include":["**/*.sav"],"exclude":[]}"#,
        )
        .unwrap(),
        saves_dir: f.game.saves_dir.clone(),
    };
    let steam = f.root.join("steam");
    fs::create_dir_all(steam.join("steamapps")).unwrap();
    fs::write(
        steam.join("steamapps/libraryfolders.vdf"),
        "\"libraryfolders\" {}",
    )
    .unwrap();
    let screenshot = save_relay::steam::screenshot_dir(&steam, "12345", 42).join("capture.jpg");
    fs::create_dir_all(screenshot.parent().unwrap()).unwrap();
    fs::write(&screenshot, JPEG).unwrap();
    OpenOptions::new()
        .write(true)
        .open(&screenshot)
        .unwrap()
        .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(SCREENSHOT_TIME)))
        .unwrap();
    let previous = std::env::var_os(save_relay::steam::STEAM_DIR_VAR);
    std::env::set_var(save_relay::steam::STEAM_DIR_VAR, &steam);

    pull(&f.session(DEVICE, NOW + 5), &game, &f.store, None, false).unwrap();
    f.write("slot.sav", b"save");
    let pushed = push(&f.session(DEVICE, NOW + 15), &game, &f.store, None).unwrap();

    if let Some(previous) = previous {
        std::env::set_var(save_relay::steam::STEAM_DIR_VAR, previous);
    } else {
        std::env::remove_var(save_relay::steam::STEAM_DIR_VAR);
    }
    assert_eq!(pushed.sync.action, Action::Pushed);
    let state_bytes = f
        .remote
        .read(&layout().state_path("steam-42"))
        .expect("the wrapped game state");
    let entry = ServerState::from_json(std::str::from_utf8(&state_bytes).unwrap())
        .unwrap()
        .entry;
    let card = read_card(&f.remote, &layout(), "steam-42", &entry)
        .unwrap()
        .expect("the wrapped game card");
    assert_eq!(card.picture.as_deref(), Some(".save-relay-picture.jpg"));
    let entry_dir = layout().entry_dir("steam-42", &entry);
    assert_eq!(
        f.remote
            .read(&format!("{entry_dir}/.save-relay-picture.jpg")),
        Some(JPEG.to_vec())
    );
}

#[test]
fn a_bad_sidecar_and_a_gif_give_a_card_with_no_place_or_picture_and_a_note() {
    let f = Fixture::new();
    f.write("a.sav", b"one");
    f.write(SIDECAR, b"{ not json");

    let report = f.push_at(NOW);

    let card = report.server_card.expect("a card anyway");
    assert_eq!((card.place, card.picture), (None, None));
    assert!(
        report.sync.message.contains("sidecar left out"),
        "{}",
        report.sync.message
    );
    assert!(!report.sync.message.contains('\n'));

    // A GIF is not a picture: the card has no picture, a note says so, and the save still goes up.
    f.write("a.sav", b"two");
    f.write(SIDECAR, br#"{"picture":"shot.gif"}"#);
    f.write("shot.gif", GIF);
    let report = f.push_at(NOW + 60);

    let card = report.server_card.expect("a card anyway");
    assert_eq!((card.place, card.picture), (None, None));
    assert!(
        report
            .sync
            .message
            .contains("picture left out: not JPEG or PNG"),
        "{}",
        report.sync.message
    );
    assert_eq!(report.sync.action, Action::Pushed);
    assert_eq!(server_state(&f.remote).revision, 2);
}

#[test]
fn playtime_comes_from_the_session_the_pull_started() {
    let f = Fixture::new();
    // A launch that syncs nothing still starts the play session.
    let launch = f.pull_at(NOW);
    assert_eq!(launch.sync.action, Action::Noop);
    assert_eq!(f.store.load_session(GAME).unwrap().unwrap().started, NOW);

    f.write("a.sav", b"after play");
    let report = push(&f.session(DEVICE, NOW + 5400), &f.game, &f.store, None).unwrap();

    assert_eq!(report.sync.action, Action::Pushed);
    assert_eq!(report.server_card.unwrap().playtime_secs, Some(5400));
    // A synced exit ends the session.
    assert_eq!(f.store.load_session(GAME).unwrap(), None);
}

#[test]
fn a_session_older_than_24_hours_gives_no_playtime() {
    let f = Fixture::new();
    f.pull_at(NOW);
    f.write("a.sav", b"late");

    let report = push(&f.session(DEVICE, NOW + 86_401), &f.game, &f.store, None).unwrap();

    assert_eq!(report.sync.action, Action::Pushed);
    let card = report.server_card.unwrap();
    assert_eq!(card.playtime_secs, None);
    assert_eq!(card.time, stamp::rfc3339(NOW + 86_401));
}

#[test]
fn a_conflict_reports_both_cards() {
    let f = Fixture::new();
    f.write("a.sav", b"base");
    f.push_at(NOW);

    // Another machine pulls that revision, saves a new one with a place, and pushes it.
    let other_store = LocalStore::new(AppDirs {
        config: f.root.join("other-config"),
        data: f.root.join("other-data"),
    });
    let other = Game {
        manifest: manifest(),
        saves_dir: f.root.join("other-saves"),
    };
    let other_session = |now| Session {
        remote: &f.remote,
        layout: layout(),
        device: OTHER.into(),
        now,
    };
    pull(&other_session(NOW + 100), &other, &other_store, None, false).unwrap();
    fs::write(other.saves_dir.join("a.sav"), b"server").unwrap();
    fs::write(other.saves_dir.join(SIDECAR), br#"{"place":"Chapter 2"}"#).unwrap();
    let pushed = push(&other_session(NOW + 200), &other, &other_store, None).unwrap();
    assert_eq!(pushed.sync.action, Action::Pushed);

    // This machine changed its own saves meanwhile, and launches.
    f.write("a.sav", b"local");
    let report = f.pull_at(NOW + 300);

    assert_eq!(report.sync.action, Action::Conflict);
    let server = report.server_card.expect("the server's card");
    assert_eq!(server.device, OTHER);
    assert_eq!(server.place.as_deref(), Some("Chapter 2"));
    let local = report.local_card.expect("this machine's card");
    assert_eq!(local.device, DEVICE);
    assert_eq!(local.place, None);
}

#[test]
fn history_gives_an_old_entry_no_card_and_a_new_one_its_card() {
    let f = Fixture::new();
    // An entry pushed before cards existed: its files and file list, but no card.
    let old_saves = f.root.join("old-saves");
    fs::create_dir_all(&old_saves).unwrap();
    fs::write(old_saves.join("a.sav"), b"old").unwrap();
    let set = scan(&old_saves, &["**/*.sav".into()], &[]).unwrap();
    let old_entry = entry_name(&stamp::entry_stamp(NOW - 3600), OTHER);
    upload_entry(&f.remote, &layout(), GAME, &old_entry, &old_saves, &set).unwrap();

    f.write("a.sav", b"new");
    f.push_at(NOW);

    let data = history(&f.session(DEVICE, NOW + 10), GAME).unwrap();
    let old = data
        .entries
        .iter()
        .find(|row| row.entry == old_entry)
        .unwrap();
    assert_eq!(old.card, None);
    let current = data.entries.iter().find(|row| row.current).unwrap();
    assert_eq!(
        current.card.as_ref().map(|card| card.device.as_str()),
        Some(DEVICE)
    );
}

#[test]
fn picture_writes_the_file_and_no_picture_is_not_found() {
    let f = Fixture::new();
    f.write("a.sav", b"with picture");
    f.write(SIDECAR, br#"{"picture":"shot.png"}"#);
    f.write("shot.png", PNG);
    f.push_at(NOW);
    let entry = server_state(&f.remote).entry;

    let dir = f.root.join("out");
    fs::create_dir_all(&dir).unwrap();
    let out = dir.join("card.png");
    let data = picture(&f.session(DEVICE, NOW + 1), GAME, &entry, &out).unwrap();
    assert_eq!(fs::read(&out).unwrap(), PNG);
    assert_eq!(data.bytes, PNG.len() as u64);
    assert_eq!(
        fs::read_dir(&dir).unwrap().count(),
        1,
        "the part file is renamed away"
    );

    // A later entry has a card with no picture: NotFound, and nothing written.
    f.write("a.sav", b"no picture");
    f.write(SIDECAR, b"{}");
    f.push_at(NOW + 100);
    let plain = server_state(&f.remote).entry;
    let none = dir.join("none.png");
    let error = picture(&f.session(DEVICE, NOW + 101), GAME, &plain, &none).unwrap_err();
    assert!(matches!(error, Error::NotFound(_)), "{error:?}");
    assert!(!none.exists());

    // An entry that does not exist has no card either.
    let missing = picture(
        &f.session(DEVICE, NOW + 101),
        GAME,
        "19700101T000000Z-x",
        &none,
    );
    assert!(matches!(missing, Err(Error::NotFound(_))));
}

#[test]
fn scan_skips_the_sidecar_and_part_files_even_when_the_pattern_would_select_them() {
    let f = Fixture::new();
    f.write("a.sav", b"save");
    f.write(SIDECAR, b"{}");
    f.write("slot1/b.sav", b"slot");
    f.write("slot1/.save-relay-part", b"partial");

    let set = scan(&f.game.saves_dir, &["**/*".into()], &[]).unwrap();

    let paths: Vec<&str> = set.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["a.sav", "slot1/b.sav"]);
}

/// Plant a revision on the server whose state and file list name a reserved file beside a save,
/// as an older or hostile client could have written it. Returns the entry's name.
fn plant_reserved(f: &Fixture) -> String {
    let entry = entry_name(&stamp::entry_stamp(NOW), OTHER);
    let dir = layout().entry_dir(GAME, &entry);
    let file = |path: &str, bytes: &[u8]| FileEntry {
        path: path.into(),
        size: bytes.len() as u64,
        mtime: NOW,
        sha256: hash_bytes(bytes),
    };
    let files = vec![
        file(SIDECAR, b"from the server"),
        file("a.sav", b"server save"),
    ];
    let set = SaveSet {
        files: files.clone(),
    };
    let put = |path: String, bytes: &[u8]| {
        f.remote.put(&path, bytes, &Precondition::Any).unwrap();
    };
    save_relay::remote::ensure_dir(&f.remote, &dir).unwrap();
    put(format!("{dir}/a.sav"), b"server save");
    put(format!("{dir}/{SIDECAR}"), b"from the server");
    put(
        format!("{dir}/{RECORD}"),
        &serde_json::to_vec(&set).unwrap(),
    );
    let state = ServerState {
        format: 1,
        game: GAME.into(),
        revision: 1,
        device: OTHER.into(),
        time: stamp::rfc3339(NOW),
        entry: entry.clone(),
        files,
    };
    put(layout().state_path(GAME), state.to_json().as_bytes());
    entry
}

#[test]
fn a_pull_never_fetches_or_replaces_a_reserved_name_in_the_server_list() {
    let f = Fixture::new();
    plant_reserved(&f);
    f.write(SIDECAR, b"mine");

    let report = f.pull_at(NOW + 10);

    assert_eq!(report.sync.action, Action::Pulled);
    assert_eq!(
        fs::read(f.game.saves_dir.join("a.sav")).unwrap(),
        b"server save"
    );
    assert_eq!(fs::read(f.game.saves_dir.join(SIDECAR)).unwrap(), b"mine");
    // The next launch sees nothing changed: the reserved name is not part of the last sync.
    assert_eq!(f.pull_at(NOW + 20).sync.action, Action::Noop);
}

#[test]
fn a_restore_backup_of_unsynced_local_changes_carries_a_card() {
    let f = Fixture::new();
    f.write("a.sav", b"first");
    f.push_at(NOW);
    f.write("a.sav", b"second");
    f.push_at(NOW + 1);
    // The older entry is restored, not the server's current one (the second push).
    let current = server_state(&f.remote).entry;
    let older = history(&f.session(DEVICE, NOW + 5), GAME)
        .unwrap()
        .entries
        .into_iter()
        .map(|row| row.entry)
        .find(|entry| *entry != current)
        .unwrap();
    f.write("a.sav", b"unsynced");
    f.write("shots/last.png", PNG);
    f.write(
        SIDECAR,
        br#"{"place":"Chapter 7","picture":"shots/last.png"}"#,
    );
    // A play session is running, yet the backup has no playtime: it is no session's push.
    f.store
        .save_session(&save_relay::local::Session {
            format: save_relay::local::FORMAT,
            game: GAME.into(),
            started: NOW + 1,
        })
        .unwrap();

    let data = restore(&f.session(DEVICE, NOW + 10), &f.game, &f.store, &older).unwrap();

    assert_eq!(fs::read(f.game.saves_dir.join("a.sav")).unwrap(), b"first");
    let backup = data.backup.expect("unsynced local changes are backed up");
    let rows = history(&f.session(DEVICE, NOW + 20), GAME).unwrap().entries;
    let row = rows.iter().find(|row| row.entry == backup).unwrap();
    let card = row.card.as_ref().expect("the backup entry has a card");
    // This machine's card of the local saves, as the keep-server backup's: no playtime.
    assert_eq!(card.format, save_relay::card::FORMAT);
    assert_eq!(card.device, DEVICE);
    assert_eq!(card.time, stamp::rfc3339(NOW + 10));
    assert_eq!(card.place.as_deref(), Some("Chapter 7"));
    assert_eq!(card.picture.as_deref(), Some(PICTURE_PNG));
    assert_eq!(card.playtime_secs, None);
    let entry_dir = layout().entry_dir(GAME, &backup);
    assert_eq!(
        f.remote.read(&format!("{entry_dir}/{PICTURE_PNG}")),
        Some(PNG.to_vec())
    );
}

#[test]
fn a_restore_backup_whose_card_write_fails_is_a_note_not_an_error() {
    let f = Fixture::new();
    f.write("a.sav", b"first");
    f.push_at(NOW);
    let first = history(&f.session(DEVICE, NOW + 5), GAME)
        .unwrap()
        .entries
        .remove(0)
        .entry;
    f.write("a.sav", b"unsynced");
    let refusing = NoCardWrites(&f.remote);
    let s = Session {
        remote: &refusing,
        layout: layout(),
        device: DEVICE.into(),
        now: NOW + 10,
    };

    let data = restore(&s, &f.game, &f.store, &first).unwrap();

    let backup = data.backup.expect("the backup still goes up");
    assert_eq!(server_state(&f.remote).entry, data.new_entry);
    assert!(data.message.contains("card left out"), "{}", data.message);
    assert!(!data.message.contains('\n'));
    let rows = history(&f.session(DEVICE, NOW + 20), GAME).unwrap().entries;
    let row = rows.iter().find(|row| row.entry == backup).unwrap();
    assert_eq!(row.card, None);
}

#[test]
fn a_restore_message_stays_on_one_line_when_a_sidecar_note_has_a_newline() {
    let f = Fixture::new();
    f.write("a.sav", b"first");
    f.push_at(NOW);
    let first = history(&f.session(DEVICE, NOW + 5), GAME)
        .unwrap()
        .entries
        .remove(0)
        .entry;
    f.write("a.sav", b"unsynced");
    // The picture path starts with `/`, so the sidecar's note quotes it, newline and all.
    f.write(SIDECAR, br#"{"place":"Chapter 7","picture":"/x\ny.png"}"#);

    let data = restore(&f.session(DEVICE, NOW + 10), &f.game, &f.store, &first).unwrap();

    assert!(
        data.backup.is_some(),
        "unsynced local changes are backed up"
    );
    assert!(!data.message.contains('\n'), "{:?}", data.message);
    assert!(
        data.message
            .ends_with("sidecar left out: picture path /x y.png is not inside the saves folder."),
        "{}",
        data.message
    );
}

#[test]
fn a_pull_message_stays_on_one_line_when_a_sidecar_note_has_a_newline() {
    let f = Fixture::new();
    f.write("a.sav", b"base");
    f.push_at(NOW);

    // Another machine pulls that revision, saves a new one, and pushes it.
    let other_store = LocalStore::new(AppDirs {
        config: f.root.join("other-config"),
        data: f.root.join("other-data"),
    });
    let other = Game {
        manifest: manifest(),
        saves_dir: f.root.join("other-saves"),
    };
    let other_session = |now| Session {
        remote: &f.remote,
        layout: layout(),
        device: OTHER.into(),
        now,
    };
    pull(&other_session(NOW + 100), &other, &other_store, None, false).unwrap();
    fs::write(other.saves_dir.join("a.sav"), b"server").unwrap();
    let pushed = push(&other_session(NOW + 200), &other, &other_store, None).unwrap();
    assert_eq!(pushed.sync.action, Action::Pushed);

    // This machine changed its saves meanwhile, and its sidecar's picture path has a newline.
    f.write("a.sav", b"local");
    f.write(SIDECAR, br#"{"picture":"/x\ny.png"}"#);

    let report = pull(
        &f.session(DEVICE, NOW + 300),
        &f.game,
        &f.store,
        Some(Resolve::KeepServer),
        false,
    )
    .unwrap();

    assert_eq!(report.sync.action, Action::Pulled);
    assert!(report.sync.backup.is_some(), "the local saves are kept");
    assert!(
        !report.sync.message.contains('\n'),
        "{:?}",
        report.sync.message
    );
    assert!(
        report
            .sync
            .message
            .ends_with("sidecar left out: picture path /x y.png is not inside the saves folder."),
        "{}",
        report.sync.message
    );
}

/// A server that refuses every card write and passes the rest on.
struct NoCardWrites<'a>(&'a MemoryRemote);

impl Remote for NoCardWrites<'_> {
    fn list(&self, dir: &str) -> save_relay::error::Result<Vec<save_relay::remote::RemoteEntry>> {
        self.0.list(dir)
    }
    fn get(&self, path: &str) -> save_relay::error::Result<(Vec<u8>, String)> {
        self.0.get(path)
    }
    fn put(
        &self,
        path: &str,
        bytes: &[u8],
        pre: &Precondition,
    ) -> save_relay::error::Result<String> {
        if path.ends_with(&format!("/{CARD_FILE}")) {
            return Err(Error::Server("card refused".into()));
        }
        self.0.put(path, bytes, pre)
    }
    fn mkcol(&self, dir: &str) -> save_relay::error::Result<()> {
        self.0.mkcol(dir)
    }
    fn copy(&self, from: &str, to: &str) -> save_relay::error::Result<()> {
        self.0.copy(from, to)
    }
    fn move_to(&self, from: &str, to: &str) -> save_relay::error::Result<()> {
        self.0.move_to(from, to)
    }
    fn delete(&self, path: &str) -> save_relay::error::Result<()> {
        self.0.delete(path)
    }
}

#[test]
fn a_restore_never_fetches_or_replaces_a_reserved_name_and_leaves_it_out_of_the_state() {
    let f = Fixture::new();
    let entry = plant_reserved(&f);
    f.write(SIDECAR, b"mine");

    let data = restore(&f.session(DEVICE, NOW + 10), &f.game, &f.store, &entry).unwrap();

    assert_eq!(
        fs::read(f.game.saves_dir.join("a.sav")).unwrap(),
        b"server save"
    );
    assert_eq!(fs::read(f.game.saves_dir.join(SIDECAR)).unwrap(), b"mine");
    let paths: Vec<String> = server_state(&f.remote)
        .files
        .into_iter()
        .map(|file| file.path)
        .collect();
    assert_eq!(paths, ["a.sav"]);
    assert_eq!(data.backup, None);
}

#[test]
fn an_upload_leaves_a_reserved_name_out_of_the_entry_and_its_file_list() {
    let f = Fixture::new();
    f.write("a.sav", b"save");
    f.write(SIDECAR, b"{}");
    let selected = scan(&f.game.saves_dir, &["**/*".into()], &[]).unwrap();
    let mut set = selected.clone();
    set.files.push(FileEntry {
        path: SIDECAR.into(),
        size: 2,
        mtime: NOW,
        sha256: hash_bytes(b"{}"),
    });
    set.files.sort_by(|a, b| a.path.cmp(&b.path));
    let entry = entry_name(&stamp::entry_stamp(NOW), DEVICE);

    upload_entry(&f.remote, &layout(), GAME, &entry, &f.game.saves_dir, &set).unwrap();

    let dir = layout().entry_dir(GAME, &entry);
    assert!(f.remote.read(&format!("{dir}/{SIDECAR}")).is_none());
    let listed = read_record(&f.remote, &layout(), GAME, &entry).unwrap();
    let paths: Vec<&str> = listed.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["a.sav"]);
}
