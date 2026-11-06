//! The baton and the save cards end to end (M11, M14, F65): two simulated machines, a handheld
//! and a desktop, hand one game back and forth over one `MemoryRemote`. Every step goes through
//! `cli::run_with` with `--json`, as a launcher would call it, each machine with its own temp
//! folders and credential store. The clock is the `now` each call is given, so no test waits.
//! The card's place and picture come from the fixture sidecar in `tests/fixtures/sidecar/`.

use save_relay::api::{Action, HistoryData, PictureData, SyncReport, WhereData, WhereGame};
use save_relay::card::SIDECAR;
use save_relay::cli::run_with;
use save_relay::creds::{self, CredentialStore, MemoryStore, Secret};
use save_relay::layout::{entry_name, Layout};
use save_relay::local::{Config, LocalStore};
use save_relay::platform::{AppDirs, PathVars};
use save_relay::remote::MemoryRemote;
use save_relay::scan::scan;
use save_relay::stamp;
use save_relay::transfer::upload_entry;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const GAME: &str = "baton-game";
const T0: u64 = 1_793_476_260; // 2026-10-31T19:51:00Z
const DAY: u64 = 86_400;
const SERVER: &str = "https://cloud.example.org";
const USER: &str = "alice";
const PASSWORD: &str = "xK9p-Q2mR-7vLd-W4tY-hN8z";
const PLACE: &str = "Chapter 7: The Fortress";
const PICTURE: &str = ".save-relay-picture.png";
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// One machine: its own folders, credentials, device name and saves folder.
struct Machine {
    vars: PathVars,
    store: LocalStore,
    creds: MemoryStore,
    saves: PathBuf,
    manifest: PathBuf,
}

impl Machine {
    fn new(base: &Path, device: &str) -> Machine {
        let root = base.join(device);
        let vars = PathVars {
            home: root.join("home"),
            data: root.join("data"),
            config: root.join("config"),
            documents: None,
        };
        let store = LocalStore::new(AppDirs::from_vars(&vars));
        store
            .save_config(&Config {
                format: 1,
                server: SERVER.into(),
                user: USER.into(),
                root: "save-relay".into(),
                device: device.into(),
            })
            .unwrap();
        let creds = MemoryStore::new();
        creds
            .set(
                &creds::account(SERVER, USER).unwrap(),
                &Secret::new(PASSWORD),
            )
            .unwrap();
        let saves = root.join("saves");
        fs::create_dir_all(&saves).unwrap();
        let manifest = root.join("baton-game.json");
        fs::write(
            &manifest,
            r#"{"format":1,"id":"baton-game","name":"Baton Game","saves_from":"launcher","saves":{},"include":["**/*.sav"]}"#,
        )
        .unwrap();
        Machine {
            vars,
            store,
            creds,
            saves,
            manifest,
        }
    }

    /// One command through `cli::run_with`: the exit code and the parsed envelope.
    fn run(&self, remote: &MemoryRemote, args: Vec<String>, now: u64) -> (i32, Value) {
        let mut stdin = Cursor::new(Vec::<u8>::new());
        let mut stdout = Vec::<u8>::new();
        let code = run_with(
            &args,
            &mut stdin,
            &mut stdout,
            remote,
            &self.creds,
            &self.store,
            &self.vars,
            now,
        );
        let text = String::from_utf8(stdout).unwrap();
        let value: Value = serde_json::from_str(text.trim())
            .unwrap_or_else(|error| panic!("not one JSON envelope ({error}): {text}"));
        (code, value)
    }

    /// `pull` or `push` of the game, with `extra` flags.
    fn sync(
        &self,
        remote: &MemoryRemote,
        command: &str,
        extra: &[&str],
        now: u64,
    ) -> (i32, SyncReport) {
        let mut args = vec![
            command.to_string(),
            GAME.to_string(),
            "--manifest".into(),
            self.manifest.to_string_lossy().into_owned(),
            "--saves-dir".into(),
            self.saves.to_string_lossy().into_owned(),
        ];
        args.extend(extra.iter().map(|arg| arg.to_string()));
        args.push("--json".into());
        let (code, value) = self.run(remote, args, now);
        assert_eq!(value["ok"], true, "{command}: {value}");
        assert_eq!(value["command"], command);
        (code, data(&value))
    }

    fn where_game(&self, remote: &MemoryRemote, now: u64) -> WhereGame {
        let (code, value) = self.run(remote, vec!["where".into(), "--json".into()], now);
        assert_eq!(code, 0, "{value}");
        let listing: WhereData = data(&value);
        listing
            .games
            .into_iter()
            .find(|row| row.game == GAME)
            .expect("the game is listed")
    }

    fn history(&self, remote: &MemoryRemote, now: u64) -> HistoryData {
        let args = vec!["history".into(), GAME.to_string(), "--json".into()];
        let (code, value) = self.run(remote, args, now);
        assert_eq!(code, 0, "{value}");
        data(&value)
    }

    /// The game's save as the player left it, with the port's sidecar and picture beside it.
    fn play(&self, bytes: &str) {
        fs::write(self.saves.join("a.sav"), bytes).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sidecar");
        for name in [SIDECAR, "chapter-7.png"] {
            fs::copy(fixtures.join(name), self.saves.join(name)).unwrap();
        }
    }

    fn save(&self) -> String {
        fs::read_to_string(self.saves.join("a.sav")).unwrap_or_default()
    }
}

fn data<T: DeserializeOwned>(envelope: &Value) -> T {
    serde_json::from_value(envelope["data"].clone())
        .unwrap_or_else(|error| panic!("data does not fit ({error}): {envelope}"))
}

/// Two machines over one server.
struct Scene {
    root: PathBuf,
    remote: MemoryRemote,
    handheld: Machine,
    desktop: Machine,
}

impl Scene {
    fn new(name: &str) -> Scene {
        let root = std::env::temp_dir().join(format!(
            "save-relay-baton-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        Scene {
            handheld: Machine::new(&root, "handheld"),
            desktop: Machine::new(&root, "desktop"),
            remote: MemoryRemote::new(),
            root,
        }
    }

    fn mark_on_server(&self) -> bool {
        self.remote
            .file_paths()
            .iter()
            .any(|path| path.ends_with("playing.json"))
    }

    /// The handheld launches the game on an empty server, plays chapter 7 and quits half an
    /// hour later: revision 1, pushed with the sidecar's card.
    fn handheld_plays_and_pushes(&self) -> SyncReport {
        let (code, launch) = self.handheld.sync(&self.remote, "pull", &[], T0);
        assert_eq!((code, launch.sync.action), (0, Action::Noop));
        self.handheld.play("chapter 7 save");
        let (code, quit) = self.handheld.sync(&self.remote, "push", &[], T0 + 1800);
        assert_eq!((code, quit.sync.action), (0, Action::Pushed));
        quit
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// ---- M11: the open-elsewhere mark -----------------------------------------------------------

#[test]
fn m11_pull_marks_the_game_open_on_this_machine_and_push_clears_the_mark() {
    let s = Scene::new("mark");
    assert!(!s.mark_on_server());

    let (_, launch) = s.handheld.sync(&s.remote, "pull", &[], T0);
    assert_eq!(launch.held, None, "the machine's own mark never holds it");
    assert!(s.mark_on_server(), "pull left the mark");
    let open = s.handheld.where_game(&s.remote, T0 + 1).held.expect("open");
    assert_eq!(open.device, "handheld");
    assert_eq!(open.since, stamp::rfc3339(T0));

    s.handheld.play("chapter 7 save");
    s.handheld.sync(&s.remote, "push", &[], T0 + 1800);
    assert!(!s.mark_on_server(), "push cleared the mark");
    assert_eq!(s.handheld.where_game(&s.remote, T0 + 1801).held, None);
}

#[test]
fn m11_another_machines_mark_holds_the_launch_until_it_clears_and_never_expires() {
    let s = Scene::new("held");
    s.handheld.sync(&s.remote, "pull", &[], T0);

    // The desktop presses Play: held, exit 6, and nothing was touched.
    let (code, held) = s.desktop.sync(&s.remote, "pull", &[], T0 + 600);
    assert_eq!(code, 6, "held is exit 6");
    assert_eq!(held.sync.action, Action::Held);
    let by = held.held.expect("who and since");
    assert_eq!(by.device, "handheld");
    assert_eq!(by.since, stamp::rfc3339(T0));
    assert_eq!(s.desktop.save(), "");
    let at_desktop = s.desktop.where_game(&s.remote, T0 + 601);
    assert_eq!(
        at_desktop.held.map(|held| held.device).as_deref(),
        Some("handheld")
    );
    let at_handheld = s.handheld.where_game(&s.remote, T0 + 601);
    assert_eq!(
        at_handheld.held.map(|held| held.device).as_deref(),
        Some("handheld")
    );

    // Waiting: forty days later the mark still holds, it never expires on its own.
    let (code, still) = s.desktop.sync(&s.remote, "pull", &[], T0 + 40 * DAY);
    assert_eq!((code, still.sync.action), (6, Action::Held));

    // The handheld quits and pushes: the mark is gone, and the desktop's next pull goes through
    // with the handheld's card.
    s.handheld.play("chapter 7 save");
    s.handheld.sync(&s.remote, "push", &[], T0 + 41 * DAY);
    assert_eq!(
        s.desktop.where_game(&s.remote, T0 + 41 * DAY + 1).held,
        None
    );
    let (code, pulled) = s.desktop.sync(&s.remote, "pull", &[], T0 + 41 * DAY + 60);
    assert_eq!((code, pulled.sync.action), (0, Action::Pulled));
    assert_eq!(s.desktop.save(), "chapter 7 save");
    let card = pulled.server_card.expect("the handheld's card");
    assert_eq!(card.device, "handheld");
    assert_eq!(card.place.as_deref(), Some(PLACE));

    // Now the desktop has it open, and the handheld can see that.
    let open = s.handheld.where_game(&s.remote, T0 + 41 * DAY + 61).held;
    assert_eq!(open.map(|held| held.device).as_deref(), Some("desktop"));
}

#[test]
fn m11_pull_anyway_plays_past_the_mark_and_the_second_push_is_a_conflict_with_two_cards() {
    let s = Scene::new("anyway");
    s.handheld_plays_and_pushes();
    s.desktop.sync(&s.remote, "pull", &[], T0 + 1900);
    s.desktop.sync(&s.remote, "push", &[], T0 + 2000);
    assert!(!s.mark_on_server());

    // The handheld launches again and is suspended mid-game: its mark stays, nothing pushed.
    let (code, _) = s.handheld.sync(&s.remote, "pull", &[], T0 + 3000);
    assert_eq!(code, 0);
    s.handheld.play("handheld, later");

    // The desktop is held, then plays anyway.
    let (code, held) = s.desktop.sync(&s.remote, "pull", &[], T0 + 3100);
    assert_eq!((code, held.sync.action), (6, Action::Held));
    let (code, anyway) = s.desktop.sync(&s.remote, "pull", &["--anyway"], T0 + 3200);
    assert_eq!(code, 0, "{anyway:?}");
    assert_ne!(anyway.sync.action, Action::Held);
    let open = s.handheld.where_game(&s.remote, T0 + 3201).held;
    assert_eq!(open.map(|held| held.device).as_deref(), Some("desktop"));

    // Both push: the desktop first, then the handheld finds the server changed.
    s.desktop.play("desktop, later");
    fs::write(
        s.desktop.saves.join(SIDECAR),
        r#"{"place":"Chapter 8: The Library"}"#,
    )
    .unwrap();
    let (code, pushed) = s.desktop.sync(&s.remote, "push", &[], T0 + 3300);
    assert_eq!((code, pushed.sync.action), (0, Action::Pushed));
    let (code, conflict) = s.handheld.sync(&s.remote, "push", &[], T0 + 3400);
    assert_eq!(code, 3, "a conflict is exit 3");
    assert_eq!(conflict.sync.action, Action::Conflict);
    let server = conflict.server_card.expect("the server's card");
    assert_eq!(server.device, "desktop");
    assert_eq!(server.place.as_deref(), Some("Chapter 8: The Library"));
    let local = conflict.local_card.expect("this machine's card");
    assert_eq!(local.device, "handheld");
    assert_eq!(local.place.as_deref(), Some(PLACE));
    assert_eq!(
        s.handheld.save(),
        "handheld, later",
        "nothing was overwritten"
    );

    // `--resolve keep-local` settles it: the handheld's saves win, the other kept in history.
    let (code, settled) =
        s.handheld
            .sync(&s.remote, "push", &["--resolve", "keep-local"], T0 + 3500);
    assert_eq!((code, settled.sync.action), (0, Action::Pushed));
    assert!(
        settled.sync.backup.is_some(),
        "the desktop's revision is kept"
    );
    for machine in [&s.handheld, &s.desktop] {
        let row = machine.where_game(&s.remote, T0 + 3501);
        assert_eq!(row.held, None);
        assert_eq!(
            row.server.map(|side| side.device).as_deref(),
            Some("handheld")
        );
    }
    let (code, pulled) = s.desktop.sync(&s.remote, "pull", &[], T0 + 3600);
    assert_eq!((code, pulled.sync.action), (0, Action::Pulled));
    assert_eq!(s.desktop.save(), "handheld, later");
}

// ---- M14: save cards ------------------------------------------------------------------------

#[test]
fn m14_every_push_carries_a_card_with_the_place_picture_and_playtime_from_the_sidecar() {
    let s = Scene::new("card");
    let pushed = s.handheld_plays_and_pushes();
    let card = pushed.server_card.expect("the push's card");
    assert_eq!(card.device, "handheld");
    assert_eq!(card.time, stamp::rfc3339(T0 + 1800));
    assert_eq!(card.playtime_secs, Some(1800), "from the pull to the push");
    assert_eq!(card.place.as_deref(), Some(PLACE));
    assert_eq!(card.picture.as_deref(), Some(PICTURE));
}

#[test]
fn m14_history_json_shows_the_cards_and_an_entry_from_before_cards_has_card_null() {
    let s = Scene::new("history");
    s.handheld_plays_and_pushes();
    s.desktop.sync(&s.remote, "pull", &[], T0 + 1900);
    s.desktop.play("desktop save");
    fs::write(
        s.desktop.saves.join(SIDECAR),
        r#"{"place":"Chapter 8: The Library"}"#,
    )
    .unwrap();
    s.desktop.sync(&s.remote, "push", &[], T0 + 2500);

    // An entry an older save-relay pushed, with no card file.
    let old_saves = s.root.join("old-saves");
    fs::create_dir_all(&old_saves).unwrap();
    fs::write(old_saves.join("a.sav"), "old").unwrap();
    let set = scan(&old_saves, &["**/*.sav".to_string()], &[]).unwrap();
    let old = entry_name(&stamp::entry_stamp(T0 - 3600), "old-device");
    let layout = Layout::new("save-relay").unwrap();
    upload_entry(&s.remote, &layout, GAME, &old, &old_saves, &set).unwrap();

    let listing = s.handheld.history(&s.remote, T0 + 3000);
    assert_eq!(listing.entries.len(), 3, "{listing:?}");
    let by_device = |device: &str| {
        listing
            .entries
            .iter()
            .find(|row| row.device == device)
            .unwrap_or_else(|| panic!("no entry from {device}"))
    };
    let newest = by_device("desktop");
    assert!(newest.current);
    let card = newest.card.as_ref().expect("the desktop's card");
    assert_eq!(card.place.as_deref(), Some("Chapter 8: The Library"));
    assert_eq!(
        card.playtime_secs,
        Some(600),
        "pulled at +1900, pushed at +2500"
    );
    let first = by_device("handheld");
    let card = first.card.as_ref().expect("the handheld's card");
    assert_eq!(card.place.as_deref(), Some(PLACE));
    assert_eq!(card.picture.as_deref(), Some(PICTURE));
    let before_cards = by_device("old-device");
    assert_eq!(before_cards.entry, old);
    assert_eq!(before_cards.card, None, "no card file, so card is null");
}

#[test]
fn m14_picture_writes_the_cards_picture_to_the_file_asked_for() {
    let s = Scene::new("picture");
    s.handheld_plays_and_pushes();
    let entry = s
        .desktop
        .history(&s.remote, T0 + 1900)
        .current
        .expect("a current entry");
    let out = s.root.join("card.png");
    let args = vec![
        "picture".to_string(),
        GAME.to_string(),
        entry,
        "--out".into(),
        out.to_string_lossy().into_owned(),
        "--json".into(),
    ];
    let (code, value) = s.desktop.run(&s.remote, args, T0 + 1901);
    assert_eq!(code, 0, "{value}");
    let written: PictureData = data(&value);
    let fixture = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sidecar/chapter-7.png"),
    )
    .unwrap();
    assert_eq!(fs::read(&out).unwrap(), fixture, "the sidecar's picture");
    assert_eq!(written.bytes, fixture.len() as u64);
}
