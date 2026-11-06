//! Real round trips against Dropbox and Google Drive with the owner's own sign-in (F46, M9, M10).
//! Run by hand on a PC after `save-relay setup --service <s>` (docs/services.md, "How to run");
//! agents never sign in, so every test here is `#[ignore]` and skips with a message when its
//! config folder variable is unset.
//!
//! Each variable names a save-relay config folder (the one holding `config.json`). The sign-in
//! comes from the system credential store, as the CLI reads it. Every test works in its own
//! folder on the service, `save-relay-live-test-<tag>`, deleted at the start and at the end, so
//! the owner's real saves are never touched. Tokens are never printed.
//!
//! The scenarios mirror `tests/remote_contract.rs` and `tests/sync_services.rs`: two machines
//! (two config folders, two saves folders) over one remote.

#![cfg(all(feature = "http", feature = "keyring"))]

use save_relay::api::{Action, SyncData};
use save_relay::clients::{ClientSet, GOOGLE_DESKTOP};
use save_relay::creds::{CredentialStore, KeyringStore};
use save_relay::gdrive::API;
use save_relay::http::{Request, Response, Transport, UreqTransport};
use save_relay::layout::Layout;
use save_relay::local::{Config, LocalStore};
use save_relay::manifest::Manifest;
use save_relay::oauth::{authorize_url, form_encode, TokenSource, GOOGLE};
use save_relay::platform::AppDirs;
use save_relay::remote::{ensure_dir, Precondition, Remote};
use save_relay::service::{self, token_account, Service};
use save_relay::state::ServerState;
use save_relay::sync::{self, Game, Resolve, Session};
use save_relay::{Error, Result};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const GDRIVE_VAR: &str = "SAVE_RELAY_LIVE_GDRIVE_CONFIG";
const DROPBOX_VAR: &str = "SAVE_RELAY_LIVE_DROPBOX_CONFIG";
const GAME: &str = "fixture";
const ANY: &Precondition = &Precondition::Any;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// The folder a test works in on the service.
fn live_root(tag: &str) -> String {
    format!("save-relay-live-test-{tag}")
}

/// The real transport behind a switch: when off, every request is unreachable (M5), so the
/// offline scenario runs against the real service's state without leaving the machine.
struct Gate {
    inner: Arc<dyn Transport>,
    open: AtomicBool,
}

impl Gate {
    fn set(&self, up: bool) {
        self.open.store(up, Ordering::SeqCst);
    }
}

impl Transport for Gate {
    fn send(&self, request: &Request) -> Result<Response> {
        if self.open.load(Ordering::SeqCst) {
            self.inner.send(request)
        } else {
            Err(Error::Unreachable("test switch is off".into()))
        }
    }
}

/// One owner's sign-in on one service, opened the way the CLI opens it.
struct Live {
    remote: Box<dyn Remote>,
    tokens: Option<TokenSource>,
    gate: Arc<Gate>,
    config: Config,
    creds: KeyringStore,
    service: Service,
}

/// Opens the service named by `var`'s config folder; `None` (with a message) when unset.
#[allow(clippy::arc_with_non_send_sync)]
fn open_live(var: &str, service: Service) -> Option<Live> {
    let Some(dir) = env::var_os(var) else {
        eprintln!("skipped: {var} is not set (see docs/services.md, How to run)");
        return None;
    };
    let store = LocalStore::new(AppDirs {
        config: PathBuf::from(dir),
        data: env::temp_dir(),
    });
    let config = store
        .load_config()
        .expect("the config folder can be read")
        .expect("the config folder has config.json: run setup first");
    assert_eq!(
        Service::from_server(&config.server).unwrap().key(),
        service.key(),
        "{var} names another service"
    );
    let gate = Arc::new(Gate {
        inner: Arc::new(UreqTransport::new()),
        open: AtomicBool::new(true),
    });
    let transport: Arc<dyn Transport> = gate.clone();
    let creds = KeyringStore::new();
    let opened = service::open_with(&config, &creds, transport, now, &ClientSet::shipped())
        .expect("the sign-in from setup is in the credential store");
    Some(Live {
        remote: opened.remote,
        tokens: opened.tokens,
        gate,
        config,
        creds,
        service,
    })
}

impl Drop for Live {
    fn drop(&mut self) {
        // A rotated refresh token replaces the stored one, as the CLI does after a command.
        let rotated = self.tokens.as_ref().and_then(TokenSource::rotated);
        if let Some(secret) = rotated {
            if let Ok(account) = token_account(self.service, &self.config.user) {
                let _ = self.creds.set(&account, &secret);
            }
        }
    }
}

/// Deletes the test's folder on the service when the test ends, even after a failure.
struct Cleanup<'a> {
    remote: &'a dyn Remote,
    root: String,
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let _ = self.remote.delete(&self.root);
    }
}

/// Clears a folder left by an earlier run, then guards this run's folder.
fn fresh<'a>(remote: &'a dyn Remote, root: &str) -> Cleanup<'a> {
    let _ = remote.delete(root);
    Cleanup {
        remote,
        root: root.into(),
    }
}

/// One machine: its own config folder (the owner's config under this machine's name), its own
/// saves folder and local records, and its own clock.
struct Machine {
    dir: PathBuf,
    root: String,
    game: Game,
    store: LocalStore,
    device: String,
    clock: Cell<u64>,
}

impl Machine {
    fn new(tag: &str, device: &str, config: &Config, root: &str) -> Machine {
        let dir = env::temp_dir().join(format!("save-relay-live-{tag}-{device}"));
        let _ = fs::remove_dir_all(&dir);
        let store = LocalStore::new(AppDirs {
            config: dir.join("config"),
            data: dir.join("data"),
        });
        store
            .save_config(&Config {
                device: device.into(),
                root: root.into(),
                ..config.clone()
            })
            .unwrap();
        let manifest = Manifest::from_json(
            r#"{"format":1,"id":"fixture","name":"Fixture",
                "saves_from":"launcher","saves":{},
                "include":["**/*.sav"],"exclude":["skip/**"]}"#,
        )
        .unwrap();
        Machine {
            game: Game {
                manifest,
                saves_dir: dir.join("saves"),
            },
            store,
            device: device.into(),
            clock: Cell::new(now()),
            root: root.into(),
            dir,
        }
    }

    fn tick(&self) -> u64 {
        let next = self.clock.get() + 1;
        self.clock.set(next);
        next
    }

    fn session<'a>(&self, remote: &'a dyn Remote) -> Session<'a> {
        Session {
            remote,
            layout: Layout::new(&self.root).unwrap(),
            device: self.device.clone(),
            now: self.tick(),
        }
    }

    /// Replace this machine's one save file with `content`.
    fn write(&self, content: &str) {
        fs::create_dir_all(&self.game.saves_dir).unwrap();
        let path = self.game.saves_dir.join("a.sav");
        fs::write(&path, content).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(now() - 500))
            .unwrap();
    }

    fn saves(&self) -> String {
        fs::read_to_string(self.game.saves_dir.join("a.sav")).unwrap()
    }

    fn pull(&self, remote: &dyn Remote, resolve: Option<Resolve>) -> Result<SyncData> {
        let session = self.session(remote);
        sync::pull(&session, &self.game, &self.store, resolve, false).map(|report| report.sync)
    }

    fn push(&self, remote: &dyn Remote, resolve: Option<Resolve>) -> Result<SyncData> {
        let session = self.session(remote);
        sync::push(&session, &self.game, &self.store, resolve).map(|report| report.sync)
    }

    fn restore(&self, remote: &dyn Remote, entry: &str) -> save_relay::api::RestoreData {
        let session = self.session(remote);
        sync::restore(&session, &self.game, &self.store, entry).unwrap()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The server's `state.json` under `root`.
fn server_state(r: &dyn Remote, root: &str) -> ServerState {
    let (bytes, _) = r.get(&Layout::new(root).unwrap().state_path(GAME)).unwrap();
    ServerState::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap()
}

/// The save file inside history entry `entry`.
fn entry_saves(r: &dyn Remote, root: &str, entry: &str) -> String {
    let dir = Layout::new(root).unwrap().entry_dir(GAME, entry);
    String::from_utf8(r.get(&format!("{dir}/a.sav")).unwrap().0).unwrap()
}

/// The Remote contract on the real service: bytes and etags, preconditions, listing, delete.
fn contract(tag: &str, live: &Live) {
    let root = live_root(tag);
    let r: &dyn Remote = live.remote.as_ref();
    let _clean = fresh(r, &root);
    ensure_dir(r, &format!("{root}/c")).unwrap();
    let file = format!("{root}/c/a.txt");
    let first = r.put(&file, b"A", ANY).unwrap();
    assert_eq!(r.get(&file).unwrap(), (b"A".to_vec(), first.clone()));
    let second = r.put(&file, b"B", ANY).unwrap();
    assert_ne!(first, second, "every put changes the etag");
    assert_eq!(
        r.put(&file, b"X", &Precondition::Match(first)),
        Err(Error::Changed)
    );
    assert_eq!(
        r.put(&file, b"X", &Precondition::Absent),
        Err(Error::Changed)
    );
    r.put(&file, b"C", &Precondition::Match(second)).unwrap();
    assert_eq!(r.get(&file).unwrap().0, b"C".to_vec());
    let names: Vec<String> = r
        .list(&format!("{root}/c"))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(names, ["a.txt"]);
    r.delete(&file).unwrap();
    assert!(matches!(r.get(&file), Err(Error::NotFound(_))));
}

/// M1-M3 and M5 on the real service: push, pull on a second config folder, conflict with
/// keep-local, restore, and an offline pull and deferred push settled once back online.
fn round_trip(tag: &str, live: &Live) {
    let root = live_root(tag);
    let r: &dyn Remote = live.remote.as_ref();
    let _clean = fresh(r, &root);
    let one = Machine::new(tag, "device-one", &live.config, &root);
    let two = Machine::new(tag, "device-two", &live.config, &root);

    one.write("first");
    assert_eq!(one.push(r, None).unwrap().action, Action::Pushed);
    assert_eq!(two.pull(r, None).unwrap().action, Action::Pulled);
    assert_eq!(two.saves(), "first");
    assert_eq!(server_state(r, &root).revision, 1);
    // Closing the game clears machine two's mark, so machine one is not held.
    assert_eq!(two.push(r, None).unwrap().action, Action::Noop);
    assert_eq!(one.pull(r, None).unwrap().action, Action::Noop);

    one.write("one-edited");
    assert_eq!(one.push(r, None).unwrap().action, Action::Pushed);
    let losing = server_state(r, &root).entry;
    two.write("two-edited");
    assert_eq!(two.pull(r, None).unwrap().action, Action::Conflict);
    let kept = two.push(r, Some(Resolve::KeepLocal)).unwrap();
    assert_eq!(kept.action, Action::Pushed);
    assert_eq!(kept.backup.as_deref(), Some(losing.as_str()));
    assert_eq!(entry_saves(r, &root, &losing), "one-edited");
    assert_eq!(one.pull(r, None).unwrap().action, Action::Pulled);
    assert_eq!(one.saves(), "two-edited");

    // Restore brings the older save back as a new current revision (revision 4 here).
    let restored = one.restore(r, &losing);
    assert_eq!(restored.revision, 4);
    assert_eq!(one.saves(), "one-edited");
    assert_eq!(server_state(r, &root).entry, restored.new_entry);
    assert_eq!(entry_saves(r, &root, &restored.new_entry), "one-edited");

    live.gate.set(false);
    assert_eq!(two.pull(r, None).unwrap().action, Action::Offline);
    two.write("offline-edit");
    assert_eq!(two.push(r, None).unwrap().action, Action::Deferred);
    assert_eq!(two.saves(), "offline-edit");

    live.gate.set(true);
    one.write("one-after");
    assert_eq!(one.push(r, None).unwrap().action, Action::Pushed);
    let before = server_state(r, &root).entry;
    assert_eq!(two.push(r, None).unwrap().action, Action::Conflict);
    let caught = two.push(r, Some(Resolve::KeepLocal)).unwrap();
    assert_eq!(caught.action, Action::Pushed);
    assert_eq!(caught.backup.as_deref(), Some(before.as_str()));
    assert_eq!(entry_saves(r, &root, &before), "one-after");
    let now_entry = server_state(r, &root).entry;
    assert_eq!(entry_saves(r, &root, &now_entry), "offline-edit");
}

/// A remote that runs `before_state` once, just before the first write of `state.json` reaches
/// the inner backend: the other machine's push slips in between this machine's read and write.
struct Racing<'a> {
    inner: &'a dyn Remote,
    state: String,
    before_state: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
}

impl Remote for Racing<'_> {
    fn list(&self, dir: &str) -> Result<Vec<save_relay::remote::RemoteEntry>> {
        self.inner.list(dir)
    }
    fn get(&self, path: &str) -> Result<(Vec<u8>, String)> {
        self.inner.get(path)
    }
    fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> Result<String> {
        if path == self.state {
            let hook = self.before_state.borrow_mut().take();
            if let Some(hook) = hook {
                hook();
            }
        }
        self.inner.put(path, bytes, pre)
    }
    fn mkcol(&self, dir: &str) -> Result<()> {
        self.inner.mkcol(dir)
    }
    fn copy(&self, from: &str, to: &str) -> Result<()> {
        self.inner.copy(from, to)
    }
    fn move_to(&self, from: &str, to: &str) -> Result<()> {
        self.inner.move_to(from, to)
    }
    fn delete(&self, path: &str) -> Result<()> {
        self.inner.delete(path)
    }
}

/// M9 on the real service: two pushes from the same revision; the first to write `state.json`
/// wins, and the second gets `Changed` and keeps its saves for the conflict prompt.
fn racing_pushes(tag: &str, live: &Live) {
    let root = live_root(tag);
    let r: &dyn Remote = live.remote.as_ref();
    let _clean = fresh(r, &root);
    let one = Machine::new(tag, "device-one", &live.config, &root);
    let two = Machine::new(tag, "device-two", &live.config, &root);
    one.write("base");
    assert_eq!(one.push(r, None).unwrap().action, Action::Pushed);
    assert_eq!(two.pull(r, None).unwrap().action, Action::Pulled);
    one.write("one-race");
    two.write("two-race");

    let one_outcome: RefCell<Option<Result<SyncData>>> = RefCell::new(None);
    let racing = Racing {
        inner: r,
        state: Layout::new(&root).unwrap().state_path(GAME),
        before_state: RefCell::new(Some(Box::new(|| {
            *one_outcome.borrow_mut() = Some(one.push(r, None));
        }))),
    };
    assert_eq!(two.push(&racing, None).unwrap_err(), Error::Changed);

    let won = one_outcome
        .borrow_mut()
        .take()
        .expect("the first machine pushed inside the race")
        .unwrap();
    assert_eq!(won.action, Action::Pushed);
    let now = server_state(r, &root);
    assert_eq!(now.revision, 2);
    assert_eq!(now.device, "device-one");
    assert_eq!(entry_saves(r, &root, &now.entry), "one-race");
    assert_eq!(two.saves(), "two-race");
    assert_eq!(two.pull(r, None).unwrap().action, Action::Conflict);
}

/// Runs one scenario on the service named by `var`, skipping when its config folder is unset.
fn run(var: &str, service: Service, tag: &str, scenario: fn(&str, &Live)) {
    if let Some(live) = open_live(var, service) {
        scenario(tag, &live);
    }
}

#[test]
#[ignore = "live: needs SAVE_RELAY_LIVE_GDRIVE_CONFIG and the owner's Google sign-in"]
fn gdrive_contract() {
    run(GDRIVE_VAR, Service::Gdrive, "gdrive-contract", contract);
}

#[test]
#[ignore = "live: needs SAVE_RELAY_LIVE_GDRIVE_CONFIG and the owner's Google sign-in"]
fn gdrive_round_trip() {
    run(GDRIVE_VAR, Service::Gdrive, "gdrive-round", round_trip);
}

#[test]
#[ignore = "live: needs SAVE_RELAY_LIVE_GDRIVE_CONFIG and the owner's Google sign-in"]
fn gdrive_racing_pushes() {
    run(GDRIVE_VAR, Service::Gdrive, "gdrive-race", racing_pushes);
}

#[test]
#[ignore = "live: needs SAVE_RELAY_LIVE_DROPBOX_CONFIG and the owner's Dropbox sign-in"]
fn dropbox_contract() {
    run(DROPBOX_VAR, Service::Dropbox, "dropbox-contract", contract);
}

#[test]
#[ignore = "live: needs SAVE_RELAY_LIVE_DROPBOX_CONFIG and the owner's Dropbox sign-in"]
fn dropbox_round_trip() {
    run(DROPBOX_VAR, Service::Dropbox, "dropbox-round", round_trip);
}

#[test]
#[ignore = "live: needs SAVE_RELAY_LIVE_DROPBOX_CONFIG and the owner's Dropbox sign-in"]
fn dropbox_racing_pushes() {
    run(DROPBOX_VAR, Service::Dropbox, "dropbox-race", racing_pushes);
}

/// Sends `request` with the session's access token, through the real transport.
fn send(live: &Live, request: Request) -> Result<Response> {
    let token = live.tokens.as_ref().ok_or(Error::NotSetUp)?.bearer()?;
    live.gate
        .send(&request.header("Authorization", &format!("Bearer {}", token.expose())))
}

/// `files.create` with the given id and no parent (My Drive), as the claim step sends it.
fn create(live: &Live, id: &str) -> Result<Response> {
    let body = json!({"id": id, "name": format!("save-relay-live-409-{id}")});
    send(
        live,
        Request::new("POST", &format!("{API}/files"))
            .header("Content-Type", "application/json")
            .body(body.to_string().into_bytes()),
    )
}

/// Deletes the test's claim file when the test ends, even after a failure.
struct Trash<'a> {
    live: &'a Live,
    id: String,
}

impl Drop for Trash<'_> {
    fn drop(&mut self) {
        let _ = send(
            self.live,
            Request::new("DELETE", &format!("{API}/files/{}", self.id)),
        );
    }
}

/// The one Drive fact the M9 guard rests on (docs/adr/0002-storage-backends.md): a second
/// `files.create` with an id already in use answers 409. The id comes from `files.generateIds`
/// in one session; a second session (its own token source, as a second machine has) tries it.
#[test]
#[ignore = "live: needs SAVE_RELAY_LIVE_GDRIVE_CONFIG and the owner's Google sign-in"]
fn gdrive_refuses_a_reused_claim_id() {
    let Some(first) = open_live(GDRIVE_VAR, Service::Gdrive) else {
        return;
    };
    let Some(second) = open_live(GDRIVE_VAR, Service::Gdrive) else {
        return;
    };
    let ids = send(
        &first,
        Request::new(
            "GET",
            &format!("{API}/files/generateIds?count=1&space=drive&type=files"),
        ),
    )
    .unwrap();
    assert_eq!(ids.status, 200, "files.generateIds");
    let id = serde_json::from_slice::<Value>(&ids.body).unwrap()["ids"][0]
        .as_str()
        .unwrap()
        .to_string();
    let _trash = Trash {
        live: &first,
        id: id.clone(),
    };
    let made = create(&first, &id).unwrap();
    assert!(
        (200..300).contains(&made.status),
        "the first create with a fresh id answered {}",
        made.status
    );
    assert_eq!(
        create(&second, &id).unwrap().status,
        409,
        "a second create with the same id, from another session, must answer 409"
    );
    assert_eq!(
        create(&first, &id).unwrap().status,
        409,
        "and the same again from the first session"
    );
}

/// M10, the part a URL can show: Google's consent asks for `drive.file` and nothing else. The
/// screen's own wording (and Dropbox's App folder name) is looked at by the owner.
#[test]
#[ignore = "live: builds the Google sign-in URL only; the owner checks the consent screen"]
fn google_consent_asks_for_drive_file_only() {
    let url = authorize_url(&GOOGLE, GOOGLE_DESKTOP.id, "http://127.0.0.1:1/", "c", "s");
    assert!(url.contains(&form_encode(&[(
        "scope",
        "https://www.googleapis.com/auth/drive.file"
    )])));
    assert_eq!(GOOGLE.scope.split(' ').count(), 1);
}
