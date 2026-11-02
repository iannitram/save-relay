use save_relay::clients::{Client, DROPBOX as DROPBOX_CLIENT};
use save_relay::creds::Secret;
use save_relay::fake::{FakeDropbox, REFRESH_TOKEN};
use save_relay::http::{Request, Response, Transport};
use save_relay::oauth::{TokenSource, DROPBOX, GOOGLE};
use save_relay::Error;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

thread_local! {
    static CLOCK_NOW: AtomicU64 = const { AtomicU64::new(1000) };
}

fn test_clock() -> u64 {
    CLOCK_NOW.with(|c| c.load(Ordering::SeqCst))
}

fn set_clock(time: u64) {
    CLOCK_NOW.with(|c| c.store(time, Ordering::SeqCst));
}

struct MockTransport {
    requests: Mutex<Vec<Request>>,
    responses: Mutex<Vec<save_relay::error::Result<Response>>>,
}

impl MockTransport {
    fn new() -> MockTransport {
        MockTransport {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(Vec::new()),
        }
    }

    fn queue_response(&self, response: save_relay::error::Result<Response>) {
        self.responses.lock().unwrap().push(response);
    }

    fn queue_json(&self, status: u16, body: &str) {
        self.queue_response(Ok(Response::new(status, body)));
    }

    fn recorded_requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Transport for MockTransport {
    fn send(&self, request: &Request) -> save_relay::error::Result<Response> {
        self.requests.lock().unwrap().push(request.clone());
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            panic!("MockTransport: unexpected request: {request:?}");
        }
        responses.remove(0)
    }
}

fn form_pairs(body: &[u8]) -> BTreeMap<String, String> {
    let text = String::from_utf8_lossy(body);
    text.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (k.to_string(), v.to_string())
        })
        .collect()
}

#[test]
fn the_first_bearer_refreshes_and_later_calls_reuse_until_needs_refresh() {
    set_clock(1000);
    let transport = Arc::new(MockTransport::new());
    transport.queue_json(200, r#"{"access_token":"acc-1","expires_in":3600}"#);

    let client = Client {
        id: "dropbox-app",
        secret: None,
    };
    let source = TokenSource::new(
        &DROPBOX,
        client,
        Secret::new("initial-refresh"),
        transport.clone(),
        test_clock,
    );

    // First call: fetches from token endpoint.
    let token1 = source.bearer().expect("first bearer token");
    assert_eq!(token1.expose(), "acc-1");
    assert_eq!(transport.recorded_requests().len(), 1);

    // Later calls reuse the cached token before expiry (expires_at is 1000 + 3600 = 4600).
    // At 4539, 4539 + 60 = 4599 < 4600, so no refresh needed.
    set_clock(4539);
    let token2 = source.bearer().expect("cached bearer token");
    assert_eq!(token2.expose(), "acc-1");
    assert_eq!(transport.recorded_requests().len(), 1);

    // At 4540, 4540 + 60 = 4600 >= 4600, needs_refresh is true.
    transport.queue_json(200, r#"{"access_token":"acc-2","expires_in":3600}"#);
    set_clock(4540);
    let token3 = source.bearer().expect("refreshed bearer token");
    assert_eq!(token3.expose(), "acc-2");
    assert_eq!(transport.recorded_requests().len(), 2);
}

#[test]
fn invalidate_forces_one_refresh() {
    set_clock(1000);
    let transport = Arc::new(MockTransport::new());
    transport.queue_json(200, r#"{"access_token":"acc-1","expires_in":3600}"#);

    let client = Client {
        id: "dropbox-app",
        secret: None,
    };
    let source = TokenSource::new(
        &DROPBOX,
        client,
        Secret::new("initial-refresh"),
        transport.clone(),
        test_clock,
    );

    let token1 = source.bearer().unwrap();
    assert_eq!(token1.expose(), "acc-1");
    assert_eq!(transport.recorded_requests().len(), 1);

    // invalidate forgets cached access token.
    source.invalidate();

    // Next bearer call must refresh.
    transport.queue_json(200, r#"{"access_token":"acc-2","expires_in":3600}"#);
    let token2 = source.bearer().unwrap();
    assert_eq!(token2.expose(), "acc-2");
    assert_eq!(transport.recorded_requests().len(), 2);

    // Subsequent bearer call reuses the new cached token (proving invalidate forced ONE refresh).
    let token3 = source.bearer().unwrap();
    assert_eq!(token3.expose(), "acc-2");
    assert_eq!(transport.recorded_requests().len(), 2);
}

#[test]
fn invalid_grant_is_error_reauth_needed() {
    set_clock(1000);
    let transport = Arc::new(MockTransport::new());
    transport.queue_json(
        400,
        r#"{"error":"invalid_grant","error_description":"token expired or revoked"}"#,
    );

    let client = Client {
        id: "gdrive-app",
        secret: Some("gdrive-sec"),
    };
    let source = TokenSource::new(
        &GOOGLE,
        client,
        Secret::new("bad-refresh"),
        transport.clone(),
        test_clock,
    );

    let err = source.bearer().unwrap_err();
    assert_eq!(err, Error::ReauthNeeded("Google Drive".into()));
    assert_eq!(err.code(), "reauth_needed");

    // Same for Dropbox
    transport.queue_json(400, r#"{"error":"invalid_grant"}"#);
    let db_client = Client {
        id: "db-app",
        secret: None,
    };
    let db_source = TokenSource::new(
        &DROPBOX,
        db_client,
        Secret::new("bad-refresh"),
        transport.clone(),
        test_clock,
    );

    let db_err = db_source.bearer().unwrap_err();
    assert_eq!(db_err, Error::ReauthNeeded("Dropbox".into()));
    assert_eq!(db_err.code(), "reauth_needed");
}

#[test]
fn a_rotated_refresh_token_is_offered_once_by_rotated_and_used_in_next_refresh() {
    set_clock(1000);
    let transport = Arc::new(MockTransport::new());
    transport.queue_json(
        200,
        r#"{"access_token":"acc-1","expires_in":3600,"refresh_token":"rotated-refresh-xyz"}"#,
    );

    let client = Client {
        id: "db-app",
        secret: None,
    };
    let source = TokenSource::new(
        &DROPBOX,
        client,
        Secret::new("original-refresh"),
        transport.clone(),
        test_clock,
    );

    assert_eq!(source.rotated(), None);

    let token = source.bearer().unwrap();
    assert_eq!(token.expose(), "acc-1");

    // rotated() returns the new refresh token once.
    let rotated1 = source.rotated();
    assert!(rotated1.is_some());
    assert_eq!(rotated1.unwrap().expose(), "rotated-refresh-xyz");

    // Subsequent call returns None.
    assert_eq!(source.rotated(), None);

    // Invalidate and verify that the NEXT refresh sends the rotated refresh token.
    source.invalidate();
    transport.queue_json(200, r#"{"access_token":"acc-2","expires_in":3600}"#);
    let token2 = source.bearer().unwrap();
    assert_eq!(token2.expose(), "acc-2");

    let requests = transport.recorded_requests();
    assert_eq!(requests.len(), 2);
    let fields = form_pairs(&requests[1].body);
    assert_eq!(
        fields.get("refresh_token").map(String::as_str),
        Some("rotated-refresh-xyz")
    );

    // rotated() is still None because this refresh didn't rotate.
    assert_eq!(source.rotated(), None);
}

#[test]
fn form_body_is_right_for_google() {
    set_clock(1000);
    let transport = Arc::new(MockTransport::new());
    transport.queue_json(200, r#"{"access_token":"g-acc","expires_in":3600}"#);

    let client = Client {
        id: "my-google-client-id",
        secret: Some("my-google-client-secret"),
    };
    let source = TokenSource::new(
        &GOOGLE,
        client,
        Secret::new("my-google-refresh"),
        transport.clone(),
        test_clock,
    );

    let _ = source.bearer().unwrap();

    let requests = transport.recorded_requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(req.method, "POST");
    assert_eq!(req.url, GOOGLE.token_url);
    assert_eq!(
        req.header_value("Content-Type"),
        Some("application/x-www-form-urlencoded")
    );

    let body_str = String::from_utf8_lossy(&req.body);
    assert_eq!(
        body_str,
        "client_id=my-google-client-id&client_secret=my-google-client-secret&refresh_token=my-google-refresh&grant_type=refresh_token"
    );
}

#[test]
fn form_body_is_right_for_dropbox_with_no_secret() {
    set_clock(1000);
    let transport = Arc::new(MockTransport::new());
    transport.queue_json(200, r#"{"access_token":"db-acc","expires_in":3600}"#);

    let client = Client {
        id: "my-dropbox-client-id",
        secret: None,
    };
    let source = TokenSource::new(
        &DROPBOX,
        client,
        Secret::new("my-dropbox-refresh"),
        transport.clone(),
        test_clock,
    );

    let _ = source.bearer().unwrap();

    let requests = transport.recorded_requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(req.method, "POST");
    assert_eq!(req.url, DROPBOX.token_url);
    assert_eq!(
        req.header_value("Content-Type"),
        Some("application/x-www-form-urlencoded")
    );

    let body_str = String::from_utf8_lossy(&req.body);
    assert_eq!(
        body_str,
        "client_id=my-dropbox-client-id&refresh_token=my-dropbox-refresh&grant_type=refresh_token"
    );
    assert!(!body_str.contains("secret"));
}

#[test]
fn no_token_appears_in_any_error_or_debug() {
    set_clock(1000);
    let transport = Arc::new(MockTransport::new());
    let secret_access = "secret-access-token-42";
    let secret_refresh = "secret-refresh-token-84";

    transport.queue_json(
        200,
        &format!(r#"{{"access_token":"{secret_access}","expires_in":3600}}"#),
    );

    let client = Client {
        id: "test-client",
        secret: Some("test-secret"),
    };
    let source = TokenSource::new(
        &GOOGLE,
        client,
        Secret::new(secret_refresh),
        transport.clone(),
        test_clock,
    );

    // Check Debug before bearer()
    let debug_before = format!("{source:?}");
    assert!(
        !debug_before.contains(secret_refresh),
        "debug before bearer: {debug_before}"
    );

    let token = source.bearer().unwrap();
    assert_eq!(token.expose(), secret_access);

    // Check Debug after bearer()
    let debug_after = format!("{source:?}");
    assert!(
        !debug_after.contains(secret_access) && !debug_after.contains(secret_refresh),
        "debug after bearer: {debug_after}"
    );

    // Check Debug on Secret itself
    let token_debug = format!("{token:?}");
    assert!(
        !token_debug.contains(secret_access),
        "token debug: {token_debug}"
    );

    // Check on errors: invalidate and trigger a server error and an invalid_grant error
    source.invalidate();
    transport.queue_json(500, "internal server failure");
    let err = source.bearer().unwrap_err();
    let err_debug = format!("{err:?}");
    let err_display = format!("{err}");
    assert!(
        !err_debug.contains(secret_access) && !err_debug.contains(secret_refresh),
        "error debug: {err_debug}"
    );
    assert!(
        !err_display.contains(secret_access) && !err_display.contains(secret_refresh),
        "error display: {err_display}"
    );

    source.invalidate();
    transport.queue_json(400, r#"{"error":"invalid_grant"}"#);
    let err2 = source.bearer().unwrap_err();
    let err2_debug = format!("{err2:?}");
    let err2_display = format!("{err2}");
    assert!(
        !err2_debug.contains(secret_access) && !err2_debug.contains(secret_refresh),
        "error2 debug: {err2_debug}"
    );
    assert!(
        !err2_display.contains(secret_access) && !err2_display.contains(secret_refresh),
        "error2 display: {err2_display}"
    );
}

#[test]
fn transport_unreachable_error_passes_through_unchanged() {
    set_clock(1000);
    let transport = Arc::new(MockTransport::new());
    transport.queue_response(Err(Error::Unreachable("offline: network down".into())));

    let client = Client {
        id: "db-app",
        secret: None,
    };
    let source = TokenSource::new(
        &DROPBOX,
        client,
        Secret::new("my-refresh"),
        transport.clone(),
        test_clock,
    );

    let err = source.bearer().unwrap_err();
    assert_eq!(err, Error::Unreachable("offline: network down".into()));
    assert_eq!(err.code(), "server_unreachable");
}

#[test]
fn works_against_fake_dropbox() {
    set_clock(1000);
    let fake = Arc::new(FakeDropbox::new());
    let source = TokenSource::new(
        &DROPBOX,
        DROPBOX_CLIENT,
        Secret::new(REFRESH_TOKEN),
        fake.clone(),
        test_clock,
    );

    // Initial fetch gives fake-access-1
    let token1 = source.bearer().unwrap();
    assert_eq!(token1.expose(), "fake-access-1");

    // Cached call reuses fake-access-1
    let token2 = source.bearer().unwrap();
    assert_eq!(token2.expose(), "fake-access-1");

    // Invalidate forces refresh to fake-access-2
    source.invalidate();
    let token3 = source.bearer().unwrap();
    assert_eq!(token3.expose(), "fake-access-2");

    // Offline returns Error::Unreachable
    source.invalidate();
    fake.set_reachable(false);
    let offline_err = source.bearer().unwrap_err();
    assert_eq!(
        offline_err,
        Error::Unreachable("fake dropbox is offline".into())
    );

    // Revoked returns Error::ReauthNeeded
    fake.set_reachable(true);
    fake.revoke();
    let revoked_err = source.bearer().unwrap_err();
    assert_eq!(revoked_err, Error::ReauthNeeded("Dropbox".into()));
}
