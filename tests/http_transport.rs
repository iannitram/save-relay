#![cfg(feature = "http")]

use save_relay::error::Error;
use save_relay::http::{Request, Transport, UreqTransport};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::Instant;

#[derive(Debug)]
struct Captured {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Captured {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

fn read_request(stream: &mut TcpStream) -> Captured {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap().to_string();
    let path = parts.next().unwrap().to_string();

    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').unwrap();
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }

    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.parse::<usize>().unwrap())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();

    Captured {
        method,
        path,
        headers,
        body,
    }
}

fn serve_once(status: u16, body: &'static str) -> (String, JoinHandle<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let captured = read_request(&mut stream);
        let reason = match status {
            200 => "OK",
            201 => "Created",
            207 => "Multi-Status",
            302 => "Found",
            404 => "Not Found",
            405 => "Method Not Allowed",
            500 => "Internal Server Error",
            _ => "Status",
        };
        write!(
            stream,
            "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            status,
            reason,
            body.len(),
            body
        )
        .unwrap();
        stream.flush().unwrap();
        captured
    });
    (format!("http://{address}"), handle)
}

fn transport() -> UreqTransport {
    UreqTransport::new()
}

#[test]
fn get_sends_headers_and_returns_the_answer() {
    let (base, server) = serve_once(200, "hello");
    let request = Request::new("GET", &format!("{base}/files/me/save"))
        .header("X-Test", "one")
        .header("Authorization", "Bearer secret");

    let response = transport().send(&request).unwrap();
    let captured = server.join().unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(response.text(), "hello");
    assert_eq!(captured.method, "GET");
    assert_eq!(captured.path, "/files/me/save");
    assert_eq!(captured.header("x-test"), Some("one"));
    assert_eq!(captured.header("authorization"), Some("Bearer secret"));
}

#[test]
fn response_body_limit_stops_an_oversized_answer() {
    let (base, server) = serve_once(200, "hello");
    let request = Request::new("GET", &format!("{base}/manifest")).max_response_bytes(4);

    assert!(
        matches!(transport().send(&request), Err(Error::Server(message)) if message.contains("4-byte limit"))
    );
    server.join().unwrap();
}

#[test]
fn put_sends_the_body() {
    let (base, server) = serve_once(201, "made");
    let request = Request::new("PUT", &format!("{base}/files/me/save")).body(b"saves".to_vec());

    let response = transport().send(&request).unwrap();
    let captured = server.join().unwrap();

    assert_eq!(response.status, 201);
    assert_eq!(captured.method, "PUT");
    assert_eq!(captured.body, b"saves");
}

#[test]
fn post_500_comes_back_as_a_response() {
    let (base, server) = serve_once(500, "boom");
    let request = Request::new("POST", &format!("{base}/token")).body(b"form".to_vec());

    let response = transport().send(&request).unwrap();
    let captured = server.join().unwrap();

    assert_eq!(response.status, 500);
    assert_eq!(response.text(), "boom");
    assert_eq!(captured.method, "POST");
}

#[test]
fn propfind_sends_depth_and_body() {
    let (base, server) = serve_once(207, "<multistatus/>");
    let request = Request::new("PROPFIND", &format!("{base}/files/me/"))
        .header("Depth", "1")
        .body(b"<propfind/>".to_vec());

    let response = transport().send(&request).unwrap();
    let captured = server.join().unwrap();

    assert_eq!(response.status, 207);
    assert_eq!(captured.method, "PROPFIND");
    assert_eq!(captured.header("depth"), Some("1"));
    assert_eq!(captured.body, b"<propfind/>");
}

#[test]
fn mkcol_405_comes_back_as_a_response() {
    let (base, server) = serve_once(405, "no");
    let request = Request::new("MKCOL", &format!("{base}/files/me/folder"));

    let response = transport().send(&request).unwrap();
    let captured = server.join().unwrap();

    assert_eq!(response.status, 405);
    assert_eq!(captured.method, "MKCOL");
}

#[test]
fn copy_sends_destination() {
    let (base, server) = serve_once(201, "copied");
    let request = Request::new("COPY", &format!("{base}/files/me/one")).header(
        "Destination",
        "https://cloud.example.org/remote.php/dav/files/me/two",
    );

    let response = transport().send(&request).unwrap();
    let captured = server.join().unwrap();

    assert_eq!(response.status, 201);
    assert_eq!(captured.method, "COPY");
    assert_eq!(
        captured.header("destination"),
        Some("https://cloud.example.org/remote.php/dav/files/me/two")
    );
}

#[test]
fn move_sends_destination() {
    let (base, server) = serve_once(201, "moved");
    let request = Request::new("MOVE", &format!("{base}/files/me/one")).header(
        "Destination",
        "https://cloud.example.org/remote.php/dav/files/me/two",
    );

    let response = transport().send(&request).unwrap();
    let captured = server.join().unwrap();

    assert_eq!(response.status, 201);
    assert_eq!(captured.method, "MOVE");
    assert_eq!(
        captured.header("destination"),
        Some("https://cloud.example.org/remote.php/dav/files/me/two")
    );
}

#[test]
fn delete_404_comes_back_as_a_response() {
    let (base, server) = serve_once(404, "gone");
    let request = Request::new("DELETE", &format!("{base}/files/me/save"));

    let response = transport().send(&request).unwrap();
    let captured = server.join().unwrap();

    assert_eq!(response.status, 404);
    assert_eq!(captured.method, "DELETE");
}

#[test]
fn move_302_is_returned_not_followed() {
    let (base, server) = serve_once(302, "elsewhere");
    let request = Request::new("MOVE", &format!("{base}/files/me/one")).header(
        "Destination",
        "https://cloud.example.org/remote.php/dav/files/me/two",
    );

    let response = transport().send(&request).unwrap();
    let captured = server.join().unwrap();

    assert_eq!(response.status, 302);
    assert_eq!(captured.method, "MOVE");
}

#[test]
fn refused_connection_is_unreachable_and_quick() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);

    let request = Request::new("GET", &format!("http://{address}/nowhere"))
        .header("Authorization", "Bearer never-shown");

    let started = Instant::now();
    let error = transport().send(&request).unwrap_err();
    let elapsed = started.elapsed();

    assert!(matches!(error, Error::Unreachable(_)), "{error:?}");
    assert!(!error.to_string().contains("never-shown"));
    if let Error::Unreachable(message) = &error {
        assert!(!message.contains("never-shown"));
    }
    assert!(elapsed.as_secs() < 6, "{elapsed:?}");
}
