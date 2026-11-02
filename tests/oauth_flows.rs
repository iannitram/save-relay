use save_relay::clients::{Client, DROPBOX_REDIRECT};
use save_relay::creds::Secret;
use save_relay::http::{Request, Response, Transport};
use save_relay::oauth::{
    challenge, finish_code, finish_code_with_sleep, in_game_mode, random_bytes, sign_in_browser,
    sign_in_browser_timeout, start_code, Pending, DROPBOX, GOOGLE,
};
use save_relay::service::Service;
use save_relay::Error;
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct MockTransport<F> {
    handler: F,
}

impl<F: Fn(&Request) -> save_relay::error::Result<Response>> MockTransport<F> {
    fn new(handler: F) -> Self {
        Self { handler }
    }
}

impl<F: Fn(&Request) -> save_relay::error::Result<Response>> Transport for MockTransport<F> {
    fn send(&self, request: &Request) -> save_relay::error::Result<Response> {
        (self.handler)(request)
    }
}

fn parse_query(url: &str) -> HashMap<String, String> {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or(url);
    query
        .split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((percent_decode(k), percent_decode(v)))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

const GOOGLE_CLIENT: Client = Client {
    id: "test-google-id",
    secret: Some("test-google-secret"),
};

const DROPBOX_CLIENT: Client = Client {
    id: "test-dropbox-key",
    secret: None,
};

#[test]
fn random_bytes_generates_32_distinct_bytes() {
    let b1 = random_bytes().expect("random_bytes succeeds");
    let b2 = random_bytes().expect("random_bytes succeeds");
    assert_eq!(b1.len(), 32);
    assert_eq!(b2.len(), 32);
    assert_ne!(b1, b2, "random bytes should not be repeated");
    assert_ne!(b1, [0u8; 32], "random bytes should not be all zeros");
}

#[test]
fn sign_in_browser_success_and_redirect_page() {
    let token_called = Arc::new(AtomicUsize::new(0));
    let token_called_clone = token_called.clone();

    let transport = MockTransport::new(move |req: &Request| {
        if req.url == GOOGLE.token_url && req.method == "POST" {
            token_called_clone.fetch_add(1, Ordering::SeqCst);
            let body = String::from_utf8_lossy(&req.body);
            let fields = parse_query(&body);
            assert_eq!(
                fields.get("grant_type").map(String::as_str),
                Some("authorization_code")
            );
            assert_eq!(
                fields.get("code").map(String::as_str),
                Some("mock-auth-code")
            );
            assert_eq!(
                fields.get("client_id").map(String::as_str),
                Some("test-google-id")
            );
            assert_eq!(
                fields.get("client_secret").map(String::as_str),
                Some("test-google-secret")
            );
            assert!(fields.contains_key("code_verifier"));
            assert!(fields.contains_key("redirect_uri"));

            return Ok(Response::new(
                200,
                json!({
                    "access_token": "mock-access-token",
                    "expires_in": 3600,
                    "refresh_token": "mock-refresh-token",
                    "token_type": "Bearer"
                })
                .to_string(),
            ));
        }
        panic!("unexpected transport call: {:?}", req);
    });

    let open = |url: &str| {
        let params = parse_query(url);
        let redirect_uri = params
            .get("redirect_uri")
            .expect("redirect_uri in auth_url")
            .clone();
        let state = params.get("state").expect("state in auth_url").clone();

        std::thread::spawn(move || {
            let addr = redirect_uri
                .strip_prefix("http://")
                .and_then(|s| s.strip_suffix('/'))
                .unwrap();
            let mut stream = TcpStream::connect(addr).expect("connect to redirect URI");
            let req = format!("GET /?code=mock-auth-code&state={state} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
            stream.write_all(req.as_bytes()).unwrap();

            let mut resp = String::new();
            stream.read_to_string(&mut resp).unwrap();
            assert!(resp.starts_with("HTTP/1.1 200 OK"));
            assert!(resp.contains("Signed in to save-relay. You can close this tab."));
        });
        Ok(())
    };

    let tokens = sign_in_browser(&GOOGLE, GOOGLE_CLIENT, &transport, &open, || 1000)
        .expect("sign_in_browser succeeds");
    assert_eq!(tokens.access.expose(), "mock-access-token");
    assert_eq!(tokens.expires_at, 4600);
    assert_eq!(tokens.refresh.unwrap().expose(), "mock-refresh-token");
    assert_eq!(token_called.load(Ordering::SeqCst), 1);
}

#[test]
fn sign_in_browser_ignores_favicon() {
    let transport = MockTransport::new(|req: &Request| {
        if req.url == GOOGLE.token_url {
            return Ok(Response::new(
                200,
                json!({
                    "access_token": "fav-access",
                    "expires_in": 3600
                })
                .to_string(),
            ));
        }
        panic!("unexpected request");
    });

    let open = |url: &str| {
        let params = parse_query(url);
        let redirect_uri = params.get("redirect_uri").unwrap().clone();
        let state = params.get("state").unwrap().clone();

        std::thread::spawn(move || {
            let addr = redirect_uri
                .strip_prefix("http://")
                .and_then(|s| s.strip_suffix('/'))
                .unwrap();

            // First request: favicon.ico
            let mut fav_stream = TcpStream::connect(addr).expect("connect for favicon");
            fav_stream
                .write_all(
                    b"GET /favicon.ico HTTP/1.1\r\nHost: loopback\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            let mut fav_resp = String::new();
            fav_stream.read_to_string(&mut fav_resp).unwrap();
            assert!(fav_resp.starts_with("HTTP/1.1 404 Not Found"));

            // Second request: the real redirect
            let mut code_stream = TcpStream::connect(addr).expect("connect for code");
            let req = format!("GET /?code=valid-code&state={state} HTTP/1.1\r\nHost: loopback\r\nConnection: close\r\n\r\n");
            code_stream.write_all(req.as_bytes()).unwrap();
            let mut code_resp = String::new();
            code_stream.read_to_string(&mut code_resp).unwrap();
            assert!(code_resp.starts_with("HTTP/1.1 200 OK"));
        });
        Ok(())
    };

    let tokens = sign_in_browser(&GOOGLE, GOOGLE_CLIENT, &transport, &open, || 500)
        .expect("sign_in_browser succeeds after ignoring favicon");
    assert_eq!(tokens.access.expose(), "fav-access");
}

#[test]
fn sign_in_browser_refuses_wrong_state() {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let transport = MockTransport::new(|_| panic!("transport should not be called on wrong state"));

    let open = |url: &str| {
        let tx = tx.clone();
        let params = parse_query(url);
        let redirect_uri = params.get("redirect_uri").unwrap().clone();

        std::thread::spawn(move || {
            let addr = redirect_uri
                .strip_prefix("http://")
                .and_then(|s| s.strip_suffix('/'))
                .unwrap();
            let mut stream = TcpStream::connect(addr).unwrap();
            let req = format!("GET /?code=some-code&state=wrong-state HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
            stream.write_all(req.as_bytes()).unwrap();
            let mut resp = String::new();
            let _ = stream.read_to_string(&mut resp);
            let _ = tx.send(resp);
        });
        Ok(())
    };

    let err = sign_in_browser(&GOOGLE, GOOGLE_CLIENT, &transport, &open, || 1000)
        .expect_err("sign_in_browser must refuse wrong state");
    assert_eq!(
        err,
        Error::SignInFailed("the answer did not match this sign-in".into())
    );
    assert_failed_page(&rx.recv_timeout(Duration::from_secs(5)).expect("browser got a response"));
}

#[test]
fn sign_in_browser_refuses_access_denied() {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let transport =
        MockTransport::new(|_| panic!("transport should not be called on access_denied"));

    let open = |url: &str| {
        let tx = tx.clone();
        let params = parse_query(url);
        let redirect_uri = params.get("redirect_uri").unwrap().clone();

        std::thread::spawn(move || {
            let addr = redirect_uri
                .strip_prefix("http://")
                .and_then(|s| s.strip_suffix('/'))
                .unwrap();
            let mut stream = TcpStream::connect(addr).unwrap();
            let req = format!(
                "GET /?error=access_denied HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(req.as_bytes()).unwrap();
            let mut resp = String::new();
            let _ = stream.read_to_string(&mut resp);
            let _ = tx.send(resp);
        });
        Ok(())
    };

    let err = sign_in_browser(&GOOGLE, GOOGLE_CLIENT, &transport, &open, || 1000)
        .expect_err("sign_in_browser must refuse access_denied");
    assert_eq!(err, Error::SignInFailed("access_denied".into()));
    assert_failed_page(&rx.recv_timeout(Duration::from_secs(5)).expect("browser got a response"));
}

#[test]
fn sign_in_browser_times_out() {
    let transport = MockTransport::new(|_| panic!("transport should not be called on timeout"));
    let open = |_url: &str| Ok(()); // does not connect to redirect URI

    let err = sign_in_browser_timeout(
        &GOOGLE,
        GOOGLE_CLIENT,
        &transport,
        &open,
        || 1000,
        Duration::from_millis(50),
    )
    .expect_err("must time out when redirect never arrives");

    assert!(
        matches!(err, Error::SignInFailed(ref msg) if msg.contains("timed out")),
        "{err:?}"
    );
}

/// Only one test may hold the Dropbox loopback port (53710) at a time.
static PORT_LOCK: Mutex<()> = Mutex::new(());

fn lock_port() -> std::sync::MutexGuard<'static, ()> {
    PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// A refused sign-in answers with a page that says so, and never claims success.
fn assert_failed_page(resp: &str) {
    assert!(resp.starts_with("HTTP/1.1 400 Bad Request"), "{resp}");
    assert!(!resp.contains("Signed in"), "{resp}");
    assert!(resp.contains("The sign-in did not complete"), "{resp}");
}

#[test]
fn sign_in_browser_dropbox_port_in_use() {
    let _port = lock_port();
    let transport = MockTransport::new(|_| panic!("transport should not be called"));
    let open = |_url: &str| Ok(());

    // Hold the port ourselves when it is free. If something else already holds it, the
    // sign-in must refuse with the same error.
    let _held = TcpListener::bind("127.0.0.1:53710").ok();
    let err = sign_in_browser(&DROPBOX, DROPBOX_CLIENT, &transport, &open, || 1000)
        .expect_err("must fail if port 53710 is in use");
    assert_eq!(
        err,
        Error::SignInFailed("port 53710 is in use: use --no-browser".into())
    );
}

#[test]
fn sign_in_browser_dropbox_success() {
    let _port = lock_port();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let expected_challenge = Arc::new(Mutex::new(String::new()));
    let expected_challenge_open = expected_challenge.clone();
    let token_called = Arc::new(AtomicUsize::new(0));
    let token_called_clone = token_called.clone();

    let transport = MockTransport::new(move |req: &Request| {
        if req.url == DROPBOX.token_url && req.method == "POST" {
            token_called_clone.fetch_add(1, Ordering::SeqCst);
            let body = String::from_utf8_lossy(&req.body);
            let fields = parse_query(&body);
            assert_eq!(
                fields.get("grant_type").map(String::as_str),
                Some("authorization_code")
            );
            assert_eq!(
                fields.get("code").map(String::as_str),
                Some("dbx-browser-code")
            );
            assert_eq!(
                fields.get("redirect_uri").map(String::as_str),
                Some(DROPBOX_REDIRECT)
            );
            assert_eq!(
                fields.get("client_id").map(String::as_str),
                Some("test-dropbox-key")
            );
            assert!(
                !fields.contains_key("client_secret"),
                "Dropbox sign-in sends no client_secret"
            );
            let verifier = fields.get("code_verifier").expect("code_verifier sent");
            assert_eq!(
                challenge(verifier),
                *expected_challenge.lock().unwrap(),
                "the verifier must match the challenge in the auth URL"
            );

            return Ok(Response::new(
                200,
                json!({
                    "access_token": "dbx-browser-access",
                    "expires_in": 14400,
                    "refresh_token": "dbx-browser-refresh",
                    "token_type": "bearer"
                })
                .to_string(),
            ));
        }
        panic!("unexpected request: {:?}", req);
    });

    let open = |url: &str| {
        let tx = tx.clone();
        let params = parse_query(url);
        assert_eq!(
            params.get("redirect_uri").map(String::as_str),
            Some(DROPBOX_REDIRECT)
        );
        assert_eq!(
            params.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        *expected_challenge_open.lock().unwrap() =
            params.get("code_challenge").cloned().unwrap_or_default();
        let state = params.get("state").expect("state in auth URL").clone();

        std::thread::spawn(move || {
            let addr = DROPBOX_REDIRECT
                .strip_prefix("http://")
                .and_then(|s| s.strip_suffix('/'))
                .unwrap();
            let mut stream = TcpStream::connect(addr).expect("connect to the Dropbox redirect");
            let req = format!("GET /?code=dbx-browser-code&state={state} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
            stream.write_all(req.as_bytes()).unwrap();
            let mut resp = String::new();
            let _ = stream.read_to_string(&mut resp);
            let _ = tx.send(resp);
        });
        Ok(())
    };

    let tokens = sign_in_browser(&DROPBOX, DROPBOX_CLIENT, &transport, &open, || 1000)
        .expect("Dropbox browser sign-in succeeds");
    assert_eq!(tokens.access.expose(), "dbx-browser-access");
    assert_eq!(tokens.refresh.unwrap().expose(), "dbx-browser-refresh");
    assert_eq!(token_called.load(Ordering::SeqCst), 1);

    let resp = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("browser got a response");
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "{resp}");
    assert!(resp.contains("Signed in to save-relay"), "{resp}");
}

#[test]
fn finish_code_google_zero_interval_is_floored_to_one_second() {
    let polls = AtomicUsize::new(0);
    let transport = MockTransport::new(|req: &Request| {
        assert_eq!(req.url, GOOGLE.token_url);
        if polls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(Response::new(
                400,
                json!({"error": "authorization_pending"}).to_string(),
            ));
        }
        Ok(Response::new(
            200,
            json!({"access_token": "zero-interval-access", "expires_in": 3600}).to_string(),
        ))
    });
    let pending = Pending {
        service: Service::Gdrive,
        url: "https://www.google.com/device".into(),
        user_code: Some("USER-CODE".into()),
        expires_at: 2000,
        interval: 0,
        secret: Secret::new("dev-code"),
    };
    let sleeps = Mutex::new(Vec::new());
    let tokens = finish_code_with_sleep(
        &GOOGLE,
        GOOGLE_CLIENT,
        &pending,
        None,
        &transport,
        || 1000,
        &|secs| sleeps.lock().unwrap().push(secs),
    )
    .expect("finish_code succeeds after one pending poll");
    assert_eq!(tokens.access.expose(), "zero-interval-access");
    assert_eq!(*sleeps.lock().unwrap(), vec![1]);
}

#[test]
fn start_code_and_finish_code_dropbox() {
    let token_called = Arc::new(AtomicUsize::new(0));
    let token_called_clone = token_called.clone();

    let transport = MockTransport::new(move |req: &Request| {
        if req.url == DROPBOX.token_url && req.method == "POST" {
            token_called_clone.fetch_add(1, Ordering::SeqCst);
            let body = String::from_utf8_lossy(&req.body);
            let fields = parse_query(&body);
            assert_eq!(
                fields.get("grant_type").map(String::as_str),
                Some("authorization_code")
            );
            assert_eq!(
                fields.get("code").map(String::as_str),
                Some("typed-dropbox-code")
            );
            assert_eq!(
                fields.get("client_id").map(String::as_str),
                Some("test-dropbox-key")
            );
            assert!(fields.contains_key("code_verifier"));

            return Ok(Response::new(
                200,
                json!({
                    "access_token": "dbx-access",
                    "expires_in": 14400,
                    "refresh_token": "dbx-refresh",
                    "token_type": "bearer"
                })
                .to_string(),
            ));
        }
        panic!("unexpected request: {:?}", req);
    });

    let pending = start_code(&DROPBOX, DROPBOX_CLIENT, &transport, 1000)
        .expect("start_code succeeds for Dropbox");
    assert_eq!(pending.service, Service::Dropbox);
    assert_eq!(pending.user_code, None);
    assert_eq!(pending.interval, 0);
    assert_eq!(pending.expires_at, 1300);
    assert!(pending.url.starts_with(DROPBOX.authorize_url));
    assert!(!pending.url.contains("redirect_uri"));

    // Finishing without code fails
    let err_no_code = finish_code(&DROPBOX, DROPBOX_CLIENT, &pending, None, &transport, || {
        1050
    })
    .expect_err("finish_code requires code for Dropbox");
    assert!(matches!(err_no_code, Error::SignInFailed(_)));

    // Finishing after expiry fails
    let err_expired = finish_code(
        &DROPBOX,
        DROPBOX_CLIENT,
        &pending,
        Some("typed-dropbox-code"),
        &transport,
        || 1301,
    )
    .expect_err("expired code fails");
    assert_eq!(
        err_expired,
        Error::SignInFailed("the code expired: run setup again".into())
    );

    // Finishing with typed code succeeds
    let tokens = finish_code(
        &DROPBOX,
        DROPBOX_CLIENT,
        &pending,
        Some(" typed-dropbox-code \n"),
        &transport,
        || 1050,
    )
    .expect("finish_code succeeds with valid code");
    assert_eq!(tokens.access.expose(), "dbx-access");
    assert_eq!(tokens.expires_at, 1050 + 14400);
    assert_eq!(tokens.refresh.unwrap().expose(), "dbx-refresh");
    assert_eq!(token_called.load(Ordering::SeqCst), 1);
}

#[test]
fn start_code_and_finish_code_google_with_polling() {
    let dev_endpoint = GOOGLE.device_url.unwrap();
    let poll_count = Arc::new(AtomicUsize::new(0));
    let poll_count_clone = poll_count.clone();

    let transport = MockTransport::new(move |req: &Request| {
        if req.url == dev_endpoint && req.method == "POST" {
            let body = String::from_utf8_lossy(&req.body);
            let fields = parse_query(&body);
            assert_eq!(
                fields.get("client_id").map(String::as_str),
                Some("test-google-id")
            );
            assert_eq!(fields.get("scope").map(String::as_str), Some(GOOGLE.scope));

            return Ok(Response::new(
                200,
                json!({
                    "device_code": "dev-code-xyz",
                    "user_code": "ABCD-EFGH",
                    "verification_url": "https://www.google.com/device",
                    "expires_in": 1800,
                    "interval": 5
                })
                .to_string(),
            ));
        }

        if req.url == GOOGLE.token_url && req.method == "POST" {
            let count = poll_count_clone.fetch_add(1, Ordering::SeqCst);
            let body = String::from_utf8_lossy(&req.body);
            let fields = parse_query(&body);
            assert_eq!(
                fields.get("grant_type").map(String::as_str),
                Some("urn:ietf:params:oauth:grant-type:device_code")
            );
            assert_eq!(
                fields.get("device_code").map(String::as_str),
                Some("dev-code-xyz")
            );
            assert_eq!(
                fields.get("client_id").map(String::as_str),
                Some("test-google-id")
            );
            assert_eq!(
                fields.get("client_secret").map(String::as_str),
                Some("test-google-secret")
            );

            if count == 0 {
                return Ok(Response::new(
                    400,
                    json!({"error": "authorization_pending"}).to_string(),
                ));
            } else if count == 1 {
                return Ok(Response::new(
                    400,
                    json!({"error": "slow_down"}).to_string(),
                ));
            } else {
                return Ok(Response::new(
                    200,
                    json!({
                        "access_token": "google-dev-access",
                        "expires_in": 3600,
                        "refresh_token": "google-dev-refresh",
                        "token_type": "Bearer"
                    })
                    .to_string(),
                ));
            }
        }

        panic!("unexpected request: {:?}", req);
    });

    let pending = start_code(&GOOGLE, GOOGLE_CLIENT, &transport, 1000)
        .expect("start_code succeeds for Google");
    assert_eq!(pending.service, Service::Gdrive);
    assert_eq!(pending.user_code.as_deref(), Some("ABCD-EFGH"));
    assert_eq!(pending.url, "https://www.google.com/device");
    assert_eq!(pending.expires_at, 2800);
    assert_eq!(pending.interval, 5);
    assert_eq!(pending.secret.expose(), "dev-code-xyz");

    let sleeps = Arc::new(Mutex::new(Vec::new()));
    let sleeps_clone = sleeps.clone();
    let tokens = finish_code_with_sleep(
        &GOOGLE,
        GOOGLE_CLIENT,
        &pending,
        None,
        &transport,
        || 1050,
        &|secs| sleeps_clone.lock().unwrap().push(secs),
    )
    .expect("finish_code succeeds after polling");

    assert_eq!(tokens.access.expose(), "google-dev-access");
    assert_eq!(tokens.expires_at, 1050 + 3600);
    assert_eq!(tokens.refresh.unwrap().expose(), "google-dev-refresh");
    assert_eq!(poll_count.load(Ordering::SeqCst), 3);
    // Initial interval 5 for authorization_pending, then interval + 5 = 10 for slow_down
    assert_eq!(*sleeps.lock().unwrap(), vec![5, 10]);
}

#[test]
fn finish_code_google_access_denied() {
    let transport = MockTransport::new(|req: &Request| {
        if req.url == GOOGLE.token_url {
            return Ok(Response::new(
                400,
                json!({"error": "access_denied"}).to_string(),
            ));
        }
        panic!("unexpected request: {:?}", req);
    });

    let pending = Pending {
        service: Service::Gdrive,
        url: "https://www.google.com/device".into(),
        user_code: Some("USER-CODE".into()),
        expires_at: 2000,
        interval: 1,
        secret: Secret::new("dev-code"),
    };

    let err = finish_code(&GOOGLE, GOOGLE_CLIENT, &pending, None, &transport, || 1000)
        .expect_err("must report access_denied");
    assert_eq!(err, Error::SignInFailed("access_denied".into()));
}

#[test]
fn finish_code_google_expired_token_response() {
    let transport = MockTransport::new(|req: &Request| {
        if req.url == GOOGLE.token_url {
            return Ok(Response::new(
                400,
                json!({"error": "expired_token"}).to_string(),
            ));
        }
        panic!("unexpected request: {:?}", req);
    });

    let pending = Pending {
        service: Service::Gdrive,
        url: "https://www.google.com/device".into(),
        user_code: Some("USER-CODE".into()),
        expires_at: 2000,
        interval: 1,
        secret: Secret::new("dev-code"),
    };

    let err = finish_code(&GOOGLE, GOOGLE_CLIENT, &pending, None, &transport, || 1000)
        .expect_err("must report expired_token");
    assert_eq!(
        err,
        Error::SignInFailed("the code expired: run setup again".into())
    );
}

#[test]
fn finish_code_google_expired_by_clock() {
    let transport =
        MockTransport::new(|_| panic!("transport should not be called if already expired"));

    let pending = Pending {
        service: Service::Gdrive,
        url: "https://www.google.com/device".into(),
        user_code: Some("USER-CODE".into()),
        expires_at: 2000,
        interval: 1,
        secret: Secret::new("dev-code"),
    };

    let err = finish_code(&GOOGLE, GOOGLE_CLIENT, &pending, None, &transport, || 2000)
        .expect_err("must fail if now >= expires_at");
    assert_eq!(
        err,
        Error::SignInFailed("the code expired: run setup again".into())
    );
}

#[test]
fn pending_round_trips_through_secret() {
    let p_google = Pending {
        service: Service::Gdrive,
        url: "https://www.google.com/device".into(),
        user_code: Some("ABCD-EFGH".into()),
        expires_at: 2500,
        interval: 5,
        secret: Secret::new("google-device-secret"),
    };

    let sec_google = p_google.to_secret();
    assert_eq!(format!("{sec_google:?}"), "Secret(***)");
    assert_eq!(format!("{sec_google}"), "***");
    let back_google = Pending::from_secret(&sec_google).expect("deserialize google pending");
    assert_eq!(p_google, back_google);

    let p_dropbox = Pending {
        service: Service::Dropbox,
        url: "https://www.dropbox.com/oauth2/authorize?client_id=...".into(),
        user_code: None,
        expires_at: 1500,
        interval: 0,
        secret: Secret::new("pkce-verifier-string"),
    };

    let sec_dropbox = p_dropbox.to_secret();
    let back_dropbox = Pending::from_secret(&sec_dropbox).expect("deserialize dropbox pending");
    assert_eq!(p_dropbox, back_dropbox);

    let bad_secret = Secret::new("{not json");
    let err = Pending::from_secret(&bad_secret).expect_err("invalid json returns error");
    assert!(matches!(err, Error::Credentials(_)));
}

#[test]
fn in_game_mode_cases() {
    // 1. gamescope on Linux
    let var_gamescope = |k: &str| match k {
        "XDG_CURRENT_DESKTOP" => Some("gamescope".into()),
        _ => None,
    };
    assert!(in_game_mode(&var_gamescope));

    // 2. SteamGamepadUI set
    let var_gamepad = |k: &str| match k {
        "SteamGamepadUI" => Some("1".into()),
        _ => None,
    };
    assert!(in_game_mode(&var_gamepad));

    // 3. Desktop session (e.g. GNOME, KDE)
    let var_desktop = |k: &str| match k {
        "XDG_CURRENT_DESKTOP" => Some("ubuntu:GNOME".into()),
        _ => None,
    };
    assert!(!in_game_mode(&var_desktop));

    // 4. Nothing set
    let var_empty = |_k: &str| None;
    assert!(!in_game_mode(&var_empty));

    // 5. SteamGamepadUI empty string
    let var_empty_gamepad = |k: &str| match k {
        "SteamGamepadUI" => Some("".into()),
        _ => None,
    };
    assert!(!in_game_mode(&var_empty_gamepad));
}
