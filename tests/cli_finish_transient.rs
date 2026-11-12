//! #250 item 7: `setup --finish` keeps the waiting sign-in when the failure is transient
//! (the network was unreachable), so the person can run it again.
use save_relay::cli::{run_with_net, Net};
use save_relay::clients::{Client, ClientSet};
use save_relay::creds::{CredentialStore, MemoryStore};
use save_relay::fake::{FakeDropbox, SIGN_IN_CODE};
use save_relay::http::{Request, Response, Transport};
use save_relay::local::LocalStore;
use save_relay::platform::{AppDirs, PathVars};
use save_relay::remote::MemoryRemote;
use save_relay::service::{pending_account, Service};
use save_relay::{Error, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const T0: u64 = 1_793_476_260;

/// The Dropbox fake, unless `down` is set: then every request is unreachable.
struct Switch {
    fake: FakeDropbox,
    down: AtomicBool,
}

impl Transport for Switch {
    fn send(&self, request: &Request) -> Result<Response> {
        if self.down.load(Ordering::SeqCst) {
            return Err(Error::Unreachable("the network is down".into()));
        }
        self.fake.send(request)
    }
}

fn no_browser(_: &str) -> Result<()> {
    Err(Error::SignInFailed("no browser".into()))
}

#[test]
fn an_unreachable_network_keeps_the_waiting_sign_in() {
    let root = std::env::temp_dir().join(format!("save-relay-finish-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let vars = PathVars {
        home: root.join("home"),
        data: root.join("data"),
        config: root.join("config"),
        documents: None,
    };
    let store = LocalStore::new(AppDirs {
        config: root.join("config/save-relay"),
        data: root.join("data/save-relay"),
    });
    let creds = MemoryStore::new();
    let switch = Arc::new(Switch {
        fake: FakeDropbox::new(),
        down: AtomicBool::new(false),
    });
    let transport: Arc<dyn Transport> = switch.clone();
    let run = |line: &str, stdin: &str| {
        let net = Net {
            transport: transport.clone(),
            open_browser: &no_browser,
            clients: ClientSet {
                google_desktop: Client {
                    id: "d",
                    secret: None,
                },
                google_device: Client {
                    id: "v",
                    secret: None,
                },
                dropbox: Client {
                    id: "k",
                    secret: None,
                },
            },
            game_mode: false,
            clock: || T0,
        };
        let args: Vec<String> = line.split_whitespace().map(String::from).collect();
        let mut input = stdin.as_bytes();
        let mut output = Vec::new();
        let code = run_with_net(
            &args,
            &mut input,
            &mut output,
            &MemoryRemote::new(),
            &creds,
            &store,
            &vars,
            T0,
            &net,
        );
        (code, String::from_utf8(output).unwrap())
    };

    let (code, out) = run(
        "setup --service dropbox --no-browser --device desk --json",
        "",
    );
    assert_eq!(code, 0, "{out}");
    switch.down.store(true, Ordering::SeqCst);
    let (code, _) = run(
        "setup --service dropbox --finish --device desk --json",
        &format!("{SIGN_IN_CODE}\n"),
    );
    assert_ne!(code, 0);
    assert!(
        creds
            .get(&pending_account(Service::Dropbox))
            .unwrap()
            .is_some(),
        "the waiting sign-in must survive a transient failure"
    );

    switch.down.store(false, Ordering::SeqCst);
    let (code, out) = run(
        "setup --service dropbox --finish --device desk --json",
        &format!("{SIGN_IN_CODE}\n"),
    );
    assert_eq!(code, 0, "{out}");
}
