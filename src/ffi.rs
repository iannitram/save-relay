//! The C interface for Android and iOS apps that embed save-relay (feature `ffi`, header
//! `include/save_relay.h`). It is a thin door over the CLI's own `run_with_net` with `--json`:
//! every function builds the same arguments a launcher would pass, runs the same code, and
//! returns the same one-line envelope (docs/code-map.md, API shape). There is no second JSON
//! shape.
//!
//! The app supplies what the core takes from the platform, through `SaveRelayHost`, once, with
//! `save_relay_init`:
//! - its folders (`PathVars`: home, data, config, documents);
//! - its credential store (`cred_get`, `cred_set`, `cred_delete`): the only place a password or
//!   token is ever written;
//! - its network (`http`): one callback that sends a request and hands the answer back through
//!   `save_relay_http_respond`, so the core needs no TLS or system library here.
//!
//! Every `*mut c_char` returned is freed with `save_relay_free`. A null pointer or text that is
//! not UTF-8 gives an error envelope, and no panic crosses the boundary.

use crate::api;
use crate::cli::{self, Args, Command};
use crate::clients::ClientSet;
use crate::creds::{CredentialStore, Secret};
use crate::error::{Error, Result};
use crate::http::{Request, Response, Transport};
use crate::local::LocalStore;
use crate::manifest::Manifest;
use crate::platform::{AppDirs, PathVars};
use crate::remote::{MemoryRemote, Remote};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The largest secret `cred_get` may hand back (a refresh token is a few hundred bytes).
const MAX_SECRET: usize = 16 * 1024;

type CredGet =
    unsafe extern "C" fn(*mut c_void, *const c_char, *mut u8, usize, *mut usize) -> c_int;
type CredSet = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type CredDelete = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type HttpSend =
    unsafe extern "C" fn(*mut c_void, *const SaveRelayHttpRequest, *mut SaveRelayHttpSink) -> c_int;

/// What the app supplies (`save_relay_init`). Text pointers are copied during the call.
#[repr(C)]
pub struct SaveRelayHost {
    /// Passed back to every callback.
    pub user_data: *mut c_void,
    pub home: *const c_char,
    pub data: *const c_char,
    pub config: *const c_char,
    /// May be null.
    pub documents: *const c_char,
    /// Write the secret for `account` into `buf` (at most `cap` bytes), its length into `*len`;
    /// return 0 when found, 1 when there is none, anything else for a failure.
    pub cred_get: Option<CredGet>,
    /// Store (or replace) `secret` under `account`; 0 on success.
    pub cred_set: Option<CredSet>,
    /// Remove `account` (a missing one is success); 0 on success.
    pub cred_delete: Option<CredDelete>,
    /// Send `request`; call `save_relay_http_respond(sink, ...)` with the answer, whatever its
    /// status, and return 0. Return nonzero for a connection, DNS, TLS or timeout failure.
    pub http: Option<HttpSend>,
}

/// One HTTP request for the app's `http` callback. Pointers are valid only during the call.
#[repr(C)]
pub struct SaveRelayHttpRequest {
    pub method: *const c_char,
    pub url: *const c_char,
    /// `Name: value` lines, each ended by `\n`.
    pub headers: *const c_char,
    pub body: *const u8,
    pub body_len: usize,
    /// Stop reading the answer after this many bytes and fail; 0 means no bound.
    pub max_response_bytes: usize,
}

/// Where the app's `http` callback puts the answer (`save_relay_http_respond`). Opaque.
pub struct SaveRelayHttpSink {
    response: Option<Response>,
}

struct Host {
    user_data: usize,
    vars: PathVars,
    cred_get: CredGet,
    cred_set: CredSet,
    cred_delete: CredDelete,
    http: HttpSend,
    /// Tests: a remote used instead of connecting through `http`.
    remote: Option<Arc<dyn Remote + Send + Sync>>,
}

static HOST: Mutex<Option<Arc<Host>>> = Mutex::new(None);

fn host() -> Result<Arc<Host>> {
    HOST.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .ok_or_else(|| Error::Usage("save_relay_init has not been called".into()))
}

/// A required text argument: null or not UTF-8 is `Error::Usage`.
unsafe fn text<'a>(pointer: *const c_char, what: &str) -> Result<&'a str> {
    if pointer.is_null() {
        return Err(Error::Usage(format!("{what} is null")));
    }
    CStr::from_ptr(pointer)
        .to_str()
        .map_err(|_| Error::Usage(format!("{what} is not valid UTF-8")))
}

unsafe fn optional_text<'a>(pointer: *const c_char, what: &str) -> Result<Option<&'a str>> {
    if pointer.is_null() {
        Ok(None)
    } else {
        text(pointer, what).map(Some)
    }
}

fn c_string(value: &str, what: &str) -> Result<CString> {
    CString::new(value).map_err(|_| Error::Usage(format!("{what} contains a NUL byte")))
}

struct HostCreds(Arc<Host>);

impl CredentialStore for HostCreds {
    fn get(&self, account: &str) -> Result<Option<Secret>> {
        let account = c_string(account, "account")?;
        let mut buffer = vec![0u8; MAX_SECRET];
        let mut length = 0usize;
        // SAFETY: the app's callback gets a buffer of the capacity it is told.
        let status = unsafe {
            (self.0.cred_get)(
                self.0.user_data as *mut c_void,
                account.as_ptr(),
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut length,
            )
        };
        let result = match status {
            0 if length <= buffer.len() => String::from_utf8(buffer[..length].to_vec())
                .map(|value| Some(Secret::new(value)))
                .map_err(|_| Error::Credentials("the credential store gave non-UTF-8".into())),
            0 => Err(Error::Credentials("the stored secret is too large".into())),
            1 => Ok(None),
            _ => Err(Error::Credentials("the credential store failed".into())),
        };
        buffer.fill(0);
        result
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<()> {
        let account = c_string(account, "account")?;
        let value = c_string(secret.expose(), "secret")?;
        // SAFETY: both pointers are NUL-terminated and live through the call.
        let status = unsafe {
            (self.0.cred_set)(
                self.0.user_data as *mut c_void,
                account.as_ptr(),
                value.as_ptr(),
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(Error::Credentials("the credential store failed".into()))
        }
    }

    fn delete(&self, account: &str) -> Result<()> {
        let account = c_string(account, "account")?;
        // SAFETY: the pointer is NUL-terminated and lives through the call.
        let status =
            unsafe { (self.0.cred_delete)(self.0.user_data as *mut c_void, account.as_ptr()) };
        if status == 0 {
            Ok(())
        } else {
            Err(Error::Credentials("the credential store failed".into()))
        }
    }

    fn kind(&self) -> &'static str {
        "host-app"
    }
}

struct HostTransport(Arc<Host>);

impl Transport for HostTransport {
    fn send(&self, request: &Request) -> Result<Response> {
        let bad = |what: &str| Error::Unreachable(format!("request {what} is not sendable"));
        let method = c_string(&request.method, "method").map_err(|_| bad("method"))?;
        let url = c_string(&request.url, "url").map_err(|_| bad("url"))?;
        let mut lines = String::new();
        for (name, value) in &request.headers {
            lines.push_str(name);
            lines.push_str(": ");
            lines.push_str(value);
            lines.push('\n');
        }
        let headers = c_string(&lines, "headers").map_err(|_| bad("headers"))?;
        let native = SaveRelayHttpRequest {
            method: method.as_ptr(),
            url: url.as_ptr(),
            headers: headers.as_ptr(),
            body: request.body.as_ptr(),
            body_len: request.body.len(),
            max_response_bytes: request.max_response_bytes.unwrap_or(0),
        };
        let mut sink = SaveRelayHttpSink { response: None };
        // SAFETY: every pointer in `native` lives through the call.
        let status = unsafe { (self.0.http)(self.0.user_data as *mut c_void, &native, &mut sink) };
        match (status, sink.response) {
            (0, Some(response)) => match request.max_response_bytes {
                Some(max) if response.body.len() > max => Err(Error::Unreachable(
                    "the answer is larger than allowed".into(),
                )),
                _ => Ok(response),
            },
            (0, None) => Err(Error::Unreachable("the app gave no answer".into())),
            _ => Err(Error::Unreachable("the app's network call failed".into())),
        }
    }
}

/// The app's `http` callback hands its answer here. `headers` is `Name: value` lines (may be
/// null); `body` may be null with `body_len` 0. Returns 0, or 1 when `sink` is null.
///
/// # Safety
/// `sink` must be the pointer the callback received; `headers` a NUL-terminated string or null;
/// `body` valid for `body_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn save_relay_http_respond(
    sink: *mut SaveRelayHttpSink,
    status: u16,
    headers: *const c_char,
    body: *const u8,
    body_len: usize,
) -> c_int {
    let Some(sink) = sink.as_mut() else { return 1 };
    let mut response = Response::new(
        status,
        if body.is_null() || body_len == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(body, body_len).to_vec()
        },
    );
    if !headers.is_null() {
        for line in CStr::from_ptr(headers).to_string_lossy().lines() {
            if let Some((name, value)) = line.split_once(':') {
                response = response.header(name.trim(), value.trim());
            }
        }
    }
    sink.response = Some(response);
    0
}

/// Run a closure for one command and turn any failure or panic into its error envelope.
fn guard(command: &str, body: impl FnOnce() -> Result<String>) -> *mut c_char {
    let line = match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(line)) => line,
        Ok(Err(error)) => api::failure(command, &error),
        Err(_) => api::failure(command, &Error::Io("internal error".into())),
    };
    CString::new(line.replace('\0', ""))
        .unwrap_or_default()
        .into_raw()
}

fn build_host(
    native: &SaveRelayHost,
    remote: Option<Arc<dyn Remote + Send + Sync>>,
) -> Result<Host> {
    // SAFETY: the app promises NUL-terminated strings (or null where allowed).
    let (home, data, config, documents) = unsafe {
        (
            text(native.home, "home")?,
            text(native.data, "data")?,
            text(native.config, "config")?,
            optional_text(native.documents, "documents")?,
        )
    };
    let missing = |what: &str| Error::Usage(format!("{what} callback is null"));
    Ok(Host {
        user_data: native.user_data as usize,
        vars: PathVars {
            home: PathBuf::from(home),
            data: PathBuf::from(data),
            config: PathBuf::from(config),
            documents: documents.map(PathBuf::from),
        },
        cred_get: native.cred_get.ok_or_else(|| missing("cred_get"))?,
        cred_set: native.cred_set.ok_or_else(|| missing("cred_set"))?,
        cred_delete: native.cred_delete.ok_or_else(|| missing("cred_delete"))?,
        http: native.http.ok_or_else(|| missing("http"))?,
        remote,
    })
}

fn install(host: Host) -> Result<String> {
    *HOST.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(host));
    Ok(api::success("init", &serde_json::json!({})))
}

/// Tell the library what the app supplies. Call once before any other function; a second call
/// replaces the first. Returns an envelope (`command` "init", empty `data`).
///
/// # Safety
/// `host` must point to a valid `SaveRelayHost` whose text pointers are NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn save_relay_init(host: *const SaveRelayHost) -> *mut c_char {
    guard("init", || {
        let native = host
            .as_ref()
            .ok_or_else(|| Error::Usage("host is null".into()))?;
        install(build_host(native, None)?)
    })
}

/// `save_relay_init` for tests: sync through `remote` (a `MemoryRemote`) instead of connecting
/// through the app's `http` callback.
///
/// # Safety
/// As `save_relay_init`.
#[doc(hidden)]
pub unsafe fn init_with_remote(
    host: &SaveRelayHost,
    remote: Arc<dyn Remote + Send + Sync>,
) -> String {
    match build_host(host, Some(remote)).and_then(install) {
        Ok(line) => line,
        Err(error) => api::failure("init", &error),
    }
}

/// Run `cli::run_with_net` with `--json` and return its envelope line. `password` is setup's,
/// given on the standard input the CLI reads it from, and goes nowhere else.
fn run_json(command: &str, args: Vec<String>, password: Option<String>) -> Result<String> {
    let host = host()?;
    let store = LocalStore::new(AppDirs::from_vars(&host.vars));
    let creds = HostCreds(host.clone());
    let transport: Arc<dyn Transport> = Arc::new(HostTransport(host.clone()));
    let for_connect = transport.clone();
    let connect = move |server: &str, user: &str, secret: Secret| {
        cli::connect_with(server, user, secret, for_connect.clone())
    };
    let parsed = cli::parse(&args);
    let owned: Box<dyn Remote>;
    let remote: &dyn Remote = match (&host.remote, &parsed) {
        (Some(remote), _) => &**remote,
        (
            None,
            Ok(Args {
                command: Command::Setup { server, user, .. },
                ..
            }),
        ) => {
            let secret = Secret::new(password.clone().unwrap_or_default());
            owned = connect(server, user, secret)?;
            &*owned
        }
        (None, Ok(parsed)) => {
            owned = cli::connect_before_via(&parsed.command, &store, &creds, &connect)?;
            &*owned
        }
        // `run_with_net` reports the parse error itself.
        (None, Err(_)) => {
            owned = Box::new(MemoryRemote::new());
            &*owned
        }
    };
    let net = cli::Net {
        transport: transport.clone(),
        open_browser: &|_| {
            Err(Error::SignInFailed(
                "sign-in through a browser is the app's job".into(),
            ))
        },
        clients: ClientSet::shipped(),
        game_mode: false,
        clock: crate::stamp::now,
    };
    let mut stdin = std::io::Cursor::new(password.map(|p| format!("{p}\n")).unwrap_or_default());
    let mut out = Vec::new();
    cli::run_with_net(
        &args,
        &mut stdin,
        &mut out,
        remote,
        &creds,
        &store,
        &host.vars,
        crate::stamp::now(),
        &net,
    );
    let line = String::from_utf8_lossy(&out).trim_end().to_string();
    if line.is_empty() {
        return Err(Error::Io(format!("{command} printed nothing")));
    }
    Ok(line)
}

/// Write the app's manifest JSON where the CLI reads a manifest file, and return the game id and
/// that file (no secrets are in a manifest).
fn stage_manifest(host: &Host, json: &str) -> Result<(String, PathBuf)> {
    let manifest = Manifest::from_json(json)?;
    let folder = AppDirs::from_vars(&host.vars).data.join("ffi");
    std::fs::create_dir_all(&folder)
        .map_err(|e| Error::Io(format!("cannot create {}: {e}", folder.display())))?;
    let path = folder.join(format!("{}.manifest.json", manifest.id));
    std::fs::write(&path, json)
        .map_err(|e| Error::Io(format!("cannot write {}: {e}", path.display())))?;
    Ok((manifest.id, path))
}

fn game_args(
    command: &str,
    manifest_json: *const c_char,
    saves_dir: *const c_char,
    resolve: *const c_char,
    with_resolve: bool,
) -> Result<Vec<String>> {
    let host = host()?;
    // SAFETY: the app promises NUL-terminated strings or null; null is refused above.
    let (manifest_json, saves_dir, resolve) = unsafe {
        (
            text(manifest_json, "manifest_json")?,
            text(saves_dir, "saves_dir")?,
            optional_text(resolve, "resolve")?,
        )
    };
    let (id, path) = stage_manifest(&host, manifest_json)?;
    let mut args = vec![
        command.to_string(),
        id,
        "--manifest".into(),
        path.to_string_lossy().into_owned(),
        "--saves-dir".into(),
        saves_dir.to_string(),
    ];
    if let Some(resolve) = resolve.filter(|_| with_resolve) {
        args.push("--resolve".into());
        args.push(resolve.to_string());
    }
    args.push("--json".into());
    Ok(args)
}

/// Pull `manifest_json`'s game into `saves_dir`. `resolve` is null, `"keep-server"` or
/// `"keep-local"` (the answer to a `conflict`). Returns the `--json` envelope (`pull`).
///
/// # Safety
/// Each pointer is null (refused, except `resolve`) or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn save_relay_pull(
    manifest_json: *const c_char,
    saves_dir: *const c_char,
    resolve: *const c_char,
) -> *mut c_char {
    guard("pull", || {
        let args = game_args("pull", manifest_json, saves_dir, resolve, true)?;
        run_json("pull", args, None)
    })
}

/// Push `saves_dir` for `manifest_json`'s game. Arguments and result as `save_relay_pull`.
///
/// # Safety
/// As `save_relay_pull`.
#[no_mangle]
pub unsafe extern "C" fn save_relay_push(
    manifest_json: *const c_char,
    saves_dir: *const c_char,
    resolve: *const c_char,
) -> *mut c_char {
    guard("push", || {
        let args = game_args("push", manifest_json, saves_dir, resolve, true)?;
        run_json("push", args, None)
    })
}

/// What a pull or push would do, changing nothing. Returns the `--json` envelope (`status`).
///
/// # Safety
/// As `save_relay_pull`; there is no `resolve`.
#[no_mangle]
pub unsafe extern "C" fn save_relay_status(
    manifest_json: *const c_char,
    saves_dir: *const c_char,
) -> *mut c_char {
    guard("status", || {
        let args = game_args("status", manifest_json, saves_dir, std::ptr::null(), false)?;
        run_json("status", args, None)
    })
}

/// Set this device up for a Nextcloud server (once per machine). The password goes to the app's
/// `cred_set` callback and nowhere else. `device` and `root` may be null (device name from the
/// CLI's default, root `save-relay`). Returns the `--json` envelope (`setup`). Signing in to
/// Google Drive or Dropbox is the app's own browser session (#254), not this function.
///
/// # Safety
/// Each pointer is null (refused, except `device` and `root`) or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn save_relay_setup(
    server: *const c_char,
    user: *const c_char,
    password: *const c_char,
    device: *const c_char,
    root: *const c_char,
) -> *mut c_char {
    guard("setup", || {
        let server = text(server, "server")?;
        let user = text(user, "user")?;
        let password = text(password, "password")?;
        let device = optional_text(device, "device")?;
        let root = optional_text(root, "root")?;
        if password.contains(['\n', '\r']) {
            return Err(Error::Usage("password must be one line".into()));
        }
        let mut args = vec![
            "setup".to_string(),
            "--server".into(),
            server.into(),
            "--user".into(),
            user.into(),
        ];
        for (flag, value) in [("--device", device), ("--root", root)] {
            if let Some(value) = value {
                args.push(flag.into());
                args.push(value.into());
            }
        }
        args.push("--json".into());
        run_json("setup", args, Some(password.to_string()))
    })
}

/// Free a string any function above returned. Null is ignored.
///
/// # Safety
/// `pointer` is null or came from this library and has not been freed.
#[no_mangle]
pub unsafe extern "C" fn save_relay_free(pointer: *mut c_char) {
    if !pointer.is_null() {
        drop(CString::from_raw(pointer));
    }
}
