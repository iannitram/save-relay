//! Locked tests for `scan` (F11). Do not edit: the code must make these pass.

use save_relay::scan::{hash_bytes, scan};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const SAVE_ONE: &str = "3c77d1f8a110b064f7ceaaba3a49f5410fa0c834648ba0392c7ad2f94e0e34dc";

/// A fresh, empty folder for one test.
fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("save-relay-scan-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, rel: &[&str], bytes: &[u8], mtime: u64) {
    let path = rel.iter().fold(dir.to_path_buf(), |p, s| p.join(s));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    let file = fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(UNIX_EPOCH + Duration::from_secs(mtime))
        .unwrap();
}

fn pats(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn hash_bytes_is_lowercase_hex_sha256() {
    assert_eq!(hash_bytes(b""), EMPTY);
    assert_eq!(hash_bytes(b"abc"), ABC);
    assert_eq!(hash_bytes(b"save one"), SAVE_ONE);
}

#[test]
fn missing_folder_is_an_empty_set() {
    let dir = temp("missing").join("not-there");
    let set = scan(&dir, &pats(&["**"]), &[]).unwrap();
    assert!(set.files.is_empty());
}

#[test]
fn lists_selected_files_sorted_with_slash_paths() {
    let dir = temp("listing");
    write(
        &dir,
        &["Uncharted 1", "SAVEDATA01", "data.bin"],
        b"abc",
        1793476260,
    );
    write(
        &dir,
        &["Uncharted 1", "SAVEDATA00", "data.bin"],
        b"save one",
        1709164800,
    );
    write(
        &dir,
        &["Uncharted 1", "SAVEDATA00", "sce_sys", "param.sfo"],
        b"",
        946684799,
    );
    write(&dir, &["Uncharted 1", "shader_cache", "x.bin"], b"abc", 1);
    write(&dir, &["Uncharted 1", "SAVEDATA00", "data.tmp"], b"abc", 1);
    write(&dir, &["last_run.log"], b"abc", 1);
    let set = scan(
        &dir,
        &pats(&["Uncharted 1/SAVEDATA*/**"]),
        &pats(&["**/*.tmp"]),
    )
    .unwrap();
    let got: Vec<(&str, u64, u64, &str)> = set
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.size, f.mtime, f.sha256.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("Uncharted 1/SAVEDATA00/data.bin", 8, 1709164800, SAVE_ONE),
            (
                "Uncharted 1/SAVEDATA00/sce_sys/param.sfo",
                0,
                946684799,
                EMPTY
            ),
            ("Uncharted 1/SAVEDATA01/data.bin", 3, 1793476260, ABC),
        ]
    );
}

#[test]
fn folders_are_not_entries() {
    let dir = temp("folders");
    fs::create_dir_all(dir.join("saves").join("empty")).unwrap();
    write(&dir, &["saves", "a.bin"], b"abc", 5);
    let set = scan(&dir, &pats(&["**"]), &[]).unwrap();
    let paths: Vec<&str> = set.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["saves/a.bin"]);
}

#[test]
fn sort_is_by_path_text() {
    let dir = temp("sorting");
    for name in ["b", "a.bin", "Z", "a", "a0"] {
        write(&dir, &[name], name.as_bytes(), 7);
    }
    write(&dir, &["dir", "z"], b"z", 7);
    let set = scan(&dir, &pats(&["**"]), &[]).unwrap();
    let paths: Vec<&str> = set.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["Z", "a", "a.bin", "a0", "b", "dir/z"]);
}

#[test]
fn nothing_selected_is_an_empty_set() {
    let dir = temp("none");
    write(&dir, &["save.bin"], b"abc", 3);
    assert!(scan(&dir, &pats(&["*.dat"]), &[]).unwrap().files.is_empty());
    assert!(scan(&dir, &pats(&["**"]), &pats(&["**"]))
        .unwrap()
        .files
        .is_empty());
}

#[cfg(unix)]
#[test]
fn symbolic_links_are_skipped_not_followed() {
    let dir = temp("links");
    let outside = temp("links-outside");
    write(&outside, &["secret.bin"], b"abc", 9);
    write(&dir, &["real.bin"], b"abc", 9);
    std::os::unix::fs::symlink(outside.join("secret.bin"), dir.join("file-link.bin")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("dir-link")).unwrap();
    let set = scan(&dir, &pats(&["**"]), &[]).unwrap();
    let paths: Vec<&str> = set.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["real.bin"]);
}
