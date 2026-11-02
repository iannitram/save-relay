use save_relay::creds::{account, CredentialStore, MemoryStore, Secret};
use save_relay::Error;

const PASSWORD: &str = "Xk3-q9Lm-77Zt-pPw1";
const ACCOUNT: &str = "me@cloud.example.org";

#[test]
fn account_is_user_at_lowercased_host_with_port() {
    let a = |s: &str, u: &str| account(s, u).unwrap();
    assert_eq!(
        a("https://Cloud.Example.org:8443/nc", "me"),
        "me@cloud.example.org:8443"
    );
    assert_eq!(a("https://cloud.example.org", "me"), "me@cloud.example.org");
    assert_eq!(
        a("http://192.168.1.20/", "Player 2"),
        "Player 2@192.168.1.20"
    );
    assert_eq!(
        a("https://CLOUD.example.org/a/b/", "me"),
        "me@cloud.example.org"
    );
}

#[test]
fn account_needs_http_or_https() {
    for bad in ["cloud.example.org", "ftp://cloud.example.org", ""] {
        assert!(matches!(account(bad, "me"), Err(Error::Usage(_))), "{bad}");
    }
}

#[test]
fn memory_store_set_get_replace_delete() {
    let store = MemoryStore::new();
    assert_eq!(store.get(ACCOUNT).unwrap(), None);
    store.set(ACCOUNT, &Secret::new(PASSWORD)).unwrap();
    let got = store.get(ACCOUNT).unwrap().unwrap();
    assert_eq!(got.expose(), PASSWORD);
    assert_eq!(store.get("other@cloud.example.org").unwrap(), None);
    store.set(ACCOUNT, &Secret::new("second")).unwrap();
    assert_eq!(store.get(ACCOUNT).unwrap().unwrap().expose(), "second");
    store.delete(ACCOUNT).unwrap();
    assert_eq!(store.get(ACCOUNT).unwrap(), None);
    store.delete(ACCOUNT).unwrap();
    assert_eq!(store.kind(), "memory");
}

#[test]
fn accounts_are_kept_apart() {
    let store = MemoryStore::new();
    store.set("a@h", &Secret::new("one")).unwrap();
    store.set("b@h", &Secret::new("two")).unwrap();
    store.delete("a@h").unwrap();
    assert_eq!(store.get("a@h").unwrap(), None);
    assert_eq!(store.get("b@h").unwrap().unwrap().expose(), "two");
}

#[test]
fn a_stored_secret_never_prints() {
    let store = MemoryStore::new();
    store.set(ACCOUNT, &Secret::new(PASSWORD)).unwrap();
    let got = store.get(ACCOUNT).unwrap();
    for text in [format!("{got:?}"), format!("{:?}", store)] {
        assert!(!text.contains(PASSWORD), "{text}");
    }
    let secret = got.unwrap();
    assert_eq!(format!("{secret}"), "***");
    assert_eq!(format!("{secret:?}"), "Secret(***)");
}
