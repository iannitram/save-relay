//! The engine through `Dropbox` and the real HTTP transport against a local server that speaks
//! Dropbox-shaped ranged downloads: the exact requests, a service that ignores Range, dropped
//! connections, and a real process kill in the middle of a file followed by a resume.
#![cfg(feature = "http")]

mod support_gamefiles;

use save_relay::gamefiles::engine::{self, Options};
use save_relay::gamefiles::{Progress, State};
use save_relay::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use support_gamefiles::*;

const APP: &str = "ios-one";
const CHUNK: u64 = 64 * 1024;

fn options() -> Options {
    Options {
        chunk: CHUNK,
        retries: 3,
        sleep: Arc::new(|_| {}),
    }
}

fn files() -> (Vec<u8>, Vec<u8>) {
    (body(1, 600_000), body(2, 70_000))
}

fn run(
    server: &RangeServer,
    dest: &Path,
    state: &Path,
    stop: &AtomicBool,
    observe: &mut dyn FnMut(&Progress),
) -> save_relay::Result<Progress> {
    let dropbox = dropbox(server.transport());
    engine::run(&dropbox, APP, dest, state, &options(), stop, observe)
}

#[test]
fn files_arrive_through_bounded_206_requests_and_verify() {
    let (big, small) = files();
    let server = RangeServer::start();
    server.put_game(APP, &[("data/big.bin", &big), ("small.bin", &small)]);
    let root = folder("http-full");
    let (dest, state) = (root.join("game"), root.join("state"));
    let done = run(&server, &dest, &state, &AtomicBool::new(false), &mut |_| {}).unwrap();
    assert_eq!(done.state, State::Complete);
    assert_eq!(fs::read(dest.join("data/big.bin")).unwrap(), big);
    assert_eq!(fs::read(dest.join("small.bin")).unwrap(), small);
    let log = server.log();
    let ranged: Vec<_> = log.iter().filter_map(|(_, r)| r.clone()).collect();
    assert_eq!(
        ranged.len(),
        600_000usize.div_ceil(65536) + 70_000usize.div_ceil(65536)
    );
    assert!(ranged.iter().all(|r| r.starts_with("bytes=")), "{ranged:?}");
    assert_eq!(ranged[0], "bytes=0-65535");
    // The service sent exactly the payload plus the manifest: nothing twice.
    assert_eq!(
        server.bytes_sent() as usize,
        big.len()
            + small.len()
            + manifest(APP, &[("data/big.bin", &big), ("small.bin", &small)]).len()
    );
}

#[test]
fn a_service_that_ignores_range_is_range_unsupported_after_one_bounded_read() {
    let (big, small) = files();
    let server = RangeServer::start();
    server.put_game(APP, &[("big.bin", &big), ("small.bin", &small)]);
    let root = folder("http-ignores");
    let (dest, state) = (root.join("game"), root.join("state"));
    let manifest_requests = 1;
    server.ignore_range(true);
    // The manifest (no Range) still arrives; the first payload request gets a 200 with the
    // whole 600 kB, which the bound cuts off at the chunk size plus one byte.
    let error = run(&server, &dest, &state, &AtomicBool::new(false), &mut |_| {}).unwrap_err();
    assert!(matches!(error, Error::RangeUnsupported(_)), "{error:?}");
    assert_eq!(server.log().len(), manifest_requests + 1, "no retry loop");
    assert!(!dest.join("big.bin").exists());
}

#[test]
fn a_dropped_connection_is_retried_and_a_missing_file_is_not_yet_uploaded() {
    let (big, small) = files();
    let server = RangeServer::start();
    server.put_game(APP, &[("big.bin", &big), ("small.bin", &small)]);
    let root = folder("http-drop");
    let (dest, state) = (root.join("game"), root.join("state"));
    server.drop_next(2);
    run(&server, &dest, &state, &AtomicBool::new(false), &mut |_| {}).unwrap();
    assert_eq!(fs::read(dest.join("big.bin")).unwrap(), big);

    let root = folder("http-missing");
    let (dest, state) = (root.join("game"), root.join("state"));
    // The manifest lists a file the service does not hold: Dropbox answers 409 path/not_found.
    let server2 = RangeServer::start();
    server2.put(
        &format!("game-files/{APP}/manifest.json"),
        manifest(APP, &[("big.bin", &big)]),
    );
    let error = run(
        &server2,
        &dest,
        &state,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(error, Error::NotYetUploaded(_)), "{error:?}");
}

/// A range shorter than the service's error body must still classify the answer by its status.
#[test]
fn error_answers_to_a_tiny_range_keep_their_class() {
    let tiny = body(3, 10);
    let fresh = |name: &str| {
        let server = RangeServer::start();
        server.put_game(APP, &[("tiny.bin", &tiny)]);
        let root = folder(name);
        (server, root.join("game"), root.join("state"))
    };
    // 409 path/not_found: the file is not up yet.
    let root = folder("tiny-missing");
    let server = RangeServer::start();
    server.put(
        &format!("game-files/{APP}/manifest.json"),
        manifest(APP, &[("tiny.bin", &tiny)]),
    );
    let error = run(
        &server,
        &root.join("game"),
        &root.join("state"),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(error, Error::NotYetUploaded(_)), "{error:?}");
    assert_eq!(error.class(), save_relay::error::Class::NotYetUploaded);

    // 401 once: the token is refreshed and the file arrives.
    let (server, dest, state) = fresh("tiny-401");
    server.fail_with(&[401]);
    run(&server, &dest, &state, &AtomicBool::new(false), &mut |_| {}).unwrap();
    assert_eq!(fs::read(dest.join("tiny.bin")).unwrap(), tiny);

    // 503 once: retried, then the file arrives.
    let (server, dest, state) = fresh("tiny-503");
    server.fail_with(&[503]);
    run(&server, &dest, &state, &AtomicBool::new(false), &mut |_| {}).unwrap();
    assert_eq!(fs::read(dest.join("tiny.bin")).unwrap(), tiny);
}

/// The child half of the kill test: download, and die the hard way (no destructors, no
/// unwinding) as soon as a few chunks of the big file are durable.
#[test]
fn kill_child() {
    let Ok(port) = std::env::var("SR_GF_KILL_PORT") else {
        return; // an ordinary test run: nothing to do
    };
    let root = PathBuf::from(std::env::var("SR_GF_KILL_ROOT").unwrap());
    let server = RangeServer::connect(port.parse().unwrap());
    let after: usize = std::env::var("SR_GF_KILL_AFTER").unwrap().parse().unwrap();
    let mut chunks = 0;
    let _ = run(
        &server,
        &root.join("game"),
        &root.join("state"),
        &AtomicBool::new(false),
        &mut |p| {
            if p.current.as_deref() == Some("big.bin") && p.bytes_done > 0 {
                chunks += 1;
                if chunks > after {
                    std::process::abort();
                }
            }
        },
    );
    std::process::exit(0);
}

#[test]
fn a_killed_process_resumes_from_the_last_durable_chunk_with_no_wasted_download() {
    let (big, small) = files();
    let server = RangeServer::start();
    server.put_game(APP, &[("big.bin", &big), ("small.bin", &small)]);
    let root = folder("http-kill");
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "kill_child", "--nocapture", "--test-threads=1"])
        .env("SR_GF_KILL_PORT", server.addr.port().to_string())
        .env("SR_GF_KILL_ROOT", &root)
        .env("SR_GF_KILL_AFTER", "3")
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "the child must have been killed mid-download"
    );
    let (dest, state) = (root.join("game"), root.join("state"));
    assert!(
        !dest.join("big.bin").exists(),
        "an unfinished file is never published"
    );
    let part = fs::read_dir(state.join("parts"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "part"))
        .expect("a part file survived the kill");
    let have = fs::metadata(&part).unwrap().len();
    assert!((CHUNK * 3..600_000).contains(&have), "{have}");

    server.clear_log();
    let sent_before = server.bytes_sent();
    let done = run(&server, &dest, &state, &AtomicBool::new(false), &mut |_| {}).unwrap();
    assert_eq!(done.state, State::Complete);
    assert_eq!(fs::read(dest.join("big.bin")).unwrap(), big);
    assert_eq!(fs::read(dest.join("small.bin")).unwrap(), small);
    // The first request after the kill continues at a chunk boundary at or after what was
    // durable: at most one chunk (the one in flight at the kill) was wasted, never the 3 done.
    let first_big = server
        .log()
        .into_iter()
        .find(|(p, r)| p.ends_with("big.bin") && r.is_some())
        .unwrap();
    let start: u64 = first_big.1.unwrap()["bytes=".len()..]
        .split('-')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        start >= have.min(CHUNK * 3) && start.is_multiple_of(CHUNK),
        "start {start}, have {have}"
    );
    assert!(
        start >= CHUNK * 3,
        "resumed at {start}, not from the beginning"
    );
    // Total payload bytes served after the kill: only what was missing.
    let resent = server.bytes_sent() - sent_before;
    let missing = (big.len() as u64 - start) + small.len() as u64;
    assert!(
        resent <= missing + manifest(APP, &[("big.bin", &big), ("small.bin", &small)]).len() as u64
    );
}
