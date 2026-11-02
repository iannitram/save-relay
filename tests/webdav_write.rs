//! Locked tests for `WebDav::put` and `WebDav::mkcol` (F17b). Do not edit.
use save_relay::creds::Secret;
use save_relay::http::{Request, Response, Transport};
use save_relay::webdav::WebDav;
use save_relay::{Error, Result};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const FILES: &str = "https://cloud.test/remote.php/dav/files/alex/";

struct Fake {
    log: Mutex<Vec<Request>>,
    replies: Mutex<VecDeque<Result<Response>>>,
}

impl Transport for Fake {
    fn send(&self, request: &Request) -> Result<Response> {
        self.log.lock().unwrap().push(request.clone());
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("a reply was queued for this request")
    }
}

fn dav(replies: Vec<Result<Response>>) -> (WebDav, Arc<Fake>) {
    let fake = Arc::new(Fake {
        log: Mutex::new(Vec::new()),
        replies: Mutex::new(replies.into()),
    });
    let transport: Arc<dyn Transport> = fake.clone();
    (
        WebDav::new(FILES, "alex", Secret::new("s3cret"), transport),
        fake,
    )
}

fn sent(fake: &Fake) -> Vec<Request> {
    fake.log.lock().unwrap().clone()
}

use save_relay::remote::{Precondition, Remote};

fn ok_put(etag: &str) -> Result<Response> {
    Ok(Response::new(201, "").header("ETag", etag))
}

#[test]
fn put_sends_the_bytes_to_the_encoded_url_and_returns_the_etag() {
    let (webdav, fake) = dav(vec![ok_put("\"n1\"")]);
    let etag = webdav
        .put("save-relay/a b/x.sav", b"bytes", &Precondition::Any)
        .unwrap();
    assert_eq!(etag, "\"n1\"");
    let request = &sent(&fake)[0];
    assert_eq!(request.method, "PUT");
    assert_eq!(
        request.url,
        "https://cloud.test/remote.php/dav/files/alex/save-relay/a%20b/x.sav"
    );
    assert_eq!(request.body, b"bytes");
    assert_eq!(
        request.header_value("Authorization"),
        Some("Basic YWxleDpzM2NyZXQ=")
    );
}

#[test]
fn put_any_sends_no_condition_header() {
    let (webdav, fake) = dav(vec![ok_put("\"n\"")]);
    webdav.put("f", b"", &Precondition::Any).unwrap();
    let request = &sent(&fake)[0];
    assert_eq!(request.header_value("If-Match"), None);
    assert_eq!(request.header_value("If-None-Match"), None);
}

#[test]
fn put_match_sends_if_match_with_the_etag_as_given() {
    let (webdav, fake) = dav(vec![ok_put("\"n\"")]);
    webdav
        .put("f", b"", &Precondition::Match("\"old\"".into()))
        .unwrap();
    let request = &sent(&fake)[0];
    assert_eq!(request.header_value("If-Match"), Some("\"old\""));
    assert_eq!(request.header_value("If-None-Match"), None);
}

#[test]
fn put_absent_sends_if_none_match_star() {
    let (webdav, fake) = dav(vec![ok_put("\"n\"")]);
    webdav.put("f", b"", &Precondition::Absent).unwrap();
    let request = &sent(&fake)[0];
    assert_eq!(request.header_value("If-None-Match"), Some("*"));
    assert_eq!(request.header_value("If-Match"), None);
}

#[test]
fn put_accepts_200_201_and_204() {
    for status in [200, 201, 204] {
        let (webdav, _) = dav(vec![Ok(Response::new(status, "").header("ETag", "\"e\""))]);
        assert_eq!(webdav.put("f", b"", &Precondition::Any).unwrap(), "\"e\"");
    }
}

#[test]
fn put_without_an_etag_header_is_a_server_error() {
    let (webdav, _) = dav(vec![Ok(Response::new(201, ""))]);
    assert_eq!(
        webdav.put("a/f", b"", &Precondition::Any).unwrap_err(),
        Error::Server("PUT a/f: no ETag".into())
    );
}

#[test]
fn put_412_is_changed() {
    let (webdav, _) = dav(vec![Ok(Response::new(412, ""))]);
    assert_eq!(
        webdav.put("f", b"", &Precondition::Absent).unwrap_err(),
        Error::Changed
    );
}

#[test]
fn put_409_is_not_found_with_the_parent() {
    let (webdav, _) = dav(vec![Ok(Response::new(409, ""))]);
    assert_eq!(
        webdav.put("a/b/f", b"", &Precondition::Any).unwrap_err(),
        Error::NotFound("a/b".into())
    );
}

#[test]
fn put_409_at_the_top_has_the_empty_parent() {
    let (webdav, _) = dav(vec![Ok(Response::new(409, ""))]);
    assert_eq!(
        webdav.put("f", b"", &Precondition::Any).unwrap_err(),
        Error::NotFound(String::new())
    );
}

#[test]
fn put_other_statuses() {
    let (webdav, _) = dav(vec![Ok(Response::new(401, ""))]);
    assert_eq!(
        webdav.put("f", b"", &Precondition::Any).unwrap_err(),
        Error::AuthFailed
    );
    let (webdav, _) = dav(vec![Ok(Response::new(507, ""))]);
    assert_eq!(
        webdav.put("f", b"", &Precondition::Any).unwrap_err(),
        Error::Server("PUT f: 507".into())
    );
}

#[test]
fn mkcol_sends_mkcol_to_the_encoded_url() {
    let (webdav, fake) = dav(vec![Ok(Response::new(201, ""))]);
    webdav.mkcol("save-relay/new dir").unwrap();
    let request = &sent(&fake)[0];
    assert_eq!(request.method, "MKCOL");
    assert_eq!(
        request.url,
        "https://cloud.test/remote.php/dav/files/alex/save-relay/new%20dir"
    );
    assert_eq!(
        request.header_value("Authorization"),
        Some("Basic YWxleDpzM2NyZXQ=")
    );
}

#[test]
fn mkcol_405_on_an_existing_folder_is_ok() {
    let (webdav, _) = dav(vec![Ok(Response::new(405, ""))]);
    assert_eq!(webdav.mkcol("d"), Ok(()));
}

#[test]
fn mkcol_409_is_not_found_with_the_parent() {
    let (webdav, _) = dav(vec![Ok(Response::new(409, ""))]);
    assert_eq!(
        webdav.mkcol("a/b/c").unwrap_err(),
        Error::NotFound("a/b".into())
    );
}

#[test]
fn mkcol_other_statuses() {
    let (webdav, _) = dav(vec![Ok(Response::new(403, ""))]);
    assert_eq!(webdav.mkcol("d").unwrap_err(), Error::AuthFailed);
    let (webdav, _) = dav(vec![Ok(Response::new(500, ""))]);
    assert_eq!(
        webdav.mkcol("d").unwrap_err(),
        Error::Server("MKCOL d: 500".into())
    );
}

#[test]
fn a_transport_failure_is_passed_on_unchanged() {
    let (webdav, _) = dav(vec![Err(Error::Unreachable("dns".into()))]);
    assert_eq!(
        webdav.put("f", b"", &Precondition::Any).unwrap_err(),
        Error::Unreachable("dns".into())
    );
    let (webdav, _) = dav(vec![Err(Error::Unreachable("dns".into()))]);
    assert_eq!(
        webdav.mkcol("d").unwrap_err(),
        Error::Unreachable("dns".into())
    );
}
