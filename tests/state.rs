use save_relay::scan::{FileEntry, SaveSet};
use save_relay::state::{LastSynced, ServerState};
use save_relay::Error;
use serde_json::{json, Value};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn server() -> Value {
    json!({
        "format": 1, "game": "uncharted-1", "revision": 7, "device": "amd-handheld",
        "time": "2026-10-31T19:51:00Z", "entry": "20261031T195100Z-amd-handheld",
        "files": [
            {"path": "Uncharted 1/SAVEDATA00/SAVEDATA.BIN", "size": 3, "mtime": 1793476260, "sha256": A},
            {"path": "Uncharted 1/SAVEDATA01/SAVEDATA.BIN", "size": 5, "mtime": 1793476000, "sha256": B}
        ]
    })
}

fn last() -> Value {
    json!({"format": 1, "game": "uncharted-1", "revision": 0, "time": "2026-10-31T19:51:00Z",
           "files": [{"path": "a/b.sav", "size": 1, "mtime": 2, "sha256": B}]})
}

fn with(mut v: Value, key: &str, val: Value) -> String {
    v[key] = val;
    v.to_string()
}

fn file(path: &str, sha: &str) -> Value {
    json!({"path": path, "size": 1, "mtime": 1, "sha256": sha})
}

fn invalid_server(cases: &[String]) {
    for text in cases {
        match ServerState::from_json(text) {
            Err(Error::StateInvalid(_)) => {}
            other => panic!("expected StateInvalid for {text}, got {other:?}"),
        }
    }
}

#[test]
fn parses_a_good_server_state() {
    let s = ServerState::from_json(&server().to_string()).unwrap();
    assert_eq!((s.format, s.revision), (save_relay::state::FORMAT, 7));
    assert_eq!(s.game, "uncharted-1");
    assert_eq!(s.device, "amd-handheld");
    assert_eq!(s.time, "2026-10-31T19:51:00Z");
    assert_eq!(s.entry, "20261031T195100Z-amd-handheld");
    assert_eq!(s.files.len(), 2);
    let f = &s.files[1];
    assert_eq!(
        (f.path.as_str(), f.size, f.mtime, f.sha256.as_str()),
        ("Uncharted 1/SAVEDATA01/SAVEDATA.BIN", 5, 1793476000, B)
    );
}

#[test]
fn server_state_round_trips_through_pretty_json() {
    let s = ServerState::from_json(&server().to_string()).unwrap();
    let text = s.to_json();
    assert!(text.ends_with("}\n") && !text.ends_with("\n\n"));
    assert!(text.contains("\n  \"format\": 1,\n  \"game\": \"uncharted-1\",\n  \"revision\": 7,"));
    let order = [
        "format", "game", "revision", "device", "time", "entry", "files",
    ];
    let at: Vec<usize> = order
        .iter()
        .map(|k| text.find(&format!("\"{k}\"")).unwrap())
        .collect();
    assert!(
        at.windows(2).all(|w| w[0] < w[1]),
        "fields out of order: {text}"
    );
    assert_eq!(ServerState::from_json(&text).unwrap(), s);
    assert_eq!(
        s.save_set(),
        SaveSet {
            files: s.files.clone()
        }
    );
}

#[test]
fn rejects_bad_json_and_wrong_format() {
    invalid_server(&["{not json".into(), with(server(), "format", json!(0))]);
    let newer = ServerState::from_json(&with(server(), "format", json!(2)));
    match newer {
        Err(Error::StateInvalid(m)) => assert!(m.contains("newer"), "message: {m}"),
        other => panic!("expected StateInvalid, got {other:?}"),
    }
}

#[test]
fn rejects_revision_zero_and_empty_names() {
    invalid_server(&[
        with(server(), "revision", json!(0)),
        with(server(), "game", json!("")),
        with(server(), "device", json!("")),
        with(server(), "entry", json!("")),
    ]);
}

#[test]
fn rejects_bad_time_files_hashes_and_paths() {
    invalid_server(&[
        with(server(), "time", json!("2026-10-31 19:51:00")),
        with(server(), "time", json!("yesterday")),
        with(server(), "files", json!([file("b", A), file("a", A)])),
        with(server(), "files", json!([file("a", A), file("a", B)])),
        with(server(), "files", json!([file("a", &A[..63])])),
        with(server(), "files", json!([file("a", &A.to_uppercase())])),
        with(
            server(),
            "files",
            json!([file("a", &format!("{}g", &A[..63]))]),
        ),
        with(server(), "files", json!([file("/etc/passwd", A)])),
        with(server(), "files", json!([file("a/../../b", A)])),
    ]);
}

#[test]
fn last_synced_parses_with_any_revision_and_round_trips() {
    let l = LastSynced::from_json(&last().to_string()).unwrap();
    assert_eq!(
        (l.format, l.revision, l.game.as_str()),
        (1, 0, "uncharted-1")
    );
    let want = FileEntry {
        path: "a/b.sav".into(),
        size: 1,
        mtime: 2,
        sha256: B.into(),
    };
    assert_eq!(l.files, vec![want]);
    let text = l.to_json();
    assert!(text.ends_with("}\n") && text.contains("\n  \"revision\": 0,"));
    assert_eq!(LastSynced::from_json(&text).unwrap(), l);
}

#[test]
fn old_last_synced_without_folder_still_loads() {
    let mut value = last();
    value.as_object_mut().unwrap().remove("saves_folder");
    let parsed = LastSynced::from_json(&value.to_string()).unwrap();
    assert_eq!(parsed.saves_folder, None);
}

#[test]
fn last_synced_has_the_same_checks() {
    let bad = [
        with(last(), "format", json!(2)),
        with(last(), "game", json!("")),
        with(last(), "time", json!("31/10/2026")),
        with(last(), "files", json!([file("b", A), file("a", A)])),
        with(last(), "files", json!([file("a", "xyz")])),
        with(last(), "files", json!([file("../a", A)])),
        "[]".to_string(),
    ];
    for text in bad {
        assert!(
            matches!(LastSynced::from_json(&text), Err(Error::StateInvalid(_))),
            "accepted {text}"
        );
    }
}
