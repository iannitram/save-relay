use save_relay::layout::Layout;
use save_relay::remote::{MemoryRemote, Precondition, Remote, RemoteEntry};
use save_relay::scan::{scan, SaveSet};
use save_relay::transfer::{download_entry, read_record, upload_entry, Counts, RECORD};
use save_relay::{Error, Result};
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

const GAME: &str = "g";
const ENTRY: &str = "20261031T195100Z-deck";
const BASE: &str = "save-relay/g/history/20261031T195100Z-deck";

fn temp(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("save-relay-transfer-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn layout() -> Layout {
    Layout::new("save-relay").unwrap()
}

fn scan_all(dir: &Path) -> SaveSet {
    scan(dir, &["**".to_string()], &[]).unwrap()
}

fn saves(name: &str) -> (PathBuf, SaveSet) {
    let dir = temp(name);
    fs::write(dir.join("a.sav"), b"AAA").unwrap();
    fs::create_dir_all(dir.join("slots").join("one")).unwrap();
    fs::write(dir.join("slots").join("one").join("b.sav"), b"BBBB").unwrap();
    let set = scan_all(&dir);
    assert_eq!(set.files.len(), 2);
    (dir, set)
}

/// A remote whose `fail_on`-th put (1-based) fails.
struct FailingPut {
    inner: MemoryRemote,
    puts: Cell<u32>,
    fail_on: u32,
}

impl Remote for FailingPut {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        self.inner.list(dir)
    }
    fn get(&self, path: &str) -> Result<(Vec<u8>, String)> {
        self.inner.get(path)
    }
    fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> Result<String> {
        self.puts.set(self.puts.get() + 1);
        if self.puts.get() == self.fail_on {
            return Err(Error::Server("put failed".to_string()));
        }
        self.inner.put(path, bytes, pre)
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
}

fn upload(remote: &dyn Remote, dir: &Path, set: &SaveSet) -> Result<Counts> {
    upload_entry(remote, &layout(), GAME, ENTRY, dir, set)
}

#[test]
fn round_trip() {
    let (src, set) = saves("rt-src");
    let remote = MemoryRemote::new();
    let counts = upload(&remote, &src, &set).unwrap();
    assert_eq!(
        counts,
        Counts {
            uploaded: 2,
            downloaded: 0,
            deleted: 0
        }
    );
    assert_eq!(
        remote.read(&format!("{BASE}/slots/one/b.sav")).unwrap(),
        b"BBBB"
    );
    assert!(remote.read(&format!("{BASE}/{RECORD}")).is_some());
    assert!(!remote.file_paths().iter().any(|p| p.contains("incoming/")));

    let dest = temp("rt-dest");
    fs::write(dest.join("stale.sav"), b"old").unwrap();
    fs::create_dir_all(dest.join("gone")).unwrap();
    fs::write(dest.join("gone").join("x.sav"), b"x").unwrap();
    let local = scan_all(&dest);
    let counts = download_entry(&remote, &layout(), GAME, ENTRY, &set, &dest, &local).unwrap();
    assert_eq!(
        counts,
        Counts {
            uploaded: 0,
            downloaded: 2,
            deleted: 2
        }
    );
    assert!(!dest.join("stale.sav").exists());
    assert!(!dest.join("gone").exists());
    assert!(scan_all(&dest).same_content(&set));

    let again = download_entry(
        &remote,
        &layout(),
        GAME,
        ENTRY,
        &set,
        &dest,
        &scan_all(&dest),
    )
    .unwrap();
    assert_eq!(again, Counts::default());
}

#[test]
fn upload_failing_halfway_leaves_nothing() {
    let (src, set) = saves("half");
    let remote = FailingPut {
        inner: MemoryRemote::new(),
        puts: Cell::new(0),
        fail_on: 2,
    };
    let err = upload(&remote, &src, &set).unwrap_err();
    assert_eq!(err, Error::Server("put failed".to_string()));
    assert!(remote.inner.file_paths().is_empty());
    let history = remote.list("save-relay/g/history");
    assert!(history.map(|h| h.is_empty()).unwrap_or(true));
    let incoming = remote.list("save-relay/g/incoming").unwrap();
    assert!(incoming.is_empty(), "left {incoming:?}");
}

#[test]
fn corrupt_server_file_replaces_nothing() {
    let (src, set) = saves("bad-src");
    let remote = MemoryRemote::new();
    upload(&remote, &src, &set).unwrap();
    remote
        .put(
            &format!("{BASE}/slots/one/b.sav"),
            b"corrupt",
            &Precondition::Any,
        )
        .unwrap();
    let dest = temp("bad-dest");
    fs::write(dest.join("a.sav"), b"mine").unwrap();
    let local = scan_all(&dest);
    let err = download_entry(&remote, &layout(), GAME, ENTRY, &set, &dest, &local).unwrap_err();
    assert_eq!(
        err,
        Error::Server("hash mismatch: slots/one/b.sav".to_string())
    );
    assert_eq!(fs::read(dest.join("a.sav")).unwrap(), b"mine");
    assert!(!dest.join("slots").join("one").join("b.sav").exists());
    let names: Vec<String> = fs::read_dir(&dest)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().all(|n| !n.ends_with(".save-relay-part")),
        "{names:?}"
    );
}

#[test]
fn download_sets_mtimes() {
    let (src, mut set) = saves("mt-src");
    set.files[0].mtime = 1_700_000_000;
    set.files[1].mtime = 1_600_000_000;
    let remote = MemoryRemote::new();
    upload(&remote, &src, &set).unwrap();
    let dest = temp("mt-dest");
    download_entry(
        &remote,
        &layout(),
        GAME,
        ENTRY,
        &set,
        &dest,
        &SaveSet::default(),
    )
    .unwrap();
    for file in &set.files {
        let path = file.path.split('/').fold(dest.clone(), |p, s| p.join(s));
        let modified = fs::metadata(path).unwrap().modified().unwrap();
        assert_eq!(modified, UNIX_EPOCH + Duration::from_secs(file.mtime));
    }
}

#[test]
fn file_changed_mid_upload_fails() {
    let (src, set) = saves("changed");
    fs::write(src.join("a.sav"), b"the game wrote this").unwrap();
    let remote = MemoryRemote::new();
    let err = upload(&remote, &src, &set).unwrap_err();
    assert_eq!(err, Error::Io("a.sav changed while uploading".to_string()));
    assert!(remote.file_paths().is_empty());
}

#[test]
fn read_record_cases() {
    let (src, set) = saves("rec");
    let remote = MemoryRemote::new();
    upload(&remote, &src, &set).unwrap();
    assert_eq!(read_record(&remote, &layout(), GAME, ENTRY).unwrap(), set);
    assert!(matches!(
        read_record(&remote, &layout(), GAME, "nope"),
        Err(Error::NotFound(_))
    ));
    remote
        .put(&format!("{BASE}/{RECORD}"), b"not json", &Precondition::Any)
        .unwrap();
    assert!(matches!(
        read_record(&remote, &layout(), GAME, ENTRY),
        Err(Error::StateInvalid(_))
    ));
}

#[test]
fn download_refuses_paths_that_leave_the_saves_folder() {
    let remote = MemoryRemote::new();
    let root = temp("unsafe");
    let dest = root.join("saves");
    fs::create_dir_all(&dest).unwrap();
    for bad in ["../escape.sav", "slots/../../escape.sav", "./a.sav", ".."] {
        let mut wanted = SaveSet::default();
        wanted.files.push(save_relay::scan::FileEntry {
            path: bad.to_string(),
            size: 1,
            mtime: 0,
            sha256: save_relay::scan::hash_bytes(b"x"),
        });
        let err = download_entry(
            &remote,
            &layout(),
            GAME,
            ENTRY,
            &wanted,
            &dest,
            &SaveSet::default(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::StateInvalid(_)), "{bad}: {err:?}");
    }
    assert!(!root.join("escape.sav").exists());
}
