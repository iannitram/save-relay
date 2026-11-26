use save_relay::service::{token_account, Service};
use save_relay::Error;

#[test]
fn keys_and_labels_in_setup_order() {
    let got: Vec<(&str, &str)> = Service::ALL.iter().map(|s| (s.key(), s.label())).collect();
    let want = [
        ("nextcloud", "Nextcloud"),
        ("gdrive", "Google Drive"),
        ("dropbox", "Dropbox"),
    ];
    assert_eq!(got, want);
}

#[test]
fn parse_takes_exactly_a_key() {
    for s in Service::ALL {
        assert_eq!(Service::parse(s.key()), Ok(s));
    }
    for bad in [
        "",
        "Gdrive",
        "google-drive",
        "drive",
        " dropbox",
        "nextcloud ",
    ] {
        match Service::parse(bad) {
            Err(Error::Usage(m)) => {
                assert!(m.contains(&format!("'{bad}'")), "{m}");
                assert!(m.contains("nextcloud, gdrive or dropbox"), "{m}");
            }
            other => panic!("{bad:?}: expected a usage error, got {other:?}"),
        }
    }
}

#[test]
fn from_server_reads_the_config_field() {
    let nc = Ok(Service::Nextcloud);
    assert_eq!(Service::from_server("https://cloud.example.org"), nc);
    assert_eq!(Service::from_server("http://192.168.1.20:8080/nc"), nc);
    assert_eq!(Service::from_server("gdrive"), Ok(Service::Gdrive));
    assert_eq!(Service::from_server("dropbox"), Ok(Service::Dropbox));
    for bad in [
        "",
        "cloud.example.org",
        "GDRIVE",
        "dropbox ",
        "ftp://x",
        "nextcloud",
    ] {
        match Service::from_server(bad) {
            Err(Error::StateInvalid(m)) => {
                assert_eq!(m, format!("config: unknown server '{bad}'"));
            }
            other => panic!("{bad:?}: expected state_invalid, got {other:?}"),
        }
    }
}

#[test]
fn only_drive_and_dropbox_sign_in_through_the_browser() {
    assert!(!Service::Nextcloud.uses_oauth());
    assert!(Service::Gdrive.uses_oauth());
    assert!(Service::Dropbox.uses_oauth());
}

#[test]
fn token_accounts() {
    let got = token_account(Service::Gdrive, "me@example.org").unwrap();
    assert_eq!(got, "gdrive:me@example.org");
    let got = token_account(Service::Dropbox, "Player 2@example.org").unwrap();
    assert_eq!(got, "dropbox:Player 2@example.org");
    let nc = token_account(Service::Nextcloud, "me");
    assert!(matches!(nc, Err(Error::Usage(_))), "{nc:?}");
    let empty = token_account(Service::Gdrive, "");
    assert!(matches!(empty, Err(Error::Usage(_))), "{empty:?}");
}

#[test]
fn serializes_as_its_key() {
    for s in Service::ALL {
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(text, format!("\"{}\"", s.key()));
        assert_eq!(serde_json::from_str::<Service>(&text).unwrap(), s);
    }
}

#[test]
fn game_sources_open_for_every_service_and_need_its_sign_in() {
    use save_relay::creds::MemoryStore;
    use save_relay::local::Config;
    use save_relay::service::open_game_source;
    use std::sync::Arc;

    struct Silent;
    impl save_relay::http::Transport for Silent {
        fn send(
            &self,
            _: &save_relay::http::Request,
        ) -> save_relay::Result<save_relay::http::Response> {
            Err(Error::Unreachable("no network in this test".into()))
        }
    }
    let config = |server: &str| Config {
        format: 1,
        server: server.into(),
        user: "me@example.org".into(),
        root: "save-relay".into(),
        device: "pc".into(),
    };
    let clients = save_relay::clients::ClientSet::shipped();
    let open = |server: &str| {
        open_game_source(
            &config(server),
            &MemoryStore::new(),
            Arc::new(Silent),
            save_relay::stamp::now,
            &clients,
        )
        .map(|_| ())
    };
    // All three services have a source (#309): without a stored sign-in each is "not set up",
    // never "range unsupported", and no request is sent.
    for server in ["https://cloud.example.org", "gdrive", "dropbox"] {
        assert!(
            matches!(open(server), Err(Error::NotSetUp)),
            "{server}: {:?}",
            open(server)
        );
    }
    use save_relay::creds::{CredentialStore, Secret};
    let signed_in = |server: &str, account: &str| {
        let store = MemoryStore::new();
        store.set(account, &Secret::new("stored")).unwrap();
        open_game_source(
            &config(server),
            &store,
            Arc::new(Silent),
            save_relay::stamp::now,
            &clients,
        )
        .map(|opened| opened.source.service())
    };
    let nextcloud =
        save_relay::creds::account("https://cloud.example.org", "me@example.org").unwrap();
    assert_eq!(
        signed_in("https://cloud.example.org", &nextcloud).unwrap(),
        "nextcloud"
    );
    assert_eq!(
        signed_in("gdrive", "gdrive:me@example.org").unwrap(),
        "gdrive"
    );
    assert_eq!(
        signed_in("dropbox", "dropbox:me@example.org").unwrap(),
        "dropbox"
    );
}
