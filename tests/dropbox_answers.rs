use save_relay::dropbox::{parse_list, parse_rev, ListPage};
use save_relay::remote::RemoteEntry;
use save_relay::Error;

const LIST: &str = r#"{"entries":[
 {".tag":"file","name":"state.json","path_display":"/g/state.json","id":"id:1",
  "rev":"a1c10ce0dd78","size":412,"server_modified":"2026-10-31T19:51:00Z"},
 {".tag":"folder","name":"history","path_display":"/g/history","id":"id:2"},
 {".tag":"deleted","name":"old.json","path_display":"/g/old.json"},
 {".tag":"file","name":"Incoming Notes.txt","id":"id:3","rev":"0f1","size":0}
],"cursor":"AAE1","has_more":true}"#;

fn file(name: &str, size: u64, rev: &str) -> RemoteEntry {
    RemoteEntry {
        name: name.into(),
        is_dir: false,
        size,
        etag: Some(rev.into()),
    }
}

#[test]
fn a_page_of_files_and_folders_sorted_by_name() {
    let folder = RemoteEntry {
        name: "history".into(),
        is_dir: true,
        size: 0,
        etag: None,
    };
    let want = ListPage {
        entries: vec![
            file("Incoming Notes.txt", 0, "0f1"),
            folder,
            file("state.json", 412, "a1c10ce0dd78"),
        ],
        cursor: "AAE1".into(),
        has_more: true,
    };
    assert_eq!(parse_list(LIST), Ok(want));
}

#[test]
fn an_empty_last_page() {
    let got = parse_list(r#"{"entries":[],"cursor":"Z","has_more":false}"#).unwrap();
    assert!(got.entries.is_empty());
    assert_eq!((got.cursor.as_str(), got.has_more), ("Z", false));
}

#[test]
fn bad_list_answers() {
    let bad = Err(Error::Server("dropbox: bad list answer".into()));
    for body in [
        "",
        "not json",
        "[1]",
        r#"{"cursor":"x"}"#,
        r#"{"entries":[{"name":"a"}],"cursor":"x","has_more":false}"#,
        r#"{"entries":[{".tag":"file"}],"cursor":"x","has_more":false}"#,
    ] {
        assert_eq!(parse_list(body), bad, "{body}");
    }
}

#[test]
fn rev_of_a_metadata_answer() {
    let body = r#"{"name":"state.json","id":"id:1","rev":"a1c10ce0dd78","size":412}"#;
    assert_eq!(parse_rev(body), Ok("a1c10ce0dd78".to_string()));
    let bad = Err(Error::Server("dropbox: no rev in the answer".into()));
    for body in ["{}", "x", r#"{"rev":5}"#, ""] {
        assert_eq!(parse_rev(body), bad, "{body}");
    }
}
