//! Locked tests for `WebDav::list` and `WebDav::get` (F17a). Do not edit.
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

use save_relay::remote::{Remote, RemoteEntry};

const LISTING: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
  <d:response>
    <d:href>/remote.php/dav/files/alex/save-relay/star%20runner/</d:href>
    <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
    <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/files/alex/save-relay/star%20runner/slot%201.sav</d:href>
    <d:propstat><d:prop><d:resourcetype/><d:getcontentlength>12</d:getcontentlength>
    <d:getetag>"e1"</d:getetag></d:prop>
    <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
</d:multistatus>"#;

fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request.header_value(name)
}

#[test]
fn every_request_has_basic_auth() {
    let (webdav, fake) = dav(vec![Ok(Response::new(200, "x").header("ETag", "\"a\""))]);
    webdav.get("f.sav").unwrap();
    assert_eq!(
        header(&sent(&fake)[0], "Authorization"),
        Some("Basic YWxleDpzM2NyZXQ=")
    );
}

#[test]
fn get_sends_get_to_the_encoded_url_and_returns_body_and_etag() {
    let (webdav, fake) = dav(vec![Ok(
        Response::new(200, "hello").header("ETag", "\"abc\"")
    )]);
    let (bytes, etag) = webdav.get("save-relay/star runner/slot 1.sav").unwrap();
    assert_eq!(bytes, b"hello");
    assert_eq!(etag, "\"abc\"");
    let request = &sent(&fake)[0];
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.url,
        "https://cloud.test/remote.php/dav/files/alex/save-relay/star%20runner/slot%201.sav"
    );
}

#[test]
fn get_without_an_etag_header_is_a_server_error() {
    let (webdav, _) = dav(vec![Ok(Response::new(200, "hello"))]);
    assert_eq!(
        webdav.get("a/b").unwrap_err(),
        Error::Server("GET a/b: no ETag".into())
    );
}

#[test]
fn get_404_is_not_found_with_the_path() {
    let (webdav, _) = dav(vec![Ok(Response::new(404, ""))]);
    assert_eq!(
        webdav.get("a/b").unwrap_err(),
        Error::NotFound("a/b".into())
    );
}

#[test]
fn list_sends_propfind_depth_1_and_parses_the_children() {
    let (webdav, fake) = dav(vec![Ok(Response::new(207, LISTING))]);
    let entries = webdav.list("save-relay/star runner").unwrap();
    assert_eq!(
        entries,
        vec![RemoteEntry {
            name: "slot 1.sav".into(),
            is_dir: false,
            size: 12,
            etag: Some("\"e1\"".into()),
        }]
    );
    let request = &sent(&fake)[0];
    assert_eq!(request.method, "PROPFIND");
    assert_eq!(header(request, "Depth"), Some("1"));
    assert_eq!(
        request.url,
        "https://cloud.test/remote.php/dav/files/alex/save-relay/star%20runner"
    );
}

#[test]
fn list_404_is_not_found_with_the_dir() {
    let (webdav, _) = dav(vec![Ok(Response::new(404, ""))]);
    assert_eq!(
        webdav.list("nope").unwrap_err(),
        Error::NotFound("nope".into())
    );
}

#[test]
fn list_with_a_non_207_answer_is_a_server_error() {
    let (webdav, _) = dav(vec![Ok(Response::new(200, ""))]);
    assert_eq!(
        webdav.list("d").unwrap_err(),
        Error::Server("PROPFIND d: 200".into())
    );
}

#[test]
fn status_401_and_403_are_auth_failed() {
    for status in [401, 403] {
        let (webdav, _) = dav(vec![Ok(Response::new(status, ""))]);
        assert_eq!(webdav.get("f").unwrap_err(), Error::AuthFailed);
    }
}

#[test]
fn status_409_is_not_found_with_the_parent() {
    let (webdav, _) = dav(vec![Ok(Response::new(409, ""))]);
    assert_eq!(
        webdav.get("a/b/c.sav").unwrap_err(),
        Error::NotFound("a/b".into())
    );
}

#[test]
fn another_status_names_the_method_path_and_status() {
    let (webdav, _) = dav(vec![Ok(Response::new(500, ""))]);
    assert_eq!(
        webdav.get("a/b").unwrap_err(),
        Error::Server("GET a/b: 500".into())
    );
}

#[test]
fn a_transport_failure_is_passed_on_unchanged() {
    let (webdav, _) = dav(vec![Err(Error::Unreachable("timed out".into()))]);
    assert_eq!(
        webdav.list("d").unwrap_err(),
        Error::Unreachable("timed out".into())
    );
}

#[test]
fn no_error_text_shows_the_password() {
    for status in [401, 403, 404, 409, 412, 500] {
        let (webdav, _) = dav(vec![Ok(Response::new(status, ""))]);
        let error = webdav.get("a/b").unwrap_err();
        let text = format!("{error} {error:?}");
        assert!(!text.contains("s3cret"), "status {status}: {text}");
        assert!(
            !text.contains("YWxleDpzM2NyZXQ="),
            "status {status}: {text}"
        );
    }
}
