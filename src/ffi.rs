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
use crate::clients::{self, Client, ClientSet};
use crate::creds::{CredentialStore, Secret};
use crate::error::{Error, Result};
use crate::http::{Request, Response, Transport};
use crate::local::LocalStore;
use crate::manifest::Manifest;
use crate::oauth::{self, PlatformSignIn};
use crate::platform::{AppDirs, PathVars};
use crate::remote::{MemoryRemote, Remote};
use crate::service::Service;
use crate::stamp;
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
    max_response_bytes: Option<usize>,
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
    /// Tests: a transport used instead of the app's `http` callback (the app is never asked).
    transport: Option<Arc<dyn Transport + Send + Sync>>,
    /// The build's app registrations; tests may pass their own.
    clients: ClientSet,
}

/// A platform browser sign-in (`save_relay_sign_in_start`) waiting for the app's redirect.
static SIGN_IN: Mutex<Option<PendingSignIn>> = Mutex::new(None);

struct PendingSignIn {
    sign_in: PlatformSignIn,
    device: Option<String>,
    root: String,
}

/// The transport the core talks through: a test's, or the app's `http` callback.
fn transport_for(host: &Arc<Host>) -> Arc<dyn Transport> {
    match &host.transport {
        Some(transport) => {
            let transport: Arc<dyn Transport> = transport.clone();
            transport
        }
        None => Arc::new(HostTransport(host.clone())),
    }
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
        let mut sink = SaveRelayHttpSink {
            response: None,
            max_response_bytes: request.max_response_bytes,
        };
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
/// null); `body` may be null with `body_len` 0. Returns 0, or 1 on invalid input or panic.
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
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sink) = sink.as_mut() else { return 1 };
        if (body.is_null() && body_len != 0)
            || body_len > isize::MAX as usize
            || sink.max_response_bytes.is_some_and(|max| body_len > max)
        {
            return 1;
        }
        let headers = match optional_text(headers, "headers") {
            Ok(headers) => headers,
            Err(_) => return 1,
        };
        let mut response = Response::new(
            status,
            if body_len == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(body, body_len).to_vec()
            },
        );
        if let Some(headers) = headers {
            for line in headers.lines() {
                if let Some((name, value)) = line.split_once(':') {
                    response = response.header(name.trim(), value.trim());
                }
            }
        }
        sink.response = Some(response);
        0
    }))
    .unwrap_or(1)
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
    transport: Option<Arc<dyn Transport + Send + Sync>>,
    clients: ClientSet,
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
        transport,
        clients,
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
        install(build_host(native, None, None, ClientSet::shipped())?)
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
    init_with_net(host, Some(remote), None, ClientSet::shipped())
}

/// `save_relay_init` for tests: sync through `remote` and, when given, send the core's requests
/// through `transport` instead of the app's `http` callback (the platform sign-in tests use a
/// fake service transport).
///
/// # Safety
/// As `save_relay_init`.
#[doc(hidden)]
pub unsafe fn init_with_net(
    host: &SaveRelayHost,
    remote: Option<Arc<dyn Remote + Send + Sync>>,
    transport: Option<Arc<dyn Transport + Send + Sync>>,
    clients: ClientSet,
) -> String {
    match build_host(host, remote, transport, clients).and_then(install) {
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
    let transport = transport_for(&host);
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
        clients: host.clients,
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

/// The app registration and Google client kind a platform sign-in uses (the same public
/// registrations as the desktop browser sign-in).
fn platform_client(host: &Host, service: Service) -> Result<(Client, &'static str)> {
    match service {
        Service::Gdrive => Ok((
            clients::registered(host.clients.google_desktop, "Google Drive")?,
            "desktop",
        )),
        Service::Dropbox => Ok((clients::registered(host.clients.dropbox, "Dropbox")?, "")),
        Service::Nextcloud => Err(Error::Usage(
            "Nextcloud signs in with an app password, not a browser session".into(),
        )),
    }
}

/// Start a Google Drive or Dropbox sign-in in the app's own browser session (Android Custom
/// Tabs, iOS `ASWebAuthenticationSession`): the same PKCE parts as the desktop browser sign-in.
/// Returns the `--json` envelope (`setup`) with a `SignInPrompt` whose `url` is the page to open
/// and whose `code` is null (the app receives the redirect at `redirect_uri` itself).
///
/// The app then hands the redirect URL to `save_relay_sign_in_finish`, which exchanges the code
/// and writes the refresh token through `cred_set` only. `device` and `root` are the setup
/// settings (null: the CLI's default name and root). No password is involved.
///
/// # Safety
/// Each pointer is null (refused, except `device` and `root`) or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn save_relay_sign_in_start(
    service: *const c_char,
    redirect_uri: *const c_char,
    device: *const c_char,
    root: *const c_char,
) -> *mut c_char {
    guard("setup", || {
        let host = host()?;
        let service = Service::parse(text(service, "service")?)?;
        let redirect_uri = text(redirect_uri, "redirect_uri")?;
        let device = optional_text(device, "device")?.map(str::to_string);
        let root = optional_text(root, "root")?
            .unwrap_or(cli::DEFAULT_ROOT)
            .to_string();
        let provider = oauth::provider(service)
            .ok_or_else(|| Error::Usage("this service does not use a browser sign-in".into()))?;
        let (client, _) = platform_client(&host, service)?;
        let sign_in = oauth::start_platform_sign_in(provider, client, redirect_uri, stamp::now())?;
        let data = api::SignInPrompt {
            service: service.key().to_string(),
            url: sign_in.url.clone(),
            code: None,
            expires_at: stamp::rfc3339(sign_in.expires_at),
            message: format!(
                "Open this page in the app's browser session, sign in to {}, then hand the redirect back to save_relay_sign_in_finish.",
                service.label()
            ),
        };
        *SIGN_IN
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(PendingSignIn {
            sign_in,
            device,
            root,
        });
        Ok(api::success("setup", &data))
    })
}

/// Finish the sign-in `save_relay_sign_in_start` began: `redirect_url` is the app's browser
/// session's answer (the URL its redirect URI received). The `state` must match this sign-in's;
/// the code is exchanged with the PKCE verifier through the app's network callback, and the
/// refresh token goes to `cred_set` only. Returns the `--json` envelope (`setup`, `SetupData`).
/// A transient network or server failure keeps the sign-in for another try; a refused, expired
/// or mismatched answer ends it.
///
/// # Safety
/// `redirect_url` is a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn save_relay_sign_in_finish(redirect_url: *const c_char) -> *mut c_char {
    guard("setup", || {
        let host = host()?;
        let redirect = text(redirect_url, "redirect_url")?;
        let pending = SIGN_IN
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .ok_or_else(|| {
                Error::SignInFailed(
                    "no sign-in is waiting: call save_relay_sign_in_start first".into(),
                )
            })?;
        let service = pending.sign_in.service;
        let provider = oauth::provider(service)
            .ok_or_else(|| Error::Usage("this service does not use a browser sign-in".into()))?;
        let (client, kind) = platform_client(&host, service)?;
        let transport = transport_for(&host);
        let tokens = match oauth::finish_platform_sign_in(
            provider,
            client,
            &pending.sign_in,
            redirect,
            &*transport,
            stamp::now,
        ) {
            Ok(tokens) => tokens,
            Err(error) => {
                // A dropped link can be retried with the same redirect; a refusal cannot.
                if matches!(error, Error::Unreachable(_) | Error::Server(_)) {
                    *SIGN_IN
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(pending);
                }
                return Err(error);
            }
        };
        let creds = HostCreds(host.clone());
        let store = LocalStore::new(AppDirs::from_vars(&host.vars));
        cli::finish_platform_setup(
            service,
            tokens,
            client,
            kind,
            pending.device.as_deref(),
            &pending.root,
            &creds,
            &store,
            transport,
            stamp::now,
        )
    })
}

/// Set this device up for a Nextcloud server (once per machine). The password goes to the app's
/// `cred_set` callback and nowhere else. `device` and `root` may be null (device name from the
/// CLI's default, root `save-relay`). Returns the `--json` envelope (`setup`). Signing in to
/// Google Drive or Dropbox is `save_relay_sign_in_start` / `_finish` (#254), not this function.
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
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !pointer.is_null() {
            drop(CString::from_raw(pointer));
        }
    }));
}

#[cfg(test)]
mod range_sink_tests {
    use super::*;

    #[test]
    fn rejects_oversized_response_before_copying() {
        let mut sink = SaveRelayHttpSink {
            response: None,
            max_response_bytes: Some(2),
        };
        let bytes = [0u8; 3];
        let result = unsafe {
            save_relay_http_respond(
                &mut sink,
                200,
                std::ptr::null(),
                bytes.as_ptr(),
                bytes.len(),
            )
        };
        assert_eq!(result, 1);
        assert!(sink.response.is_none());
    }
}
