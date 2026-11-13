//! #250 item 12: a command that panics is a 500 for that request; the server keeps serving.
use save_relay::api::PanelData;
use save_relay::panel::{self, TOKEN_HEADER};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn exchange(port: u16, request: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    String::from_utf8_lossy(&answer).to_string()
}

fn call(port: u16, token: &str, command: &str) -> String {
    let body = if command == "where" {
        "{}"
    } else {
        r#"{"game":"g"}"#
    };
    exchange(
        port,
        &format!(
            "POST /api/{command} HTTP/1.1\r\nHost: 127.0.0.1\r\n{TOKEN_HEADER}: {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    )
}

#[test]
fn a_panicking_command_is_a_500_and_the_next_request_is_served() {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let announce = |data: &PanelData| {
            let _ = sender.send(data.clone());
        };
        let run = |args: &[String]| -> (i32, Vec<u8>) {
            if args[0] == "status" {
                panic!("boom");
            }
            (0, br#"{"ok":true,"command":"where","data":{}}"#.to_vec())
        };
        let _ = panel::serve(Some(0), false, &announce, &run);
    });
    let data = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    let token = data.url.split("token=").nth(1).unwrap().to_string();

    let first = call(data.port, &token, "status");
    assert!(first.starts_with("HTTP/1.1 500"), "{first}");
    let second = call(data.port, &token, "where");
    assert!(second.starts_with("HTTP/1.1 200"), "{second}");
}
