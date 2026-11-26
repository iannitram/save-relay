//! The game-files manifest: strict v1 JSON and safe relative paths (docs/assets.md).

use save_relay::gamefiles::manifest::{check_app_id, parse, parse_with, Mode};
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

#[test]
fn unicode_forms_and_wide_letters_collide_and_reserve_like_their_plain_forms() {
    let entry = |p: &str| format!(r#"{{"path":"{p}","size":1,"sha256":"{HASH}"}}"#);
    let with = |paths: &[&str]| {
        format!(
            r#"{{"version":1,"app_id":"g","files":[{}]}}"#,
            paths.iter().map(|p| entry(p)).collect::<Vec<_>>().join(",")
        )
    };
    // "e" + combining acute against the single code point.
    rejected(&with(&["caf\u{e9}", "cafe\u{301}"]));
    // Full-width letters fold to the plain ones (and so do upper and lower case).
    rejected(&with(&["data", "\u{ff24}\u{ff21}\u{ff34}\u{ff21}"]));
    // A folded file/folder collision.
    rejected(&with(&["Stra\u{df}e", "STRASSE/x"]));
    // A wide reserved device name and a metadata name in another case.
    rejected(&one("\u{ff23}\u{ff2f}\u{ff2e}.txt"));
    rejected(&one("a/.SAVE-RELAY-state"));
    rejected(&one("CON .txt"));
    rejected(&one("conin$"));
    parse(with(&["caf\u{e9}", "cafes"]).as_bytes(), "g").unwrap();
}

#[test]
fn more_than_100000_files_are_refused() {
    let entries: Vec<String> = (0..100_001)
        .map(|i| format!(r#"{{"path":"f{i}","size":0,"sha256":"{HASH}"}}"#))
        .collect();
    let json = format!(
        r#"{{"version":1,"app_id":"g","files":[{}]}}"#,
        entries.join(",")
    );
    // With 64-digit hashes the 4 MiB bound is reached first; either way it is refused.
    let message = rejected(&json);
    assert!(
        message.contains("100000") || message.contains("4 MiB"),
        "{message}"
    );
}

const LEGACY: Mode<'static> = Mode::Legacy {
    root: "acnh-universal-game/android",
};

fn legacy(json: &str) -> save_relay::Result<save_relay::gamefiles::manifest::Manifest> {
    parse_with(json.as_bytes(), "acnh-universal", LEGACY)
}

#[test]
fn legacy_mode_accepts_a_manifest_without_app_id_and_binds_the_id_and_root() {
    let json =
        format!(r#"{{"version":1,"files":[{{"path":"data/a.bin","size":3,"sha256":"{HASH}"}}]}}"#);
    let manifest = legacy(&json).unwrap();
    assert_eq!(manifest.app_id, "acnh-universal");
    // The digest names the id and the root, not only the bytes.
    let other_root = parse_with(
        json.as_bytes(),
        "acnh-universal",
        Mode::Legacy { root: "elsewhere" },
    )
    .unwrap();
    let other_app = parse_with(json.as_bytes(), "another", LEGACY).unwrap();
    assert_ne!(manifest.digest, other_root.digest);
    assert_ne!(manifest.digest, other_app.digest);
    // The standard mode still demands an app id, and a stated id must match.
    assert!(matches!(
        parse(json.as_bytes(), "acnh-universal"),
        Err(Error::ManifestInvalid(_))
    ));
    let named = json.replace(r#""files""#, r#""app_id":"acnh-universal","files""#);
    legacy(&named).unwrap();
    let wrong = json.replace(r#""files""#, r#""app_id":"other","files""#);
    assert!(legacy(&wrong).is_err());
}

#[test]
fn legacy_mode_refuses_player_data_and_a_bad_root_but_keeps_every_other_rule() {
    let bad = |path: &str| {
        let json =
            format!(r#"{{"version":1,"files":[{{"path":"{path}","size":1,"sha256":"{HASH}"}}]}}"#);
        assert!(
            matches!(legacy(&json), Err(Error::ManifestInvalid(_))),
            "{path}"
        );
    };
    for path in [
        "user/save.dat",
        "User/x",
        "config.ini",
        "CONFIG.INI",
        "logs/a.txt",
        "logs",
        "manifest.json",
        "../x",
        "a/con",
    ] {
        bad(path);
    }
    let json = format!(r#"{{"version":1,"files":[{{"path":"a","size":1,"sha256":"{HASH}"}}]}}"#);
    for root in ["", "/abs", "a/../b", r"a\b", "x//y"] {
        let result = parse_with(json.as_bytes(), "acnh-universal", Mode::Legacy { root });
        assert!(matches!(result, Err(Error::Usage(_))), "root {root:?}");
    }
}

// ---- #309 manifest lane: folding, reserved names, optional fields, device-owned paths ----

fn files_json(paths: &[&str]) -> String {
    let entries: Vec<String> = paths
        .iter()
        .map(|p| format!(r#"{{"path":"{p}","size":1,"sha256":"{HASH}"}}"#))
        .collect();
    format!(
        r#"{{"version":1,"app_id":"g","files":[{}]}}"#,
        entries.join(",")
    )
}

fn no_app_id(json: &str) -> String {
    json.replace(r#""app_id":"g","#, "")
}

#[test]
fn full_case_folding_and_normalization_collide() {
    // Final sigma, sharp s, long s, NFC against NFD, capital sharp s, and prefix forms.
    for (a, b) in [
        ("a\u{3c3}", "a\u{3c2}"),
        ("Stra\u{df}e", "strasse"),
        ("\u{17f}.bin", "s.bin"),
        ("caf\u{e9}.bin", "cafe\u{301}.bin"),
        ("\u{c4}", "A\u{308}"),
        ("\u{1e9e}", "ss"),
    ] {
        rejected(&files_json(&[a, b]));
    }
    rejected(&files_json(&["A\u{308}", "\u{c4}/x"]));
    rejected(&files_json(&["\u{3a3}", "\u{3c2}/x"]));
    parse(files_json(&["a\u{3c3}", "a\u{3c3}b"]).as_bytes(), "g").unwrap();
}

#[test]
fn windows_reserved_names_in_every_spelling_are_refused() {
    for name in [
        "COM0",
        "lpt0",
        "Com0.txt",
        "COM\u{b9}",
        "lpt\u{b2}.bin",
        "COM\u{b3}",
        "CONIN$",
        "conout$",
        "CONOUT$.txt",
        "NUL .txt",
        "aux  .x",
        "dir/Prn .a",
        "CON",
        "com9",
        "LPT9.log",
    ] {
        rejected(&one(name));
    }
    for name in ["com10", "console", "conin", "nulx", "com", "lpt"] {
        parse(one(name).as_bytes(), "g").unwrap();
    }
}

#[test]
fn device_owned_paths_are_refused_in_every_mode() {
    for path in [
        "user",
        "user/save.dat",
        "USER/x",
        "config.ini",
        "Config.INI",
        "root/user",
        "root/user/a",
        "ROOT/User/a",
        "root/config.ini",
        "Root/CONFIG.ini",
    ] {
        let json = files_json(&[path]);
        assert!(
            matches!(parse(json.as_bytes(), "g"), Err(Error::ManifestInvalid(_))),
            "standard {path}"
        );
        assert!(
            matches!(legacy(&no_app_id(&json)), Err(Error::ManifestInvalid(_))),
            "legacy {path}"
        );
    }
    // Neighbours stay fine; logs and manifest.json are refused by legacy mode only.
    for path in [
        "users/a",
        "root/users",
        "root/a/user",
        "a/config.ini",
        "root",
        "root/data",
    ] {
        parse(files_json(&[path]).as_bytes(), "g").unwrap();
    }
    parse(files_json(&["logs/a", "manifest.json"]).as_bytes(), "g").unwrap();
    assert!(legacy(&no_app_id(&files_json(&["logs/a"]))).is_err());
}

#[test]
fn optional_fields_build_id_nx_game_abi_and_exec() {
    let linux = format!(
        r#"{{"version":1,"build_id":"2026.11-a1","nx_game_abi":"nx-1","files":[{{"path":"bin/game","size":3,"sha256":"{HASH}","exec":true}},{{"path":"data/a.bin","size":3,"sha256":"{HASH}"}}]}}"#
    );
    // The #325 Linux shape has no app_id: it parses in legacy (host-trusted) mode ...
    let m = legacy(&linux).unwrap();
    assert_eq!(m.build_id.as_deref(), Some("2026.11-a1"));
    assert_eq!(m.nx_game_abi.as_deref(), Some("nx-1"));
    assert!(m.files[0].exec);
    assert!(!m.files[1].exec);
    // ... and with an app_id in standard mode; without one, standard mode still refuses it.
    let named = linux.replace(r#""files""#, r#""app_id":"g","files""#);
    assert!(parse(named.as_bytes(), "g").unwrap().files[0].exec);
    rejected(&linux);
    // Absent fields stay absent.
    let plain = parse(one("a").as_bytes(), "g").unwrap();
    assert!(plain.build_id.is_none() && plain.nx_game_abi.is_none() && !plain.files[0].exec);
    // Bounds, characters, types and unknown fields.
    for bad in [
        r#""build_id":"""#.to_string(),
        format!(r#""build_id":"{}""#, "x".repeat(129)),
        r#""build_id":"a\nb""#.to_string(),
        "\"nx_game_abi\":\"caf\u{e9}\"".to_string(),
        r#""nx_game_abi":7"#.to_string(),
    ] {
        rejected(&one("a").replace(r#""files""#, &format!(r#"{bad},"files""#)));
    }
    let longest = format!(r#""build_id":"{}","files""#, "x".repeat(128));
    parse(one("a").replace(r#""files""#, &longest).as_bytes(), "g").unwrap();
    rejected(&one("a").replace(r#""size":3"#, r#""size":3,"exec":"yes""#));
    rejected(&one("a").replace(r#""size":3"#, r#""size":3,"exec":true,"exec":false"#));
    rejected(&one("a").replace(r#""files""#, r#""abi":"x","files""#));
}

#[test]
fn acnh_android_shape_parses_in_legacy_mode_only() {
    let acnh = format!(
        r#"{{"version":1,"files":[{{"path":"romfs/a.bin","size":3,"sha256":"{HASH}"}},{{"path":"main.bin","size":9,"sha256":"{HASH}"}}]}}"#
    );
    assert_eq!(legacy(&acnh).unwrap().total_bytes, 12);
    assert!(parse(acnh.as_bytes(), "acnh-universal").is_err());
}

#[test]
fn legacy_universal_layout_is_allowlisted_by_host_os() {
    use save_relay::gamefiles::{HostOs, Layout};
    assert_eq!(
        Layout::legacy_universal("acnh", HostOs::Android).unwrap(),
        Layout::Legacy {
            root: "acnh-universal-game/android".into()
        }
    );
    for (token, os) in [
        ("android", HostOs::Android),
        ("ios", HostOs::Ios),
        ("linux", HostOs::Linux),
        ("macos", HostOs::Macos),
        ("windows", HostOs::Windows),
    ] {
        assert_eq!(HostOs::from_token(token), Some(os));
        assert_eq!(os.token(), token);
        Layout::legacy_universal("my-port2", os)
            .unwrap()
            .validate()
            .unwrap();
    }
    for token in [
        "", "Android", "../x", "android/", "linux ", "freebsd", "ios\n",
    ] {
        assert_eq!(HostOs::from_token(token), None, "{token:?}");
    }
    for app in [
        "", "A", "a b", "a/b", "../x", "a..", "-a", "a-", "a--b", "a.b", "a\u{e9}",
    ] {
        assert!(
            Layout::legacy_universal(app, HostOs::Linux).is_err(),
            "{app:?}"
        );
    }
    assert!(HostOs::current().is_some());
}
