use save_relay::assets::parse_manifest;

const HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
fn manifest(paths: &[&str], app: &str) -> Vec<u8> {
    let files: Vec<_> = paths
        .iter()
        .map(|path| serde_json::json!({"path": path, "size": 3, "sha256": HASH}))
        .collect();
    serde_json::json!({"version":1,"app_id":app,"files":files})
        .to_string()
        .into_bytes()
}
#[test]
fn rejects_unsafe_and_colliding_paths() {
    for paths in [
        vec!["../a"],
        vec!["C:/x"],
        vec!["user/a"],
        vec!["config.ini"],
        vec!["logs/x"],
        vec!["a", "a/b"],
        vec!["A", "a"],
        vec!["CON.txt"],
        vec!["a\\b"],
    ] {
        assert!(
            parse_manifest(&manifest(&paths, "game"), "game", false).is_err(),
            "{paths:?}"
        );
    }
    assert!(parse_manifest(&manifest(&["data/a"], "game"), "game", false).is_ok());
}
#[test]
fn legacy_requires_explicit_opt_in_and_bound_id() {
    let raw =
        format!(r#"{{"version":1,"files":[{{"path":"data.bin","size":3,"sha256":"{HASH}"}}]}}"#);
    assert!(parse_manifest(raw.as_bytes(), "acnh", false).is_err());
    assert!(parse_manifest(raw.as_bytes(), "acnh", true).is_ok());
    assert!(parse_manifest(&manifest(&["x"], "other"), "acnh", true).is_err());
    let duplicate = raw.replace("\"version\":1", "\"version\":1,\"version\":1");
    assert!(parse_manifest(duplicate.as_bytes(), "acnh", true).is_err());
}

#[test]
fn rejects_malformed_or_ignored_ranges() {
    use save_relay::{assets::validate_range_response, http::Response};
    let valid = Response::new(206, b"bc".to_vec()).header("Content-Range", "bytes 1-2/4");
    assert!(validate_range_response(&valid, 1, 3, 4).is_ok());
    for response in [
        Response::new(200, b"abcd".to_vec()),
        Response::new(206, b"bc".to_vec()).header("Content-Range", "bytes 0-1/4"),
        Response::new(206, b"bc".to_vec()).header("Content-Range", "bytes 1-3/4"),
        Response::new(206, b"bc".to_vec()).header("Content-Range", "bytes 1-2/5"),
        Response::new(206, b"bc".to_vec())
            .header("Content-Range", "bytes 1-2/4")
            .header("Content-Encoding", "gzip"),
    ] {
        assert!(validate_range_response(&response, 1, 3, 4).is_err());
    }
}

mod common;
#[test]
fn range_contract_for_memory_dropbox_and_drive() {
    use save_relay::remote::{Precondition, Remote};
    fn check(remote: &dyn Remote, _reach: &dyn Fn(bool)) {
        remote
            .put("example", b"abcdef", &Precondition::Absent)
            .unwrap();
        let first = remote.read_range("example", 0, 2, None).unwrap();
        assert_eq!(first.bytes, b"ab");
        assert_eq!(first.total, 6);
        let next = remote
            .read_range("example", 2, 2, Some(&first.identity))
            .unwrap();
        assert_eq!(next.bytes, b"cd");
        remote
            .put("example", b"uvwxyz", &Precondition::Any)
            .unwrap();
        assert!(remote
            .read_range("example", 4, 2, Some(&first.identity))
            .is_err());
    }
    common::memory(check);
    common::dropbox(check);
    common::drive(check);
}

#[test]
fn webdav_range_is_bounded_and_conditional() {
    use save_relay::{
        creds::Secret,
        error::Result,
        http::{Request, Response, Transport},
        remote::Remote,
        webdav::WebDav,
    };
    use std::sync::Arc;
    struct Server;
    impl Transport for Server {
        fn send(&self, request: &Request) -> Result<Response> {
            assert_eq!(request.header_value("Range"), Some("bytes=2-3"));
            assert_eq!(request.header_value("If-Match"), Some("\"one\""));
            assert_eq!(request.max_response_bytes, Some(2));
            Ok(Response::new(206, b"cd".to_vec())
                .header("Content-Range", "bytes 2-3/6")
                .header("ETag", "\"one\""))
        }
    }
    let webdav = WebDav::new(
        "https://example.invalid/",
        "user",
        Secret::new("password"),
        Arc::new(Server),
    );
    let chunk = webdav
        .read_range("game/file", 2, 2, Some("\"one\""))
        .unwrap();
    assert_eq!(chunk.bytes, b"cd");
    assert_eq!(chunk.total, 6);
}
