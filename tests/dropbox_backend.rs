use save_relay::clients::Client;
use save_relay::creds::Secret;
use save_relay::dropbox::Dropbox;
use save_relay::fake::{FakeDropbox, REFRESH_TOKEN};
use save_relay::http::{Request, Response, Transport};
use save_relay::oauth::{TokenSource, DROPBOX};
use save_relay::remote::{ensure_dir, Precondition, Remote};
use save_relay::Error;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn tokens(transport: Arc<dyn Transport>) -> TokenSource {
    TokenSource::new(
        &DROPBOX,
        Client {
            id: "test",
            secret: None,
        },
        Secret::new(REFRESH_TOKEN),
        transport,
        || 1000,
    )
}

struct Harness {
    backend: Dropbox,
    fake: Arc<FakeDropbox>,
}
impl Harness {
    fn new() -> Self {
        let fake = Arc::new(FakeDropbox::new());
        let backend = Dropbox::with_sleep(tokens(fake.clone()), fake.clone(), |_| {
            panic!("unexpected retry")
        });
        Self { backend, fake }
    }
    fn file_paths(&self) -> Vec<String> {
        self.fake.file_paths()
    }
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.get(path).ok().map(|v| v.0)
    }
    fn set_reachable(&self, value: bool) {
        self.fake.set_reachable(value);
    }
}
impl std::ops::Deref for Harness {
    type Target = Dropbox;
    fn deref(&self) -> &Dropbox {
        &self.backend
    }
}

const ANY: &Precondition = &Precondition::Any;

/// A server holding `g/a.txt` ("A"), `g/sub/b.txt` ("BB") and an empty folder `g/empty`.
fn sample() -> Harness {
    let r = Harness::new();
    for d in ["g", "g/sub", "g/empty"] {
        r.mkcol(d).unwrap();
    }
    r.put("g/a.txt", b"A", ANY).unwrap();
    r.put("g/sub/b.txt", b"BB", ANY).unwrap();
    r
}

#[test]
fn put_then_get_and_every_put_changes_the_etag() {
    let r = sample();
    let first = r.put("g/c.bin", &[0, 1, 2], ANY).unwrap();
    assert_eq!(r.get("g/c.bin").unwrap(), (vec![0, 1, 2], first.clone()));
    let second = r.put("g/c.bin", &[0, 1, 2], ANY).unwrap();
    assert_ne!(first, second);
    assert_eq!(r.get("g/c.bin").unwrap().1, second);
}

#[test]
fn preconditions_match_and_absent() {
    let r = sample();
    let etag = r.get("g/a.txt").unwrap().1;
    let stale = Precondition::Match("\"nope\"".to_string());
    assert_eq!(r.put("g/a.txt", b"X", &stale), Err(Error::Changed));
    assert_eq!(
        r.put("g/a.txt", b"X", &Precondition::Absent),
        Err(Error::Changed)
    );
    assert_eq!(r.put("g/new.txt", b"N", &stale), Err(Error::Changed));
    assert_eq!(r.read("g/a.txt").unwrap(), b"A");
    r.put("g/a.txt", b"Y", &Precondition::Match(etag)).unwrap();
    r.put("g/new.txt", b"N", &Precondition::Absent).unwrap();
    assert_eq!(r.read("g/a.txt").unwrap(), b"Y");
    assert_eq!(r.read("g/new.txt").unwrap(), b"N");
}

#[test]
fn missing_things_are_not_found() {
    let r = sample();
    let nf = |p: &str| Error::NotFound(p.to_string());
    assert_eq!(r.put("x/y/z.txt", b"1", ANY).unwrap_err(), nf("x/y"));
    assert_eq!(r.mkcol("x/y").unwrap_err(), nf("x"));
    assert_eq!(r.get("g/none.txt").unwrap_err(), nf("g/none.txt"));
    assert_eq!(r.get("g/sub").unwrap_err(), nf("g/sub"));
    assert_eq!(r.list("nope").unwrap_err(), nf("nope"));
    assert_eq!(r.delete("g/none").unwrap_err(), nf("g/none"));
    assert_eq!(r.copy("g/none", "g/n2").unwrap_err(), nf("g/none"));
    assert_eq!(r.move_to("g/a.txt", "q/a.txt").unwrap_err(), nf("q"));
}

#[test]
fn mkcol_is_idempotent_but_not_over_a_file() {
    let r = sample();
    r.mkcol("g/sub").unwrap();
    assert_eq!(r.read("g/sub/b.txt").unwrap(), b"BB");
    assert!(matches!(r.mkcol("g/a.txt"), Err(Error::Server(_))));
}

type Row = (String, bool, u64, Option<String>);

/// `list` as (name, is_dir, size, etag) rows.
fn rows(r: &Harness, dir: &str) -> Vec<Row> {
    let list = r.list(dir).unwrap().into_iter();
    list.map(|e| (e.name, e.is_dir, e.size, e.etag)).collect()
}

#[test]
fn list_gives_direct_children_sorted() {
    let r = sample();
    let etag = r.get("g/a.txt").unwrap().1;
    let dir = |n: &str| (n.to_string(), true, 0, None);
    let file = ("a.txt".to_string(), false, 1, Some(etag));
    assert_eq!(rows(&r, "g"), vec![file, dir("empty"), dir("sub")]);
    assert_eq!(rows(&r, ""), vec![dir("g")]);
    assert_eq!(rows(&r, "g/empty"), vec![]);
}

#[test]
fn copy_duplicates_a_whole_folder() {
    let r = sample();
    r.copy("g", "h").unwrap();
    let want = ["g/a.txt", "g/sub/b.txt", "h/a.txt", "h/sub/b.txt"];
    assert_eq!(r.file_paths(), want);
    assert_eq!(r.read("h/sub/b.txt").unwrap(), b"BB");
    assert_eq!(rows(&r, "h/empty"), vec![]);
    assert!(matches!(
        r.copy("g/a.txt", "h/a.txt"),
        Err(Error::Server(_))
    ));
}

#[test]
fn move_renames_a_whole_folder() {
    let r = sample();
    r.mkcol("h").unwrap();
    r.move_to("g/sub", "h/moved").unwrap();
    assert_eq!(r.file_paths(), ["g/a.txt", "h/moved/b.txt"]);
    assert_eq!(
        r.list("g/sub").unwrap_err(),
        Error::NotFound("g/sub".into())
    );
    assert!(matches!(
        r.move_to("g/a.txt", "h/moved"),
        Err(Error::Server(_))
    ));
    assert_eq!(r.read("g/a.txt").unwrap(), b"A");
}

#[test]
fn delete_removes_a_file_or_a_whole_folder() {
    let r = sample();
    r.delete("g/a.txt").unwrap();
    assert_eq!(r.file_paths(), ["g/sub/b.txt"]);
    r.delete("g").unwrap();
    assert!(r.file_paths().is_empty());
    assert_eq!(rows(&r, ""), vec![]);
}

#[test]
fn offline_fails_every_call_but_inspection_still_works() {
    let r = sample();
    r.set_reachable(false);
    let off = Error::Unreachable("fake dropbox is offline".to_string());
    assert_eq!(r.list("g").unwrap_err(), off);
    assert_eq!(r.get("g/a.txt").unwrap_err(), off);
    assert_eq!(r.put("g/z", b"", ANY).unwrap_err(), off);
    assert_eq!(r.mkcol("g/n").unwrap_err(), off);
    assert_eq!(r.copy("g", "h").unwrap_err(), off);
    assert_eq!(r.move_to("g", "h").unwrap_err(), off);
    assert_eq!(r.delete("g").unwrap_err(), off);
    assert_eq!(r.account_email().unwrap_err(), off);
    assert_eq!(ensure_dir(&r.backend, "x/y").unwrap_err(), off);
    assert_eq!(r.file_paths(), ["g/a.txt", "g/sub/b.txt"]);
    r.set_reachable(true);
    assert_eq!(r.get("g/a.txt").unwrap().0, b"A");
}

#[test]
fn ensure_dir_creates_every_missing_level() {
    let r = sample();
    ensure_dir(&r.backend, "g/sub/x/y").unwrap();
    ensure_dir(&r.backend, "g/sub/x/y").unwrap();
    r.put("g/sub/x/y/f", b"F", ANY).unwrap();
    assert_eq!(r.file_paths(), ["g/a.txt", "g/sub/b.txt", "g/sub/x/y/f"]);
    assert!(r.list("g/sub/x").unwrap()[0].is_dir);
}

#[test]
fn expired_access_refreshes_once_and_revocation_needs_sign_in() {
    let r = sample();
    let before = r.fake.requests().len();
    r.fake.expire_access_tokens();
    assert_eq!(r.get("g/a.txt").unwrap().0, b"A");
    let calls = r.fake.requests();
    assert_eq!(calls.len() - before, 3);
    assert!(calls[before].ends_with("files/download"));
    assert!(calls[before + 1].ends_with("oauth2/token"));
    assert!(calls[before + 2].ends_with("files/download"));
    r.fake.revoke();
    assert_eq!(r.get("g/a.txt").unwrap_err().code(), "reauth_needed");
    assert_eq!(r.fake.requests().len() - calls.len(), 2);
}

#[test]
fn account_email_and_non_ascii_names_round_trip() {
    let r = Harness::new();
    assert_eq!(r.account_email().unwrap(), "fake@example.com");
    r.mkcol("é").unwrap();
    let rev = r
        .put("é/保存 😀.sav", b"save", &Precondition::Absent)
        .unwrap();
    assert_eq!(r.get("é/保存 😀.sav").unwrap(), (b"save".to_vec(), rev));
    assert_eq!(r.list("é").unwrap()[0].name, "保存 😀.sav");
    r.copy("é/保存 😀.sav", "é/copie").unwrap();
    r.move_to("é/copie", "é/déplacé").unwrap();
    r.delete("é/déplacé").unwrap();
}

// Only this wrapper and FakeDropbox ever receive requests: there is no network transport.
struct Inject {
    fake: Arc<FakeDropbox>,
    replies: Mutex<std::collections::VecDeque<Response>>,
    calls: Mutex<usize>,
}
impl Transport for Inject {
    fn send(&self, request: &Request) -> save_relay::error::Result<Response> {
        if request.url.ends_with("oauth2/token") {
            return self.fake.send(request);
        }
        *self.calls.lock().unwrap() += 1;
        if let Some(reply) = self.replies.lock().unwrap().pop_front() {
            return Ok(reply);
        }
        self.fake.send(request)
    }
}
type Injected = (Dropbox, Arc<Inject>, Arc<Mutex<Vec<Duration>>>);

fn injected(replies: Vec<Response>) -> Injected {
    let transport = Arc::new(Inject {
        fake: Arc::new(FakeDropbox::new()),
        replies: Mutex::new(replies.into()),
        calls: Mutex::new(0),
    });
    let waits = Arc::new(Mutex::new(Vec::new()));
    let recorded = waits.clone();
    let backend = Dropbox::with_sleep(tokens(transport.clone()), transport.clone(), move |d| {
        recorded.lock().unwrap().push(d)
    });
    (backend, transport, waits)
}

#[test]
fn second_401_stops_after_one_refresh() {
    let (r, t, waits) = injected(vec![Response::new(401, ""), Response::new(401, "")]);
    assert_eq!(
        r.list("").unwrap_err(),
        Error::ReauthNeeded("Dropbox".into())
    );
    assert_eq!(*t.calls.lock().unwrap(), 2);
    assert_eq!(t.fake.requests().len(), 2); // Initial token, then exactly one refresh.
    assert!(waits.lock().unwrap().is_empty());
}

#[test]
fn rate_limits_use_retry_after_then_exponential_backoff() {
    let (r, t, waits) = injected(vec![
        Response::new(429, "").header("Retry-After", "7"),
        Response::new(429, ""),
        Response::new(503, ""),
    ]);
    assert!(r.list("").unwrap().is_empty());
    assert_eq!(*t.calls.lock().unwrap(), 4);
    assert_eq!(*waits.lock().unwrap(), [7, 2, 4].map(Duration::from_secs));
}

#[test]
fn retry_after_is_capped() {
    let (r, _, waits) = injected(vec![
        Response::new(429, "").header("Retry-After", "86400"),
        Response::new(429, "").header("Retry-After", "60"),
    ]);
    assert!(r.list("").unwrap().is_empty());
    assert_eq!(*waits.lock().unwrap(), [60, 60].map(Duration::from_secs));
}

#[test]
fn retries_stop_and_errors_do_not_echo_tokens() {
    let (r, t, waits) = injected(vec![Response::new(429, "fake-access-1"); 4]);
    let error = r.list("").unwrap_err();
    assert!(matches!(error, Error::Server(_)));
    assert!(!error.to_string().contains("fake-access"));
    assert_eq!(*t.calls.lock().unwrap(), 4);
    assert_eq!(*waits.lock().unwrap(), [1, 2, 4].map(Duration::from_secs));
    let (r, _, _) = injected(vec![Response::new(400, "fake-access-1")]);
    assert!(!r.list("").unwrap_err().to_string().contains("fake-access"));
}

#[test]
fn parent_checks_do_not_recreate_removed_folders() {
    let r = sample();
    r.put("g/sub/first", b"1", ANY).unwrap();
    r.delete("g/sub").unwrap();
    assert_eq!(
        r.put("g/sub/second", b"2", ANY),
        Err(Error::NotFound("g/sub".into()))
    );
    assert_eq!(
        r.put("g/a.txt/child", b"x", ANY),
        Err(Error::NotFound("g/a.txt".into()))
    );
    assert_eq!(
        r.copy("g/a.txt", "missing/copy"),
        Err(Error::NotFound("missing".into()))
    );
    assert_eq!(
        r.move_to("g/none", "g/other"),
        Err(Error::NotFound("g/none".into()))
    );
    assert_eq!(r.list("g/a.txt"), Err(Error::NotFound("g/a.txt".into())));
}

#[test]
fn another_clients_push_cannot_be_overwritten_by_a_stale_revision() {
    let r = sample();
    let other = Dropbox::new(tokens(r.fake.clone()), r.fake.clone());
    let old_rev = r.get("g/a.txt").unwrap().1;
    other
        .put("g/a.txt", b"newer", &Precondition::Match(old_rev.clone()))
        .unwrap();
    assert_eq!(
        r.put("g/a.txt", b"lost push", &Precondition::Match(old_rev)),
        Err(Error::Changed)
    );
    assert_eq!(
        r.put("g/a.txt", b"lost push", &Precondition::Absent),
        Err(Error::Changed)
    );
    assert_eq!(r.get("g/a.txt").unwrap().0, b"newer");
}

#[test]
fn list_sorts_globally_even_when_pages_arrive_out_of_order() {
    let page = |names: &[&str], more| {
        Response::new(200, serde_json::json!({
        "entries": names.iter().map(|name| serde_json::json!({".tag": "folder", "name": name})).collect::<Vec<_>>(),
        "cursor": "fake-cursor", "has_more": more
    }).to_string())
    };
    let (r, t, _) = injected(vec![page(&["z", "m"], true), page(&["b", "a"], false)]);
    assert_eq!(
        r.list("")
            .unwrap()
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "m", "z"]
    );
    assert_eq!(*t.calls.lock().unwrap(), 2);
}

#[test]
fn folders_reject_unconditional_uploads_and_root_is_idempotent() {
    let r = sample();
    r.mkcol("").unwrap();
    assert!(matches!(r.put("g/sub", b"x", ANY), Err(Error::Server(_))));
    assert_eq!(
        r.put("g/sub", b"x", &Precondition::Absent),
        Err(Error::Changed)
    );
    assert!(matches!(r.put("", b"x", ANY), Err(Error::Server(_))));
    assert!(r.list("g/sub").unwrap().iter().any(|e| e.name == "b.txt"));
}

#[test]
fn unsafe_paths_are_refused_before_any_request() {
    let r = sample();
    let before = r.fake.requests().len();
    let bads = [
        "../x",
        "g/../a.txt",
        "/g/a.txt",
        "g//a.txt",
        "g/./a.txt",
        "g/",
        "g\\a.txt",
    ];
    for bad in bads {
        let refused = Error::NotFound(bad.into());
        assert_eq!(r.list(bad), Err(refused.clone()), "{bad}");
        assert_eq!(r.get(bad).unwrap_err(), refused, "{bad}");
        assert_eq!(r.put(bad, b"x", ANY), Err(refused.clone()), "{bad}");
        assert_eq!(r.mkcol(bad), Err(refused.clone()), "{bad}");
        assert_eq!(r.delete(bad), Err(refused.clone()), "{bad}");
        assert_eq!(r.copy(bad, "g/ok"), Err(refused.clone()), "{bad}");
        assert_eq!(r.move_to("g/a.txt", bad), Err(refused), "{bad}");
    }
    assert_eq!(r.fake.requests().len(), before);
    assert_eq!(r.read("g/a.txt").unwrap(), b"A");
}
