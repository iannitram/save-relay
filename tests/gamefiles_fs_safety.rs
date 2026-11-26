//! The engine's filesystem and identity safety: crash at every boundary and resume, no
//! replacing publish, protected save folders, ownership of the state folder, and the
//! provider-identity rules.

mod support_gamefiles;

use save_relay::error::Class;
use save_relay::gamefiles::engine::{self, Boundary, Options};
use save_relay::gamefiles::{Progress, State};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use support_gamefiles::*;

const APP: &str = "game-safe";

struct Setup {
    source: Arc<MemorySource>,
    dest: PathBuf,
    state: PathBuf,
    big: Vec<u8>,
    small: Vec<u8>,
    options: Options,
}

fn setup(name: &str) -> Setup {
    let big = body(5, 3500);
    let small = body(6, 200);
    let source = MemorySource::new(APP, &[("big.bin", &big), ("dir/small.bin", &small)]);
    let root = folder(name);
    Setup {
        source,
        dest: root.join("game"),
        state: root.join("state"),
        big,
        small,
        options: Options {
            chunk: 1000,
            retries: 2,
            sleep: Arc::new(|_| {}),
            ..Options::default()
        },
    }
}

impl Setup {
    fn run(&self) -> save_relay::Result<Progress> {
        self.run_options(&self.options)
    }
    fn run_options(&self, options: &Options) -> save_relay::Result<Progress> {
        engine::run(
            &*self.source,
            APP,
            &self.dest,
            &self.state,
            options,
            &AtomicBool::new(false),
            &mut |_| {},
        )
    }
    fn crashing_at(&self, at: Boundary, nth: usize) -> Options {
        let seen = Arc::new(Mutex::new(0usize));
        let mut options = self.options.clone();
        options.crash_at = Some(Arc::new(move |b| {
            if b != at {
                return false;
            }
            let mut n = seen.lock().unwrap();
            *n += 1;
            *n == nth
        }));
        options
    }
    fn assert_installed(&self) {
        assert_eq!(fs::read(self.dest.join("big.bin")).unwrap(), self.big);
        assert_eq!(
            fs::read(self.dest.join("dir/small.bin")).unwrap(),
            self.small
        );
    }
}

fn no_stray_files(dest: &Path) {
    for entry in walk(dest) {
        let name = entry.file_name().unwrap().to_string_lossy().into_owned();
        assert!(!name.ends_with(".part"), "stray {entry:?}");
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(read) = fs::read_dir(dir) {
        for e in read.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn a_crash_at_every_boundary_resumes_to_a_complete_correct_install() {
    for at in [
        Boundary::ChunkSynced,
        Boundary::Checkpointed,
        Boundary::Verified,
        Boundary::CheckpointRemoved,
        Boundary::Published,
        Boundary::BeforeReady,
    ] {
        let s = setup(&format!("crash-{at:?}"));
        let crashed =
            s.run_options(&s.crashing_at(at, if at == Boundary::BeforeReady { 1 } else { 2 }));
        assert!(crashed.is_err(), "{at:?} did not crash");
        let done = s
            .run()
            .unwrap_or_else(|e| panic!("{at:?}: resume failed: {e}"));
        assert_eq!(done.state, State::Complete, "{at:?}");
        s.assert_installed();
        no_stray_files(&s.dest);
    }
}

#[test]
fn a_crash_after_publish_does_not_fetch_the_published_file_again() {
    let s = setup("crash-published");
    assert!(s
        .run_options(&s.crashing_at(Boundary::Published, 1))
        .is_err());
    s.source.clear_log();
    s.run().unwrap();
    s.assert_installed();
    let again = s
        .source
        .fetches()
        .iter()
        .filter(|(p, _, _)| p.ends_with("/files/big.bin") || p.ends_with("/files/dir/small.bin"))
        .map(|(_, _, len)| len)
        .sum::<u64>();
    assert!(
        again <= (s.big.len() + s.small.len()) as u64,
        "fetched {again} bytes"
    );
}

#[test]
fn an_existing_destination_file_is_never_replaced_by_a_publish() {
    let s = setup("no-replace");
    fs::create_dir_all(&s.dest).unwrap();
    fs::write(s.dest.join("big.bin"), b"the player's own file").unwrap();
    let error = s.run().expect_err("must refuse");
    assert_eq!(error.code(), "destination_conflict", "{error}");
    assert_eq!(
        fs::read(s.dest.join("big.bin")).unwrap(),
        b"the player's own file"
    );
}

/// A hook that plants `bytes` at `dir/small.bin` when the second file (`small.bin`, after
/// `big.bin`) has had its checkpoint removed: the last step before the no-replace publish.
fn plant_before_small_publishes(s: &Setup, bytes: Vec<u8>) -> Options {
    let target = s.dest.join("dir").join("small.bin");
    let seen = Arc::new(Mutex::new(0usize));
    let mut options = s.options.clone();
    options.crash_at = Some(Arc::new(move |b| {
        if b == Boundary::CheckpointRemoved {
            let mut n = seen.lock().unwrap();
            *n += 1;
            if *n == 2 {
                fs::write(&target, &bytes).unwrap();
            }
        }
        false
    }));
    options
}

#[test]
fn a_different_file_that_appears_between_hash_and_publish_is_not_replaced() {
    let s = setup("race");
    let target = s.dest.join("dir").join("small.bin");
    let error = s
        .run_options(&plant_before_small_publishes(&s, b"raced in".to_vec()))
        .expect_err("the no-replace publish must refuse");
    assert_eq!(error.code(), "destination_conflict", "{error}");
    assert_eq!(fs::read(&target).unwrap(), b"raced in");
    no_stray_files(&s.dest);
}

#[test]
fn an_identical_file_that_appears_between_hash_and_publish_is_adopted() {
    let s = setup("race-same");
    let done = s
        .run_options(&plant_before_small_publishes(&s, s.small.clone()))
        .expect("identical bytes are adopted");
    assert_eq!(done.state, State::Complete);
    s.assert_installed();
    no_stray_files(&s.dest);
}

#[test]
fn a_protected_save_folder_cannot_be_the_destination_or_hold_it() {
    let s = setup("protected");
    let saves = s.dest.parent().unwrap().join("saves");
    fs::create_dir_all(&saves).unwrap();
    let mut options = s.options.clone();
    options.protected = vec![saves.clone()];
    for dest in [
        saves.clone(),
        saves.join("inside"),
        s.dest.parent().unwrap().to_path_buf(),
    ] {
        let error = engine::run(
            &*s.source,
            APP,
            &dest,
            &s.state,
            &options,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .expect_err("must refuse");
        assert_eq!(error.code(), "destination_conflict", "{dest:?}: {error}");
    }
    assert!(!saves.join("inside").exists());
}

#[test]
fn a_state_folder_that_is_not_ours_is_refused_and_left_alone() {
    let s = setup("foreign-state");
    fs::create_dir_all(&s.state).unwrap();
    fs::write(s.state.join("precious.txt"), b"keep").unwrap();
    let error = s.run().expect_err("must refuse");
    assert_eq!(error.code(), "destination_conflict", "{error}");
    assert_eq!(fs::read(s.state.join("precious.txt")).unwrap(), b"keep");
    let _ = engine::cleanup(&s.state);
    assert_eq!(fs::read(s.state.join("precious.txt")).unwrap(), b"keep");
}

#[test]
fn cleanup_removes_only_what_a_run_made() {
    let s = setup("cleanup");
    s.run().unwrap();
    fs::write(s.state.join("keep-me.txt"), b"mine").unwrap();
    engine::cleanup(&s.state).unwrap();
    assert_eq!(fs::read(s.state.join("keep-me.txt")).unwrap(), b"mine");
    assert!(!s.state.join("session.json").exists());
    s.assert_installed();
}

#[test]
fn an_answer_without_an_identity_cannot_extend_bytes_that_have_one() {
    let s = setup("identity-lost");
    assert!(s
        .run_options(&s.crashing_at(Boundary::Checkpointed, 1))
        .is_err());
    let dropped = Arc::new(AtomicBool::new(true));
    let flag = dropped.clone();
    s.source.set_hook(move |_, _, _, mut answer| {
        if flag.load(Ordering::SeqCst) {
            answer.identity = None;
        }
        Ok(answer)
    });
    let error = s.run().expect_err("must not append");
    assert_eq!(error.class(), Class::Retryable, "{error}");
    dropped.store(false, Ordering::SeqCst);
    s.source.clear_hook();
    let done = s.run().unwrap();
    assert_eq!(done.state, State::Complete);
    s.assert_installed();
}

#[test]
fn a_changed_identity_between_runs_restarts_the_file_and_installs_the_new_bytes() {
    let s = setup("identity-changed");
    assert!(s
        .run_options(&s.crashing_at(Boundary::Checkpointed, 1))
        .is_err());
    s.source
        .revs
        .lock()
        .unwrap()
        .insert(format!("game-files/{APP}/files/big.bin"), "rev-2".into());
    let first = s.run();
    if let Err(error) = &first {
        assert_eq!(error.code(), "remote_changed");
        assert_eq!(error.class(), Class::Retryable);
    }
    let done = s.run().unwrap();
    assert_eq!(done.state, State::Complete);
    s.assert_installed();
}

#[test]
fn a_short_body_that_disagrees_with_its_content_range_is_refused() {
    let s = setup("short");
    s.source.set_hook(|_, _, _, mut answer| {
        let keep = answer.body.len() / 2;
        answer.body.truncate(keep);
        Ok(answer)
    });
    let error = s.run().expect_err("a short body must not be accepted");
    assert_eq!(error.code(), "range_unsupported", "{error}");
    assert!(!s.dest.join("big.bin").exists());
}

/// Shorten every answer to `keep` bytes, with a `Content-Range` that says so: a valid,
/// smaller, contiguous 206.
fn shorten_answers(s: &Setup, keep: usize) {
    s.source.set_hook(move |_, start, _, mut answer| {
        if answer.body.len() > keep {
            answer.body.truncate(keep);
            let total = answer
                .content_range
                .as_deref()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .to_string();
            answer.content_range =
                Some(format!("bytes {start}-{}/{total}", start + keep as u64 - 1));
        }
        Ok(answer)
    });
}

fn checkpoint_durable(state: &Path) -> Vec<u64> {
    let mut found = Vec::new();
    for entry in fs::read_dir(state.join("parts")).unwrap().flatten() {
        if entry.path().extension().is_some_and(|e| e == "json") {
            let json: serde_json::Value =
                serde_json::from_slice(&fs::read(entry.path()).unwrap()).unwrap();
            found.push(json["durable"].as_u64().unwrap());
        }
    }
    found
}

#[test]
fn a_valid_shorter_206_advances_by_its_length_and_the_next_request_resumes_there() {
    let s = setup("short-valid");
    shorten_answers(&s, 300);
    assert!(s
        .run_options(&s.crashing_at(Boundary::Checkpointed, 2))
        .is_err());
    assert_eq!(checkpoint_durable(&s.state), vec![600]);
    s.source.clear_log();
    let done = s.run().unwrap();
    assert_eq!(done.state, State::Complete);
    s.assert_installed();
    let starts: Vec<u64> = s
        .source
        .fetches()
        .iter()
        .filter(|(p, _, _)| p.ends_with("/files/big.bin"))
        .map(|(_, start, _)| *start)
        .collect();
    assert_eq!(starts.first(), Some(&600), "{starts:?}");
    assert!(
        starts.windows(2).all(|w| w[1] == w[0] + 300),
        "each request starts where the last short answer ended: {starts:?}"
    );
}

#[test]
fn bytes_held_without_an_identity_are_not_extended_by_an_answer_that_has_one() {
    let s = setup("identity-appears");
    let none = Arc::new(AtomicBool::new(true));
    let flag = none.clone();
    s.source.set_hook(move |_, _, _, mut answer| {
        if flag.load(Ordering::SeqCst) {
            answer.identity = None;
        }
        Ok(answer)
    });
    assert!(s
        .run_options(&s.crashing_at(Boundary::Checkpointed, 1))
        .is_err());
    none.store(false, Ordering::SeqCst);
    let error = s.run().expect_err("held bytes have no identity to bind to");
    assert_eq!(error.class(), Class::Retryable, "{error}");
    assert_eq!(error.code(), "remote_changed");
    s.source.clear_hook();
    let done = s.run().unwrap();
    assert_eq!(done.state, State::Complete);
    s.assert_installed();
}

#[test]
fn the_two_remote_changed_cases_are_told_apart_by_class() {
    use save_relay::Error;
    let revised = Error::RemoteRevised("x".into());
    let size = Error::RemoteChanged("x".into());
    assert_eq!(
        (revised.code(), size.code()),
        ("remote_changed", "remote_changed")
    );
    assert_eq!(revised.class(), Class::Retryable);
    assert_eq!(size.class(), Class::Fatal);
    let limited = Error::RateLimited {
        message: "dropbox 429".into(),
        retry_after_secs: Some(3),
    };
    assert_eq!(
        (limited.code(), limited.class()),
        ("server_error", Class::Retryable)
    );
}

fn write_session(s: &Setup, service: &str, dest: &str) {
    let json = serde_json::json!({
        "version": 1, "app_id": APP, "service": service, "layout": "standard",
        "dest": dest, "manifest_sha256": "0", "ready": false
    });
    fs::write(s.state.join("session.json"), json.to_string()).unwrap();
}

fn canonical_dest(s: &Setup) -> String {
    fs::canonicalize(&s.dest)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// Stop mid-way, apply `edit`, and count the bytes the next run fetches for `big.bin`.
fn refetched_after(s: &Setup, edit: impl FnOnce()) -> u64 {
    assert!(s
        .run_options(&s.crashing_at(Boundary::Checkpointed, 2))
        .is_err());
    edit();
    s.source.clear_log();
    s.run().unwrap();
    s.assert_installed();
    s.source
        .fetches()
        .iter()
        .filter(|(p, _, _)| p.ends_with("/files/big.bin"))
        .map(|(_, _, len)| *len)
        .sum()
}

#[test]
fn partials_of_another_service_or_destination_are_discarded_and_matching_ones_kept() {
    let s = setup("reset-keep");
    let kept = refetched_after(&s, || {});
    assert!(kept < 3500, "an unchanged session resumes: fetched {kept}");
    let s = setup("reset-service");
    let other_service = refetched_after(&s, || write_session(&s, "other", &canonical_dest(&s)));
    assert_eq!(other_service, 3500, "another service's partials start over");
    let s = setup("reset-dest");
    let other_dest = refetched_after(&s, || write_session(&s, "memory", "/somewhere/else"));
    assert_eq!(
        other_dest, 3500,
        "partials for another destination start over"
    );
}

#[test]
fn a_state_folder_with_only_this_librarys_names_is_adopted_without_a_session() {
    let s = setup("markerless-lock");
    fs::create_dir_all(&s.state).unwrap();
    fs::write(s.state.join("lock"), b"").unwrap();
    let done = s.run().expect("a folder with only `lock` is ours");
    assert_eq!(done.state, State::Complete);
    s.assert_installed();

    let other = setup("markerless-parts");
    fs::create_dir_all(other.state.join("parts")).unwrap();
    fs::write(other.state.join("lock"), b"").unwrap();
    fs::write(
        other
            .state
            .join("parts")
            .join(format!("{}.part", "a".repeat(32))),
        b"x",
    )
    .unwrap();
    engine::cleanup(&other.state).expect("cleanup adopts that shape too");
    assert!(!other.state.join("parts").exists());
}

#[test]
fn a_foreign_file_inside_parts_keeps_the_folder_unowned() {
    let s = setup("markerless-foreign");
    fs::create_dir_all(s.state.join("parts")).unwrap();
    fs::write(s.state.join("parts").join("notes.txt"), b"mine").unwrap();
    let error = s.run().expect_err("not ours");
    assert_eq!(error.code(), "destination_conflict");
    assert_eq!(
        fs::read(s.state.join("parts").join("notes.txt")).unwrap(),
        b"mine"
    );
}

#[test]
fn a_retry_after_from_the_service_lengthens_the_engines_wait() {
    use save_relay::Error;
    let s = setup("retry-after");
    let waits = Arc::new(Mutex::new(Vec::new()));
    let record = waits.clone();
    let mut options = s.options.clone();
    options.sleep = Arc::new(move |d| record.lock().unwrap().push(d));
    let first = Arc::new(AtomicBool::new(true));
    s.source.set_hook(move |_, _, _, answer| {
        if first.swap(false, Ordering::SeqCst) {
            return Err(Error::RateLimited {
                message: "dropbox 429".into(),
                retry_after_secs: Some(9),
            });
        }
        Ok(answer)
    });
    s.run_options(&options).unwrap();
    assert_eq!(
        *waits.lock().unwrap(),
        vec![std::time::Duration::from_secs(9)]
    );
}

#[cfg(unix)]
mod links {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn started(name: &str) -> (Setup, PathBuf) {
        let s = setup(name);
        assert!(s
            .run_options(&s.crashing_at(Boundary::Checkpointed, 1))
            .is_err());
        let outside = s.dest.parent().unwrap().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret"), b"untouched").unwrap();
        (s, outside)
    }

    fn untouched(outside: &Path) {
        assert_eq!(fs::read(outside.join("secret")).unwrap(), b"untouched");
        assert_eq!(
            fs::read_dir(outside).unwrap().count(),
            1,
            "nothing was added outside"
        );
    }

    #[test]
    fn a_parts_folder_that_is_a_link_is_refused() {
        let (s, outside) = started("link-parts");
        fs::remove_dir_all(s.state.join("parts")).unwrap();
        symlink(&outside, s.state.join("parts")).unwrap();
        let error = s.run().expect_err("must not stage through a link");
        assert_eq!(error.code(), "destination_conflict", "{error}");
        untouched(&outside);
    }

    #[test]
    fn a_lock_file_that_is_a_link_is_refused() {
        let (s, outside) = started("link-lock");
        fs::remove_file(s.state.join("lock")).unwrap();
        symlink(outside.join("secret"), s.state.join("lock")).unwrap();
        let error = s.run().expect_err("must not lock through a link");
        assert_eq!(error.code(), "destination_conflict", "{error}");
        untouched(&outside);
    }

    #[test]
    fn a_session_file_that_is_a_link_is_never_written_through() {
        let (s, outside) = started("link-session");
        fs::remove_file(s.state.join("session.json")).unwrap();
        symlink(outside.join("secret"), s.state.join("session.json")).unwrap();
        let _ = s.run();
        untouched(&outside);
    }

    #[test]
    fn a_part_that_is_a_link_is_never_written_through() {
        let (s, outside) = started("link-part");
        let part = fs::read_dir(s.state.join("parts"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "part"))
            .unwrap();
        fs::remove_file(&part).unwrap();
        symlink(outside.join("secret"), &part).unwrap();
        let done = s.run().expect("the file restarts from byte 0");
        assert_eq!(done.state, State::Complete);
        s.assert_installed();
        untouched(&outside);
    }

    #[test]
    fn a_destination_below_a_folder_that_cannot_be_read_is_installed() {
        // The app sandbox case: the folders above the roots can be searched but not read
        // (Android's /data is 0771 system:system). Mode 0311 is the same for the owner.
        let mut s = setup("unreadable-ancestor");
        let outer = s.dest.parent().unwrap().join("outer");
        s.dest = outer.join("game");
        s.state = outer.join("state");
        fs::create_dir_all(&outer).unwrap();
        fs::set_permissions(&outer, fs::Permissions::from_mode(0o311)).unwrap();
        let result = s.run();
        fs::set_permissions(&outer, fs::Permissions::from_mode(0o755)).unwrap();
        let done = result.expect("search permission on ancestors is enough");
        assert_eq!(done.state, State::Complete);
        s.assert_installed();
    }
}

#[test]
fn a_state_folder_holding_only_a_foreign_session_file_is_not_adopted() {
    for name in ["session.json", "session.json.tmp"] {
        let s = setup("foreign-session");
        fs::create_dir_all(&s.state).unwrap();
        let mine = s.state.join(name);
        fs::write(&mine, "my own notes, not save-relay's").unwrap();
        let error = s.run().expect_err("must refuse");
        assert_eq!(error.code(), "destination_conflict", "{name}: {error}");
        let error = engine::cleanup(&s.state).expect_err("cleanup must refuse too");
        assert_eq!(error.code(), "destination_conflict", "{name}: {error}");
        assert_eq!(
            fs::read_to_string(&mine).unwrap(),
            "my own notes, not save-relay's",
            "{name} is untouched"
        );
        assert!(!s.state.join(".save-relay-state").exists(), "no marker");
    }
}

#[test]
fn a_game_folder_may_hold_a_saves_folder_when_no_manifest_path_touches_it() {
    let s = setup("holds-saves");
    let saves = s.dest.join("user");
    fs::create_dir_all(&saves).unwrap();
    fs::write(saves.join("slot1.sav"), b"save").unwrap();
    let mut options = s.options.clone();
    options.protected = vec![saves.clone()];
    s.run_options(&options)
        .expect("the saves folder sits inside the game folder, the manifest keeps clear");
    s.assert_installed();
    assert_eq!(fs::read(saves.join("slot1.sav")).unwrap(), b"save");
    // A plan agrees.
    engine::plan(&*s.source, APP, &s.dest, &s.state, &options).expect("plan");
}

#[test]
fn a_manifest_path_in_or_above_a_saves_folder_inside_the_game_folder_is_refused() {
    // `dir/small.bin` is in the manifest: saves at `dir` (above the file), at `dir/small.bin`
    // (the file itself) and at `DIR` (a case-insensitive file system's same folder).
    for saves_rel in ["dir", "dir/small.bin", "DIR"] {
        let s = setup("manifest-hits-saves");
        let saves = s.dest.join(saves_rel);
        fs::create_dir_all(&saves).unwrap();
        let mut options = s.options.clone();
        options.protected = vec![saves.clone()];
        let error = s.run_options(&options).expect_err("must refuse");
        assert_eq!(error.code(), "destination_conflict", "{saves_rel}: {error}");
        let error = engine::plan(&*s.source, APP, &s.dest, &s.state, &options)
            .expect_err("plan must refuse too");
        assert_eq!(error.code(), "destination_conflict", "{saves_rel}: {error}");
        assert!(
            !s.dest.join("big.bin").exists(),
            "{saves_rel}: nothing written"
        );
    }
}

/// Windows, Apple and Android (shared storage) file systems are case-insensitive: a folder
/// spelled with other letters is still the saves folder. (The same `cfg` list gates the engine's
/// fold; Android is checked by `cargo check --target aarch64-linux-android`, not run here.)
#[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "android"))]
#[test]
fn the_saves_folder_check_ignores_letter_case_where_the_file_system_does() {
    let s = setup("protected-case");
    let saves = s.dest.parent().unwrap().join("Saves");
    fs::create_dir_all(&saves).unwrap();
    let mut options = s.options.clone();
    options.protected = vec![saves];
    let dest = s.dest.parent().unwrap().join("sAVES").join("inside");
    let error = engine::run(
        &*s.source,
        APP,
        &dest,
        &s.state,
        &options,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .expect_err("must refuse");
    assert_eq!(error.code(), "destination_conflict", "{error}");
}
