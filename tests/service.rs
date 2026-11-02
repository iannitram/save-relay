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
