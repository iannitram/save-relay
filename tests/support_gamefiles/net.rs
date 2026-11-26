//! A real HTTP server (std sockets) that serves Dropbox-shaped ranged downloads, and the
//! transport that reaches it through the real `UreqTransport`.
#![allow(dead_code)]

use super::manifest;
use save_relay::clients::Client;
use save_relay::creds::Secret;
use save_relay::dropbox::{Dropbox, CONTENT};
use save_relay::fake::{FakeDropbox, REFRESH_TOKEN};
use save_relay::http::{Request, Response, Transport, UreqTransport};
use save_relay::oauth::{TokenSource, DROPBOX};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

struct ServerState {
    files: HashMap<String, Vec<u8>>,
    revs: HashMap<String, String>,
    ignore_range: bool,
    drop_next: u32,
    fail: Vec<u16>,
    /// `Retry-After` seconds sent with each scripted failure.
    retry_after: Option<u64>,
    /// Leave `rev` out of `Dropbox-API-Result`.
    no_rev: bool,
    log: Vec<(String, Option<String>)>,
    bytes_sent: u64,
}

/// A real HTTP/1.1 server speaking `POST /2/files/download` with `Dropbox-API-Arg`, `Range`
/// and `Dropbox-API-Result`, on 127.0.0.1. It lives until the test process ends.
#[derive(Clone)]
pub struct RangeServer {
    pub addr: SocketAddr,
    state: Arc<Mutex<ServerState>>,
}

impl RangeServer {
    pub fn start() -> RangeServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(ServerState {
            files: HashMap::new(),
            revs: HashMap::new(),
            ignore_range: false,
            drop_next: 0,
            fail: Vec::new(),
            retry_after: None,
            no_rev: false,
            log: Vec::new(),
            bytes_sent: 0,
        }));
        let shared = state.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let shared = shared.clone();
                std::thread::spawn(move || serve(stream, shared));
            }
        });
        RangeServer { addr, state }
    }
    /// A handle to a server another process runs on `port` (only its transport is usable).
    pub fn connect(port: u16) -> RangeServer {
        RangeServer {
            addr: SocketAddr::from(([127, 0, 0, 1], port)),
            state: Arc::new(Mutex::new(ServerState {
                files: HashMap::new(),
                revs: HashMap::new(),
                ignore_range: false,
                drop_next: 0,
                fail: Vec::new(),
                retry_after: None,
                no_rev: false,
                log: Vec::new(),
                bytes_sent: 0,
            })),
        }
    }
    pub fn put(&self, path: &str, bytes: Vec<u8>) {
        self.state.lock().unwrap().files.insert(path.into(), bytes);
    }
    pub fn put_game(&self, app: &str, files: &[(&str, &[u8])]) {
        self.put(
            &format!("game-files/{app}/manifest.json"),
            manifest(app, files),
        );
        for (path, bytes) in files {
            self.put(&format!("game-files/{app}/files/{path}"), bytes.to_vec());
        }
    }
    pub fn set_rev(&self, path: &str, rev: &str) {
        self.state
            .lock()
            .unwrap()
            .revs
            .insert(path.into(), rev.into());
    }
    pub fn ignore_range(&self, on: bool) {
        self.state.lock().unwrap().ignore_range = on;
    }
    pub fn drop_next(&self, n: u32) {
        self.state.lock().unwrap().drop_next = n;
    }
    /// Answer the next requests with these statuses (an error JSON body), in order.
    pub fn fail_with(&self, statuses: &[u16]) {
        self.state.lock().unwrap().fail = statuses.to_vec();
    }
    /// Send `Retry-After: seconds` with every scripted failure (`None`: no header).
    pub fn retry_after(&self, seconds: Option<u64>) {
        self.state.lock().unwrap().retry_after = seconds;
    }
    /// Answer without a `rev` in `Dropbox-API-Result`.
    pub fn no_rev(&self, on: bool) {
        self.state.lock().unwrap().no_rev = on;
    }
    /// (path, Range header) of every download so far.
    pub fn log(&self) -> Vec<(String, Option<String>)> {
        self.state.lock().unwrap().log.clone()
    }
    pub fn clear_log(&self) {
        self.state.lock().unwrap().log.clear();
    }
    pub fn bytes_sent(&self) -> u64 {
        self.state.lock().unwrap().bytes_sent
    }
    pub fn transport(&self) -> Arc<LocalTransport> {
        Arc::new(LocalTransport {
            server: self.addr,
            real: UreqTransport::new(),
            fake: Arc::new(FakeDropbox::new()),
        })
    }
}

fn serve(mut stream: TcpStream, state: Arc<Mutex<ServerState>>) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let header = |name: &str| -> Option<String> {
        text.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name)
                .then(|| value.trim().to_string())
        })
    };
    if let Some(length) = header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        let mut sink = vec![0u8; length];
        let _ = stream.read_exact(&mut sink);
    }
    let arg: serde_json::Value =
        serde_json::from_str(&header("dropbox-api-arg").unwrap_or_default()).unwrap_or_default();
    let path = arg["path"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches('/')
        .to_string();
    let range = header("range");
    let mut st = state.lock().unwrap();
    st.log.push((path.clone(), range.clone()));
    if st.drop_next > 0 {
        st.drop_next -= 1;
        return;
    }
    if !st.fail.is_empty() {
        let status = st.fail.remove(0);
        let retry = st
            .retry_after
            .map(|s| format!("Retry-After: {s}\r\n"))
            .unwrap_or_default();
        let json = br#"{"error_summary":"scripted failure for the test, longer than ten bytes"}"#;
        let _ = write!(
            stream,
            "HTTP/1.1 {status} Scripted\r\n{retry}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            json.len()
        );
        let _ = stream.write_all(json);
        return;
    }
    let Some(file) = st.files.get(&path).cloned() else {
        let json = br#"{"error_summary":"path/not_found/.."}"#;
        let _ = write!(
            stream,
            "HTTP/1.1 409 Conflict\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            json.len()
        );
        let _ = stream.write_all(json);
        return;
    };
    let rev = st
        .revs
        .get(&path)
        .cloned()
        .unwrap_or_else(|| "rev-1".into());
    let result = if st.no_rev {
        format!(r#"{{"size":{}}}"#, file.len())
    } else {
        format!(r#"{{"rev":"{rev}","size":{}}}"#, file.len())
    };
    let parsed = range.as_deref().and_then(|r| {
        let (a, b) = r.strip_prefix("bytes=")?.split_once('-')?;
        Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
    });
    let (status, extra, payload) = match parsed {
        Some((a, b)) if !st.ignore_range => {
            if a >= file.len() {
                (
                    416,
                    format!("Content-Range: bytes */{}\r\n", file.len()),
                    Vec::new(),
                )
            } else {
                let b = b.min(file.len() - 1);
                (
                    206,
                    format!("Content-Range: bytes {a}-{b}/{}\r\n", file.len()),
                    file[a..=b].to_vec(),
                )
            }
        }
        _ => (200, String::new(), file),
    };
    st.bytes_sent += payload.len() as u64;
    drop(st);
    let reason = match status {
        206 => "Partial Content",
        416 => "Range Not Satisfiable",
        _ => "OK",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/octet-stream\r\nDropbox-API-Result: {result}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let _ = stream.write_all(&payload);
}

/// Sends content-host requests to a `RangeServer` over real sockets and everything else (the
/// token endpoint) to a `FakeDropbox`.
pub struct LocalTransport {
    pub server: SocketAddr,
    pub real: UreqTransport,
    pub fake: Arc<FakeDropbox>,
}

impl Transport for LocalTransport {
    fn send(&self, request: &Request) -> save_relay::Result<Response> {
        match request.url.strip_prefix(&CONTENT.replace("/2", "")) {
            Some(rest) => {
                let mut local = request.clone();
                local.url = format!("http://{}{rest}", self.server);
                self.real.send(&local)
            }
            None => self.fake.send(request),
        }
    }
}

/// A Dropbox whose content requests go to `transport`'s server, with no retry waits.
pub fn dropbox(transport: Arc<LocalTransport>) -> Dropbox {
    let tokens = TokenSource::new(
        &DROPBOX,
        Client {
            id: "test",
            secret: None,
        },
        Secret::new(REFRESH_TOKEN),
        transport.clone(),
        || 1000,
    );
    Dropbox::with_sleep(tokens, transport, |_| {})
}
