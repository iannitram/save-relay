//! The download engine over an in-memory source whose answers a test can bend: resume,
//! checkpoints, corrupt and torn parts, verification, the range contract, errors, conflicts.

mod support_gamefiles;

use save_relay::error::Class;
use save_relay::gamefiles::engine::{self, validate, Options};
use save_relay::gamefiles::{Progress, RangeAnswer, State};
use save_relay::Error;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use support_gamefiles::*;

const APP: &str = "game-one";
const CHUNK: u64 = 1000;

struct Setup {
    source: Arc<MemorySource>,
    dest: PathBuf,
    state: PathBuf,
    big: Vec<u8>,
    small: Vec<u8>,
    sleeps: Arc<Mutex<Vec<u64>>>,
    options: Options,
}

fn setup(name: &str) -> Setup {
    let big = body(1, 4500);
    let small = body(2, 300);
    let nested = body(3, 1000);
    let empty: &[u8] = b"";
    let source = MemorySource::new(
        APP,
        &[
            ("big.bin", &big),
            ("dir/small.bin", &small),
            ("dir/deep/exact.bin", &nested),
            ("empty.txt", empty),
        ],
    );
    let root = folder(name);
    let sleeps = Arc::new(Mutex::new(Vec::new()));
    let recorded = sleeps.clone();
    let options = Options {
        chunk: CHUNK,
        retries: 3,
        sleep: Arc::new(move |d| recorded.lock().unwrap().push(d.as_secs())),
    };
    Setup {
        source,
        dest: root.join("game"),
        state: root.join("state"),
        big,
        small,
        sleeps,
        options,
    }
}

impl Setup {
    fn run(&self) -> save_relay::Result<Progress> {
        self.run_with(&AtomicBool::new(false), &mut |_| {})
    }
    fn run_with(
        &self,
        stop: &AtomicBool,
        observe: &mut dyn FnMut(&Progress),
    ) -> save_relay::Result<Progress> {
        engine::run(
            &*self.source,
            APP,
            &self.dest,
            &self.state,
            &self.options,
            stop,
            observe,
        )
    }
    /// Stop the run right after `chunks` chunks have been made durable.
    fn run_until_chunks(&self, chunks: usize) -> Progress {
        let stop = AtomicBool::new(false);
        let mut seen = 0;
        let before = self.source.fetches().len();
        let result = self
            .run_with(&stop, &mut |_| {
                if self.source.fetches().len() - before >= chunks && !stop.load(Ordering::SeqCst) {
                    seen += 1;
                    stop.store(true, Ordering::SeqCst);
                }
            })
            .unwrap();
        assert!(seen > 0);
        result
    }
    fn part_files(&self) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = fs::read_dir(self.state.join("parts"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "part"))
            .collect();
        found.sort();
        found
    }
    fn assert_installed(&self) {
        assert_eq!(fs::read(self.dest.join("big.bin")).unwrap(), self.big);
        assert_eq!(
            fs::read(self.dest.join("dir/small.bin")).unwrap(),
            self.small
        );
        assert_eq!(
            fs::read(self.dest.join("dir/deep/exact.bin")).unwrap(),
            body(3, 1000)
        );
        assert_eq!(fs::read(self.dest.join("empty.txt")).unwrap(), b"");
    }
}

fn part_name(path: &str) -> String {
    let digest = Sha256::digest(path.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}.part", &hex[..32])
}

#[test]
fn a_run_installs_every_file_in_bounded_chunks_and_leaves_no_staging() {
    let s = setup("full");
    let mut seen = Vec::new();
    let done = s
        .run_with(&AtomicBool::new(false), &mut |p| seen.push(p.clone()))
        .unwrap();
    s.assert_installed();
    assert_eq!(done.state, State::Complete);
    assert_eq!((done.files_done, done.files_total), (4, 4));
    assert_eq!((done.bytes_done, done.bytes_total), (5800, 5800));
    assert!(s.part_files().is_empty());
    // Every request is a bounded range, never over the chunk size, never past the file.
    let fetches = s.source.fetches();
    assert_eq!(fetches.iter().map(|f| f.2).sum::<u64>(), 5800);
    assert!(fetches.iter().all(|(_, _, len)| *len <= CHUNK && *len > 0));
    // The empty file needs no request at all.
    assert!(fetches.iter().all(|(p, _, _)| !p.ends_with("empty.txt")));
    // Progress is monotonic and reports the file in flight.
    assert!(seen.windows(2).all(|w| w[0].bytes_done <= w[1].bytes_done));
    assert!(seen.iter().any(|p| p.current.as_deref() == Some("big.bin")));
    assert_eq!(seen.last().unwrap().state, State::Complete);
    // A second run changes nothing and fetches nothing.
    s.source.clear_log();
    assert_eq!(s.run().unwrap().state, State::Complete);
    assert!(s.source.fetches().is_empty());
}

#[test]
fn a_stopped_run_keeps_its_progress_and_the_next_run_continues_from_the_durable_offset() {
    let s = setup("stop");
    let first = s.run_until_chunks(2);
    assert_eq!(first.state, State::Cancelled);
    assert!(
        !s.dest.join("big.bin").exists(),
        "an unfinished file is never published"
    );
    assert_eq!(s.part_files().len(), 1);
    let have = fs::metadata(&s.part_files()[0]).unwrap().len();
    assert!(have > 0 && have < 4500);
    s.source.clear_log();
    let done = s.run().unwrap();
    assert_eq!(done.state, State::Complete);
    s.assert_installed();
    // The first request of the resumed file starts exactly where the stopped run left off.
    let resumed = s
        .source
        .fetches()
        .into_iter()
        .find(|f| f.0.ends_with("big.bin"))
        .unwrap();
    assert_eq!(resumed.1, have);
    // No byte was fetched twice: 5800 in all, and the part's bytes were not asked for again.
    assert_eq!(s.source.bytes_fetched(), 5800 - have);
}

#[test]
fn a_torn_tail_beyond_the_checkpoint_is_cut_off_and_a_missing_part_restarts_the_file() {
    let s = setup("torn");
    s.run_until_chunks(2);
    let part = s.part_files().remove(0);
    let durable = fs::metadata(&part).unwrap().len();
    // Unsynced garbage after the last checkpoint (a crash between write and checkpoint).
    let mut bytes = fs::read(&part).unwrap();
    bytes.extend_from_slice(&[0xAB; 700]);
    fs::write(&part, bytes).unwrap();
    s.source.clear_log();
    s.run().unwrap();
    s.assert_installed();
    let resumed = s
        .source
        .fetches()
        .into_iter()
        .find(|f| f.0.ends_with("big.bin"))
        .unwrap();
    assert_eq!(
        resumed.1, durable,
        "resumes at the durable offset, not the torn length"
    );

    // A checkpoint whose part vanished starts that file over rather than trusting it.
    let s = setup("vanished");
    s.run_until_chunks(2);
    fs::remove_file(&s.part_files()[0]).unwrap();
    s.source.clear_log();
    s.run().unwrap();
    s.assert_installed();
    let restart = s
        .source
        .fetches()
        .into_iter()
        .find(|f| f.0.ends_with("big.bin"))
        .unwrap();
    assert_eq!(restart.1, 0);
}

#[test]
fn a_corrupt_part_is_found_by_the_hash_and_the_file_restarts_once_from_zero() {
    let s = setup("corrupt");
    s.run_until_chunks(3);
    let part = s.part_files().remove(0);
    let mut bytes = fs::read(&part).unwrap();
    assert!(bytes.len() >= 2000);
    bytes[100] ^= 0xFF;
    fs::write(&part, bytes).unwrap();
    s.source.clear_log();
    let done = s.run().unwrap();
    assert_eq!(done.state, State::Complete);
    s.assert_installed();
    let starts: Vec<u64> = s
        .source
        .fetches()
        .into_iter()
        .filter(|f| f.0.ends_with("big.bin"))
        .map(|f| f.1)
        .collect();
    // It resumed at the durable offset, hashed bad, then re-fetched the whole file.
    assert!(starts[0] > 0, "{starts:?}");
    assert_eq!(starts.iter().filter(|s| **s == 0).count(), 1, "{starts:?}");
    assert!(s.part_files().is_empty());
}

#[test]
fn a_hash_mismatch_is_never_installed_and_a_fresh_bad_download_is_integrity_failed() {
    let s = setup("mismatch");
    // The service holds other bytes than the manifest promised, same length.
    let bad = body(9, 4500);
    s.source
        .files
        .lock()
        .unwrap()
        .insert(format!("game-files/{APP}/files/big.bin"), bad);
    let error = s.run().unwrap_err();
    assert!(matches!(error, Error::IntegrityFailed(_)), "{error:?}");
    assert_eq!(error.code(), "integrity_failed");
    assert_eq!(error.class(), Class::Fatal);
    assert!(!s.dest.join("big.bin").exists());
    assert!(
        s.part_files().is_empty(),
        "a bad part is removed, not kept for resuming"
    );
    // A fixed service installs on the next run.
    s.source
        .files
        .lock()
        .unwrap()
        .insert(format!("game-files/{APP}/files/big.bin"), s.big.clone());
    s.run().unwrap();
    s.assert_installed();
}

#[test]
fn a_changed_revision_stops_before_the_chunk_is_appended_and_the_next_run_restarts_the_file() {
    let s = setup("rev");
    s.run_until_chunks(2);
    let part = s.part_files().remove(0);
    let before = fs::metadata(&part).unwrap().len();
    s.source
        .revs
        .lock()
        .unwrap()
        .insert(format!("game-files/{APP}/files/big.bin"), "rev-2".into());
    let error = s.run().unwrap_err();
    assert!(matches!(error, Error::RemoteChanged(_)), "{error:?}");
    assert_eq!(error.code(), "remote_changed");
    assert!(
        before > 0 && !part.exists(),
        "the stale partial is discarded"
    );
    s.source.clear_log();
    s.run().unwrap();
    s.assert_installed();
    assert_eq!(
        s.source
            .fetches()
            .iter()
            .find(|f| f.0.ends_with("big.bin"))
            .unwrap()
            .1,
        0
    );
}

#[test]
fn a_changed_revision_between_two_chunks_of_one_run_is_caught_too() {
    let s = setup("rev-mid");
    let calls = Arc::new(Mutex::new(0));
    let counter = calls.clone();
    s.source.set_hook(move |path, _, _, mut answer| {
        if path.ends_with("big.bin") {
            let mut n = counter.lock().unwrap();
            *n += 1;
            if *n >= 3 {
                answer.identity = Some("rev-other".into());
            }
        }
        Ok(answer)
    });
    assert!(matches!(s.run(), Err(Error::RemoteChanged(_))));
    assert!(!s.dest.join("big.bin").exists());
}

fn answer(status: u16, range: Option<&str>, len: usize) -> RangeAnswer {
    RangeAnswer {
        status,
        content_range: range.map(str::to_string),
        content_encoding: None,
        content_type: None,
        identity: None,
        body: vec![7; len],
    }
}

#[test]
fn the_range_contract_accepts_only_an_exact_single_part_206() {
    // start 100, len 50, file of 1000.
    let check = |a: RangeAnswer| validate(a, 100, 50, 1000);
    assert!(check(answer(206, Some("bytes 100-149/1000"), 50)).is_ok());
    // A shorter contiguous 206 is fine; it advances by what it carries.
    assert!(check(answer(206, Some("bytes 100-119/1000"), 20)).is_ok());
    for (status, range, len) in [
        (200, None, 1000),
        (206, None, 50),
        (206, Some("bytes 0-49/1000"), 50),
        (206, Some("bytes 101-149/1000"), 49),
        (206, Some("bytes 100-150/1000"), 51),
        (206, Some("bytes 100-149/*"), 50),
        (206, Some("bytes 100-149/1000"), 49),
        (206, Some("bytes 100-149/1000"), 51),
        (206, Some("bytes 100-99/1000"), 0),
        (206, Some("items 100-149/1000"), 50),
        (416, Some("bytes */1000"), 0),
        (500, None, 0),
    ] {
        let error = check(answer(status, range, len)).unwrap_err();
        assert!(
            matches!(error, Error::RangeUnsupported(_) | Error::Server(_)),
            "{status} {range:?} {len}: {error:?}"
        );
    }
    // Wrong total is a changed file, not a range problem.
    assert!(matches!(
        check(answer(206, Some("bytes 100-149/999"), 50)),
        Err(Error::RemoteChanged(_))
    ));
    let mut encoded = answer(206, Some("bytes 100-149/1000"), 50);
    encoded.content_encoding = Some("gzip".into());
    assert!(matches!(check(encoded), Err(Error::RangeUnsupported(_))));
    let mut multipart = answer(206, Some("bytes 100-149/1000"), 50);
    multipart.content_type = Some("multipart/byteranges; boundary=x".into());
    assert!(matches!(check(multipart), Err(Error::RangeUnsupported(_))));
}

#[test]
fn a_service_that_ignores_ranges_stops_the_run_without_writing_anything() {
    let s = setup("ignores");
    s.source.set_hook(|_, _, _, mut a| {
        a.status = 200;
        a.content_range = None;
        Ok(a)
    });
    let error = s.run().unwrap_err();
    assert!(matches!(error, Error::RangeUnsupported(_)), "{error:?}");
    assert_eq!(error.code(), "range_unsupported");
    assert!(!s.dest.join("big.bin").exists());
    assert!(s
        .part_files()
        .iter()
        .all(|p| fs::metadata(p).unwrap().len() == 0));
}

#[test]
fn files_not_on_the_service_yet_say_so_and_do_not_retry() {
    let s = setup("missing");
    s.source
        .files
        .lock()
        .unwrap()
        .remove(&format!("game-files/{APP}/files/dir/small.bin"));
    let error = s.run().unwrap_err();
    assert!(matches!(error, Error::NotYetUploaded(_)), "{error:?}");
    assert_eq!(error.class(), Class::NotYetUploaded);
    assert!(
        s.sleeps.lock().unwrap().is_empty(),
        "no waiting for a file that is not there"
    );

    let s = setup("no-manifest");
    s.source
        .files
        .lock()
        .unwrap()
        .remove(&format!("game-files/{APP}/manifest.json"));
    let error = s.run().unwrap_err();
    assert!(matches!(error, Error::NotYetUploaded(_)), "{error:?}");
}

#[test]
fn retryable_failures_wait_and_retry_and_progress_resets_the_count() {
    let s = setup("retry");
    // Two drops, then success; again later in the run: three retries allowed per request.
    *s.source.fail_next.lock().unwrap() = 2;
    s.run().unwrap();
    s.assert_installed();
    assert_eq!(*s.sleeps.lock().unwrap(), vec![1, 2]);

    // Failing twice before every chunk never ends the run: each success starts the count again.
    let s = setup("retry-resets");
    let calls = Arc::new(Mutex::new(0u32));
    let counter = calls.clone();
    s.source.set_hook(move |_, _, _, answer| {
        let mut n = counter.lock().unwrap();
        *n += 1;
        if !n.is_multiple_of(3) {
            return Err(Error::Unreachable("drop".into()));
        }
        Ok(answer)
    });
    s.run().unwrap();
    s.assert_installed();
    assert!(s.sleeps.lock().unwrap().len() > 6);

    // A request that keeps failing ends the run as a retryable error with the state kept.
    let s = setup("retry-exhausted");
    *s.source.fail_next.lock().unwrap() = 99;
    let error = s.run().unwrap_err();
    assert!(matches!(error, Error::Unreachable(_)), "{error:?}");
    assert_eq!(error.class(), Class::Retryable);
    assert_eq!(*s.sleeps.lock().unwrap(), vec![1, 2, 4]);
    *s.source.fail_next.lock().unwrap() = 0;
    s.run().unwrap();
    s.assert_installed();
}

#[test]
fn a_reauth_or_other_service_error_is_not_retried() {
    let s = setup("reauth");
    s.source
        .set_hook(|_, _, _, _| Err(Error::ReauthNeeded("Dropbox".into())));
    let error = s.run().unwrap_err();
    assert_eq!(error.class(), Class::ReauthNeeded);
    assert!(s.sleeps.lock().unwrap().is_empty());
    s.source
        .set_hook(|_, _, _, _| Err(Error::Server("dropbox 400".into())));
    assert_eq!(s.run().unwrap_err().class(), Class::Fatal);
    s.source
        .set_hook(|_, _, _, _| Err(Error::Server("dropbox 503".into())));
    assert_eq!(s.run().unwrap_err().class(), Class::Retryable);
}

#[test]
fn files_already_in_place_are_adopted_and_others_are_never_overwritten() {
    let s = setup("adopt");
    fs::create_dir_all(s.dest.join("dir")).unwrap();
    fs::write(s.dest.join("dir/small.bin"), &s.small).unwrap();
    s.run().unwrap();
    s.assert_installed();
    assert!(
        s.source
            .fetches()
            .iter()
            .all(|f| !f.0.ends_with("small.bin")),
        "a verified file already there is not fetched again"
    );

    // Same size, different bytes: left alone, run stops.
    let s = setup("conflict");
    fs::write(s.dest.join("big.bin"), vec![0u8; 4500]).unwrap_or_else(|_| {
        fs::create_dir_all(&s.dest).unwrap();
        fs::write(s.dest.join("big.bin"), vec![0u8; 4500]).unwrap();
    });
    let error = s.run().unwrap_err();
    assert!(matches!(error, Error::DestinationConflict(_)), "{error:?}");
    assert_eq!(fs::read(s.dest.join("big.bin")).unwrap(), vec![0u8; 4500]);

    // A different size is a conflict as well, with no hashing needed.
    let s = setup("conflict-size");
    fs::create_dir_all(&s.dest).unwrap();
    fs::write(s.dest.join("big.bin"), b"short").unwrap();
    assert!(matches!(s.run(), Err(Error::DestinationConflict(_))));
    assert_eq!(fs::read(s.dest.join("big.bin")).unwrap(), b"short");

    // A folder where a file belongs.
    let s = setup("conflict-dir");
    fs::create_dir_all(s.dest.join("big.bin")).unwrap();
    assert!(matches!(s.run(), Err(Error::DestinationConflict(_))));
}

#[cfg(unix)]
#[test]
fn links_in_the_destination_are_refused_not_followed() {
    let s = setup("links");
    let outside = folder("links-outside");
    fs::create_dir_all(&s.dest).unwrap();
    std::os::unix::fs::symlink(&outside, s.dest.join("dir")).unwrap();
    let error = s.run().unwrap_err();
    assert!(matches!(error, Error::DestinationConflict(_)), "{error:?}");
    assert!(
        fs::read_dir(&outside).unwrap().next().is_none(),
        "nothing written through the link"
    );

    let s = setup("links-file");
    fs::create_dir_all(&s.dest).unwrap();
    let target = outside.join("victim");
    fs::write(&target, b"keep").unwrap();
    std::os::unix::fs::symlink(&target, s.dest.join("big.bin")).unwrap();
    assert!(matches!(s.run(), Err(Error::DestinationConflict(_))));
    assert_eq!(fs::read(&target).unwrap(), b"keep");
}

#[test]
fn one_writer_per_state_folder_and_the_folders_must_be_separate() {
    let s = setup("busy");
    let mut nested = None;
    s.run_with(&AtomicBool::new(false), &mut |p| {
        if p.state == State::Running && nested.is_none() && p.current.is_some() {
            nested = Some(s.run());
        }
    })
    .unwrap();
    let error = nested.expect("observer ran").unwrap_err();
    assert!(matches!(error, Error::Busy(_)), "{error:?}");
    assert_eq!(error.class(), Class::Retryable);

    let s = setup("overlap");
    let inside = s.dest.join("state-inside");
    let error = engine::run(
        &*s.source,
        APP,
        &s.dest,
        &inside,
        &s.options,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(error, Error::DestinationConflict(_)), "{error:?}");
}

#[test]
fn the_state_folder_belongs_to_one_game_and_a_new_manifest_drops_stale_parts() {
    let s = setup("other-game");
    s.run().unwrap();
    let other = MemorySource::new("other", &[("x", b"abc")]);
    let error = engine::run(
        &*other,
        "other",
        &s.dest,
        &s.state,
        &s.options,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(error, Error::DestinationConflict(_)), "{error:?}");

    // A part kept for the old bytes of a file is not reused once the manifest says other bytes.
    let s = setup("new-manifest");
    s.run_until_chunks(2);
    assert_eq!(s.part_files().len(), 1);
    let new_big = body(5, 4500);
    s.source
        .files
        .lock()
        .unwrap()
        .insert(format!("game-files/{APP}/files/big.bin"), new_big.clone());
    let other = [
        ("big.bin", new_big.as_slice()),
        ("dir/small.bin", s.small.as_slice()),
    ];
    s.source.files.lock().unwrap().insert(
        format!("game-files/{APP}/manifest.json"),
        manifest(APP, &other),
    );
    s.source.clear_log();
    s.run().unwrap();
    assert_eq!(fs::read(s.dest.join("big.bin")).unwrap(), new_big);
    assert_eq!(
        s.source
            .fetches()
            .iter()
            .find(|f| f.0.ends_with("big.bin"))
            .unwrap()
            .1,
        0
    );
}

#[test]
fn a_cancel_lands_between_chunks_and_a_checkpoint_survives_for_resume() {
    let s = setup("cancel");
    let stop = AtomicBool::new(false);
    let mut chunks = 0;
    let result = s
        .run_with(&stop, &mut |p| {
            if p.state == State::Running && p.current.as_deref() == Some("big.bin") {
                chunks += 1;
                if chunks == 3 {
                    stop.store(true, Ordering::SeqCst);
                }
            }
        })
        .unwrap();
    assert_eq!(result.state, State::Cancelled);
    assert!(result.bytes_done > 0 && result.bytes_done < result.bytes_total);
    assert!(
        s.state.join("parts").read_dir().unwrap().count() >= 2,
        "part and checkpoint kept"
    );
    // `cleanup` removes only staging.
    engine::cleanup(&s.state).unwrap();
    assert!(!s.state.join("parts").exists());
    assert!(s.dest.exists());
    s.run().unwrap();
    s.assert_installed();
}

#[test]
fn plan_counts_present_partial_and_missing_files_without_writing() {
    let s = setup("plan");
    let plan = engine::plan(&*s.source, APP, &s.dest, &s.state, &s.options).unwrap();
    assert_eq!(
        (
            plan.files_total,
            plan.bytes_total,
            plan.files_missing,
            plan.bytes_to_fetch
        ),
        (4, 5800, 4, 5800)
    );
    assert!(
        !s.dest.exists() && !s.state.exists(),
        "planning writes nothing"
    );

    s.run_until_chunks(2);
    let plan = engine::plan(&*s.source, APP, &s.dest, &s.state, &s.options).unwrap();
    assert_eq!(plan.files_partial, 1);
    assert!(plan.bytes_partial > 0);
    assert_eq!(
        plan.bytes_to_fetch,
        5800 - plan.bytes_partial - plan.bytes_present
    );

    s.run().unwrap();
    let plan = engine::plan(&*s.source, APP, &s.dest, &s.state, &s.options).unwrap();
    assert_eq!(
        (plan.files_present, plan.bytes_to_fetch, plan.conflict_count),
        (4, 0, 0)
    );

    fs::write(s.dest.join("big.bin"), b"x").unwrap();
    let plan = engine::plan(&*s.source, APP, &s.dest, &s.state, &s.options).unwrap();
    assert_eq!(plan.conflicts, vec!["big.bin".to_string()]);
}

#[test]
fn part_files_are_named_by_a_hash_of_the_path_and_never_carry_a_game_path() {
    let s = setup("names");
    s.run_until_chunks(2);
    let parts = s.part_files();
    assert_eq!(
        parts[0].file_name().unwrap().to_string_lossy(),
        part_name("big.bin")
    );
    assert!(!parts[0].to_string_lossy().contains("big"));
}
