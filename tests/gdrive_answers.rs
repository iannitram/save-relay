use save_relay::gdrive::{children_query, escape_q, parse_file, parse_files, pick, DriveFile};
use save_relay::Error;

const LIST: &str = r#"{"nextPageToken":"p2","files":[
 {"id":"F1","name":"history","mimeType":"application/vnd.google-apps.folder",
  "version":"3","createdTime":"2026-10-31T19:51:00.120Z"},
 {"id":"S1","name":"state.json","mimeType":"application/json","size":"412",
  "version":"12","createdTime":"2026-10-30T08:00:00.000Z",
  "appProperties":{"save_relay_claim":"1AbC","other":"x"}}
]}"#;

fn f(id: &str, name: &str, created: &str) -> DriveFile {
    DriveFile {
        id: id.into(),
        name: name.into(),
        is_dir: false,
        size: 0,
        version: 1,
        created: created.into(),
        claim: None,
    }
}

#[test]
fn query_literals_are_escaped() {
    assert_eq!(escape_q("plain"), "plain");
    assert_eq!(escape_q("it's"), r"it\'s");
    assert_eq!(escape_q(r"a\b"), r"a\\b");
    assert_eq!(escape_q(r"'\"), r"\'\\");
}

#[test]
fn children_queries() {
    let q = children_query("abc", None);
    assert_eq!(q, "'abc' in parents and trashed = false");
    let q = children_query("root", Some("save-relay"));
    assert_eq!(
        q,
        "'root' in parents and trashed = false and name = 'save-relay'"
    );
    let q = children_query("p'1", Some("Joe's"));
    assert_eq!(
        q,
        r"'p\'1' in parents and trashed = false and name = 'Joe\'s'"
    );
}

#[test]
fn a_page_of_files() {
    let page = parse_files(LIST).unwrap();
    assert_eq!(page.next_page.as_deref(), Some("p2"));
    let folder = DriveFile {
        is_dir: true,
        version: 3,
        ..f("F1", "history", "2026-10-31T19:51:00.120Z")
    };
    let state = DriveFile {
        size: 412,
        version: 12,
        claim: Some("1AbC".into()),
        ..f("S1", "state.json", "2026-10-30T08:00:00.000Z")
    };
    assert_eq!(page.files, vec![folder, state]);
}

#[test]
fn an_empty_last_page() {
    let page = parse_files(r#"{"files":[]}"#).unwrap();
    assert_eq!((page.files.len(), page.next_page), (0, None));
    assert!(parse_files("{}").unwrap().files.is_empty());
}

#[test]
fn bad_list_answers() {
    let bad = Err(Error::Server("drive: bad list answer".into()));
    for body in [
        "",
        "nope",
        "[]",
        r#"{"files":{}}"#,
        r#"{"files":[{"name":"a"}]}"#,
        r#"{"files":[{"id":"a"}]}"#,
        r#"{"files":[{"id":"a","name":"a","version":"x"}]}"#,
        r#"{"files":[{"id":"a","name":"a","size":"-1"}]}"#,
    ] {
        assert_eq!(parse_files(body), bad, "{body}");
    }
}

#[test]
fn one_file() {
    let body = r#"{"id":"S1","name":"state.json","version":"13","size":"9"}"#;
    let want = DriveFile {
        size: 9,
        version: 13,
        ..f("S1", "state.json", "")
    };
    assert_eq!(parse_file(body), Ok(want));
    let bad = Err(Error::Server("drive: bad file answer".into()));
    assert_eq!(parse_file(r#"{"name":"x"}"#), bad);
    assert_eq!(parse_file("x"), bad);
}

#[test]
fn pick_takes_the_oldest_then_the_lowest_id() {
    let files = [
        f("B", "g", "2026-10-31T10:00:00.000Z"),
        f("Z", "other", "2020-01-01T00:00:00.000Z"),
        f("C", "g", "2026-10-31T09:00:00.000Z"),
        f("A", "g", "2026-10-31T09:00:00.000Z"),
    ];
    assert_eq!(pick(&files, "g").map(|x| x.id.as_str()), Some("A"));
    assert_eq!(pick(&files, "other").map(|x| x.id.as_str()), Some("Z"));
    assert_eq!(pick(&files, "none"), None);
    assert_eq!(pick(&[], "g"), None);
}
