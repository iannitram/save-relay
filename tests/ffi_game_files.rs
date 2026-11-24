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
use std::sync::Mutex;
use support_gamefiles::*;

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

    // Progress before any download is null.
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
}
