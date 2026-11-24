//! The game-files manifest: strict v1 JSON and safe relative paths (docs/assets.md).

use save_relay::gamefiles::manifest::{check_app_id, parse};
use save_relay::Error;

const HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn one(path: &str) -> String {
    format!(
        r#"{{"version":1,"app_id":"g","files":[{{"path":"{path}","size":3,"sha256":"{HASH}"}}]}}"#
    )
}

fn rejected(json: &str) -> String {
    match parse(json.as_bytes(), "g") {
        Err(Error::ManifestInvalid(message)) => message,
        other => panic!("expected manifest_invalid for {json}, got {other:?}"),
    }
}

#[test]
fn a_valid_manifest_parses_with_its_total_and_digest() {
    let json = format!(
        r#"{{"version":1,"app_id":"g","files":[{{"path":"a/b.bin","size":3,"sha256":"{HASH}"}},{{"path":"c","size":0,"sha256":"{HASH}"}}]}}"#
    );
    let manifest = parse(json.as_bytes(), "g").unwrap();
    assert_eq!(manifest.files.len(), 2);
    assert_eq!(manifest.total_bytes, 3);
    assert_eq!(manifest.digest.len(), 64);
    // The digest is of the exact bytes: another spelling is another session.
    let spaced = json.replace(",\"files\"", ", \"files\"");
    assert_ne!(
        parse(spaced.as_bytes(), "g").unwrap().digest,
        manifest.digest
    );
}

#[test]
fn version_app_id_unknown_fields_and_duplicate_keys_are_refused() {
    assert!(rejected(&one("a").replace("\"version\":1", "\"version\":2")).contains("version"));
    assert!(rejected(&one("a").replace("\"app_id\":\"g\"", "\"app_id\":\"h\"")).contains("app_id"));
    rejected(&one("a").replace("\"size\":3", "\"size\":3,\"mode\":7"));
    rejected(&one("a").replace("\"size\":3", "\"size\":3,\"size\":4"));
    rejected(&one("a").replace("\"version\":1", "\"version\":1,\"version\":1"));
    rejected(&one("a").replace("\"size\":3", "\"size\":-3"));
    rejected(&one("a").replace("\"size\":3", "\"size\":3.5"));
    rejected(r#"{"version":1,"app_id":"g","files":[]}"#);
    rejected("not json");
    rejected(&format!("{} trailing", one("a")));
}

#[test]
fn unsafe_and_unportable_paths_are_refused() {
    for path in [
        "",
        "/abs",
        "../x",
        "a/../x",
        "a//b",
        "./a",
        "a/",
        "a\\\\b",
        "C:x",
        "a/con",
        "NUL.txt",
        "a/com1.log",
        "lpt9",
        "bad?",
        "bad*",
        "end.",
        "end ",
        "a/.save-relay-x",
        "tab\\tchar",
        "nul\\u0000",
    ] {
        rejected(&one(path));
    }
    // Long names and paths.
    rejected(&one(&"x".repeat(256)));
    rejected(&one(&vec!["abcdefgh"; 600].join("/")));
    // Still fine.
    for path in [
        "a",
        "a.b/c-d_e",
        "console",
        "com10",
        "lpt0",
        ".hidden/x",
        "ünï/cødé",
    ] {
        parse(one(path).as_bytes(), "g").unwrap();
    }
}

#[test]
fn digests_must_be_64_lowercase_hex() {
    for bad in [
        HASH.to_uppercase(),
        HASH[..63].to_string(),
        format!("{HASH}0"),
        "g".repeat(64),
    ] {
        rejected(&one("a").replace(HASH, &bad));
    }
}

#[test]
fn duplicate_case_fold_and_file_folder_collisions_are_refused() {
    let entry = |p: &str| format!(r#"{{"path":"{p}","size":1,"sha256":"{HASH}"}}"#);
    let with = |paths: &[&str]| {
        format!(
            r#"{{"version":1,"app_id":"g","files":[{}]}}"#,
            paths.iter().map(|p| entry(p)).collect::<Vec<_>>().join(",")
        )
    };
    rejected(&with(&["a", "a"]));
    rejected(&with(&["Data/A.bin", "data/a.BIN"]));
    rejected(&with(&["a", "a/b"]));
    rejected(&with(&["x/y", "X/y/z"]));
    parse(with(&["a", "ab", "a.b/c"]).as_bytes(), "g").unwrap();
}

#[test]
fn sizes_that_overflow_and_manifests_over_4_mib_are_refused() {
    let json = format!(
        r#"{{"version":1,"app_id":"g","files":[{{"path":"a","size":18446744073709551615,"sha256":"{HASH}"}},{{"path":"b","size":1,"sha256":"{HASH}"}}]}}"#
    );
    assert!(rejected(&json).contains("overflow"));
    let big = vec![b' '; 4 * 1024 * 1024 + 1];
    assert!(matches!(parse(&big, "g"), Err(Error::ManifestInvalid(_))));
}

#[test]
fn app_ids_are_single_portable_names() {
    for good in ["a", "uncharted-1.ios", "9x_y"] {
        check_app_id(good).unwrap();
    }
    for bad in ["", ".", "..", "-a", "A", "a/b", "a b", &"a".repeat(129)] {
        assert!(matches!(check_app_id(bad), Err(Error::Usage(_))), "{bad}");
    }
}
