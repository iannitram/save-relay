//! `setup` and `status` for Google Drive and Dropbox, end to end through `cli::run_with_net`
//! against the in-memory fakes (`FakeDropbox`, `FakeDrive`) and `MemoryStore`: the sign-in
//! prompt, the code and device flows, what the config and the store hold afterwards (M6), the
//! scopes asked for (M10), switching services, and `reauth_needed` / `sign_in_failed` (exit 4).
//! No network and no browser: the transport is a fake and the browser opener is injected.

use save_relay::cli::{run_with_net, Net};
use save_relay::clients::{Client, ClientSet};
use save_relay::creds::{CredentialStore, MemoryStore};
use save_relay::fake::{FakeDrive, FakeDropbox, DEVICE_CODE, REFRESH_TOKEN, SIGN_IN_CODE};
use save_relay::http::{Request, Response, Transport};
use save_relay::local::LocalStore;
use save_relay::platform::{AppDirs, PathVars};
use save_relay::remote::MemoryRemote;
use save_relay::service::{google_client_account, pending_account, token_account, Service};
use save_relay::{Error, Result};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// 2026-10-31T19:51:00Z: the fakes' own clock starts here too.
const T0: u64 = 1_793_476_260;
/// The refresh token a rotating service hands out in place of `REFRESH_TOKEN`.
const ROTATED: &str = "fake-rotated-refresh-token";
/// The account the fakes report (`users/get_current_account`, `about`).
const ACCOUNT: &str = "fake@example.com";
/// What `setup --service dropbox` may ask for: the App folder and nothing else (M10).
const DROPBOX_SCOPES: &str =
    "account_info.read files.metadata.read files.metadata.write files.content.read files.content.write";
/// What `setup --service gdrive` may ask for: the files this app creates and nothing else (M10).
const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn clock() -> u64 {
    T0
}

/// Stand-in app ids, so the sign-in is allowed (the build's own are empty until registered).
fn test_clients() -> ClientSet {
    ClientSet {
        google_desktop: Client {
            id: "test-google-desktop-id",
            secret: Some("test-google-desktop-secret"),
        },
        google_device: Client {
            id: "test-google-device-id",
            secret: Some("test-google-device-secret"),
        },
        dropbox: Client {
            id: "test-dropbox-app-key",
            secret: None,
        },
    }
}

fn no_browser(_url: &str) -> Result<()> {
    Err(Error::SignInFailed("this test has no browser".into()))
}

/// One machine: its own folders and credential store.
struct Machine {
    root: PathBuf,
    store: LocalStore,
    vars: PathVars,
    creds: MemoryStore,
}

impl Machine {
    fn new() -> Machine {
        let root = std::env::temp_dir().join(format!(
            "save-relay-cli-services-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("saves")).unwrap();
        fs::write(
            root.join("demo.json"),
            r#"{"format":1,"id":"demo-game","name":"Demo Game",
            "saves_from":"launcher","saves":{},"include":["**/*.sav"]}"#,
        )
        .unwrap();
        let vars = PathVars {
            home: root.join("home"),
            data: root.join("data"),
            config: root.join("config"),
            documents: None,
        };
        let store = LocalStore::new(AppDirs {
            config: root.join("config").join("save-relay"),
            data: root.join("data").join("save-relay"),
        });
        Machine {
            root,
            store,
            vars,
            creds: MemoryStore::new(),
        }
    }

    /// One command line with `stdin`, over `transport`, with `open` as the browser and Game Mode
    /// as given. Returns the exit code and stdout.
    fn run_in(
        &self,
        transport: &Arc<dyn Transport>,
        open: &dyn Fn(&str) -> Result<()>,
        game_mode: bool,
        line: &str,
        stdin: &str,
    ) -> (i32, String) {
        let net = Net {
            transport: transport.clone(),
            open_browser: open,
            clients: test_clients(),
            game_mode,
            clock,
        };
        let args: Vec<String> = line.split_whitespace().map(String::from).collect();
        let mut input = stdin.as_bytes();
        let mut output = Vec::new();
        let remote = MemoryRemote::new();
        let code = run_with_net(
            &args,
            &mut input,
            &mut output,
            &remote,
            &self.creds,
            &self.store,
            &self.vars,
            T0,
            &net,
        );
        (code, String::from_utf8(output).unwrap())
    }

    /// `line --json` with no stdin, out of the envelope.
    fn json(&self, transport: &Arc<dyn Transport>, line: &str, stdin: &str) -> (i32, Value) {
        let (code, out) = self.run_in(
            transport,
            &no_browser,
            false,
            &format!("{line} --json"),
            stdin,
        );
        (code, one_envelope(&out))
    }

    /// The common flags of a game command.
    fn game_flags(&self) -> String {
        format!(
            "demo-game --manifest {} --saves-dir {}",
            self.root.join("demo.json").display(),
            self.root.join("saves").display()
        )
    }

    /// Fails if any of `secrets` is in any file under this machine's folders (config and data).
    fn assert_no_secret_on_disk(&self, secrets: &[&str]) {
        let mut files = Vec::new();
        files_under(&self.root, &mut files);
        assert!(!files.is_empty());
        for file in files {
            let text = String::from_utf8_lossy(&fs::read(&file).unwrap()).into_owned();
            for secret in secrets {
                assert!(!text.contains(secret), "{} holds a secret", file.display());
            }
        }
    }
}

fn files_under(dir: &Path, found: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files_under(&path, found);
            } else {
                found.push(path);
            }
        }
    }
}

/// Exactly one line, and it is JSON.
fn one_envelope(out: &str) -> Value {
    assert_eq!(
        out.lines().count(),
        1,
        "expected one envelope line, got: {out}"
    );
    serde_json::from_str(out).unwrap_or_else(|error| panic!("not JSON ({error}): {out}"))
}

fn keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

/// A query parameter of a URL, percent-decoded (`+` is a space).
fn query(url: &str, key: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then(|| decode(value))
    })
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[i + 1..i + 3])
                    .ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                {
                    Some(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap()
}

/// `FakeDropbox`, except that a token refresh also rotates the refresh token, as a service may.
struct Rotating(Arc<FakeDropbox>);

impl Transport for Rotating {
    fn send(&self, request: &Request) -> Result<Response> {
        let response = self.0.send(request)?;
        let refresh = request.url.ends_with("/oauth2/token")
            && String::from_utf8_lossy(&request.body).contains("grant_type=refresh_token");
        if !refresh || response.status != 200 {
            return Ok(response);
        }
        let mut answer: Value = serde_json::from_slice(&response.body).unwrap();
        answer["refresh_token"] = Value::from(ROTATED);
        Ok(Response::new(200, answer.to_string()))
    }
}

fn dropbox() -> (Arc<FakeDropbox>, Arc<dyn Transport>) {
    let fake = Arc::new(FakeDropbox::new());
    let transport: Arc<dyn Transport> = fake.clone();
    (fake, transport)
}

// `FakeDrive` is not `Send` (its test hook is a plain closure). These tests run it on one thread;
// `Net` takes an `Arc<dyn Transport>` by design, so the handle is an `Arc` all the same.
#[allow(clippy::arc_with_non_send_sync)]
fn drive() -> (Arc<FakeDrive>, Arc<dyn Transport>) {
    let fake = Arc::new(FakeDrive::new());
    let transport: Arc<dyn Transport> = fake.clone();
    (fake, transport)
}

/// A complete Dropbox setup: the prompt, then the finish with the code the fake accepts.
fn sign_in_dropbox(machine: &Machine, transport: &Arc<dyn Transport>) {
    let (code, envelope) = machine.json(
        transport,
        "setup --service dropbox --no-browser --device desk",
        "",
    );
    assert_eq!(code, 0, "{envelope}");
    let (code, envelope) = machine.json(
        transport,
        "setup --service dropbox --finish --device desk",
        &format!("{SIGN_IN_CODE}\n"),
    );
    assert_eq!(code, 0, "{envelope}");
}

#[test]
fn dropbox_no_browser_prints_a_prompt_whose_scopes_are_the_app_folder_only() {
    let (fake, transport) = dropbox();
    let machine = Machine::new();

    let (code, envelope) = machine.json(
        &transport,
        "setup --service dropbox --no-browser --device desk",
        "",
    );
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(envelope["command"], "setup");
    let data = &envelope["data"];
    assert_eq!(
        keys(data),
        BTreeSet::from(["service", "url", "code", "expires_at", "message"])
    );
    assert_eq!(data["service"], "dropbox");
    assert!(data["code"].is_null(), "Dropbox shows its code on the page");
    let url = data["url"].as_str().unwrap();
    assert!(
        url.starts_with("https://www.dropbox.com/oauth2/authorize?"),
        "{url}"
    );
    assert_eq!(query(url, "scope").as_deref(), Some(DROPBOX_SCOPES));
    assert_eq!(
        query(url, "client_id").as_deref(),
        Some("test-dropbox-app-key")
    );
    assert_eq!(query(url, "code_challenge_method").as_deref(), Some("S256"));
    assert!(data["expires_at"].as_str().unwrap().ends_with('Z'));
    assert!(data["message"].as_str().unwrap().contains("--finish"));

    // Nothing is set up yet; the waiting sign-in is in the store, and no request was sent.
    assert!(machine.store.load_config().unwrap().is_none());
    assert!(machine
        .creds
        .get(&pending_account(Service::Dropbox))
        .unwrap()
        .is_some());
    assert!(fake.requests().is_empty());
}

#[test]
fn dropbox_finish_with_the_code_stores_the_refresh_token_and_writes_server_dropbox() {
    let (fake, transport) = dropbox();
    let machine = Machine::new();
    let (code, _) = machine.json(
        &transport,
        "setup --service dropbox --no-browser --device desk",
        "",
    );
    assert_eq!(code, 0);

    let (code, envelope) = machine.json(
        &transport,
        "setup --service dropbox --finish --device desk",
        &format!("{SIGN_IN_CODE}\n"),
    );
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(
        keys(&envelope["data"]),
        BTreeSet::from([
            "service",
            "server",
            "user",
            "device",
            "root",
            "credential_store"
        ])
    );
    let data = &envelope["data"];
    assert_eq!(data["service"], "dropbox");
    assert_eq!(data["server"], "dropbox");
    assert_eq!(data["user"], ACCOUNT);
    assert_eq!(data["device"], "desk");
    assert_eq!(data["root"], "save-relay");
    assert_eq!(data["credential_store"], "memory");

    // M6: the refresh token sits under token_account; the config names the service; the
    // waiting sign-in is used up; no token is in any file.
    let stored = machine
        .creds
        .get(&token_account(Service::Dropbox, ACCOUNT).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(stored.expose(), REFRESH_TOKEN);
    let config = machine.store.load_config().unwrap().unwrap();
    assert_eq!(config.server, "dropbox");
    assert_eq!(config.user, ACCOUNT);
    assert!(machine
        .creds
        .get(&pending_account(Service::Dropbox))
        .unwrap()
        .is_none());
    machine.assert_no_secret_on_disk(&[REFRESH_TOKEN, SIGN_IN_CODE, "fake-access-"]);
    assert!(
        fake.requests()
            .iter()
            .any(|request| request.contains("create_folder")),
        "the save-relay folder is made in the App folder"
    );
}

#[test]
fn drive_device_flow_asks_for_drive_file_only_and_stores_the_refresh_token() {
    let (fake, transport) = drive();
    let machine = Machine::new();

    let (code, envelope) = machine.json(
        &transport,
        "setup --service gdrive --no-browser --device desk",
        "",
    );
    assert_eq!(code, 0, "{envelope}");
    let data = &envelope["data"];
    assert_eq!(data["service"], "gdrive");
    assert_eq!(data["url"], "https://www.google.com/device");
    assert_eq!(data["code"], "ABCD-EFGH");
    assert_eq!(fake.device_scope().as_deref(), Some(DRIVE_SCOPE), "M10");

    let (code, envelope) = machine.json(
        &transport,
        "setup --service gdrive --finish --device desk",
        "",
    );
    assert_eq!(code, 0, "{envelope}");
    let data = &envelope["data"];
    assert_eq!(data["service"], "gdrive");
    assert_eq!(data["server"], "gdrive");
    assert_eq!(data["user"], ACCOUNT);

    let stored = machine
        .creds
        .get(&token_account(Service::Gdrive, ACCOUNT).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(stored.expose(), REFRESH_TOKEN);
    assert_eq!(
        machine
            .creds
            .get(&google_client_account(ACCOUNT))
            .unwrap()
            .unwrap()
            .expose(),
        "device",
        "the refresh token goes back to the device client"
    );
    assert_eq!(
        machine.store.load_config().unwrap().unwrap().server,
        "gdrive"
    );
    machine.assert_no_secret_on_disk(&[REFRESH_TOKEN, DEVICE_CODE, "fake-access-"]);
}

#[test]
fn status_json_carries_the_service_name() {
    let (_fake, transport) = dropbox();
    let machine = Machine::new();
    sign_in_dropbox(&machine, &transport);

    let (code, envelope) =
        machine.json(&transport, &format!("status {}", machine.game_flags()), "");
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(envelope["data"]["service"], "dropbox");

    let (code, out) = machine.run_in(
        &transport,
        &no_browser,
        false,
        &format!("status {}", machine.game_flags()),
        "",
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("Service: Dropbox\n"), "{out}");
}

#[test]
fn a_revoked_sign_in_gives_reauth_needed_and_exit_4() {
    let (fake, transport) = dropbox();
    let machine = Machine::new();
    sign_in_dropbox(&machine, &transport);
    fake.revoke();

    for line in [
        format!("status {}", machine.game_flags()),
        format!("pull {}", machine.game_flags()),
    ] {
        let (code, envelope) = machine.json(&transport, &line, "");
        assert_eq!(code, 4, "{line}: {envelope}");
        assert_eq!(envelope["error"]["code"], "reauth_needed", "{line}");
    }
}

#[test]
fn a_refused_code_or_no_waiting_sign_in_gives_sign_in_failed_exit_4() {
    let (_fake, transport) = dropbox();
    let machine = Machine::new();

    let (code, envelope) = machine.json(
        &transport,
        "setup --service dropbox --finish --device desk",
        &format!("{SIGN_IN_CODE}\n"),
    );
    assert_eq!(code, 4, "{envelope}");
    assert_eq!(envelope["error"]["code"], "sign_in_failed");

    let (code, _) = machine.json(
        &transport,
        "setup --service dropbox --no-browser --device desk",
        "",
    );
    assert_eq!(code, 0);
    let (code, envelope) = machine.json(
        &transport,
        "setup --service dropbox --finish --device desk",
        "not-the-code\n",
    );
    assert_eq!(code, 4, "{envelope}");
    assert_eq!(envelope["error"]["code"], "sign_in_failed");

    // Nothing was set up or stored, and the used-up sign-in is gone.
    assert!(machine.store.load_config().unwrap().is_none());
    assert!(machine
        .creds
        .get(&token_account(Service::Dropbox, ACCOUNT).unwrap())
        .unwrap()
        .is_none());
    assert!(machine
        .creds
        .get(&pending_account(Service::Dropbox))
        .unwrap()
        .is_none());
}

#[test]
fn setup_without_a_service_asks_and_takes_a_number_or_a_key() {
    let (_fake, transport) = drive();
    let machine = Machine::new();
    let opened = Mutex::new(Vec::<String>::new());
    let record = |url: &str| -> Result<()> {
        opened.lock().unwrap().push(url.to_string());
        Err(Error::SignInFailed("the test browser did not open".into()))
    };
    let browser: &dyn Fn(&str) -> Result<()> = &record;

    let (code, out) = machine.run_in(&transport, browser, false, "setup --device desk", "9\n");
    assert_eq!(code, 2, "{out}");
    assert!(out.starts_with("Where do you keep your saves?"), "{out}");
    assert!(out.contains("unknown service '9'"), "{out}");

    let (code, out) = machine.run_in(&transport, browser, false, "setup --device desk", "\n");
    assert_eq!(code, 2, "{out}");

    let (code, out) = machine.run_in(&transport, browser, false, "setup --device desk", "1\n");
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("--server"), "{out}");

    // "2" and "gdrive" both start the Google browser sign-in, which asks for drive.file only.
    for answer in ["2\n", "gdrive\n"] {
        let (code, out) = machine.run_in(&transport, browser, false, "setup --device desk", answer);
        assert_eq!(code, 4, "{out}");
        assert!(out.contains("sign-in failed"), "{out}");
    }
    // "3" and "dropbox" start the Dropbox browser sign-in, which asks for the App folder only.
    for answer in ["3\n", "dropbox\n"] {
        let (code, out) = machine.run_in(&transport, browser, false, "setup --device desk", answer);
        assert_eq!(code, 4, "{out}");
    }

    let urls = opened.lock().unwrap().clone();
    let scopes: Vec<String> = urls
        .iter()
        .map(|url| query(url, "scope").unwrap())
        .collect();
    assert_eq!(
        scopes,
        [DRIVE_SCOPE, DRIVE_SCOPE, DROPBOX_SCOPES, DROPBOX_SCOPES]
            .map(String::from)
            .to_vec()
    );
    assert!(machine.store.load_config().unwrap().is_none());
}

#[test]
fn game_mode_turns_the_browser_step_into_the_code_step() {
    let (fake, transport) = dropbox();
    let machine = Machine::new();
    let opened = Mutex::new(0);
    let record = |_: &str| -> Result<()> {
        *opened.lock().unwrap() += 1;
        Err(Error::SignInFailed("the test browser did not open".into()))
    };
    let browser: &dyn Fn(&str) -> Result<()> = &record;

    let (code, out) = machine.run_in(
        &transport,
        browser,
        true,
        "setup --service dropbox --device desk --json",
        "",
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(*opened.lock().unwrap(), 0, "no browser in Game Mode");
    assert_eq!(one_envelope(&out)["data"]["service"], "dropbox");
    assert!(fake.requests().is_empty());
}

#[test]
fn an_app_without_an_id_says_it_is_not_registered_yet() {
    let (_fake, transport) = dropbox();
    let machine = Machine::new();
    let unregistered = ClientSet {
        google_desktop: Client {
            id: "",
            secret: None,
        },
        google_device: Client {
            id: "",
            secret: None,
        },
        dropbox: Client {
            id: "",
            secret: None,
        },
    };
    for (line, label) in [
        ("setup --service dropbox --no-browser --json", "Dropbox"),
        ("setup --service gdrive --no-browser --json", "Google Drive"),
        ("setup --service gdrive --json", "Google Drive"),
    ] {
        let net = Net {
            transport: transport.clone(),
            open_browser: &no_browser,
            clients: unregistered,
            game_mode: false,
            clock,
        };
        let args: Vec<String> = line.split_whitespace().map(String::from).collect();
        let mut output = Vec::new();
        let code = run_with_net(
            &args,
            &mut "".as_bytes(),
            &mut output,
            &MemoryRemote::new(),
            &machine.creds,
            &machine.store,
            &machine.vars,
            T0,
            &net,
        );
        let envelope = one_envelope(&String::from_utf8(output).unwrap());
        assert_eq!(code, 2, "{line}");
        let message = envelope["error"]["message"].as_str().unwrap();
        assert!(message.contains(label), "{message}");
        assert!(message.contains("not registered yet"), "{message}");
    }
}

#[test]
fn switching_service_deletes_the_old_sign_in_and_starts_fresh() {
    let (_fake, dropbox_transport) = dropbox();
    let (_drive, drive_transport) = drive();
    let machine = Machine::new();
    sign_in_dropbox(&machine, &dropbox_transport);
    assert!(machine
        .creds
        .get(&token_account(Service::Dropbox, ACCOUNT).unwrap())
        .unwrap()
        .is_some());

    let (code, envelope) = machine.json(
        &drive_transport,
        "setup --service gdrive --no-browser --device desk",
        "",
    );
    assert_eq!(code, 0, "{envelope}");
    let (code, envelope) = machine.json(
        &drive_transport,
        "setup --service gdrive --finish --device desk",
        "",
    );
    assert_eq!(code, 0, "{envelope}");

    assert!(
        machine
            .creds
            .get(&token_account(Service::Dropbox, ACCOUNT).unwrap())
            .unwrap()
            .is_none(),
        "the old token is deleted"
    );
    assert!(machine
        .creds
        .get(&token_account(Service::Gdrive, ACCOUNT).unwrap())
        .unwrap()
        .is_some());
    assert_eq!(
        machine.store.load_config().unwrap().unwrap().server,
        "gdrive"
    );
}

#[test]
fn a_refresh_token_the_service_rotates_is_stored_after_the_command() {
    let (fake, transport) = dropbox();
    let machine = Machine::new();
    sign_in_dropbox(&machine, &transport);
    assert_eq!(
        machine
            .creds
            .get(&token_account(Service::Dropbox, ACCOUNT).unwrap())
            .unwrap()
            .unwrap()
            .expose(),
        REFRESH_TOKEN
    );

    // The next command's token refresh comes back with a new refresh token.
    let rotating: Arc<dyn Transport> = Arc::new(Rotating(fake));
    let (code, envelope) = machine.json(&rotating, &format!("status {}", machine.game_flags()), "");
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(
        machine
            .creds
            .get(&token_account(Service::Dropbox, ACCOUNT).unwrap())
            .unwrap()
            .unwrap()
            .expose(),
        ROTATED
    );
}
