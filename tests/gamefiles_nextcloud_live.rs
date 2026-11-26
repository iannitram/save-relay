//! Game files (#309) through `WebDav` against a real Nextcloud: the manifest read, bounded
//! `Range` GETs answered 206, the strong ETag as identity, and `If-Match` refusing another
//! version (412, `remote_changed`). Ignored by default; the `integration` job's Linux leg in
//! `.github/workflows/check.yml` runs it against its Nextcloud container (rclone, the Windows and
//! macOS server, ignores `If-Match`, so it does not run there):
//!
//! ```text
//! SR_WEBDAV_SERVER=http://127.0.0.1:8080 SR_WEBDAV_USER=admin SR_WEBDAV_PASSWORD=<app password> \
//!   cargo test --locked --test gamefiles_nextcloud_live -- --ignored --test-threads=1
//! ```
//!
//! With any variable unset it prints that it was skipped and checks nothing: a pass then is
//! not evidence of anything. It works in `game-files/sr-live-<pid>-<stamp>/` and deletes that
//! folder at the end. The password is never printed.
#![cfg(feature = "http")]

mod support_gamefiles;

use save_relay::creds::Secret;
use save_relay::gamefiles::engine::{self, Options};
use save_relay::gamefiles::{remote_file_path, remote_manifest_path, GameSource, State};
use save_relay::http::{Request, Response, Transport, UreqTransport};
use save_relay::remote::{Precondition, Remote};
use save_relay::webdav::{files_url, WebDav};
use save_relay::Error;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use support_gamefiles::{body, folder, manifest};

const CHUNK: u64 = 64 * 1024;

/// The real transport, logging (Range header, status) of every GET.
struct Logged {
    real: UreqTransport,
    log: Mutex<Vec<(Option<String>, u16)>>,
}

impl Transport for Logged {
    fn send(&self, request: &Request) -> save_relay::Result<Response> {
        let response = self.real.send(request)?;
        if request.method == "GET" {
            self.log.lock().unwrap().push((
                request.header_value("Range").map(str::to_string),
                response.status,
            ));
        }
        Ok(response)
    }
}

fn vars() -> Option<(String, String, String)> {
    let get = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    Some((
        get("SR_WEBDAV_SERVER")?,
        get("SR_WEBDAV_USER")?,
        get("SR_WEBDAV_PASSWORD")?,
    ))
}

#[test]
#[ignore = "needs a Nextcloud server: see the file comment"]
fn nextcloud_serves_game_files_in_bounded_ranges_with_if_match() {
    let Some((server, user, password)) = vars() else {
        eprintln!(
            "SKIPPED: SR_WEBDAV_SERVER, SR_WEBDAV_USER or SR_WEBDAV_PASSWORD is unset; nothing was checked"
        );
        return;
    };
    let transport = Arc::new(Logged {
        real: UreqTransport::new(),
        log: Mutex::new(Vec::new()),
    });
    let dav = WebDav::new(
        &files_url(&server, &user).unwrap(),
        &user,
        Secret::new(&password),
        transport.clone(),
    );
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let app = format!("sr-live-{}-{stamp}", std::process::id());
    let big = body(9, 300_000);
    let small = body(8, 1_000);
    let files = [
        ("data/big.bin", big.as_slice()),
        ("small.bin", small.as_slice()),
    ];

    let app_dir = format!("game-files/{app}");
    dav.mkcol("game-files").unwrap();
    for dir in [
        app_dir.clone(),
        format!("{app_dir}/files"),
        format!("{app_dir}/files/data"),
    ] {
        dav.mkcol(&dir).unwrap();
    }
    dav.put(
        &remote_manifest_path(&app),
        &manifest(&app, &files),
        &Precondition::Any,
    )
    .unwrap();
    for (rel, bytes) in files {
        dav.put(&remote_file_path(&app, rel), bytes, &Precondition::Any)
            .unwrap();
    }

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let root = folder(&format!("nc-live-{app}"));
        let options = Options {
            chunk: CHUNK,
            retries: 2,
            sleep: Arc::new(|_| {}),
            ..Options::default()
        };
        let done = engine::run(
            &dav,
            &app,
            &root.join("game"),
            &root.join("state"),
            &options,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(done.state, State::Complete);
        assert_eq!(std::fs::read(root.join("game/data/big.bin")).unwrap(), big);
        let log = transport.log.lock().unwrap().clone();
        let ranged: Vec<_> = log.iter().filter(|(range, _)| range.is_some()).collect();
        assert_eq!(ranged.len(), 300_000usize.div_ceil(CHUNK as usize) + 1);
        assert!(ranged.iter().all(|(_, status)| *status == 206), "{log:?}");
        println!("{} ranged GETs, all 206", ranged.len());

        // The strong ETag is the identity, and another one is refused by the server.
        let path = remote_file_path(&app, "data/big.bin");
        let answer = dav.fetch(&path, 0, 10).unwrap();
        let etag = answer
            .identity
            .clone()
            .expect("Nextcloud sends a strong ETag");
        assert!(etag.starts_with('"'), "{etag}");
        assert!(dav.fetch_matching(&path, 10, 10, Some(&etag)).is_ok());
        let error = dav
            .fetch_matching(&path, 10, 10, Some("\"not-the-etag\""))
            .unwrap_err();
        assert!(matches!(error, Error::RemoteRevised(_)), "{error:?}");
        println!("If-Match with another ETag: {}", error.code());
    }));
    let _ = dav.delete(&app_dir);
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
