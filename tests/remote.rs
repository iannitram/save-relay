use save_relay::remote::{ensure_dir, MemoryRemote, Precondition, Remote};
use save_relay::Error;

const ANY: &Precondition = &Precondition::Any;

/// A server holding `g/a.txt` ("A"), `g/sub/b.txt` ("BB") and an empty folder `g/empty`.
fn sample() -> MemoryRemote {
    let r = MemoryRemote::new();
    for d in ["g", "g/sub", "g/empty"] {
        r.mkcol(d).unwrap();
    }
    r.put("g/a.txt", b"A", ANY).unwrap();
    r.put("g/sub/b.txt", b"BB", ANY).unwrap();
    r
}

#[test]
fn put_then_get_and_every_put_changes_the_etag() {
    let r = sample();
    let first = r.put("g/c.bin", &[0, 1, 2], ANY).unwrap();
    assert_eq!(r.get("g/c.bin").unwrap(), (vec![0, 1, 2], first.clone()));
    let second = r.put("g/c.bin", &[0, 1, 2], ANY).unwrap();
    assert_ne!(first, second);
    assert_eq!(r.get("g/c.bin").unwrap().1, second);
}

#[test]
fn preconditions_match_and_absent() {
    let r = sample();
    let etag = r.get("g/a.txt").unwrap().1;
    let stale = Precondition::Match("\"nope\"".to_string());
    assert_eq!(r.put("g/a.txt", b"X", &stale), Err(Error::Changed));
    assert_eq!(
        r.put("g/a.txt", b"X", &Precondition::Absent),
        Err(Error::Changed)
    );
    assert_eq!(r.put("g/new.txt", b"N", &stale), Err(Error::Changed));
    assert_eq!(r.read("g/a.txt").unwrap(), b"A");
    r.put("g/a.txt", b"Y", &Precondition::Match(etag)).unwrap();
    r.put("g/new.txt", b"N", &Precondition::Absent).unwrap();
    assert_eq!(r.read("g/a.txt").unwrap(), b"Y");
    assert_eq!(r.read("g/new.txt").unwrap(), b"N");
}

#[test]
fn missing_things_are_not_found() {
    let r = sample();
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

#[test]
fn mkcol_is_idempotent_but_not_over_a_file() {
    let r = sample();
    r.mkcol("g/sub").unwrap();
    assert_eq!(r.read("g/sub/b.txt").unwrap(), b"BB");
    assert!(matches!(r.mkcol("g/a.txt"), Err(Error::Server(_))));
}

type Row = (String, bool, u64, Option<String>);

/// `list` as (name, is_dir, size, etag) rows.
fn rows(r: &MemoryRemote, dir: &str) -> Vec<Row> {
    let list = r.list(dir).unwrap().into_iter();
    list.map(|e| (e.name, e.is_dir, e.size, e.etag)).collect()
}

#[test]
fn list_gives_direct_children_sorted() {
    let r = sample();
    let etag = r.get("g/a.txt").unwrap().1;
    let dir = |n: &str| (n.to_string(), true, 0, None);
    let file = ("a.txt".to_string(), false, 1, Some(etag));
    assert_eq!(rows(&r, "g"), vec![file, dir("empty"), dir("sub")]);
    assert_eq!(rows(&r, ""), vec![dir("g")]);
    assert_eq!(rows(&r, "g/empty"), vec![]);
}

#[test]
fn copy_duplicates_a_whole_folder() {
    let r = sample();
    r.copy("g", "h").unwrap();
    let want = ["g/a.txt", "g/sub/b.txt", "h/a.txt", "h/sub/b.txt"];
    assert_eq!(r.file_paths(), want);
    assert_eq!(r.read("h/sub/b.txt").unwrap(), b"BB");
    assert_eq!(rows(&r, "h/empty"), vec![]);
    assert!(matches!(
        r.copy("g/a.txt", "h/a.txt"),
        Err(Error::Server(_))
    ));
}

#[test]
fn move_renames_a_whole_folder() {
    let r = sample();
    r.mkcol("h").unwrap();
    r.move_to("g/sub", "h/moved").unwrap();
    assert_eq!(r.file_paths(), ["g/a.txt", "h/moved/b.txt"]);
    assert_eq!(
        r.list("g/sub").unwrap_err(),
        Error::NotFound("g/sub".into())
    );
    assert!(matches!(
        r.move_to("g/a.txt", "h/moved"),
        Err(Error::Server(_))
    ));
    assert_eq!(r.read("g/a.txt").unwrap(), b"A");
}

#[test]
fn delete_removes_a_file_or_a_whole_folder() {
    let r = sample();
    r.delete("g/a.txt").unwrap();
    assert_eq!(r.file_paths(), ["g/sub/b.txt"]);
    r.delete("g").unwrap();
    assert!(r.file_paths().is_empty());
    assert_eq!(rows(&r, ""), vec![]);
}

#[test]
fn offline_fails_every_call_but_inspection_still_works() {
    let r = sample();
    r.set_reachable(false);
    let off = Error::Unreachable("memory remote is offline".to_string());
    assert_eq!(r.list("g").unwrap_err(), off);
    assert_eq!(r.get("g/a.txt").unwrap_err(), off);
    assert_eq!(r.put("g/z", b"", ANY).unwrap_err(), off);
    assert_eq!(r.mkcol("g/n").unwrap_err(), off);
    assert_eq!(r.copy("g", "h").unwrap_err(), off);
    assert_eq!(r.move_to("g", "h").unwrap_err(), off);
    assert_eq!(r.delete("g").unwrap_err(), off);
    assert_eq!(ensure_dir(&r, "x/y").unwrap_err(), off);
    assert_eq!(r.file_paths(), ["g/a.txt", "g/sub/b.txt"]);
    assert_eq!(r.read("g/a.txt").unwrap(), b"A");
    r.set_reachable(true);
    assert_eq!(r.get("g/a.txt").unwrap().0, b"A");
}

#[test]
fn ensure_dir_creates_every_missing_level() {
    let r = sample();
    ensure_dir(&r, "g/sub/x/y").unwrap();
    ensure_dir(&r, "g/sub/x/y").unwrap();
    r.put("g/sub/x/y/f", b"F", ANY).unwrap();
    assert_eq!(r.file_paths(), ["g/a.txt", "g/sub/b.txt", "g/sub/x/y/f"]);
    assert!(r.list("g/sub/x").unwrap()[0].is_dir);
}
