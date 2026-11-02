//! The `Remote` contract (issue #12's expectations), written over `&dyn Remote` only, so each
//! check runs unchanged against `MemoryRemote`, `Dropbox` + `FakeDropbox` and `Drive` +
//! `FakeDrive`. Every test builds a fresh backend (`common`). Test names are `<backend>::<check>`.
//! The offline check asserts `Unreachable` on every call; the exact text "memory remote is
//! offline" is `MemoryRemote`'s own and is locked in `tests/remote.rs`.

mod common;

use save_relay::remote::{ensure_dir, Precondition, Remote, RemoteEntry};
use save_relay::Error;

const ANY: &Precondition = &Precondition::Any;

/// A server holding `g/a.txt` ("A"), `g/sub/b.txt` ("BB") and an empty folder `g/empty`.
fn sample(r: &dyn Remote) {
    for d in ["g", "g/sub", "g/empty"] {
        r.mkcol(d).unwrap();
    }
    r.put("g/a.txt", b"A", ANY).unwrap();
    r.put("g/sub/b.txt", b"BB", ANY).unwrap();
}

/// Every file path under `dir`, sorted, folders left out (through `list` only).
fn file_paths(r: &dyn Remote, dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    collect(r, dir, &mut out);
    out.sort();
    out
}

fn collect(r: &dyn Remote, dir: &str, out: &mut Vec<String>) {
    for entry in r.list(dir).unwrap() {
        let path = if dir.is_empty() {
            entry.name.clone()
        } else {
            format!("{dir}/{}", entry.name)
        };
        if entry.is_dir {
            collect(r, &path, out);
        } else {
            out.push(path);
        }
    }
}

type Row = (String, bool, u64, Option<String>);

/// `list` as (name, is_dir, size, etag) rows.
fn rows(r: &dyn Remote, dir: &str) -> Vec<Row> {
    let list: Vec<RemoteEntry> = r.list(dir).unwrap();
    list.into_iter()
        .map(|e| (e.name, e.is_dir, e.size, e.etag))
        .collect()
}

fn read(r: &dyn Remote, path: &str) -> Vec<u8> {
    r.get(path).unwrap().0
}

fn put_then_get_and_every_put_changes_the_etag(r: &dyn Remote, _online: &dyn Fn(bool)) {
    sample(r);
    let first = r.put("g/c.bin", &[0, 1, 2], ANY).unwrap();
    assert_eq!(r.get("g/c.bin").unwrap(), (vec![0, 1, 2], first.clone()));
    let second = r.put("g/c.bin", &[0, 1, 2], ANY).unwrap();
    assert_ne!(first, second);
    assert_eq!(r.get("g/c.bin").unwrap().1, second);
}

fn preconditions_match_and_absent(r: &dyn Remote, _online: &dyn Fn(bool)) {
    sample(r);
    let etag = r.get("g/a.txt").unwrap().1;
    let stale = Precondition::Match("\"nope\"".to_string());
    assert_eq!(r.put("g/a.txt", b"X", &stale), Err(Error::Changed));
    assert_eq!(
        r.put("g/a.txt", b"X", &Precondition::Absent),
        Err(Error::Changed)
    );
    assert_eq!(r.put("g/new.txt", b"N", &stale), Err(Error::Changed));
    assert_eq!(read(r, "g/a.txt"), b"A");
    r.put("g/a.txt", b"Y", &Precondition::Match(etag)).unwrap();
    r.put("g/new.txt", b"N", &Precondition::Absent).unwrap();
    assert_eq!(read(r, "g/a.txt"), b"Y");
    assert_eq!(read(r, "g/new.txt"), b"N");
}

fn missing_things_are_not_found(r: &dyn Remote, _online: &dyn Fn(bool)) {
    sample(r);
    let nf = |p: &str| Error::NotFound(p.to_string());
    assert_eq!(r.put("x/y/z.txt", b"1", ANY).unwrap_err(), nf("x/y"));
    assert_eq!(r.mkcol("x/y").unwrap_err(), nf("x"));
    assert_eq!(r.get("g/none.txt").unwrap_err(), nf("g/none.txt"));
    assert_eq!(r.get("g/sub").unwrap_err(), nf("g/sub"));
    assert_eq!(r.list("nope").unwrap_err(), nf("nope"));
    assert_eq!(r.delete("g/none").unwrap_err(), nf("g/none"));
    assert_eq!(r.copy("g/none", "g/n2").unwrap_err(), nf("g/none"));
    assert_eq!(r.move_to("g/a.txt", "q/a.txt").unwrap_err(), nf("q"));
}

fn mkcol_is_idempotent_but_not_over_a_file(r: &dyn Remote, _online: &dyn Fn(bool)) {
    sample(r);
    r.mkcol("g/sub").unwrap();
    assert_eq!(read(r, "g/sub/b.txt"), b"BB");
    assert!(matches!(r.mkcol("g/a.txt"), Err(Error::Server(_))));
}

fn list_gives_direct_children_sorted(r: &dyn Remote, _online: &dyn Fn(bool)) {
    sample(r);
    let etag = r.get("g/a.txt").unwrap().1;
    let dir = |n: &str| (n.to_string(), true, 0, None);
    let file = ("a.txt".to_string(), false, 1, Some(etag));
    assert_eq!(rows(r, "g"), vec![file, dir("empty"), dir("sub")]);
    assert_eq!(rows(r, ""), vec![dir("g")]);
    assert_eq!(rows(r, "g/empty"), vec![]);
}

fn copy_duplicates_a_whole_folder(r: &dyn Remote, _online: &dyn Fn(bool)) {
    sample(r);
    r.copy("g", "h").unwrap();
    let want = ["g/a.txt", "g/sub/b.txt", "h/a.txt", "h/sub/b.txt"];
    assert_eq!(file_paths(r, ""), want);
    assert_eq!(read(r, "h/sub/b.txt"), b"BB");
    assert_eq!(rows(r, "h/empty"), vec![]);
    assert!(matches!(
        r.copy("g/a.txt", "h/a.txt"),
        Err(Error::Server(_))
    ));
}

fn move_renames_a_whole_folder(r: &dyn Remote, _online: &dyn Fn(bool)) {
    sample(r);
    r.mkcol("h").unwrap();
    r.move_to("g/sub", "h/moved").unwrap();
    assert_eq!(file_paths(r, ""), ["g/a.txt", "h/moved/b.txt"]);
    assert_eq!(
        r.list("g/sub").unwrap_err(),
        Error::NotFound("g/sub".into())
    );
    assert!(matches!(
        r.move_to("g/a.txt", "h/moved"),
        Err(Error::Server(_))
    ));
    assert_eq!(read(r, "g/a.txt"), b"A");
}

fn delete_removes_a_file_or_a_whole_folder(r: &dyn Remote, _online: &dyn Fn(bool)) {
    sample(r);
    r.delete("g/a.txt").unwrap();
    assert_eq!(file_paths(r, ""), ["g/sub/b.txt"]);
    r.delete("g").unwrap();
    assert!(file_paths(r, "").is_empty());
    assert_eq!(rows(r, ""), vec![]);
}

fn offline_fails_every_call(r: &dyn Remote, online: &dyn Fn(bool)) {
    sample(r);
    online(false);
    let unreachable = |e: Error| matches!(e, Error::Unreachable(_));
    assert!(unreachable(r.list("g").unwrap_err()));
    assert!(unreachable(r.get("g/a.txt").unwrap_err()));
    assert!(unreachable(r.put("g/z", b"", ANY).unwrap_err()));
    assert!(unreachable(r.mkcol("g/n").unwrap_err()));
    assert!(unreachable(r.copy("g", "h").unwrap_err()));
    assert!(unreachable(r.move_to("g", "h").unwrap_err()));
    assert!(unreachable(r.delete("g").unwrap_err()));
    assert!(unreachable(ensure_dir(r, "x/y").unwrap_err()));
    online(true);
    assert_eq!(read(r, "g/a.txt"), b"A");
    assert_eq!(file_paths(r, ""), ["g/a.txt", "g/sub/b.txt"]);
}

fn ensure_dir_creates_every_missing_level(r: &dyn Remote, _online: &dyn Fn(bool)) {
    sample(r);
    ensure_dir(r, "g/sub/x/y").unwrap();
    ensure_dir(r, "g/sub/x/y").unwrap();
    r.put("g/sub/x/y/f", b"F", ANY).unwrap();
    assert_eq!(file_paths(r, ""), ["g/a.txt", "g/sub/b.txt", "g/sub/x/y/f"]);
    assert!(r.list("g/sub/x").unwrap()[0].is_dir);
}

/// One `#[test]` per check and backend: `memory::put_then_get_...`, `dropbox::...`, `drive::...`.
macro_rules! contract {
    ($($check:ident),* $(,)?) => {
        mod memory {
            use super::*;
            $(
                #[test]
                fn $check() {
                    common::memory(super::$check);
                }
            )*
        }
        mod dropbox {
            use super::*;
            $(
                #[test]
                fn $check() {
                    common::dropbox(super::$check);
                }
            )*
        }
        mod drive {
            use super::*;
            $(
                #[test]
                fn $check() {
                    common::drive(super::$check);
                }
            )*
        }
    };
}

contract!(
    put_then_get_and_every_put_changes_the_etag,
    preconditions_match_and_absent,
    missing_things_are_not_found,
    mkcol_is_idempotent_but_not_over_a_file,
    list_gives_direct_children_sorted,
    copy_duplicates_a_whole_folder,
    move_renames_a_whole_folder,
    delete_removes_a_file_or_a_whole_folder,
    offline_fails_every_call,
    ensure_dir_creates_every_missing_level,
);
