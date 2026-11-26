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
        vec!["COM¹.txt"],
        vec!["café", "cafe\u{301}"],
        vec!["STRASSE", "straße"],
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

#[cfg(unix)]
#[test]
fn session_resumes_durable_bytes_and_never_replaces() {
    use save_relay::{
        assets::session::{AssetOptions, AssetSession},
        remote::{ensure_dir, MemoryRemote, Precondition, Remote},
    };
    use std::{fs, sync::Arc};
    let root = std::env::temp_dir().join(format!("sr-assets-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let dest = root.join("dest");
    let state = root.join("state");
    fs::create_dir_all(&dest).unwrap();
    fs::create_dir_all(&state).unwrap();
    let remote = Arc::new(MemoryRemote::new());
    ensure_dir(remote.as_ref(), "game-files/game/files/data").unwrap();
    remote
        .put(
            "game-files/game/files/data/a",
            b"abc",
            &Precondition::Absent,
        )
        .unwrap();
    let raw = manifest(&["data/a"], "game");
    let mut session = AssetSession::open(
        remote.clone(),
        &raw,
        AssetOptions {
            app_id: "game",
            allow_legacy: false,
            source_root: "game-files/game/files",
            scope: "test-account",
            destination: &dest,
            state_dir: &state,
        },
    )
    .unwrap();
    assert_eq!(session.step().unwrap().durable_bytes, 3);
    assert!(!session.progress().complete);
    assert!(AssetSession::open(
        remote.clone(),
        &raw,
        AssetOptions {
            app_id: "game",
            allow_legacy: false,
            source_root: "game-files/game/files",
            scope: "test-account",
            destination: &dest,
            state_dir: &state
        }
    )
    .is_err());
    drop(session);
    // Uncheckpointed tail is truncated rather than treated as durable.
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(state.join("part-0"))
        .unwrap()
        .write_all(b"bad")
        .unwrap();
    let mut resumed = AssetSession::open(
        remote,
        &raw,
        AssetOptions {
            app_id: "game",
            allow_legacy: false,
            source_root: "game-files/game/files",
            scope: "test-account",
            destination: &dest,
            state_dir: &state,
        },
    )
    .unwrap();
    assert!(resumed.step().unwrap().complete);
    assert_eq!(fs::read(dest.join("data/a")).unwrap(), b"abc");
    drop(resumed);
    fs::write(dest.join("data/a"), b"other").unwrap();
    assert!(AssetSession::open(
        Arc::new(MemoryRemote::new()),
        &raw,
        AssetOptions {
            app_id: "game",
            allow_legacy: false,
            source_root: "game-files/game/files",
            scope: "test-account",
            destination: &dest,
            state_dir: &state
        }
    )
    .is_err());
    assert_eq!(fs::read(dest.join("data/a")).unwrap(), b"other");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn interrupted_sessions_resume_across_provider_fakes() {
    use save_relay::{
        assets::session::{AssetOptions, AssetSession},
        clients::{Client, GOOGLE_DESKTOP},
        creds::Secret,
        dropbox::Dropbox,
        fake::{FakeDrive, FakeDropbox, REFRESH_TOKEN},
        gdrive::Drive,
        oauth::{TokenSource, DROPBOX, GOOGLE},
        remote::{ensure_dir, MemoryRemote, Precondition, Remote},
    };
    use sha2::{Digest, Sha256};
    use std::{fs, sync::Arc};
    let db = Arc::new(FakeDropbox::new());
    let db_tokens = TokenSource::new(
        &DROPBOX,
        Client {
            id: "test",
            secret: None,
        },
        Secret::new(REFRESH_TOKEN),
        db.clone(),
        || 1000,
    );
    let drive = Arc::new(FakeDrive::new());
    let drive_tokens = TokenSource::new(
        &GOOGLE,
        GOOGLE_DESKTOP,
        Secret::new(REFRESH_TOKEN),
        drive.clone(),
        || 1000,
    );
    let services: Vec<(&str, Arc<dyn Remote>)> = vec![
        ("memory", Arc::new(MemoryRemote::new())),
        (
            "dropbox",
            Arc::new(Dropbox::with_sleep(db_tokens, db, |_| {
                panic!("unexpected retry")
            })),
        ),
        (
            "drive",
            Arc::new(
                Drive::new(drive_tokens, drive, || 1000)
                    .with_wait(Arc::new(|_| panic!("unexpected retry"))),
            ),
        ),
    ];
    let data = vec![42; 1024 * 1024 + 3];
    let digest = format!("{:x}", Sha256::digest(&data));
    let raw = serde_json::json!({"version":1,"app_id":"game","files":[{"path":"payload.bin","size":data.len(),"sha256":digest}]}).to_string();
    for (service, remote) in services {
        let root = std::env::temp_dir().join(format!("sr-assets-{}-{service}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dest = root.join("dest");
        let state = root.join("state");
        fs::create_dir_all(&dest).unwrap();
        fs::create_dir_all(&state).unwrap();
        ensure_dir(remote.as_ref(), "game-files/game/files").unwrap();
        remote
            .put(
                "game-files/game/files/payload.bin",
                &data,
                &Precondition::Absent,
            )
            .unwrap();
        let open = || {
            AssetSession::open(
                remote.clone(),
                raw.as_bytes(),
                AssetOptions {
                    app_id: "game",
                    allow_legacy: false,
                    source_root: "game-files/game/files",
                    scope: service,
                    destination: &dest,
                    state_dir: &state,
                },
            )
            .unwrap()
        };
        let mut first = open();
        assert_eq!(first.step().unwrap().durable_bytes, 1024 * 1024);
        drop(first);
        let mut resumed = open();
        assert_eq!(resumed.progress().durable_bytes, 1024 * 1024);
        assert_eq!(resumed.step().unwrap().durable_bytes, data.len() as u64);
        assert!(resumed.step().unwrap().complete);
        assert_eq!(fs::read(dest.join("payload.bin")).unwrap(), data);
        drop(resumed);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn manifest_fetch_is_bounded_and_complete() {
    use save_relay::{
        assets::fetch_manifest,
        remote::{MemoryRemote, Precondition, Remote},
    };
    let remote = MemoryRemote::new();
    let raw = manifest(&["data/a"], "game");
    remote
        .put("manifest.json", &raw, &Precondition::Absent)
        .unwrap();
    assert_eq!(fetch_manifest(&remote, "manifest.json").unwrap(), raw);
    assert!(fetch_manifest(&remote, "../manifest.json").is_err());
}
