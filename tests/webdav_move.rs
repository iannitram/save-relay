//! Locked tests for `WebDav::copy`, `WebDav::move_to` and `WebDav::delete` (F17c). Do not edit.
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

use save_relay::remote::Remote;

const SRC: &str = "save-relay/g/incoming/e 1";
const DST: &str = "save-relay/g/history/e 1";

fn check_headers(request: &Request, method: &str) {
    assert_eq!(request.method, method);
    assert_eq!(
        request.url,
        "https://cloud.test/remote.php/dav/files/alex/save-relay/g/incoming/e%201"
    );
    assert_eq!(
        request.header_value("Destination"),
        Some("https://cloud.test/remote.php/dav/files/alex/save-relay/g/history/e%201")
    );
    assert_eq!(request.header_value("Overwrite"), Some("F"));
    assert_eq!(
        request.header_value("Authorization"),
        Some("Basic YWxleDpzM2NyZXQ=")
    );
}

#[test]
fn copy_sends_copy_with_destination_and_overwrite_f() {
    let (webdav, fake) = dav(vec![Ok(Response::new(201, ""))]);
    webdav.copy(SRC, DST).unwrap();
    check_headers(&sent(&fake)[0], "COPY");
}

#[test]
fn move_sends_move_with_destination_and_overwrite_f() {
    let (webdav, fake) = dav(vec![Ok(Response::new(201, ""))]);
    webdav.move_to(SRC, DST).unwrap();
    check_headers(&sent(&fake)[0], "MOVE");
}

#[test]
fn copy_and_move_accept_201_and_204() {
    for status in [201, 204] {
        let (webdav, _) = dav(vec![Ok(Response::new(status, ""))]);
        assert_eq!(webdav.copy("a", "b"), Ok(()));
        let (webdav, _) = dav(vec![Ok(Response::new(status, ""))]);
        assert_eq!(webdav.move_to("a", "b"), Ok(()));
    }
}

#[test]
fn copy_and_move_412_is_server_exists() {
    let (webdav, _) = dav(vec![Ok(Response::new(412, ""))]);
    assert_eq!(
        webdav.copy("a", "b").unwrap_err(),
        Error::Server("exists".into())
    );
    let (webdav, _) = dav(vec![Ok(Response::new(412, ""))]);
    assert_eq!(
        webdav.move_to("a", "b").unwrap_err(),
        Error::Server("exists".into())
    );
}

#[test]
fn copy_and_move_404_names_the_source() {
    let (webdav, _) = dav(vec![Ok(Response::new(404, ""))]);
    assert_eq!(
        webdav.copy("a/x", "b").unwrap_err(),
        Error::NotFound("a/x".into())
    );
    let (webdav, _) = dav(vec![Ok(Response::new(404, ""))]);
    assert_eq!(
        webdav.move_to("a/x", "b").unwrap_err(),
        Error::NotFound("a/x".into())
    );
}

#[test]
fn copy_and_move_409_names_the_destination_parent() {
    let (webdav, _) = dav(vec![Ok(Response::new(409, ""))]);
    assert_eq!(
        webdav.copy("a/x", "p/q/r").unwrap_err(),
        Error::NotFound("p/q".into())
    );
    let (webdav, _) = dav(vec![Ok(Response::new(409, ""))]);
    assert_eq!(
        webdav.move_to("a/x", "p/q/r").unwrap_err(),
        Error::NotFound("p/q".into())
    );
}

#[test]
fn copy_and_move_other_statuses() {
    let (webdav, _) = dav(vec![Ok(Response::new(401, ""))]);
    assert_eq!(webdav.copy("a", "b").unwrap_err(), Error::AuthFailed);
    let (webdav, _) = dav(vec![Ok(Response::new(500, ""))]);
    assert_eq!(
        webdav.copy("a", "b").unwrap_err(),
        Error::Server("COPY a: 500".into())
    );
    let (webdav, _) = dav(vec![Ok(Response::new(500, ""))]);
    assert_eq!(
        webdav.move_to("a", "b").unwrap_err(),
        Error::Server("MOVE a: 500".into())
    );
}

#[test]
fn delete_sends_delete_to_the_encoded_url() {
    let (webdav, fake) = dav(vec![Ok(Response::new(204, ""))]);
    webdav.delete("save-relay/g/old entry").unwrap();
    let request = &sent(&fake)[0];
    assert_eq!(request.method, "DELETE");
    assert_eq!(
        request.url,
        "https://cloud.test/remote.php/dav/files/alex/save-relay/g/old%20entry"
    );
    assert_eq!(
        request.header_value("Authorization"),
        Some("Basic YWxleDpzM2NyZXQ=")
    );
}

#[test]
fn delete_accepts_200_and_204() {
    for status in [200, 204] {
        let (webdav, _) = dav(vec![Ok(Response::new(status, ""))]);
        assert_eq!(webdav.delete("f"), Ok(()));
    }
}

#[test]
fn delete_404_names_the_path() {
    let (webdav, _) = dav(vec![Ok(Response::new(404, ""))]);
    assert_eq!(
        webdav.delete("a/b").unwrap_err(),
        Error::NotFound("a/b".into())
    );
}

#[test]
fn delete_other_statuses() {
    let (webdav, _) = dav(vec![Ok(Response::new(403, ""))]);
    assert_eq!(webdav.delete("f").unwrap_err(), Error::AuthFailed);
    let (webdav, _) = dav(vec![Ok(Response::new(423, ""))]);
    assert_eq!(
        webdav.delete("f").unwrap_err(),
        Error::Server("DELETE f: 423".into())
    );
}

#[test]
fn a_transport_failure_is_passed_on_unchanged() {
    let (webdav, _) = dav(vec![Err(Error::Unreachable("tls".into()))]);
    assert_eq!(
        webdav.delete("f").unwrap_err(),
        Error::Unreachable("tls".into())
    );
    let (webdav, _) = dav(vec![Err(Error::Unreachable("tls".into()))]);
    assert_eq!(
        webdav.move_to("a", "b").unwrap_err(),
        Error::Unreachable("tls".into())
    );
}
