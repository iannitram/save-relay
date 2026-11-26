//! The game-files C functions driven from the host side: sign in to Dropbox through the app's
//! browser session, then plan, download (with a cancel and a resume), poll progress and clean
//! up, over a local server that serves Dropbox-shaped ranged downloads.
#![cfg(all(feature = "ffi", feature = "http"))]

mod support_gamefiles;

use save_relay::clients::ClientSet;
use save_relay::fake::SIGN_IN_CODE;
use save_relay::ffi::*;
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::{Arc, Condvar, Mutex};
use support_gamefiles::*;

/// The C functions keep process-wide state (host, download slot, progress): one test at a time.
static SERIAL: Mutex<()> = Mutex::new(());

static SECRETS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

unsafe extern "C" fn cred_get(
    _: *mut c_void,
    account: *const c_char,
    buf: *mut u8,
    cap: usize,
    len: *mut usize,
) -> c_int {
    let account = CStr::from_ptr(account).to_str().unwrap();
    match SECRETS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .get(account)
    {
        Some(secret) if secret.len() <= cap => {
            std::ptr::copy_nonoverlapping(secret.as_ptr(), buf, secret.len());
            *len = secret.len();
            0
        }
        Some(_) => 2,
        None => 1,
    }
}

unsafe extern "C" fn cred_set(
    _: *mut c_void,
    account: *const c_char,
    secret: *const c_char,
) -> c_int {
    SECRETS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(
            CStr::from_ptr(account).to_str().unwrap().into(),
            CStr::from_ptr(secret).to_str().unwrap().into(),
        );
    0
}

unsafe extern "C" fn cred_delete(_: *mut c_void, account: *const c_char) -> c_int {
    SECRETS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .remove(CStr::from_ptr(account).to_str().unwrap());
    0
}

/// The test's transport answers everything; the app's own callback is never asked.
unsafe extern "C" fn http(
    _: *mut c_void,
    _: *const SaveRelayHttpRequest,
    _: *mut SaveRelayHttpSink,
) -> c_int {
    9
}

fn c(text: &str) -> CString {
    CString::new(text).unwrap()
}

fn envelope(pointer: *mut c_char) -> Value {
    assert!(!pointer.is_null());
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .unwrap()
        .to_string();
    unsafe { save_relay_free(pointer) };
    serde_json::from_str(&text).unwrap()
}

#[test]
fn plan_download_cancel_resume_progress_and_cleanup_through_the_c_functions() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let root = folder("ffi");
    let (home, data, config) = (root.join("h"), root.join("d"), root.join("c"));
    for dir in [&home, &data, &config] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let (dest, state) = (root.join("game"), root.join("state"));
    let big = body(4, 700_000);
    let small = body(5, 1234);
    let server = RangeServer::start();
    server.put_game("ios-two", &[("big.bin", &big), ("sub/small.bin", &small)]);

    let paths = [&home, &data, &config].map(|p| c(p.to_str().unwrap()));
    let host = SaveRelayHost {
        user_data: std::ptr::null_mut(),
        home: paths[0].as_ptr(),
        data: paths[1].as_ptr(),
        config: paths[2].as_ptr(),
        documents: std::ptr::null(),
        cred_get: Some(cred_get),
        cred_set: Some(cred_set),
        cred_delete: Some(cred_delete),
        http: Some(http),
    };
    let init: Value = serde_json::from_str(&unsafe {
        init_with_net(&host, None, Some(server.transport()), ClientSet::shipped())
    })
    .unwrap();
    assert_eq!(init["ok"], true, "{init}");

    let app = c("ios-two");
    let (dest_c, state_c) = (c(dest.to_str().unwrap()), c(state.to_str().unwrap()));

    // Before sign-in there is nothing to download with.
    let early = envelope(unsafe {
        save_relay_game_files_plan(app.as_ptr(), dest_c.as_ptr(), state_c.as_ptr())
    });
    assert_eq!(early["ok"], false);
    assert_eq!(early["error"]["code"], "not_set_up", "{early}");
    assert_eq!(early["error"]["class"], "reauth_needed");

    let started = envelope(unsafe {
        save_relay_sign_in_start(
            c("dropbox").as_ptr(),
            c("saverelay://oauth").as_ptr(),
            c("phone").as_ptr(),
            std::ptr::null(),
        )
    });
    let url = started["data"]["url"].as_str().unwrap();
    let sign_state = url
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap();
    let redirect = format!("saverelay://oauth?code={SIGN_IN_CODE}&state={sign_state}");
    let finished = envelope(unsafe { save_relay_sign_in_finish(c(&redirect).as_ptr()) });
    assert_eq!(finished["ok"], true, "{finished}");

    // Plan: nothing on disk yet.
    let plan = envelope(unsafe {
        save_relay_game_files_plan(app.as_ptr(), dest_c.as_ptr(), state_c.as_ptr())
    });
    assert_eq!(plan["ok"], true, "{plan}");
    assert_eq!(plan["command"], "game-files-plan");
    assert_eq!(plan["data"]["files_total"], 2);
    assert_eq!(plan["data"]["bytes_to_fetch"], 701_234);

    // Progress before any download is null (cleanup forgets an earlier test's download).
    assert_eq!(
        envelope(unsafe { save_relay_game_files_cleanup(state_c.as_ptr()) })["ok"],
        true
    );
    let idle = envelope(save_relay_game_files_progress());
    assert!(idle["data"].is_null(), "{idle}");

    // Cancel when nothing runs says so.
    assert_eq!(
        envelope(save_relay_game_files_cancel())["data"]["running"],
        false
    );

    // Download on a thread; cancel from here after it makes some progress.
    let (a, d, s) = (app.clone(), dest_c.clone(), state_c.clone());
    let worker = std::thread::spawn(move || {
        envelope(unsafe { save_relay_game_files_download(a.as_ptr(), d.as_ptr(), s.as_ptr()) })
    });
    let mut cancelled = false;
    for _ in 0..4000 {
        let progress = envelope(save_relay_game_files_progress());
        if progress["data"]["bytes_done"].as_u64().unwrap_or(0) > 0 && !cancelled {
            let reply = envelope(save_relay_game_files_cancel());
            cancelled = reply["data"]["running"] == true;
        }
        if worker.is_finished() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let first = worker.join().unwrap();
    // On a fast machine the whole thing can finish before the cancel lands; both are valid ends.
    assert_eq!(first["ok"], true, "{first}");
    let state_of_first = first["data"]["state"].as_str().unwrap().to_string();
    assert!(
        ["cancelled", "complete"].contains(&state_of_first.as_str()),
        "{first}"
    );

    // Run again to completion (a no-op when the first already finished).
    let second = envelope(unsafe {
        save_relay_game_files_download(app.as_ptr(), dest_c.as_ptr(), state_c.as_ptr())
    });
    assert_eq!(second["ok"], true, "{second}");
    assert_eq!(second["command"], "game-files-download");
    assert_eq!(second["data"]["state"], "complete");
    assert_eq!(second["data"]["files_done"], 2);
    assert_eq!(std::fs::read(dest.join("big.bin")).unwrap(), big);
    assert_eq!(std::fs::read(dest.join("sub/small.bin")).unwrap(), small);
    let last = envelope(save_relay_game_files_progress());
    assert_eq!(last["data"]["state"], "complete");
    assert_eq!(last["data"]["bytes_done"], 701_234);

    // The sign-in's secrets never appear in any game-files answer.
    assert!(!second.to_string().contains("refresh"));

    // Cleanup removes staging only.
    let cleaned = envelope(unsafe { save_relay_game_files_cleanup(state_c.as_ptr()) });
    assert_eq!(cleaned["ok"], true, "{cleaned}");
    assert!(!state.join("parts").exists());
    assert!(dest.join("big.bin").exists());

    // Errors are envelopes with a class: a missing game is "not yet uploaded".
    let ghost = envelope(unsafe {
        save_relay_game_files_plan(c("ghost").as_ptr(), dest_c.as_ptr(), state_c.as_ptr())
    });
    assert_eq!(ghost["error"]["code"], "not_yet_uploaded", "{ghost}");
    assert_eq!(ghost["error"]["class"], "not_yet_uploaded");

    // Null and invalid arguments are error envelopes, never crashes.
    let null = envelope(unsafe {
        save_relay_game_files_download(std::ptr::null(), dest_c.as_ptr(), state_c.as_ptr())
    });
    assert_eq!(null["error"]["code"], "usage");
    let bad_id = envelope(unsafe {
        save_relay_game_files_plan(c("Bad/Id").as_ptr(), dest_c.as_ptr(), state_c.as_ptr())
    });
    assert_eq!(bad_id["error"]["code"], "usage", "{bad_id}");

    // The two #324 functions still answer, as wrappers over the same engine: the files sit
    // directly under game-files/<game_id>/, and the checkpoints live inside the game folder.
    let payload = body(6, 150_000);
    server.put("game-files/compat-game/data/p.bin", payload.clone());
    let compat_dest = root.join("compat");
    let compat_dest_c = c(compat_dest.to_str().unwrap());
    let manifest = c(&format!(
        r#"{{"game_id":"compat-game","files":[{{"path":"data/p.bin","size":{},"sha256":"{}"}}]}}"#,
        payload.len(),
        sha(&payload)
    ));
    let before = envelope(unsafe {
        save_relay_game_files_status(manifest.as_ptr(), compat_dest_c.as_ptr())
    });
    assert_eq!(before["command"], "game-files-status", "{before}");
    assert_eq!(before["data"]["completed_files"], 0);
    assert_eq!(before["data"]["complete"], false);
    let got = envelope(unsafe {
        save_relay_download_game_files(manifest.as_ptr(), compat_dest_c.as_ptr())
    });
    assert_eq!(got["ok"], true, "{got}");
    assert_eq!(got["command"], "game-files");
    assert_eq!(got["data"]["downloaded_files"], 1);
    assert_eq!(got["data"]["bytes_downloaded"], 150_000);
    assert_eq!(
        std::fs::read(compat_dest.join("data/p.bin")).unwrap(),
        payload
    );
    assert!(compat_dest.join(".save-relay-game-files").is_dir());
    let after = envelope(unsafe {
        save_relay_game_files_status(manifest.as_ptr(), compat_dest_c.as_ptr())
    });
    assert_eq!(after["data"]["complete"], true, "{after}");
    // A file that does not match is a conflict, never replaced.
    std::fs::write(compat_dest.join("data/p.bin"), b"mine").unwrap();
    let clash = envelope(unsafe {
        save_relay_download_game_files(manifest.as_ptr(), compat_dest_c.as_ptr())
    });
    assert_eq!(clash["error"]["code"], "destination_conflict", "{clash}");
    assert_eq!(
        std::fs::read(compat_dest.join("data/p.bin")).unwrap(),
        b"mine"
    );
}

// ---------------------------------------------------------------------------------------------
// Control: cancel mid-file, one download at a time, progress per run, init while running.
// ---------------------------------------------------------------------------------------------

use save_relay::http::{Request, Response, Transport};

const CHUNK: u64 = 200_000;

#[derive(Default)]
struct GateState {
    armed: bool,
    /// Hold the next request of any kind (the manifest's, before any file chunk).
    armed_first: bool,
    /// Panic in the next request instead of answering it.
    panic_next: bool,
    reached: u32,
    released: bool,
}

/// Holds the first download request that starts at or after `CHUNK` (a mid-file chunk) of an
/// armed gate until the test releases it.
struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

impl Gate {
    fn new() -> Arc<Gate> {
        Arc::new(Gate {
            state: Mutex::new(GateState::default()),
            changed: Condvar::new(),
        })
    }
    fn arm(&self) {
        *self.state.lock().unwrap() = GateState {
            armed: true,
            ..GateState::default()
        };
    }
    fn arm_first_request(&self) {
        *self.state.lock().unwrap() = GateState {
            armed_first: true,
            ..GateState::default()
        };
    }
    fn arm_panic(&self) {
        *self.state.lock().unwrap() = GateState {
            panic_next: true,
            ..GateState::default()
        };
    }
    fn wait_reached(&self) {
        let mut state = self.state.lock().unwrap();
        while state.reached == 0 {
            let (next, timeout) = self
                .changed
                .wait_timeout(state, std::time::Duration::from_secs(30))
                .unwrap();
            assert!(!timeout.timed_out(), "the download never reached the gate");
            state = next;
        }
    }
    fn release(&self) {
        self.state.lock().unwrap().released = true;
        self.changed.notify_all();
    }
}

struct GatedTransport {
    inner: Arc<LocalTransport>,
    gate: Arc<Gate>,
}

impl Transport for GatedTransport {
    fn send(&self, request: &Request) -> save_relay::Result<Response> {
        {
            let mut state = self.gate.state.lock().unwrap();
            if state.panic_next {
                state.panic_next = false;
                drop(state);
                panic!("test transport panic");
            }
            if state.armed_first {
                state.armed_first = false;
                state.reached += 1;
                self.gate.changed.notify_all();
                while !state.released {
                    state = self.gate.changed.wait(state).unwrap();
                }
            }
        }
        let start = request
            .header_value("range")
            .and_then(|r| r.strip_prefix("bytes="))
            .and_then(|r| r.split('-').next())
            .and_then(|a| a.parse::<u64>().ok());
        if start.is_some_and(|a| a >= CHUNK) {
            let mut state = self.gate.state.lock().unwrap();
            if state.armed {
                state.armed = false;
                state.reached += 1;
                self.gate.changed.notify_all();
                while !state.released {
                    state = self.gate.changed.wait(state).unwrap();
                }
            }
        }
        self.inner.send(request)
    }
}

struct Rig {
    root: std::path::PathBuf,
    server: RangeServer,
    gate: Arc<Gate>,
    big: Vec<u8>,
    small: Vec<u8>,
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        set_game_files_chunk_for_tests(0);
    }
}

fn native_host(dir: &CString) -> SaveRelayHost {
    SaveRelayHost {
        user_data: std::ptr::null_mut(),
        home: dir.as_ptr(),
        data: dir.as_ptr(),
        config: dir.as_ptr(),
        documents: std::ptr::null(),
        cred_get: Some(cred_get),
        cred_set: Some(cred_set),
        cred_delete: Some(cred_delete),
        http: Some(http),
    }
}

/// Init with a gated local transport, sign in to Dropbox, and publish the game `ios-two`
/// (a 700 kB file that spans four 200 kB chunks, and a small one) and `ios-three`.
fn rig(name: &str) -> Rig {
    let serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    set_game_files_chunk_for_tests(CHUNK);
    let root = folder(name);
    let (home, data, config) = (root.join("h"), root.join("d"), root.join("c"));
    for dir in [&home, &data, &config] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let big = body(4, 700_000);
    let small = body(5, 1234);
    let server = RangeServer::start();
    server.put_game("ios-two", &[("big.bin", &big), ("sub/small.bin", &small)]);
    server.put_game("ios-three", &[("one.bin", &small)]);
    let gate = Gate::new();
    let transport = Arc::new(GatedTransport {
        inner: server.transport(),
        gate: gate.clone(),
    });
    let paths = [&home, &data, &config].map(|p| c(p.to_str().unwrap()));
    let host = SaveRelayHost {
        home: paths[0].as_ptr(),
        data: paths[1].as_ptr(),
        config: paths[2].as_ptr(),
        ..native_host(&paths[0])
    };
    let init: Value = serde_json::from_str(&unsafe {
        init_with_net(&host, None, Some(transport), ClientSet::shipped())
    })
    .unwrap();
    assert_eq!(init["ok"], true, "{init}");
    let started = envelope(unsafe {
        save_relay_sign_in_start(
            c("dropbox").as_ptr(),
            c("saverelay://oauth").as_ptr(),
            c("phone").as_ptr(),
            std::ptr::null(),
        )
    });
    let url = started["data"]["url"].as_str().unwrap();
    let sign_state = url
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap();
    let redirect = format!("saverelay://oauth?code={SIGN_IN_CODE}&state={sign_state}");
    let finished = envelope(unsafe { save_relay_sign_in_finish(c(&redirect).as_ptr()) });
    assert_eq!(finished["ok"], true, "{finished}");
    Rig {
        root,
        server,
        gate,
        big,
        small,
        _serial: serial,
    }
}

impl Rig {
    /// `ios-two` uses `game` and `state`; any other app has folders of its own (a state folder
    /// belongs to one game).
    fn folders(&self, app: &str) -> (CString, CString) {
        let (game, state) = match app {
            "ios-two" => ("game".to_string(), "state".to_string()),
            other => (format!("game-{other}"), format!("state-{other}")),
        };
        (
            c(self.root.join(game).to_str().unwrap()),
            c(self.root.join(state).to_str().unwrap()),
        )
    }
    fn download_on_thread(&self, app: &str) -> std::thread::JoinHandle<Value> {
        let (d, s) = self.folders(app);
        let a = c(app);
        std::thread::spawn(move || {
            envelope(unsafe { save_relay_game_files_download(a.as_ptr(), d.as_ptr(), s.as_ptr()) })
        })
    }
    fn download(&self, app: &str) -> Value {
        let (d, s) = self.folders(app);
        let a = c(app);
        envelope(unsafe { save_relay_game_files_download(a.as_ptr(), d.as_ptr(), s.as_ptr()) })
    }
}

#[test]
fn ffi_cancel_mid_file_then_download_resumes_from_the_part() {
    let rig = rig("ffi-cancel-mid-file");
    rig.gate.arm();
    let worker = rig.download_on_thread("ios-two");
    // The download is now inside big.bin: chunk one is durable, chunk two is in flight.
    rig.gate.wait_reached();
    let reply = envelope(save_relay_game_files_cancel());
    assert_eq!(reply["data"]["running"], true, "{reply}");
    rig.gate.release();
    let first = worker.join().unwrap();
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["data"]["state"], "cancelled", "{first}");
    assert_eq!(first["data"]["files_done"], 0, "{first}");

    // A real partial exists: strictly between nothing and the whole file.
    let part_lens: Vec<u64> = std::fs::read_dir(rig.root.join("state/parts"))
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().extension().is_some_and(|x| x == "part"))
        .map(|e| e.metadata().unwrap().len())
        .collect();
    assert_eq!(part_lens.len(), 1, "{part_lens:?}");
    assert!(
        part_lens[0] >= CHUNK && part_lens[0] < rig.big.len() as u64,
        "{part_lens:?}"
    );
    assert!(!rig.root.join("game/big.bin").exists());
    let after = envelope(save_relay_game_files_progress());
    assert_eq!(after["data"]["state"], "cancelled", "{after}");
    assert!(after["data"]["bytes_done"].as_u64().unwrap() >= CHUNK);

    // The second download continues from the durable offset, not from byte 0.
    rig.server.clear_log();
    let second = rig.download("ios-two");
    assert_eq!(second["data"]["state"], "complete", "{second}");
    let starts: Vec<u64> = rig
        .server
        .log()
        .iter()
        .filter(|(path, _)| path.ends_with("big.bin"))
        .filter_map(|(_, range)| {
            range
                .as_deref()?
                .strip_prefix("bytes=")?
                .split('-')
                .next()?
                .parse()
                .ok()
        })
        .collect();
    assert!(!starts.is_empty(), "{starts:?}");
    assert!(starts[0] >= CHUNK, "resumed from {starts:?}");
    assert_eq!(
        std::fs::read(rig.root.join("game/big.bin")).unwrap(),
        rig.big
    );
    assert_eq!(
        std::fs::read(rig.root.join("game/sub/small.bin")).unwrap(),
        rig.small
    );
}

#[test]
fn concurrent_download_calls_never_drop_or_inherit_a_cancel() {
    let rig = rig("ffi-concurrent");
    // A cancel with nothing running is answered as such and does not linger for the next run.
    assert_eq!(
        envelope(save_relay_game_files_cancel())["data"]["running"],
        false
    );

    rig.gate.arm();
    let winner = rig.download_on_thread("ios-two");
    rig.gate.wait_reached();
    // Five more callers race for the slot while the winner is mid-file; all are refused and
    // none of them may touch the winner's stop flag.
    let barrier = Arc::new(std::sync::Barrier::new(5));
    let losers: Vec<_> = (0..5)
        .map(|_| {
            let (d, s) = rig.folders("ios-two");
            let (a, barrier) = (c("ios-two"), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                envelope(unsafe {
                    save_relay_game_files_download(a.as_ptr(), d.as_ptr(), s.as_ptr())
                })
            })
        })
        .collect();
    for loser in losers {
        let refused = loser.join().unwrap();
        assert_eq!(refused["error"]["code"], "busy", "{refused}");
        assert_eq!(refused["error"]["class"], "retryable");
    }
    // Cleanup and a second init are refused while it runs; the host it uses stays in place.
    let (_, state_two) = rig.folders("ios-two");
    let cleanup = envelope(unsafe { save_relay_game_files_cleanup(state_two.as_ptr()) });
    assert_eq!(cleanup["error"]["code"], "busy", "{cleanup}");
    let other = rig.root.join("again");
    std::fs::create_dir_all(&other).unwrap();
    let other_c = c(other.to_str().unwrap());
    let host = native_host(&other_c);
    let reinit: Value =
        serde_json::from_str(&unsafe { init_with_net(&host, None, None, ClientSet::shipped()) })
            .unwrap();
    assert_eq!(reinit["ok"], false, "{reinit}");
    assert_eq!(reinit["error"]["code"], "busy", "{reinit}");

    // The cancel reaches the winner, which is the only run there is.
    let reply = envelope(save_relay_game_files_cancel());
    assert_eq!(reply["data"]["running"], true, "{reply}");
    rig.gate.release();
    let first = winner.join().unwrap();
    assert_eq!(first["data"]["state"], "cancelled", "{first}");

    // The next run does not inherit the cancel: it completes.
    let next = rig.download("ios-two");
    assert_eq!(next["data"]["state"], "complete", "{next}");
    // With nothing active, a cancel is "not running" and is not kept for the next run.
    assert_eq!(
        envelope(save_relay_game_files_cancel())["data"]["running"],
        false
    );
    let after_idle_cancel = rig.download("ios-three");
    assert_eq!(
        after_idle_cancel["data"]["state"], "complete",
        "{after_idle_cancel}"
    );
    assert_eq!(after_idle_cancel["data"]["files_done"], 1);
    // The library accepts a new init once nothing runs.
    let ok_init: Value =
        serde_json::from_str(&unsafe { init_with_net(&host, None, None, ClientSet::shipped()) })
            .unwrap();
    assert_eq!(ok_init["ok"], true, "{ok_init}");
}

#[test]
fn progress_belongs_to_the_run_and_ends_terminal_after_an_early_failure() {
    let rig = rig("ffi-progress");
    let done = rig.download("ios-three");
    assert_eq!(done["data"]["state"], "complete", "{done}");
    let finished = envelope(save_relay_game_files_progress());
    assert_eq!(finished["data"]["state"], "complete");

    // A new run starts with its own progress at once, never the previous run's `complete`.
    rig.gate.arm();
    let worker = rig.download_on_thread("ios-two");
    rig.gate.wait_reached();
    let live = envelope(save_relay_game_files_progress());
    assert_eq!(live["data"]["app_id"], "ios-two", "{live}");
    assert_eq!(live["data"]["state"], "running", "{live}");
    rig.gate.release();
    assert_eq!(worker.join().unwrap()["data"]["state"], "complete");

    // A failure before the engine reports anything (here: the game is not uploaded) leaves
    // the progress of that run in `error`, not the earlier run's `complete` or a stuck `running`.
    let ghost = rig.download("ghost");
    assert_eq!(ghost["error"]["code"], "not_yet_uploaded", "{ghost}");
    let failed = envelope(save_relay_game_files_progress());
    assert_eq!(failed["data"]["app_id"], "ghost", "{failed}");
    assert_eq!(failed["data"]["state"], "error", "{failed}");

    // The version of the game-files interface is exported.
    assert_eq!(save_relay_game_files_version(), 1);
}

#[test]
fn progress_shows_the_new_run_from_the_claim_and_ends_in_error_for_failures_the_engine_never_saw() {
    let rig = rig("ffi-progress-early");
    let done = rig.download("ios-three");
    assert_eq!(done["data"]["state"], "complete", "{done}");

    // The run is held at its first request (the manifest's): it already shows as the new
    // app's `running`, not the finished run's `complete`.
    rig.gate.arm_first_request();
    let worker = rig.download_on_thread("ios-two");
    rig.gate.wait_reached();
    let live = envelope(save_relay_game_files_progress());
    assert_eq!(live["data"]["app_id"], "ios-two", "{live}");
    assert_eq!(live["data"]["state"], "running", "{live}");
    rig.gate.release();
    assert_eq!(worker.join().unwrap()["data"]["state"], "complete");

    // A failure before the engine observes anything: both folders are the same, which is
    // rejected while the folders are prepared. The progress is the new app's `error`, not the
    // earlier run's `complete`; and the slot is free again.
    let same = rig.root.join("same-folder");
    let same_c = c(same.to_str().unwrap());
    let rejected = envelope(unsafe {
        save_relay_game_files_download(c("ios-three").as_ptr(), same_c.as_ptr(), same_c.as_ptr())
    });
    assert_eq!(rejected["ok"], false, "{rejected}");
    let after = envelope(save_relay_game_files_progress());
    assert_eq!(after["data"]["app_id"], "ios-three", "{after}");
    assert_eq!(after["data"]["state"], "error", "{after}");
    let again = rig.download("ios-three");
    assert_eq!(again["data"]["state"], "complete", "{again}");

    // A usage error after the claim (an app id that is not valid) does the same.
    let (d, s) = rig.folders("ios-three");
    let invalid = envelope(unsafe {
        save_relay_game_files_download(c("../x").as_ptr(), d.as_ptr(), s.as_ptr())
    });
    assert_eq!(invalid["error"]["code"], "usage", "{invalid}");
    let after = envelope(save_relay_game_files_progress());
    assert_eq!(after["data"]["app_id"], "../x", "{after}");
    assert_eq!(after["data"]["state"], "error", "{after}");
}

#[test]
fn a_panic_in_a_download_ends_in_error_and_frees_the_slot() {
    let rig = rig("ffi-progress-panic");
    rig.gate.arm_panic();
    let crashed = rig.download("ios-two");
    assert_eq!(crashed["ok"], false, "{crashed}");
    assert_eq!(crashed["error"]["code"], "io", "{crashed}");
    let progress = envelope(save_relay_game_files_progress());
    assert_eq!(progress["data"]["app_id"], "ios-two", "{progress}");
    assert_eq!(progress["data"]["state"], "error", "{progress}");
    // Not busy: the next run goes through.
    let next = rig.download("ios-two");
    assert_eq!(next["data"]["state"], "complete", "{next}");
}

/// The C functions fill the engine's protected folders from the library's remembered games
/// (`gamefiles::saves_folders`): a game folder or state folder in a saves folder is refused for
/// plan, download and the compatibility download, before any request; an unreadable record
/// refuses every call (fail closed).
#[test]
fn the_c_functions_refuse_a_folder_in_a_remembered_saves_folder() {
    // Signed in, so a refusal here is the saves-folder check and not a missing sign-in.
    let rig = rig("ffi-protected");
    let root = rig.root.clone();
    let (home, data, config) = (root.join("h"), root.join("d"), root.join("c"));
    let saves = root.join("saves");
    std::fs::create_dir_all(&saves).unwrap();
    let store = save_relay::local::LocalStore::new(save_relay::platform::AppDirs::from_vars(
        &save_relay::platform::PathVars {
            home: home.clone(),
            data: data.clone(),
            config: config.clone(),
            documents: None,
        },
    ));
    store
        .save_game(&save_relay::local::GameRecord {
            manifest: save_relay::manifest::Manifest::from_json(
                r#"{"format":1,"id":"demo-game","name":"Demo Game","saves_from":"launcher","saves":{},"include":["**/*.sav"]}"#,
            )
            .unwrap(),
            saves_dir: Some(saves.clone()),
        })
        .unwrap();

    let app = c("ios-two");
    let elsewhere = root.join("elsewhere");
    let cases = [
        (saves.join("game"), elsewhere.join("state")),
        (saves.clone(), elsewhere.join("state")),
        (elsewhere.join("game"), saves.join("state")),
        (root.clone(), elsewhere.join("state")),
    ];
    for (dest, state) in &cases {
        let (dest_c, state_c) = (c(dest.to_str().unwrap()), c(state.to_str().unwrap()));
        for answer in [
            envelope(unsafe {
                save_relay_game_files_plan(app.as_ptr(), dest_c.as_ptr(), state_c.as_ptr())
            }),
            envelope(unsafe {
                save_relay_game_files_download(app.as_ptr(), dest_c.as_ptr(), state_c.as_ptr())
            }),
        ] {
            assert_eq!(answer["ok"], false, "{dest:?} {state:?}: {answer}");
            assert_eq!(
                answer["error"]["code"], "destination_conflict",
                "{dest:?} {state:?}: {answer}"
            );
        }
    }
    let manifest = c(
        r#"{"game_id":"compat-game","files":[{"path":"data/p.bin","size":3,"sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}]}"#,
    );
    for dest in [saves.join("compat"), saves.clone()] {
        let dest_c = c(dest.to_str().unwrap());
        let got =
            envelope(unsafe { save_relay_download_game_files(manifest.as_ptr(), dest_c.as_ptr()) });
        assert_eq!(got["ok"], false, "{got}");
        assert_eq!(got["error"]["code"], "destination_conflict", "{got}");
    }
    assert_eq!(
        std::fs::read_dir(&saves).unwrap().count(),
        0,
        "nothing written"
    );
    assert!(!elsewhere.exists(), "nothing created");

    // A record that cannot be read refuses the call: the check never passes by not looking.
    let games = data.join("save-relay").join("games");
    std::fs::write(games.join("broken.json"), b"{ not json").unwrap();
    let (dest_c, state_c) = (
        c(elsewhere.join("game").to_str().unwrap()),
        c(elsewhere.join("state").to_str().unwrap()),
    );
    let refused = envelope(unsafe {
        save_relay_game_files_download(app.as_ptr(), dest_c.as_ptr(), state_c.as_ptr())
    });
    assert_eq!(
        refused["error"]["code"], "destination_conflict",
        "{refused}"
    );
    assert!(!elsewhere.exists(), "nothing created");
}

#[test]
fn the_compatibility_download_is_stopped_by_the_cancel_function() {
    let rig = rig("ffi-compat-cancel");
    let dest = rig.root.join("compat-game-folder");
    let manifest = c(&format!(
        r#"{{"game_id":"compat-two","files":[{{"path":"big.bin","size":{},"sha256":"{}"}},{{"path":"sub/small.bin","size":{},"sha256":"{}"}}]}}"#,
        rig.big.len(),
        sha(&rig.big),
        rig.small.len(),
        sha(&rig.small)
    ));
    rig.server
        .put("game-files/compat-two/big.bin", rig.big.clone());
    rig.server
        .put("game-files/compat-two/sub/small.bin", rig.small.clone());
    // The first request is held; the cancel arrives while the run is in it.
    rig.gate.arm_first_request();
    let dest_c = c(dest.to_str().unwrap());
    let (m, d) = (manifest.clone(), dest_c.clone());
    let worker = std::thread::spawn(move || {
        envelope(unsafe { save_relay_download_game_files(m.as_ptr(), d.as_ptr()) })
    });
    rig.gate.wait_reached();
    let reply = envelope(save_relay_game_files_cancel());
    assert_eq!(reply["data"]["running"], true, "{reply}");
    rig.gate.release();
    let done = worker.join().unwrap();
    assert_eq!(done["ok"], true, "{done}");
    assert_eq!(done["data"]["complete"], false, "{done}");
    assert!(
        !dest.join("sub/small.bin").exists(),
        "stopped before the second file"
    );
    // The slot is free again, and a second call continues to the end.
    let again =
        envelope(unsafe { save_relay_download_game_files(manifest.as_ptr(), dest_c.as_ptr()) });
    assert_eq!(again["data"]["complete"], true, "{again}");
    assert_eq!(
        std::fs::read(dest.join("sub/small.bin")).unwrap(),
        rig.small
    );
}
