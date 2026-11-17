//! The C functions driven from the host side against `MemoryRemote`: the envelopes of pulled,
//! pushed, conflict and offline; null and invalid input; and that a password reaches only the
//! app's credential callback.
#![cfg(feature = "ffi")]

use save_relay::clients::ClientSet;
use save_relay::fake::{FakeDropbox, REFRESH_TOKEN, SIGN_IN_CODE};
use save_relay::ffi::*;
use save_relay::remote::MemoryRemote;
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const MANIFEST: &str = r#"{"format":1,"id":"ffi-game","name":"FFI Game","saves":{},"saves_from":"launcher","include":["**/*"]}"#;
const PASSWORD: &str = "hunter2-app-password";

/// The app's side: a credential store in a map, and a record of the network calls.
#[derive(Default)]
struct App {
    secrets: HashMap<String, String>,
    set_calls: Vec<String>,
    requests: Vec<(String, String, String)>,
    http_status: Option<u16>,
}

unsafe fn app<'a>(user_data: *mut c_void) -> std::sync::MutexGuard<'a, App> {
    (*(user_data as *const Mutex<App>)).lock().unwrap()
}

unsafe extern "C" fn cred_get(
    ud: *mut c_void,
    account: *const c_char,
    buf: *mut u8,
    cap: usize,
    len: *mut usize,
) -> c_int {
    let state = app(ud);
    let account = CStr::from_ptr(account).to_str().unwrap();
    match state.secrets.get(account) {
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
    ud: *mut c_void,
    account: *const c_char,
    secret: *const c_char,
) -> c_int {
    let mut state = app(ud);
    let account = CStr::from_ptr(account).to_str().unwrap().to_string();
    state.set_calls.push(account.clone());
    state.secrets.insert(
        account,
        CStr::from_ptr(secret).to_str().unwrap().to_string(),
    );
    0
}

unsafe extern "C" fn cred_delete(ud: *mut c_void, account: *const c_char) -> c_int {
    app(ud)
        .secrets
        .remove(CStr::from_ptr(account).to_str().unwrap());
    0
}

unsafe extern "C" fn http(
    ud: *mut c_void,
    request: *const SaveRelayHttpRequest,
    sink: *mut SaveRelayHttpSink,
) -> c_int {
    // Invalid callback answers must be refused, without populating the response sink.
    let invalid_headers = [0xffu8, 0];
    assert_eq!(
        save_relay_http_respond(
            sink,
            200,
            invalid_headers.as_ptr().cast(),
            std::ptr::null(),
            0
        ),
        1
    );
    assert_eq!(
        save_relay_http_respond(sink, 200, std::ptr::null(), std::ptr::null(), 1),
        1
    );
    assert_eq!(
        save_relay_http_respond(
            std::ptr::null_mut(),
            200,
            std::ptr::null(),
            std::ptr::null(),
            0,
        ),
        1
    );
    let mut state = app(ud);
    let request = &*request;
    state.requests.push((
        CStr::from_ptr(request.method).to_str().unwrap().to_string(),
        CStr::from_ptr(request.url).to_str().unwrap().to_string(),
        CStr::from_ptr(request.headers)
            .to_str()
            .unwrap()
            .to_string(),
    ));
    match state.http_status {
        Some(status) => save_relay_http_respond(
            sink,
            status,
            c"Content-Type: text/plain\n".as_ptr(),
            std::ptr::null(),
            0,
        ),
        None => 7,
    }
}

/// One device: its folders and its app state. `state` is leaked on purpose (the callbacks hold
/// its address for the rest of the test process).
struct Device {
    root: PathBuf,
    state: &'static Mutex<App>,
}

impl Device {
    fn new(name: &str) -> Device {
        let root = std::env::temp_dir().join(format!("sr-ffi-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["home", "data", "config", "saves"] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }
        Device {
            root,
            state: Box::leak(Box::new(Mutex::new(App::default()))),
        }
    }

    fn saves(&self) -> PathBuf {
        self.root.join("saves")
    }

    /// Make this device the one the C functions act for.
    fn init(&self, remote: Option<Arc<MemoryRemote>>) -> Value {
        self.init_with(remote, None)
    }

    /// `init`, with the core's requests going through `transport` instead of the `http`
    /// callback (a fake service transport for the sign-in test).
    fn init_with(
        &self,
        remote: Option<Arc<MemoryRemote>>,
        transport: Option<Arc<dyn save_relay::http::Transport + Send + Sync>>,
    ) -> Value {
        let path = |sub: &str| CString::new(self.root.join(sub).to_str().unwrap()).unwrap();
        let (home, data, config) = (path("home"), path("data"), path("config"));
        let host = SaveRelayHost {
            user_data: self.state as *const Mutex<App> as *mut c_void,
            home: home.as_ptr(),
            data: data.as_ptr(),
            config: config.as_ptr(),
            documents: std::ptr::null(),
            cred_get: Some(cred_get),
            cred_set: Some(cred_set),
            cred_delete: Some(cred_delete),
            http: Some(http),
        };
        match (remote, transport) {
            (Some(remote), transport) => serde_json::from_str(&unsafe {
                init_with_net(&host, Some(remote), transport, ClientSet::shipped())
            })
            .unwrap(),
            (None, None) => envelope(unsafe { save_relay_init(&host) }),
            (None, transport) => serde_json::from_str(&unsafe {
                init_with_net(&host, None, transport, ClientSet::shipped())
            })
            .unwrap(),
        }
    }
}

/// Read, parse and free a returned string.
fn envelope(pointer: *mut c_char) -> Value {
    assert!(!pointer.is_null());
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .unwrap()
        .to_string();
    unsafe { save_relay_free(pointer) };
    assert!(!text.contains('\n'), "one line only: {text}");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

fn c(text: &str) -> CString {
    CString::new(text).unwrap()
}

fn setup(device: &str) -> Value {
    envelope(unsafe {
        save_relay_setup(
            c("https://nc.example.org").as_ptr(),
            c("me").as_ptr(),
            c(PASSWORD).as_ptr(),
            c(device).as_ptr(),
            std::ptr::null(),
        )
    })
}

type GameCall = unsafe extern "C" fn(*const c_char, *const c_char, *const c_char) -> *mut c_char;

fn call(f: GameCall, dir: &Path, resolve: Option<&str>) -> Value {
    let resolve = resolve.map(c);
    envelope(unsafe {
        f(
            c(MANIFEST).as_ptr(),
            c(dir.to_str().unwrap()).as_ptr(),
            resolve.as_ref().map_or(std::ptr::null(), |r| r.as_ptr()),
        )
    })
}

fn pull(dir: &Path, resolve: Option<&str>) -> Value {
    call(save_relay_pull, dir, resolve)
}

fn push(dir: &Path, resolve: Option<&str>) -> Value {
    call(save_relay_push, dir, resolve)
}

fn action(envelope: &Value) -> &str {
    assert_eq!(envelope["ok"], true, "{envelope}");
    envelope["data"]["action"].as_str().unwrap()
}

/// The C functions share one host, so the tests take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn pulled_pushed_conflict_and_offline_envelopes() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let server = Arc::new(MemoryRemote::new());
    let phone = Device::new("phone");
    let tablet = Device::new("tablet");

    assert_eq!(phone.init(Some(server.clone()))["ok"], true);
    let done = setup("phone");
    assert_eq!(done["ok"], true, "{done}");
    assert_eq!(done["command"], "setup");
    assert_eq!(done["data"]["credential_store"], "host-app");
    std::fs::write(phone.saves().join("slot1.sav"), b"chapter one").unwrap();
    let status = envelope(unsafe {
        save_relay_status(
            c(MANIFEST).as_ptr(),
            c(phone.saves().to_str().unwrap()).as_ptr(),
        )
    });
    assert_eq!(status["command"], "status");
    assert_eq!(status["data"]["would"], "push");

    // pushed
    let pushed = push(&phone.saves(), None);
    assert_eq!(pushed["command"], "push");
    assert_eq!(action(&pushed), "pushed");
    assert_eq!(pushed["data"]["counts"]["uploaded"], 1);

    // pulled, on a second device
    tablet.init(Some(server.clone()));
    assert_eq!(setup("tablet")["ok"], true);
    let pulled = pull(&tablet.saves(), None);
    assert_eq!(pulled["command"], "pull");
    assert_eq!(action(&pulled), "pulled");
    assert_eq!(
        std::fs::read(tablet.saves().join("slot1.sav")).unwrap(),
        b"chapter one"
    );

    // both change: the second to push meets a conflict, then resolves it
    std::fs::write(tablet.saves().join("slot1.sav"), b"tablet edit").unwrap();
    assert_eq!(action(&push(&tablet.saves(), None)), "pushed");
    phone.init(Some(server.clone()));
    std::fs::write(phone.saves().join("slot1.sav"), b"phone edit").unwrap();
    let conflict = push(&phone.saves(), None);
    assert_eq!(action(&conflict), "conflict");
    assert_eq!(
        std::fs::read(phone.saves().join("slot1.sav")).unwrap(),
        b"phone edit"
    );
    let resolved = push(&phone.saves(), Some("keep-server"));
    assert_eq!(action(&resolved), "pulled");
    assert_eq!(
        std::fs::read(phone.saves().join("slot1.sav")).unwrap(),
        b"tablet edit"
    );

    // offline: a pull still answers ok so the game can start; a push is deferred
    server.set_reachable(false);
    let offline = pull(&phone.saves(), None);
    assert_eq!(action(&offline), "offline");
    std::fs::write(phone.saves().join("slot1.sav"), b"offline play").unwrap();
    assert_eq!(action(&push(&phone.saves(), None)), "deferred");
    server.set_reachable(true);

    // no password anywhere but the app's credential callback
    for device in [&phone, &tablet] {
        let state = device.state.lock().unwrap();
        assert!(!state.set_calls.is_empty());
        assert_eq!(state.secrets.len(), 1);
        assert_eq!(state.secrets.values().next().unwrap(), PASSWORD);
        let mut files = Vec::new();
        files_under(&device.root, &mut files);
        for file in files {
            let bytes = std::fs::read(&file).unwrap();
            assert!(
                !String::from_utf8_lossy(&bytes).contains(PASSWORD),
                "password found in {}",
                file.display()
            );
        }
    }
    for envelope in [&done, &pushed, &pulled, &conflict, &offline] {
        assert!(!envelope.to_string().contains(PASSWORD));
    }
}

#[test]
fn null_and_invalid_input_give_error_envelopes() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let device = Device::new("bad-input");
    device.init(Some(Arc::new(MemoryRemote::new())));
    let dir = c(device.saves().to_str().unwrap());
    let invalid = CString::new(vec![0xff, 0xfe]).unwrap();
    let null = std::ptr::null();

    let nulls = [
        envelope(unsafe { save_relay_pull(null, dir.as_ptr(), null) }),
        envelope(unsafe { save_relay_push(c(MANIFEST).as_ptr(), null, null) }),
        envelope(unsafe { save_relay_status(null, null) }),
        envelope(unsafe { save_relay_setup(null, null, null, null, null) }),
        envelope(unsafe { save_relay_init(std::ptr::null()) }),
    ];
    for result in &nulls {
        assert_eq!(result["ok"], false, "{result}");
        assert_eq!(result["error"]["code"], "usage", "{result}");
    }
    let bad_utf8 = envelope(unsafe { save_relay_pull(invalid.as_ptr(), dir.as_ptr(), null) });
    assert_eq!(bad_utf8["error"]["code"], "usage");
    let bad_dir =
        envelope(unsafe { save_relay_push(c(MANIFEST).as_ptr(), invalid.as_ptr(), null) });
    assert_eq!(bad_dir["error"]["code"], "usage");
    let bad_manifest = envelope(unsafe { save_relay_pull(c("{").as_ptr(), dir.as_ptr(), null) });
    assert_eq!(bad_manifest["error"]["code"], "manifest_invalid");
    let bad_resolve = pull(&device.saves(), Some("keep-both"));
    assert_eq!(bad_resolve["ok"], false);
    unsafe { save_relay_free(std::ptr::null_mut()) };
}

#[test]
fn the_network_goes_through_the_apps_http_callback() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let device = Device::new("http");
    assert_eq!(device.init(None)["ok"], true);

    // a refused login: the request reached the app with its method, URL and headers, and
    // nothing was stored
    device.state.lock().unwrap().http_status = Some(401);
    let refused = setup("phone");
    assert_eq!(refused["error"]["code"], "auth_failed", "{refused}");
    {
        let state = device.state.lock().unwrap();
        let (method, url, headers) = state.requests.first().expect("a request");
        assert_eq!(method, "PROPFIND");
        assert!(url.starts_with("https://nc.example.org/"), "{url}");
        assert!(headers.contains("Authorization: Basic "), "{headers}");
        assert!(state.set_calls.is_empty());
    }

    // a failed network call is an unreachable server
    device.state.lock().unwrap().http_status = None;
    let down = setup("phone");
    assert_eq!(down["error"]["code"], "server_unreachable", "{down}");
}

fn start_dropbox_sign_in() -> Value {
    envelope(unsafe {
        save_relay_sign_in_start(
            c("dropbox").as_ptr(),
            c("saverelay://oauth").as_ptr(),
            c("phone").as_ptr(),
            std::ptr::null(),
        )
    })
}

/// The `state` a sign-in URL carries.
fn state_of(url: &str) -> String {
    url.split("state=")
        .nth(1)
        .and_then(|rest| rest.split('&').next())
        .expect("a state in the sign-in URL")
        .to_string()
}

#[test]
fn platform_sign_in_takes_the_apps_browser_session_and_stores_the_token_only_there() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(FakeDropbox::new());
    let device = Device::new("platform-sign-in");
    assert_eq!(
        device.init_with(Some(Arc::new(MemoryRemote::new())), Some(fake.clone()))["ok"],
        true
    );

    // The app opens data.url in its browser session; it carries the same PKCE parts as the
    // desktop sign-in and the app's own redirect URI.
    let started = start_dropbox_sign_in();
    assert_eq!(started["ok"], true, "{started}");
    assert_eq!(started["command"], "setup");
    assert_eq!(started["data"]["service"], "dropbox");
    assert!(started["data"]["code"].is_null(), "{started}");
    let url = started["data"]["url"].as_str().unwrap();
    assert!(
        url.starts_with("https://www.dropbox.com/oauth2/authorize?"),
        "{url}"
    );
    assert!(url.contains("code_challenge_method=S256"), "{url}");
    assert!(
        url.contains("redirect_uri=saverelay%3A%2F%2Foauth"),
        "{url}"
    );
    assert!(url.contains("token_access_type=offline"), "{url}");

    // A redirect for another sign-in is refused before any network call, and uses it up.
    let wrong = envelope(unsafe {
        save_relay_sign_in_finish(
            c("saverelay://oauth?code=fake-sign-in-code&state=another-sign-in").as_ptr(),
        )
    });
    assert_eq!(wrong["ok"], false, "{wrong}");
    assert_eq!(wrong["error"]["code"], "sign_in_failed", "{wrong}");
    assert!(fake.requests().is_empty(), "{:?}", fake.requests());

    // Start again; the app hands back the redirect it received.
    let started = start_dropbox_sign_in();
    let state = state_of(started["data"]["url"].as_str().unwrap());
    let redirect = format!("saverelay://oauth?code={SIGN_IN_CODE}&state={state}");
    let finished = envelope(unsafe { save_relay_sign_in_finish(c(&redirect).as_ptr()) });
    assert_eq!(finished["ok"], true, "{finished}");
    assert_eq!(finished["command"], "setup");
    assert_eq!(finished["data"]["service"], "dropbox");
    assert_eq!(finished["data"]["user"], "fake@example.com");
    assert_eq!(finished["data"]["device"], "phone");

    // The refresh token is only in the app's credential callback, never in a file or envelope.
    {
        let state = device.state.lock().unwrap();
        assert_eq!(
            state
                .secrets
                .values()
                .filter(|value| value.as_str() == REFRESH_TOKEN)
                .count(),
            1,
            "{:?}",
            state.secrets.keys().collect::<Vec<_>>()
        );
        assert!(!state
            .secrets
            .values()
            .any(|value| value.contains(SIGN_IN_CODE)));
    }
    let mut files = Vec::new();
    files_under(&device.root, &mut files);
    for file in files {
        let bytes = std::fs::read(&file).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains(REFRESH_TOKEN) && !text.contains(SIGN_IN_CODE),
            "a sign-in secret is in {}",
            file.display()
        );
    }
    assert!(!started.to_string().contains(REFRESH_TOKEN));
    assert!(!finished.to_string().contains(REFRESH_TOKEN));

    // A second finish has no waiting sign-in.
    let again = envelope(unsafe { save_relay_sign_in_finish(c(&redirect).as_ptr()) });
    assert_eq!(again["error"]["code"], "sign_in_failed", "{again}");
}
