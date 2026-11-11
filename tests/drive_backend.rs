use save_relay::clients::GOOGLE_DESKTOP;
use save_relay::creds::Secret;
use save_relay::fake::{FakeDrive, REFRESH_TOKEN};
use save_relay::gdrive::{
    guard, multipart_body, parse_file, Drive, DriveFile, API, FOLDER_MIME, UPLOAD,
};
use save_relay::http::{Request, Response, Transport};
use save_relay::oauth::{TokenSource, GOOGLE};
use save_relay::remote::{ensure_dir, Precondition, Remote};
use save_relay::Error;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

const ANY: &Precondition = &Precondition::Any;

const START: u64 = 1793476260;
thread_local! {
    static NOW: Cell<u64> = const { Cell::new(START) };
}
fn clock() -> u64 {
    NOW.with(Cell::get)
}
fn advance(fake: &FakeDrive, secs: u64) {
    fake.advance(secs);
    NOW.with(|now| now.set(now.get() + secs));
}
fn tokens(transport: Arc<dyn Transport>) -> TokenSource {
    TokenSource::new(
        &GOOGLE,
        GOOGLE_DESKTOP,
        Secret::new(REFRESH_TOKEN),
        transport,
        clock,
    )
}
fn drive(fake: &Arc<FakeDrive>) -> Drive {
    Drive::new(tokens(fake.clone()), fake.clone(), clock)
        .with_wait(Arc::new(|_| panic!("unexpected retry")))
}
// Transport's public seam uses Arc; FakeDrive's re-entrant hooks intentionally run on
// one thread, as do these two simulated machines.
#[allow(clippy::arc_with_non_send_sync)]
fn fake() -> Arc<FakeDrive> {
    NOW.with(|now| now.set(START));
    Arc::new(FakeDrive::new())
}
struct Backend {
    drive: Drive,
    fake: Arc<FakeDrive>,
}
impl Deref for Backend {
    type Target = Drive;
    fn deref(&self) -> &Drive {
        &self.drive
    }
}
impl Remote for Backend {
    fn list(&self, path: &str) -> save_relay::Result<Vec<save_relay::remote::RemoteEntry>> {
        self.drive.list(path)
    }
    fn get(&self, path: &str) -> save_relay::Result<(Vec<u8>, String)> {
        self.drive.get(path)
    }
    fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> save_relay::Result<String> {
        self.drive.put(path, bytes, pre)
    }
    fn mkcol(&self, path: &str) -> save_relay::Result<()> {
        self.drive.mkcol(path)
    }
    fn copy(&self, from: &str, to: &str) -> save_relay::Result<()> {
        self.drive.copy(from, to)
    }
    fn move_to(&self, from: &str, to: &str) -> save_relay::Result<()> {
        self.drive.move_to(from, to)
    }
    fn delete(&self, path: &str) -> save_relay::Result<()> {
        self.drive.delete(path)
    }
}
impl Backend {
    fn file_paths(&self) -> Vec<String> {
        self.fake.file_paths()
    }
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.get(path).ok().map(|v| v.0)
    }
    fn set_reachable(&self, reachable: bool) {
        self.fake.set_reachable(reachable);
    }
}
/// A server holding `g/a.txt` ("A"), `g/sub/b.txt` ("BB") and an empty folder `g/empty`.
fn sample() -> Backend {
    let fake = fake();
    let r = Backend {
        drive: drive(&fake),
        fake,
    };
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
fn rows(r: &Backend, dir: &str) -> Vec<Row> {
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
    let off = Error::Unreachable("fake drive is offline".to_string());
    assert_eq!(r.list("g").unwrap_err(), off);
    assert_eq!(r.get("g/a.txt").unwrap_err(), off);
    assert_eq!(r.put("g/z", b"", ANY).unwrap_err(), off);
    assert_eq!(r.mkcol("g/n").unwrap_err(), off);
    assert_eq!(r.copy("g", "h").unwrap_err(), off);
    assert_eq!(r.move_to("g", "h").unwrap_err(), off);
    assert_eq!(r.delete("g").unwrap_err(), off);
    assert_eq!(ensure_dir(&r, "x/y").unwrap_err(), off);
    assert_eq!(r.file_paths(), ["g/a.txt", "g/sub/b.txt"]);
    r.set_reachable(true);
    assert_eq!(r.get("g/a.txt").unwrap().0, b"A");
}

#[test]
fn ensure_dir_creates_every_missing_level() {
    let r = sample();
    ensure_dir(&r, "g/sub/x/y").unwrap();
    ensure_dir(&r, "g/sub/x/y").unwrap();
    r.put("g/sub/x/y/f", b"F", ANY).unwrap();
    assert_eq!(r.file_paths(), ["g/a.txt", "g/sub/b.txt", "g/sub/x/y/f"]);
    assert!(r.list("g/sub/x").unwrap()[0].is_dir);
}

// Direct fake requests arrange server states that a working backend must handle, including
// abandoned claims and duplicates. All traffic, including token refresh, stays in FakeDrive.
fn raw(fake: &FakeDrive, method: &str, endpoint: &str, body: Value) -> Response {
    let token = fake
        .send(
            &Request::new("POST", GOOGLE.token_url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(
                    format!("grant_type=refresh_token&refresh_token={REFRESH_TOKEN}").into_bytes(),
                ),
        )
        .unwrap();
    let token: Value = serde_json::from_slice(&token.body).unwrap();
    fake.send(
        &Request::new(method, &format!("{API}/{endpoint}"))
            .header(
                "Authorization",
                &format!("Bearer {}", token["access_token"].as_str().unwrap()),
            )
            .header("Content-Type", "application/json")
            .body(body.to_string().into_bytes()),
    )
    .unwrap()
}
fn raw_file(fake: &FakeDrive, metadata: Value) -> DriveFile {
    let response = raw(fake, "POST", "files", metadata);
    assert_eq!(response.status, 201);
    parse_file(&response.text()).unwrap()
}
fn dead_claim(fake: &FakeDrive, etag: &str, next: Option<&str>) -> String {
    let (_, claim) = guard::parse_etag(etag).unwrap();
    let props = next.map_or_else(|| json!({}), |next| json!({guard::NEXT_PROPERTY: next}));
    raw_file(
        fake,
        json!({"id": claim, "name": format!(".claim-{claim}"), "parents": ["root"], "appProperties": props}),
    );
    claim
}
fn state(fake: &Arc<FakeDrive>) -> (Drive, Rc<Drive>, String) {
    let first = drive(fake);
    let second = Rc::new(drive(fake));
    first.account_email().unwrap();
    second.account_email().unwrap();
    let etag = first.put("state.json", b"base", ANY).unwrap();
    (first, second, etag)
}

#[test]
fn entries_and_multipart_preserve_metadata_binary_and_crlf() {
    let file = parse_file(r#"{"id":"f","name":"x","version":"12","size":"4","appProperties":{"save_relay_claim":"C"}}"#).unwrap();
    assert_eq!(file.entry().etag.as_deref(), Some("v12.C"));
    assert_eq!(file.entry().size, 4);
    assert_eq!(
        DriveFile {
            is_dir: true,
            ..file.clone()
        }
        .entry()
        .etag,
        None
    );
    assert_eq!(
        DriveFile {
            is_dir: true,
            ..file
        }
        .entry()
        .size,
        0
    );
    assert_eq!(
        parse_file(r#"{"id":"x","name":"x"}"#)
            .unwrap()
            .entry()
            .etag
            .as_deref(),
        Some("v0.")
    );
    let mut expected = b"--B\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{\"name\":\"x\"}\r\n--B\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
    expected.extend_from_slice(&[0, 255, 13, 10]);
    expected.extend_from_slice(b"\r\n--B--\r\n");
    assert_eq!(
        multipart_body(&json!({"name": "x"}), &[0, 255, 13, 10], "B"),
        expected
    );
}

#[test]
fn paths_escape_unicode_quotes_backslashes_and_query_characters() {
    let r = sample();
    for name in ["雪's\\save ?#%&+", "space name", "emoji 🦀"] {
        let path = format!("g/{name}");
        r.put(&path, &[0, 255, 13, 10], &Precondition::Absent)
            .unwrap();
        assert_eq!(r.get(&path).unwrap().0, [0, 255, 13, 10]);
    }
    assert!(r
        .fake
        .requests()
        .iter()
        .any(|request| request.contains("pageToken=")));
}

#[test]
fn duplicate_folders_and_files_resolve_oldest_on_both_machines() {
    let fake = fake();
    let first = drive(&fake);
    let second = drive(&fake);
    let oldest = raw_file(
        &fake,
        json!({"id":"z-old", "name":"g", "mimeType": FOLDER_MIME}),
    );
    advance(&fake, 1);
    raw_file(
        &fake,
        json!({"id":"a-new", "name":"g", "mimeType": FOLDER_MIME}),
    );
    // Same-time duplicates must use the id tie breaker, across multiple pages.
    for id in ["z-file", "m-file", "a-file"] {
        raw_file(
            &fake,
            json!({"id": id, "name":"same", "parents":[oldest.id]}),
        );
    }
    first.put("g/same", b"oldest", ANY).unwrap();
    for remote in [&first, &second] {
        assert_eq!(remote.get("g/same").unwrap().0, b"oldest");
        assert_eq!(remote.list("").unwrap().len(), 1);
        assert_eq!(remote.list("g").unwrap().len(), 1);
    }
    let chosen = raw(&fake, "GET", "files/a-file", json!({}));
    assert_eq!(parse_file(&chosen.text()).unwrap().version, 2);
    assert_eq!(
        parse_file(&raw(&fake, "GET", "files/z-file", json!({})).text())
            .unwrap()
            .version,
        1
    );
    second.put("g/new", b"shared", ANY).unwrap();
    assert_eq!(first.get("g/new").unwrap().0, b"shared");
}

#[test]
fn concurrent_folder_creation_still_resolves_one_tree() {
    let fake = fake();
    let first = drive(&fake);
    let second = Rc::new(drive(&fake));
    second.account_email().unwrap();
    let fired = Rc::new(Cell::new(false));
    let seen = fired.clone();
    let other = second.clone();
    fake.set_hook(Some(Box::new(move |request| {
        if request.method == "POST" && request.url == format!("{API}/files") && !seen.replace(true)
        {
            other.mkcol("g").unwrap();
        }
    })));
    first.mkcol("g").unwrap();
    fake.set_hook(None);
    assert!(fired.get());
    first.put("g/first", b"1", ANY).unwrap();
    second.put("g/second", b"2", ANY).unwrap();
    assert_eq!(first.list("g").unwrap(), second.list("g").unwrap());
    assert_eq!(first.list("").unwrap().len(), 1);
    assert_eq!(first.get("g/second").unwrap().0, b"2");
}

#[test]
fn absent_races_leave_one_file_and_the_loser_is_changed() {
    // Before every request in a create, including the create itself and its re-list.
    let fake = fake();
    let first = drive(&fake);
    first.account_email().unwrap();
    let before = fake.requests().len();
    first
        .put("state.json", b"first", &Precondition::Absent)
        .unwrap();
    let steps = fake.requests().len() - before;
    for at in 0..steps {
        let fake = self::fake();
        let first = drive(&fake);
        let second = Rc::new(drive(&fake));
        first.account_email().unwrap();
        second.account_email().unwrap();
        let result = Rc::new(RefCell::new(None));
        let output = result.clone();
        let mut step = 0;
        fake.set_hook(Some(Box::new(move |_| {
            if step == at {
                *output.borrow_mut() =
                    Some(second.put("state.json", b"second", &Precondition::Absent));
            }
            step += 1;
        })));
        let a = first.put("state.json", b"first", &Precondition::Absent);
        fake.set_hook(None);
        let b = result
            .borrow_mut()
            .take()
            .expect("every create step reached");
        assert_ne!(a.is_ok(), b.is_ok(), "step {at}: {a:?}, {b:?}");
        assert_eq!(
            match a {
                Err(error) => error,
                Ok(_) => b.unwrap_err(),
            },
            Error::Changed
        );
        assert_eq!(fake.file_paths(), ["state.json"], "step {at}");
    }
}

fn race_each_request(stale: bool) {
    let fake = fake();
    let (first, _, etag) = state(&fake);
    if stale {
        dead_claim(&fake, &etag, Some("recover"));
        advance(&fake, guard::LEASE_SECS);
    }
    let trace = Rc::new(RefCell::new(Vec::<Request>::new()));
    let output = trace.clone();
    fake.set_hook(Some(Box::new(move |request| {
        output.borrow_mut().push(request.clone())
    })));
    first
        .put("state.json", b"first", &Precondition::Match(etag))
        .unwrap();
    fake.set_hook(None);
    let trace = trace.borrow();
    assert!(trace.iter().any(|r| r.url.contains("generateIds")));
    assert!(trace
        .iter()
        .any(|r| r.method == "POST" && r.url.starts_with(API)));
    assert!(trace
        .iter()
        .any(|r| r.method == "PATCH" && r.url.starts_with(UPLOAD)));
    assert!(
        trace
            .iter()
            .filter(|r| r.method == "GET"
                && r.url.contains("/files/")
                && !r.url.contains("generateIds"))
            .count()
            >= 3
    );
    if stale {
        assert!(trace.iter().any(|r| {
            r.method == "POST" && String::from_utf8_lossy(&r.body).contains("\"id\":\"recover\"")
        }));
    }
    for (at, step) in trace.iter().enumerate() {
        let fake = self::fake();
        let (first, second, etag) = state(&fake);
        if stale {
            dead_claim(&fake, &etag, Some("recover"));
            advance(&fake, guard::LEASE_SECS);
        }
        let result = Rc::new(RefCell::new(None));
        let output = result.clone();
        let pre = Precondition::Match(etag.clone());
        let mut index = 0;
        fake.set_hook(Some(Box::new(move |_| {
            if index == at {
                *output.borrow_mut() = Some(second.put("state.json", b"second", &pre));
            }
            index += 1;
        })));
        let a = first.put("state.json", b"first", &Precondition::Match(etag));
        fake.set_hook(None);
        let b = result
            .borrow_mut()
            .take()
            .expect("every guard request reached");
        assert_ne!(
            a.is_ok(),
            b.is_ok(),
            "stale={stale}, step {at} {step:?}: {a:?}, {b:?}"
        );
        let (winner, loser, bytes) = if a.is_ok() {
            (a, b, b"first".as_slice())
        } else {
            (b, a, b"second".as_slice())
        };
        let error = loser.unwrap_err();
        assert_eq!(error.code(), "changed_on_server", "step {at} {step:?}");
        assert_eq!(
            first.get("state.json").unwrap(),
            (bytes.to_vec(), winner.unwrap())
        );
        assert_eq!(
            first
                .list("")
                .unwrap()
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["state.json"]
        );
    }
}

#[test]
fn m9_two_machines_at_every_guard_request() {
    race_each_request(false);
}
#[test]
fn m9_two_lease_breakers_at_every_guard_request() {
    race_each_request(true);
}

#[test]
fn dead_writer_lease_breaks_at_120_seconds_using_server_time() {
    let fake = fake();
    let (first, second, etag) = state(&fake);
    dead_claim(&fake, &etag, Some("recover"));
    // Deliberately skew the client's clock: the lease age must come from Drive's Date.
    NOW.with(|now| now.set(START + 50_000));
    assert_eq!(
        second.put("state.json", b"early", &Precondition::Match(etag.clone())),
        Err(Error::Changed)
    );
    fake.advance(guard::LEASE_SECS - 1);
    assert_eq!(
        second.put("state.json", b"early", &Precondition::Match(etag.clone())),
        Err(Error::Changed)
    );
    fake.advance(1);
    second
        .put(
            "state.json",
            b"recovered",
            &Precondition::Match(etag.clone()),
        )
        .unwrap();
    assert_eq!(first.get("state.json").unwrap().0, b"recovered");
    assert_eq!(
        first.put("state.json", b"stale", &Precondition::Match(etag)),
        Err(Error::Changed)
    );
}

#[test]
fn missing_next_and_hop_limit_refuse_to_break() {
    for hops in [None, Some(guard::MAX_HOPS)] {
        let fake = fake();
        let (first, _, etag) = state(&fake);
        let claim = dead_claim(&fake, &etag, hops.map(|_| "hop-0"));
        if let Some(hops) = hops {
            for index in 0..hops {
                raw_file(
                    &fake,
                    json!({"id": format!("hop-{index}"), "name": format!(".claim-hop-{index}"),
                    "appProperties": {guard::NEXT_PROPERTY: format!("hop-{}", index + 1)}}),
                );
            }
        }
        advance(&fake, guard::LEASE_SECS);
        assert_eq!(
            first.put("state.json", b"refused", &Precondition::Match(etag)),
            Err(Error::Changed)
        );
        assert_eq!(first.get("state.json").unwrap().0, b"base");
        assert!(fake.file_paths().contains(&format!(".claim-{claim}")));
    }
}

#[test]
fn won_claim_stops_at_slow_boundary_and_recovers_later() {
    for delay in [guard::SLOW_SECS - 1, guard::SLOW_SECS] {
        let fake = fake();
        let (first, _, etag) = state(&fake);
        let weak = Arc::downgrade(&fake);
        let mut claim_sent = false;
        let mut delayed = false;
        fake.set_hook(Some(Box::new(move |request| {
            if request.method == "POST" && request.url.starts_with(&format!("{API}/files?")) {
                claim_sent = true;
            }
            if claim_sent
                && !delayed
                && request.method == "GET"
                && request.url.contains("/files/fake-file-")
            {
                advance(&weak.upgrade().unwrap(), delay);
                delayed = true;
            }
        })));
        let result = first.put("state.json", b"write", &Precondition::Match(etag.clone()));
        fake.set_hook(None);
        if delay < guard::SLOW_SECS {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(Error::Unreachable(_))));
            assert_eq!(first.get("state.json").unwrap().0, b"base");
            advance(&fake, guard::LEASE_SECS);
            first
                .put("state.json", b"recovered", &Precondition::Match(etag))
                .unwrap();
        }
    }
}

#[test]
fn check_written_detects_a_write_between_upload_and_readback() {
    let fake = fake();
    let (first, second, etag) = state(&fake);
    let mut uploaded = false;
    let mut changed = false;
    fake.set_hook(Some(Box::new(move |request| {
        if request.method == "PATCH" {
            uploaded = true;
        }
        if uploaded && !changed && request.method == "GET" {
            second.put("state.json", b"unconditional", ANY).unwrap();
            changed = true;
        }
    })));
    let result = first.put("state.json", b"conditional", &Precondition::Match(etag));
    fake.set_hook(None);
    assert!(
        matches!(result, Err(Error::Server(ref message)) if message.contains("state changed during the write"))
    );
}

#[test]
fn old_claims_are_retained_a_day_then_cleaned_by_a_later_write() {
    let fake = fake();
    let (first, _, etag) = state(&fake);
    first
        .put("state.json", b"one", &Precondition::Match(etag.clone()))
        .unwrap();
    let (_, claim) = guard::parse_etag(&etag).unwrap();
    let path = format!(".claim-{claim}");
    advance(&fake, 86_399);
    first.put("another", b"1", ANY).unwrap();
    assert!(fake.file_paths().contains(&path));
    advance(&fake, 1);
    first.put("another", b"2", ANY).unwrap();
    assert!(!fake.file_paths().contains(&path));
    assert_eq!(first.list("").unwrap().len(), 2);
}

#[test]
fn the_claim_named_by_state_survives_day_old_cleanup_and_allows_the_next_push() {
    let fake = fake();
    let (first, _, etag) = state(&fake);
    let (_, claim) = guard::parse_etag(&etag).unwrap();
    let path = format!(".claim-{claim}");
    dead_claim(&fake, &etag, Some("chain-1"));
    raw_file(
        &fake,
        json!({"id": "chain-1", "name": ".claim-chain-1", "parents": ["root"],
        "appProperties": {guard::NEXT_PROPERTY: "chain-2"}}),
    );

    advance(&fake, 86_400);
    first.put("playing.json", b"open", ANY).unwrap();

    assert!(fake.file_paths().contains(&path));
    assert!(fake.file_paths().contains(&".claim-chain-1".to_string()));
    first
        .put(
            "state.json",
            b"recovered by next push",
            &Precondition::Match(etag),
        )
        .unwrap();
    assert_eq!(
        first.get("state.json").unwrap().0,
        b"recovered by next push"
    );
}

#[test]
fn explicit_repair_rotates_only_a_proven_deleted_claim() {
    let fake = fake();
    let drive = drive(&fake);
    let old = drive.put("state.json", b"saved state", ANY).unwrap();
    let (_, missing) = guard::parse_etag(&old).unwrap();
    dead_claim(&fake, "v1.predecessor", Some(&missing));
    // The test models a permanently deleted ID at the end of a surviving claim chain.
    assert!(matches!(
        drive.get(".claim-does-not-exist"),
        Err(Error::NotFound(_))
    ));

    let preview = drive
        .repair_missing_state_claim("state.json", false)
        .unwrap();
    assert!(preview
        .as_deref()
        .is_some_and(|m| m.starts_with("missing Drive claim")));
    let (before, _) = drive.get("state.json").unwrap();
    assert_eq!(before, b"saved state");

    let applied = drive
        .repair_missing_state_claim("state.json", true)
        .unwrap()
        .unwrap();
    assert!(applied.contains("state bytes were preserved"));
    let (after, new_etag) = drive.get("state.json").unwrap();
    assert_eq!(after, b"saved state");
    assert_ne!(new_etag, old);
    assert!(drive
        .put("state.json", b"next", &Precondition::Match(new_etag))
        .is_ok());
}

#[test]
fn explicit_repair_leaves_a_normal_unused_claim_unchanged() {
    let fake = fake();
    let drive = drive(&fake);
    let etag = drive.put("state.json", b"saved state", ANY).unwrap();
    assert_eq!(
        drive
            .repair_missing_state_claim("state.json", false)
            .unwrap(),
        None
    );
    assert_eq!(
        drive
            .repair_missing_state_claim("state.json", true)
            .unwrap(),
        None
    );
    assert_eq!(drive.get("state.json").unwrap().1, etag);
}

#[test]
fn stale_or_forged_etags_never_write() {
    let fake = fake();
    let (first, _, etag) = state(&fake);
    for stale in [
        "garbage".into(),
        "v1.".into(),
        "v1.wrong".into(),
        "v99.fake".into(),
    ] {
        assert_eq!(
            first.put("state.json", b"bad", &Precondition::Match(stale)),
            Err(Error::Changed)
        );
    }
    let next = first
        .put("state.json", b"next", &Precondition::Match(etag.clone()))
        .unwrap();
    assert_ne!(next, etag);
    assert_eq!(
        first.put("state.json", b"bad", &Precondition::Match(etag)),
        Err(Error::Changed)
    );
    assert_eq!(first.get("state.json").unwrap().0, b"next");
}

#[test]
fn move_is_one_update_and_copies_are_one_request_per_file() {
    let r = sample();
    r.mkcol("history").unwrap();
    let before = r.fake.requests().len();
    r.move_to("g", "history/whole").unwrap();
    let requests = r.fake.requests();
    let mutations: Vec<_> = requests[before..]
        .iter()
        .filter(|r| !r.starts_with("GET "))
        .collect();
    assert_eq!(mutations.len(), 1);
    assert!(mutations[0].starts_with("PATCH "));
    assert!(mutations[0].contains("addParents="));
    assert!(mutations[0].contains("removeParents="));
    let before = r.fake.requests().len();
    r.copy("history/whole", "restored").unwrap();
    let requests = r.fake.requests();
    assert_eq!(
        requests[before..]
            .iter()
            .filter(|r| r.contains("/copy"))
            .count(),
        2
    );
    r.copy("restored/a.txt", "copied-file").unwrap();
    r.move_to("copied-file", "renamed-file").unwrap();
    assert_eq!(r.get("renamed-file").unwrap().0, b"A");
    assert!(r.list("restored/empty").unwrap().is_empty());
}

#[test]
fn folder_cycles_and_root_file_operations_are_refused() {
    let r = sample();
    for (from, to) in [("g", "g/sub/new"), ("g", "g"), ("", "child")] {
        assert!(matches!(r.copy(from, to), Err(Error::Server(_))));
        assert!(matches!(r.move_to(from, to), Err(Error::Server(_))));
    }
    assert!(matches!(r.put("", b"x", ANY), Err(Error::Server(_))));
    assert!(matches!(r.put("g", b"x", ANY), Err(Error::Server(_))));
    r.mkcol("").unwrap();
    r.delete("").unwrap();
    assert!(r.list("").unwrap().is_empty());
}

#[test]
fn read_refuses_new_bytes_with_an_old_etag() {
    let fake = fake();
    let (first, second, _) = state(&fake);
    let mut fired = false;
    fake.set_hook(Some(Box::new(move |request| {
        if request.url.contains("alt=media") && !fired {
            fired = true;
            second.put("state.json", b"new", ANY).unwrap();
        }
    })));
    assert_eq!(first.get("state.json"), Err(Error::Changed));
    fake.set_hook(None);
}

// Only Drive API responses are injected; successful calls and every token request still
// use FakeDrive. The script is consumed in order and never opens a socket.
struct Script {
    fake: Arc<FakeDrive>,
    answers: Mutex<VecDeque<Response>>,
    requests: Mutex<Vec<Request>>,
}
impl Transport for Script {
    fn send(&self, request: &Request) -> save_relay::Result<Response> {
        self.requests.lock().unwrap().push(request.clone());
        if request.url != GOOGLE.token_url {
            if let Some(response) = self.answers.lock().unwrap().pop_front() {
                return Ok(response);
            }
        }
        self.fake.send(request)
    }
}
#[allow(clippy::arc_with_non_send_sync)]
fn scripted(answers: Vec<Response>, waits: Arc<Mutex<Vec<u64>>>) -> (Drive, Arc<Script>) {
    let script = Arc::new(Script {
        fake: fake(),
        answers: Mutex::new(answers.into()),
        requests: Mutex::new(Vec::new()),
    });
    let output = waits.clone();
    let drive = Drive::new(tokens(script.clone()), script.clone(), clock).with_wait(Arc::new(
        move |seconds| output.lock().unwrap().push(seconds),
    ));
    (drive, script)
}

#[test]
fn refresh_once_on_401_then_reauth_without_tokens_in_errors() {
    let (drive, script) = scripted(
        vec![
            Response::new(401, "echo secret"),
            Response::new(401, "echo secret"),
        ],
        Arc::default(),
    );
    let error = drive.account_email().unwrap_err();
    assert_eq!(error.code(), "reauth_needed");
    let requests = script.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url == GOOGLE.token_url)
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url != GOOGLE.token_url)
            .count(),
        2
    );
    assert!(!error.to_string().contains("secret"));
    assert!(!error.to_string().contains(REFRESH_TOKEN));
}

#[test]
fn expired_access_refreshes_and_revoked_sign_in_needs_reauth() {
    let fake = fake();
    let remote = drive(&fake);
    let expected = raw(&fake, "GET", "about?fields=user", json!({}));
    let expected: Value = serde_json::from_slice(&expected.body).unwrap();
    assert_eq!(
        remote.account_email().unwrap(),
        expected["user"]["emailAddress"]
    );
    fake.expire_access_tokens();
    assert_eq!(
        remote.account_email().unwrap(),
        expected["user"]["emailAddress"]
    );
    fake.revoke();
    let error = remote.account_email().unwrap_err();
    assert_eq!(error.code(), "reauth_needed");
    assert!(!error.to_string().contains(REFRESH_TOKEN));
}

#[test]
fn retry_backoff_is_bounded_for_429_5xx_and_both_rate_limits() {
    for response in [
        Response::new(429, "secret"),
        Response::new(500, "secret"),
        Response::new(503, "secret"),
        Response::new(
            403,
            json!({"error":{"errors":[{"reason":"rateLimitExceeded"}]}}).to_string(),
        ),
        Response::new(
            403,
            json!({"error":{"errors":[{"reason":"userRateLimitExceeded"}]}}).to_string(),
        ),
    ] {
        let waits = Arc::new(Mutex::new(Vec::new()));
        let (remote, script) = scripted(vec![response.clone(); 4], waits.clone());
        let error = remote.list("").unwrap_err();
        assert!(matches!(error, Error::Server(_)));
        assert!(!error.to_string().contains("secret"));
        assert_eq!(*waits.lock().unwrap(), [1, 2, 4]);
        assert_eq!(
            script
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.url != GOOGLE.token_url)
                .count(),
            4
        );
    }
}

#[test]
fn retries_recover_honor_retry_after_and_do_not_retry_other_403s() {
    let waits = Arc::new(Mutex::new(Vec::new()));
    let (remote, _) = scripted(
        vec![
            Response::new(429, "").header("Retry-After", "7"),
            Response::new(502, ""),
        ],
        waits.clone(),
    );
    assert!(remote.list("").unwrap().is_empty());
    assert_eq!(*waits.lock().unwrap(), [7, 2]);
    let waits = Arc::new(Mutex::new(Vec::new()));
    let (remote, _) = scripted(vec![Response::new(403, "secret")], waits.clone());
    assert!(matches!(remote.list(""), Err(Error::Server(_))));
    assert!(waits.lock().unwrap().is_empty());
}

#[test]
fn copied_files_have_independent_claims_and_claim_files_stay_internal() {
    let r = sample();
    let etag = r.get("g/a.txt").unwrap().1;
    r.put("g/a.txt", b"next", &Precondition::Match(etag))
        .unwrap();
    r.copy("g", "h").unwrap();
    assert_eq!(r.list("g").unwrap().len(), 3);
    assert_eq!(r.list("h").unwrap().len(), 3);
    let a = r.get("g/a.txt").unwrap().1;
    let b = r.get("h/a.txt").unwrap().1;
    assert_ne!(
        guard::parse_etag(&a).unwrap().1,
        guard::parse_etag(&b).unwrap().1
    );
    r.put("g/a.txt", b"original", &Precondition::Match(a))
        .unwrap();
    r.put("h/a.txt", b"copied", &Precondition::Match(b))
        .unwrap();
    assert_eq!(r.get("g/a.txt").unwrap().0, b"original");
    assert_eq!(r.get("h/a.txt").unwrap().0, b"copied");
}

#[test]
fn remote_errors_cover_file_parents_missing_sources_and_existing_destinations() {
    let r = sample();
    assert_eq!(r.list("g/a.txt"), Err(Error::NotFound("g/a.txt".into())));
    for result in [r.copy("absent", "new"), r.move_to("absent", "new")] {
        assert_eq!(result, Err(Error::NotFound("absent".into())));
    }
    for result in [r.copy("g", "missing/new"), r.move_to("g", "missing/new")] {
        assert_eq!(result, Err(Error::NotFound("missing".into())));
    }
    for result in [
        r.copy("g/a.txt", "g/sub"),
        r.move_to("g/a.txt", "g/sub"),
        r.copy("g", ""),
    ] {
        assert!(matches!(result, Err(Error::Server(_))));
    }
    assert_eq!(
        r.put("g/a.txt/x", b"x", ANY),
        Err(Error::NotFound("g/a.txt".into()))
    );
    assert_eq!(r.mkcol("g/a.txt/x"), Err(Error::NotFound("g/a.txt".into())));
    assert_eq!(r.get("g/sub/b.txt").unwrap().0, b"BB");
}

struct UploadRetry {
    fake: Arc<FakeDrive>,
    attempted: Cell<u32>,
}
impl Transport for UploadRetry {
    fn send(&self, request: &Request) -> save_relay::Result<Response> {
        if request.method == "PATCH" && request.url.starts_with(UPLOAD) {
            self.attempted.set(self.attempted.get() + 1);
            return Ok(Response::new(503, "secret"));
        }
        self.fake.send(request)
    }
}

#[test]
#[allow(clippy::arc_with_non_send_sync)]
fn retry_wait_cannot_extend_a_won_claim_past_its_deadline() {
    let fake = fake();
    let (first, _, etag) = state(&fake);
    let transport = Arc::new(UploadRetry {
        fake: fake.clone(),
        attempted: Cell::new(0),
    });
    let remote =
        Drive::new(tokens(transport.clone()), transport.clone(), clock).with_wait(Arc::new(|_| {
            NOW.with(|now| now.set(now.get() + guard::SLOW_SECS))
        }));
    assert!(matches!(
        remote.put("state.json", b"slow", &Precondition::Match(etag)),
        Err(Error::Unreachable(_))
    ));
    assert_eq!(transport.attempted.get(), 1);
    assert_eq!(first.get("state.json").unwrap().0, b"base");
}
