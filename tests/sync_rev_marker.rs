//! Tests for #149: a push on a backend whose etag can repeat (Nextcloud) claims its revision with
//! `rev/<n>` before it writes `state.json`, and the claims are pruned with the history.
//! `CoarseEtags` stands in for that server: its etag is the write's second and its size.
use save_relay::api::Action;
use save_relay::layout::{entry_name, Layout};
use save_relay::local::LocalStore;
use save_relay::manifest::Manifest;
use save_relay::platform::AppDirs;
use save_relay::remote::{MemoryRemote, Precondition, Remote, RemoteEntry};
use save_relay::stamp;
use save_relay::state::ServerState;
use save_relay::sync::{pull, push, Game, Session};
use save_relay::{Error, Result};
use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const NOW: u64 = 1_793_476_260;
const GAME: &str = "fixture";
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A server whose etag is Nextcloud's: the write's second and its size. Every write in these tests
/// is made in `NOW`'s second.
struct CoarseEtags {
    inner: MemoryRemote,
    state_path: String,
    /// The next read of `state.json` returns this instead: a read made before another push.
    stale_state: RefCell<Option<(Vec<u8>, String)>>,
    /// The next write of `state.json` fails with `Error::Server`.
    fail_state_write: Cell<bool>,
}

impl CoarseEtags {
    fn new() -> CoarseEtags {
        CoarseEtags {
            inner: MemoryRemote::new(),
            state_path: layout().state_path(GAME),
            stale_state: RefCell::new(None),
            fail_state_write: Cell::new(false),
        }
    }

    fn etag(bytes: &[u8]) -> String {
        format!("\"{}-{}\"", NOW, bytes.len())
    }
}

impl Remote for CoarseEtags {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        self.inner.list(dir)
    }

    fn get(&self, path: &str) -> Result<(Vec<u8>, String)> {
        if path == self.state_path {
            if let Some(stale) = self.stale_state.borrow_mut().take() {
                return Ok(stale);
            }
            let (bytes, _) = self.inner.get(path)?;
            let etag = Self::etag(&bytes);
            return Ok((bytes, etag));
        }
        self.inner.get(path)
    }

    fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> Result<String> {
        if path == self.state_path {
            if self.fail_state_write.replace(false) {
                return Err(Error::Server("state write refused".into()));
            }
            // `Match` compares the coarse etag, as Nextcloud's `If-Match` does.
            if let Precondition::Match(etag) = pre {
                match self.inner.get(path) {
                    Ok((current, _)) if Self::etag(&current) == *etag => {}
                    Ok(_) | Err(Error::NotFound(_)) => return Err(Error::Changed),
                    Err(error) => return Err(error),
                }
                self.inner.put(path, bytes, &Precondition::Any)?;
                return Ok(Self::etag(bytes));
            }
        }
        self.inner.put(path, bytes, pre)?;
        Ok(Self::etag(bytes))
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

    fn needs_rev_marker(&self) -> bool {
        true
    }
}

/// One machine: its device name, its game (the saves folder) and its local store.
struct Machine {
    device: &'static str,
    game: Game,
    store: LocalStore,
}

impl Machine {
    fn new(root: &Path, device: &'static str) -> Machine {
        let dir = root.join(device);
        let manifest = Manifest::from_json(
            r#"{"format":1,"id":"fixture","name":"Fixture",
                "saves_from":"launcher","saves":{},"include":["**/*.sav"]}"#,
        )
        .unwrap();
        let saves_dir = dir.join("saves");
        fs::create_dir_all(&saves_dir).unwrap();
        Machine {
            device,
            game: Game {
                manifest,
                saves_dir,
            },
            store: LocalStore::new(AppDirs {
                config: dir.join("config"),
                data: dir.join("data"),
            }),
        }
    }

    /// Writes this machine's one save file.
    fn save(&self, content: &str) {
        fs::write(self.game.saves_dir.join("a.sav"), content).unwrap();
    }

    fn session<'a>(&self, remote: &'a dyn Remote) -> Session<'a> {
        Session {
            remote,
            layout: layout(),
            device: self.device.into(),
            now: NOW,
        }
    }
}

struct Root(PathBuf);

impl Root {
    fn new() -> Root {
        let path = std::env::temp_dir().join(format!(
            "save-relay-rev-marker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Root(path)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn layout() -> Layout {
    Layout::new("save-relay").unwrap()
}

/// The names directly under `dir`: its folders when `folders`, its files otherwise.
fn names(remote: &dyn Remote, dir: &str, folders: bool) -> Vec<String> {
    remote
        .list(dir)
        .unwrap()
        .into_iter()
        .filter(|entry| entry.is_dir == folders)
        .map(|entry| entry.name)
        .collect()
}

fn state_of(remote: &dyn Remote) -> ServerState {
    let (bytes, _) = remote.get(&layout().state_path(GAME)).unwrap();
    ServerState::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap()
}

/// The entry `device` makes in the first second, `NOW`.
fn first_entry(device: &str) -> String {
    entry_name(&stamp::entry_stamp(NOW), device)
}

#[test]
fn same_second_same_size_push_is_refused_and_keeps_the_first_writers_bytes() {
    let root = Root::new();
    let remote = CoarseEtags::new();
    let c = Machine::new(&root.0, "dev-c");
    let a = Machine::new(&root.0, "dev-a");
    let b = Machine::new(&root.0, "dev-b");

    // C creates revision 1; A and B pull it, so both hold revision 1 and then change their saves.
    // A's pull leaves its mark (`playing.json`), so B's pull is held unless it plays anyway.
    c.save("save one");
    push(&c.session(&remote), &c.game, &c.store, None).unwrap();
    pull(&a.session(&remote), &a.game, &a.store, None, false).unwrap();
    pull(&b.session(&remote), &b.game, &b.store, None, true).unwrap();

    // B reads revision 1 and holds that read. A then pushes revision 2 with the same size in the
    // same second, so the etag B holds is the etag the server reports again.
    let read_by_b = remote.get(&layout().state_path(GAME)).unwrap();
    a.save("save two");
    let pushed = push(&a.session(&remote), &a.game, &a.store, None)
        .unwrap()
        .sync;
    assert_eq!(pushed.action, Action::Pushed);
    assert_eq!(
        remote.get(&layout().state_path(GAME)).unwrap().1,
        read_by_b.1,
        "the etags must match, or this test proves nothing"
    );

    // B's push, with its stale read, passes `If-Match` and is refused by the revision claim.
    *remote.stale_state.borrow_mut() = Some(read_by_b);
    b.save("save bee");
    let error = push(&b.session(&remote), &b.game, &b.store, None).unwrap_err();
    assert_eq!(error, Error::Changed);

    // A's revision stays: it is revision 2 from dev-a, its entry is kept, B's entry is gone.
    let state = state_of(&remote);
    assert_eq!((state.revision, state.device.as_str()), (2, "dev-a"));
    assert_eq!(state.entry, first_entry("dev-a"));
    let history = names(&remote, &layout().history_dir(GAME), true);
    assert_eq!(history, [first_entry("dev-a"), first_entry("dev-c")]);
    let kept = remote
        .get(&format!(
            "{}/a.sav",
            layout().entry_dir(GAME, &first_entry("dev-a"))
        ))
        .unwrap()
        .0;
    assert_eq!(kept, b"save two");
    let claim = remote.get(&layout().rev_path(GAME, "2")).unwrap().0;
    assert_eq!(claim, first_entry("dev-a").as_bytes());
}

#[test]
fn a_failed_state_write_releases_its_revision_claim() {
    let root = Root::new();
    let remote = CoarseEtags::new();
    let c = Machine::new(&root.0, "dev-c");
    c.save("save one");

    remote.fail_state_write.set(true);
    let error = push(&c.session(&remote), &c.game, &c.store, None).unwrap_err();
    assert!(matches!(error, Error::Server(_)), "{error:?}");

    // Revision 1 is free again: the next push claims it and writes it.
    let pushed = push(&c.session(&remote), &c.game, &c.store, None)
        .unwrap()
        .sync;
    assert_eq!(pushed.action, Action::Pushed);
    assert_eq!(pushed.revision, Some(1));
    assert_eq!(state_of(&remote).revision, 1);
}

#[test]
fn revision_claims_keep_only_the_newest_ten() {
    let root = Root::new();
    let remote = CoarseEtags::new();
    let c = Machine::new(&root.0, "dev-c");
    for step in 1..=12 {
        c.save(&format!("save {step:02}"));
        let pushed = push(&c.session(&remote), &c.game, &c.store, None)
            .unwrap()
            .sync;
        assert_eq!(pushed.action, Action::Pushed, "push {step}");
    }
    let claims = names(&remote, &layout().rev_dir(GAME), false);
    assert_eq!(claims.len(), 10, "{claims:?}");
    assert!(claims.contains(&"12".to_string()), "{claims:?}");
    assert!(!claims.contains(&"2".to_string()), "{claims:?}");
    assert_eq!(names(&remote, &layout().history_dir(GAME), true).len(), 10);
}

#[test]
fn a_backend_without_the_claim_writes_no_revision_claims() {
    let root = Root::new();
    let remote = MemoryRemote::new();
    let c = Machine::new(&root.0, "dev-c");
    c.save("save one");
    push(&c.session(&remote), &c.game, &c.store, None).unwrap();
    let paths = remote.file_paths();
    assert!(
        !paths.iter().any(|path| path.contains("/rev/")),
        "{paths:?}"
    );
}

fn claims(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

#[test]
fn claims_beyond_keep_are_dropped_oldest_first_by_number() {
    use save_relay::retention::markers_to_drop;
    // By number, not text: 2, 3, 9, 10, 11.
    let all = claims(&["9", "10", "11", "2", "3"]);
    assert_eq!(markers_to_drop(&all, 3, 11), ["2", "3"]);
    assert!(markers_to_drop(&all, 5, 11).is_empty());
}

#[test]
fn names_that_are_not_canonical_revisions_are_kept_and_not_counted() {
    use save_relay::retention::markers_to_drop;
    // "007", "+4", "x" and "" are not revision names: only "1" and "2" count.
    let all = claims(&["1", "2", "007", "x", "", "+4", "0"]);
    assert_eq!(markers_to_drop(&all, 1, 2), ["1"]);
}

#[test]
fn keep_zero_counts_as_one_claim() {
    use save_relay::retention::markers_to_drop;
    assert_eq!(markers_to_drop(&claims(&["4", "5"]), 0, 5), ["4"]);
}

#[test]
fn current_and_later_claims_are_never_pruned() {
    use save_relay::retention::markers_to_drop;
    let all = claims(&[
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12",
    ]);
    assert_eq!(markers_to_drop(&all, 1, 2), ["1"]);
    assert_eq!(
        markers_to_drop(&claims(&["1", "1", "2"]), 2, 2),
        Vec::<String>::new()
    );
}
