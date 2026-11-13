//! #250 item 6: a stray request to the sign-in loopback listener does not abort the sign-in.
use save_relay::clients::GOOGLE_DESKTOP;
use save_relay::http::{Request, Response, Transport};
use save_relay::oauth::{sign_in_browser, GOOGLE};
use save_relay::Result;
use std::io::{Read, Write};
use std::net::TcpStream;

struct Mock;
impl Transport for Mock {
    fn send(&self, request: &Request) -> Result<Response> {
        assert_eq!(request.url, GOOGLE.token_url);
        Ok(Response::new(
            200,
            r#"{"access_token":"a","refresh_token":"r","expires_in":3600}"#,
        ))
    }
}

fn query(url: &str, key: &str) -> String {
    let query = url.split_once('?').unwrap().1;
    let pair = query
        .split('&')
        .find_map(|p| p.strip_prefix(&format!("{key}=")))
        .unwrap();
    pair.replace("%3A", ":").replace("%2F", "/")
}

fn request(addr: &str, target: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect");
    let line = format!("GET {target} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(line.as_bytes()).unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    answer
}

#[test]
fn a_request_with_no_sign_in_parameters_is_answered_and_ignored() {
    let open = |url: &str| {
        let redirect = query(url, "redirect_uri");
        let state = query(url, "state");
        let addr = redirect
            .strip_prefix("http://")
            .and_then(|s| s.strip_suffix('/'))
            .unwrap()
            .to_string();
        std::thread::spawn(move || {
            let stray = request(&addr, "/");
            assert!(!stray.contains("Signed in"), "{stray}");
            let real = request(&addr, &format!("/?code=c&state={state}"));
            assert!(real.starts_with("HTTP/1.1 200 OK"), "{real}");
        });
        Ok(())
    };
    let tokens = sign_in_browser(&GOOGLE, GOOGLE_DESKTOP, &Mock, &open, || 1000);
    assert_eq!(
        tokens
            .expect("sign-in survives the stray request")
            .access
            .expose(),
        "a"
    );
}
