use save_relay::api::{Action, SyncData};
use save_relay::layout::{entry_name, Layout};
use save_relay::local::LocalStore;
use save_relay::manifest::Manifest;
use save_relay::plan::Intent;
use save_relay::platform::AppDirs;
use save_relay::presence;
use save_relay::remote::{MemoryRemote, Precondition, Remote, RemoteEntry};
use save_relay::scan::{hash_bytes, scan, SaveSet};
use save_relay::stamp;
use save_relay::state::{LastSynced, ServerState};
use save_relay::sync::{pull, push, Game, Resolve, Session};
use save_relay::transfer::{read_record, upload_entry, Counts};
use save_relay::{Error, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

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
            "save-relay-sync-{}-{}",
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

    fn session<'a>(&self, remote: &'a dyn Remote, now: u64) -> Session<'a> {
        Session {
            remote,
            layout: layout(),
            device: DEVICE.into(),
            now,
        }
    }

    fn local(&self, content: &str) -> SaveSet {
        write_set(&self.game.saves_dir, content)
    }

    fn server(&self, content: &str, revision: u64) -> ServerState {
        let dir = self.root.join("source");
        let set = write_set(&dir, content);
        let time = NOW - 100 + revision;
        let entry = entry_name(&stamp::entry_stamp(time), "other-device");
        upload_entry(&self.remote, &layout(), GAME, &entry, &dir, &set).unwrap();
        let state = ServerState {
            format: 1,
            game: GAME.into(),
            revision,
            device: "other-device".into(),
            time: stamp::rfc3339(time),
            entry,
            files: set.files,
        };
        self.remote
            .put(
                &layout().state_path(GAME),
                state.to_json().as_bytes(),
                &Precondition::Any,
            )
            .unwrap();
        state
    }

    fn last(&self, content: &str, revision: u64) {
        let files = write_set(&self.root.join("last-source"), content).files;
        self.store
            .save_last(&LastSynced {
                format: 1,
                game: GAME.into(),
                revision,
                time: stamp::rfc3339(NOW - 200),
                files,
            })
            .unwrap();
    }

    fn run(&self, intent: Intent, resolve: Option<Resolve>) -> SyncData {
        run(
            &self.session(&self.remote, NOW),
            &self.game,
            &self.store,
            intent,
            resolve,
        )
        .unwrap()
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

fn scan_all(dir: &Path) -> SaveSet {
    scan(dir, &["**/*.sav".into()], &["skip/**".into()]).unwrap()
}

fn write_set(dir: &Path, content: &str) -> SaveSet {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join("a.sav");
    if content.is_empty() {
        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    } else {
        fs::write(&path, content).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(NOW - 500))
            .unwrap();
    }
    scan_all(dir)
}

fn run(
    session: &Session,
    game: &Game,
    store: &LocalStore,
    intent: Intent,
    resolve: Option<Resolve>,
) -> Result<SyncData> {
    let report = match intent {
        Intent::Launch => pull(session, game, store, resolve, false)?,
        Intent::Exit => push(session, game, store, resolve)?,
    };
    assert_eq!(report.held, None);
    assert_eq!(report.server_card, None);
    assert_eq!(report.local_card, None);
    assert!(!report.sync.message.is_empty());
    assert!(!report.sync.message.contains('\n'));
    Ok(report.sync)
}

fn server_state(remote: &MemoryRemote) -> ServerState {
    let bytes = remote.read(&layout().state_path(GAME)).unwrap();
    ServerState::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap()
}

fn snapshot(remote: &MemoryRemote) -> Vec<(String, Vec<u8>)> {
    remote
        .file_paths()
        .into_iter()
        // A launch's pull writes the open-elsewhere mark (M11); these rows check the saves and state.
        .filter(|path| !path.ends_with(&format!("/{}", presence::FILE)))
        .map(|path| {
            let bytes = remote.read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

#[test]
fn decision_rows_for_both_intents() {
    use Action::*;
    // local content, server (revision, content), last (revision, content), launch, exit
    let rows = [
        ("", None, None, Noop, Noop),
        ("", None, Some((2, "old")), Noop, Noop),
        ("local", None, None, Pushed, Pushed),
        ("local", None, Some((2, "local")), Pushed, Pushed),
        ("same", Some((5, "same")), None, Adopted, Adopted),
        ("", Some((5, "")), None, Adopted, Adopted),
        ("", Some((5, "server")), None, Pulled, Pulled),
        ("local", Some((5, "server")), None, Conflict, Conflict),
        ("old", Some((5, "old")), Some((5, "old")), Noop, Noop),
        ("new", Some((5, "old")), Some((5, "old")), Pushed, Pushed),
        ("", Some((5, "old")), Some((5, "old")), Pushed, Pushed),
        ("old", Some((6, "new")), Some((5, "old")), Pulled, Noop),
        ("old", Some((4, "new")), Some((5, "old")), Pulled, Noop),
        ("new", Some((6, "new")), Some((5, "old")), Adopted, Adopted),
        (
            "local",
            Some((6, "server")),
            Some((5, "old")),
            Conflict,
            Conflict,
        ),
    ];
    for (index, (content, server, last, launch, exit)) in rows.into_iter().enumerate() {
        for (intent, expected) in [(Intent::Launch, launch), (Intent::Exit, exit)] {
            let f = Fixture::new();
            let local = f.local(content);
            let state = server.map(|(revision, content)| f.server(content, revision));
            if let Some((revision, content)) = last {
                f.last(content, revision);
            }
            let before = snapshot(&f.remote);
            let previous = f.store.load_last(GAME).unwrap();
            let data = f.run(intent, None);
            assert_eq!(data.action, expected, "row {index}, {intent:?}");
            assert_eq!(data.backup, None);
            assert_eq!(
                data.local.as_ref().map(|side| side.revision),
                local
                    .newest_mtime()
                    .map(|_| last.map(|(revision, _)| revision))
            );
            if let Some(side) = &data.local {
                assert_eq!(side.device, DEVICE);
                assert_eq!(side.time, stamp::rfc3339(NOW - 500));
            }
            match expected {
                Noop | Conflict => {
                    assert_eq!(snapshot(&f.remote), before);
                    assert_eq!(f.store.load_last(GAME).unwrap(), previous);
                    assert_eq!(scan_all(&f.game.saves_dir), local);
                    assert_eq!(data.counts, Counts::default());
                    let revision = if expected == Conflict || state.is_none() {
                        None
                    } else {
                        last.map(|(revision, _)| revision)
                    };
                    assert_eq!(data.revision, revision);
                    if expected == Conflict {
                        assert!(data.server.is_some());
                        assert!(data.local.is_some());
                    }
                }
                Adopted | Pulled | Pushed => {
                    let current = server_state(&f.remote);
                    let record = f.store.load_last(GAME).unwrap().unwrap();
                    assert_eq!(data.revision, Some(current.revision));
                    assert_eq!(record.revision, current.revision);
                    assert_eq!(record.time, stamp::rfc3339(NOW));
                    assert_eq!(record.files, scan_all(&f.game.saves_dir).files);
                    assert!(record.save_set().same_content(&current.save_set()));
                    assert_eq!(
                        read_record(&f.remote, &layout(), GAME, &current.entry).unwrap(),
                        current.save_set()
                    );
                    if expected == Pushed {
                        assert_eq!(
                            current.revision,
                            server.map_or(1, |(revision, _)| revision + 1)
                        );
                        assert_eq!(current.device, DEVICE);
                        assert_eq!(current.entry, entry_name(&stamp::entry_stamp(NOW), DEVICE));
                        assert_eq!(data.counts.uploaded, local.files.len() as u32);
                    } else {
                        assert_eq!(snapshot(&f.remote), before);
                        assert_eq!(data.counts.uploaded, 0);
                        assert_eq!(data.counts.downloaded, u32::from(expected == Pulled));
                    }
                }
                _ => panic!("unexpected action"),
            }
        }
    }
}

#[test]
fn conflict_resolutions_keep_the_losing_side_for_both_intents() {
    for intent in [Intent::Launch, Intent::Exit] {
        for resolve in [Resolve::KeepServer, Resolve::KeepLocal] {
            let f = Fixture::new();
            let local = f.local("local");
            let old = f.server("server", 6);
            f.last("old", 5);
            let data = f.run(intent, Some(resolve));
            let current = server_state(&f.remote);
            let backup = data.backup.unwrap();
            let backed_up = read_record(&f.remote, &layout(), GAME, &backup).unwrap();
            let backup_path = format!("{}/a.sav", layout().entry_dir(GAME, &backup));
            match resolve {
                Resolve::KeepServer => {
                    assert_eq!(data.action, Action::Pulled);
                    assert_eq!(current, old);
                    assert_eq!(backed_up, local);
                    assert_eq!(f.remote.read(&backup_path).unwrap(), b"local");
                    assert_eq!(fs::read(f.game.saves_dir.join("a.sav")).unwrap(), b"server");
                    assert_eq!(data.counts.uploaded, 1);
                    assert_eq!(data.counts.downloaded, 1);
                }
                Resolve::KeepLocal => {
                    assert_eq!(data.action, Action::Pushed);
                    assert_eq!(current.revision, 7);
                    assert_eq!(backup, old.entry);
                    assert_eq!(backed_up, old.save_set());
                    assert_eq!(f.remote.read(&backup_path).unwrap(), b"server");
                    assert_eq!(fs::read(f.game.saves_dir.join("a.sav")).unwrap(), b"local");
                }
            }
            let record = f.store.load_last(GAME).unwrap().unwrap();
            assert_eq!(record.revision, current.revision);
            assert!(record.save_set().same_content(&current.save_set()));
        }
    }
}

#[test]
fn offline_play_pushes_at_next_launch_after_checking_for_conflict() {
    for moved in [false, true] {
        let f = Fixture::new();
        f.local("old");
        f.server("old", 5);
        f.last("old", 5);
        let before = snapshot(&f.remote);
        let previous = f.store.load_last(GAME).unwrap();
        f.remote.set_reachable(false);
        for intent in [Intent::Launch, Intent::Exit] {
            let expected = if intent == Intent::Launch {
                Action::Offline
            } else {
                Action::Deferred
            };
            let data = f.run(intent, Some(Resolve::KeepLocal));
            assert_eq!(data.action, expected);
            assert_eq!(data.revision, None);
            assert_eq!(data.server, None);
            assert_eq!(data.counts, Counts::default());
            assert_eq!(snapshot(&f.remote), before);
            assert_eq!(f.store.load_last(GAME).unwrap(), previous);
            f.local("offline play");
        }
        f.remote.set_reachable(true);
        if moved {
            f.server("other play", 6);
        }
        let before = snapshot(&f.remote);
        let data = f.run(Intent::Launch, None);
        if moved {
            assert_eq!(data.action, Action::Conflict);
            assert_eq!(snapshot(&f.remote), before);
            assert_eq!(f.store.load_last(GAME).unwrap(), previous);
        } else {
            assert_eq!(data.action, Action::Pushed);
            assert_eq!(server_state(&f.remote).revision, 6);
            assert_eq!(f.run(Intent::Exit, None).action, Action::Noop);
        }
    }
}

enum Fault {
    Race(ServerState),
    StateWrite(Error, bool),
    Upload,
}

struct FaultRemote<'a> {
    inner: &'a MemoryRemote,
    fault: Fault,
    expected_pre: Precondition,
}

impl Remote for FaultRemote<'_> {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        self.inner.list(dir)
    }
    fn get(&self, path: &str) -> Result<(Vec<u8>, String)> {
        self.inner.get(path)
    }
    fn mkcol(&self, dir: &str) -> Result<()> {
        self.inner.mkcol(dir)
    }
    fn copy(&self, from: &str, to: &str) -> Result<()> {
        self.inner.copy(from, to)
    }
    fn move_to(&self, from: &str, to: &str) -> Result<()> {
        self.inner.move_to(from, to)
    }
    fn delete(&self, path: &str) -> Result<()> {
        self.inner.delete(path)
    }

    fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> Result<String> {
        if path == layout().state_path(GAME) {
            assert_eq!(*pre, self.expected_pre);
            let next = ServerState::from_json(std::str::from_utf8(bytes).unwrap()).unwrap();
            // Check completeness at the exact moment state.json could become visible.
            assert_eq!(
                read_record(self.inner, &layout(), GAME, &next.entry).unwrap(),
                next.save_set()
            );
            for file in &next.files {
                let saved = self
                    .inner
                    .read(&format!(
                        "{}/{}",
                        layout().entry_dir(GAME, &next.entry),
                        file.path
                    ))
                    .unwrap();
                assert_eq!(hash_bytes(&saved), file.sha256);
            }
            match &self.fault {
                Fault::Race(winner) => {
                    self.inner
                        .put(path, winner.to_json().as_bytes(), &Precondition::Any)?;
                }
                Fault::StateWrite(error, committed) => {
                    if *committed {
                        self.inner.put(path, bytes, pre)?;
                    }
                    return Err(error.clone());
                }
                Fault::Upload => {}
            }
        } else if matches!(self.fault, Fault::Upload) && path.ends_with("a.sav") {
            return Err(Error::Server("upload failed".into()));
        }
        self.inner.put(path, bytes, pre)
    }
}

#[test]
fn conditional_write_races_clean_up_only_the_losing_entry() {
    for existing in [false, true] {
        for intent in [Intent::Launch, Intent::Exit] {
            let f = Fixture::new();
            // Make the winning entry first, then arrange the state the loser reads.
            let winner = f.server("winner", 6);
            if existing {
                f.server("old", 5);
                f.last("old", 5);
            } else {
                f.remote.delete(&layout().state_path(GAME)).unwrap();
            }
            let local = f.local("loser");
            let previous = f.store.load_last(GAME).unwrap();
            let expected_pre = if existing {
                Precondition::Match(f.remote.get(&layout().state_path(GAME)).unwrap().1)
            } else {
                Precondition::Absent
            };
            let remote = FaultRemote {
                inner: &f.remote,
                fault: Fault::Race(winner.clone()),
                expected_pre,
            };
            let error = run(&f.session(&remote, NOW), &f.game, &f.store, intent, None).unwrap_err();
            assert_eq!(error, Error::Changed);
            assert_eq!(server_state(&f.remote), winner);
            assert_eq!(
                read_record(&f.remote, &layout(), GAME, &winner.entry).unwrap(),
                winner.save_set()
            );
            let losing_entry = entry_name(&stamp::entry_stamp(NOW), DEVICE);
            assert!(matches!(
                f.remote.list(&layout().entry_dir(GAME, &losing_entry)),
                Err(Error::NotFound(_))
            ));
            assert!(f
                .remote
                .list(&format!("{}/incoming", layout().game_dir(GAME)))
                .unwrap()
                .is_empty());
            assert_eq!(f.store.load_last(GAME).unwrap(), previous);
            assert_eq!(scan_all(&f.game.saves_dir), local);
            // Re-planning surfaces the conflict rather than overwriting the winner.
            assert_eq!(f.run(intent, None).action, Action::Conflict);
        }
    }
}

#[test]
fn failed_upload_or_ambiguous_state_write_keeps_current_entry_complete() {
    for fault in [
        Fault::Upload,
        Fault::StateWrite(Error::Server("write failed".into()), false),
        Fault::StateWrite(Error::Unreachable("lost response".into()), true),
    ] {
        let f = Fixture::new();
        let old = f.server("old", 5);
        f.last("old", 5);
        f.local("new");
        let previous = f.store.load_last(GAME).unwrap();
        let expected_pre = Precondition::Match(f.remote.get(&layout().state_path(GAME)).unwrap().1);
        let committed = matches!(fault, Fault::StateWrite(_, true));
        let remote = FaultRemote {
            inner: &f.remote,
            fault,
            expected_pre,
        };
        assert!(push(&f.session(&remote, NOW), &f.game, &f.store, None).is_err());
        let current = server_state(&f.remote);
        assert_eq!(current.revision, if committed { 6 } else { 5 });
        assert_eq!(
            read_record(&f.remote, &layout(), GAME, &current.entry).unwrap(),
            current.save_set()
        );
        assert_eq!(
            read_record(&f.remote, &layout(), GAME, &old.entry).unwrap(),
            old.save_set()
        );
        assert_eq!(f.store.load_last(GAME).unwrap(), previous);
    }
}

#[test]
fn retention_after_eleven_pushes_keeps_ten_and_current_even_with_an_old_clock() {
    let f = Fixture::new();
    fs::create_dir_all(f.game.saves_dir.join("skip")).unwrap();
    fs::write(f.game.saves_dir.join("skip/ignored.sav"), "ignored").unwrap();
    fs::write(f.game.saves_dir.join("notes.txt"), "ignored").unwrap();
    let mut entries = Vec::new();
    for index in 0..11 {
        f.local(&format!("save {index}"));
        let now = if index == 10 { NOW - 1000 } else { NOW + index };
        let data = push(&f.session(&f.remote, now), &f.game, &f.store, None)
            .unwrap()
            .sync;
        assert_eq!(data.action, Action::Pushed);
        assert_eq!(data.revision, Some(index + 1));
        assert_eq!(data.counts.uploaded, 1);
        entries.push(server_state(&f.remote).entry);
    }
    let history = f.remote.list(&layout().history_dir(GAME)).unwrap();
    assert_eq!(history.len(), 10);
    assert!(!history.iter().any(|entry| entry.name == entries[0]));
    assert!(history.iter().any(|entry| entry.name == entries[10]));
    let state = server_state(&f.remote);
    assert_eq!(
        read_record(&f.remote, &layout(), GAME, &state.entry).unwrap(),
        state.save_set()
    );
    assert_eq!(f.store.load_last(GAME).unwrap().unwrap().revision, 11);
}

#[test]
fn invalid_server_state_is_an_error_without_writes() {
    for bytes in [b"not json".as_slice(), &[0xff], br#"{"format":2}"#] {
        let f = Fixture::new();
        f.local("local");
        f.server("server", 5);
        f.remote
            .put(&layout().state_path(GAME), bytes, &Precondition::Any)
            .unwrap();
        let before = snapshot(&f.remote);
        for intent in [Intent::Launch, Intent::Exit] {
            let result = run(&f.session(&f.remote, NOW), &f.game, &f.store, intent, None);
            assert!(matches!(result, Err(Error::StateInvalid(_))));
            assert_eq!(snapshot(&f.remote), before);
            assert_eq!(f.store.load_last(GAME).unwrap(), None);
        }
    }
}

#[test]
fn adoption_records_local_mtime_and_pull_applies_manifest_selection() {
    for intent in [Intent::Launch, Intent::Exit] {
        let f = Fixture::new();
        f.local("same");
        let state = f.server("same", 5);
        let mtime = NOW - 10;
        fs::OpenOptions::new()
            .write(true)
            .open(f.game.saves_dir.join("a.sav"))
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(mtime))
            .unwrap();
        let data = f.run(intent, None);
        assert_eq!(data.action, Action::Adopted);
        let last = f.store.load_last(GAME).unwrap().unwrap();
        assert_eq!(last.files[0].mtime, mtime);
        assert_ne!(last.files[0].mtime, state.files[0].mtime);
        assert_eq!(data.local.unwrap().time, stamp::rfc3339(mtime));

        f.server("new server", 6);
        fs::create_dir_all(f.game.saves_dir.join("skip")).unwrap();
        fs::write(f.game.saves_dir.join("skip/ignored.sav"), b"keep").unwrap();
        fs::write(f.game.saves_dir.join("notes.txt"), b"keep").unwrap();
        // Include an obsolete selected file in the last record, so the local side is unchanged.
        fs::write(f.game.saves_dir.join("stale.sav"), b"remove").unwrap();
        f.store
            .save_last(&LastSynced {
                files: scan_all(&f.game.saves_dir).files,
                ..last
            })
            .unwrap();
        let data = f.run(Intent::Launch, None);
        assert_eq!(data.action, Action::Pulled);
        assert_eq!(
            data.counts,
            Counts {
                uploaded: 0,
                downloaded: 1,
                deleted: 1
            }
        );
        assert!(!f.game.saves_dir.join("stale.sav").exists());
        assert_eq!(
            fs::read(f.game.saves_dir.join("skip/ignored.sav")).unwrap(),
            b"keep"
        );
        assert_eq!(
            fs::read(f.game.saves_dir.join("notes.txt")).unwrap(),
            b"keep"
        );
    }
}

#[test]
fn unreachable_without_local_saves_or_last_sync_writes_nothing() {
    for content in ["", "local"] {
        let f = Fixture::new();
        let local = f.local(content);
        f.remote.set_reachable(false);
        for intent in [Intent::Launch, Intent::Exit] {
            let data = f.run(intent, None);
            assert_eq!(
                data.action,
                if intent == Intent::Launch {
                    Action::Offline
                } else {
                    Action::Deferred
                }
            );
            assert_eq!(data.revision, None);
            assert_eq!(data.server, None);
            assert!(f.remote.file_paths().is_empty());
            assert_eq!(f.store.load_last(GAME).unwrap(), None);
            assert_eq!(scan_all(&f.game.saves_dir), local);
        }
    }
}

#[test]
fn keep_local_retains_an_old_server_backup_within_the_ten_entry_limit() {
    let f = Fixture::new();
    let backup = f.server("losing side", 1);
    for index in 0..9 {
        f.local(&format!("save {index}"));
        // Record a base without changing the old entry.
        if index == 0 {
            f.last("losing side", 1);
        }
        push(&f.session(&f.remote, NOW + index), &f.game, &f.store, None).unwrap();
    }
    // Simulate a server restore to the oldest entry under a new revision.
    let restored = ServerState {
        revision: 11,
        ..backup.clone()
    };
    f.remote
        .put(
            &layout().state_path(GAME),
            restored.to_json().as_bytes(),
            &Precondition::Any,
        )
        .unwrap();
    f.local("chosen local");
    let data = push(
        &f.session(&f.remote, NOW + 10),
        &f.game,
        &f.store,
        Some(Resolve::KeepLocal),
    )
    .unwrap()
    .sync;
    assert_eq!(data.action, Action::Pushed);
    assert_eq!(data.revision, Some(12));
    assert_eq!(data.backup.as_deref(), Some(backup.entry.as_str()));
    let history = f.remote.list(&layout().history_dir(GAME)).unwrap();
    assert_eq!(history.len(), 10);
    assert!(history.iter().any(|entry| entry.name == backup.entry));
    assert_eq!(
        read_record(&f.remote, &layout(), GAME, &backup.entry).unwrap(),
        backup.save_set()
    );
    let current = server_state(&f.remote);
    assert!(history.iter().any(|entry| entry.name == current.entry));
}
