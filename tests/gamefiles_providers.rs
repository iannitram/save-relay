//! One provider contract for game files (docs/assets.md, #309): the same scenarios run through
//! the HTTP layer of each `GameSource` (`Dropbox`, `WebDav` for Nextcloud, `Drive`) and the
//! engine, over a scripted transport that renders one simulated file store in each service's
//! shape. Every request is logged, and the transport enforces `max_response_bytes` the way the
//! real one does, so the tests also check that every request is bounded.
//!
//! What this does not show: how the real services answer (that needs a real account; the
//! Nextcloud leg is `tests/gamefiles_nextcloud_live.rs`, CI only), Drive per-file access under
//! `drive.file`, or device termination.

mod support_gamefiles;

use save_relay::clients::Client;
use save_relay::creds::Secret;
use save_relay::dropbox::{Dropbox, CONTENT};
use save_relay::error::Class;
use save_relay::gamefiles::engine::{self, Options};
use save_relay::gamefiles::{
    remote_file_path, remote_manifest_path, GameSource, State, MANIFEST_MAX_BYTES,
    RANGE_RESPONSE_FLOOR,
};
use save_relay::gdrive::{Drive, API};
use save_relay::http::{Request, Response, Transport};
use save_relay::oauth::{TokenSource, DROPBOX, GOOGLE};
use save_relay::webdav::{files_url, WebDav};
use save_relay::Error;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use support_gamefiles::{body, folder, manifest};

const APP: &str = "contract-app";
const CHUNK: u64 = 64 * 1024;
const NC_SERVER: &str = "https://cloud.example.test";
const NC_USER: &str = "player";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Dropbox,
    Nextcloud,
    Drive,
}

const KINDS: [Kind; 3] = [Kind::Dropbox, Kind::Nextcloud, Kind::Drive];

/// One request as the service saw it.
#[derive(Debug, Clone)]
struct Seen {
    url: String,
    /// The service path a content request named (Dropbox arg, WebDAV URL, Drive file id).
    path: Option<String>,
    range: Option<String>,
    if_match: Option<String>,
    accept_encoding: Option<String>,
    max: Option<usize>,
    /// A content request (download) rather than a lookup, metadata or token request.
    content: bool,
}

#[derive(Clone)]
struct SimFile {
    bytes: Vec<u8>,
    version: u64,
}

/// The knobs a scenario turns. Each is off by default (an honest service).
#[derive(Default)]
struct Script {
    /// Answer every range with 200 and the whole file.
    ignore_range: bool,
    /// Answer ranges as `multipart/byteranges`.
    multipart: bool,
    /// Replace the Content-Range of a 206 with this text.
    content_range: Option<String>,
    /// Report this total in Content-Range instead of the real size.
    total: Option<u64>,
    /// Answer every range with 416.
    refuse_range: bool,
    /// Serve at most this many bytes per 206 (a short, valid answer).
    short: Option<usize>,
    /// Answer every request but the token endpoint with 401.
    unauthorized: bool,
    /// Drop the next n content requests (unreachable).
    drop_next: u32,
    /// After this many content answers, bump every file's version (same bytes).
    revise_after: Option<u32>,
    /// Nextcloud: send weak ETags.
    weak_etag: bool,
    /// Drive: send no version.
    no_version: bool,
    /// Drive: list every child twice.
    duplicates: bool,
    /// Drive: every listing page has a next page and no files.
    endless_pages: bool,
    /// Drive: list a child of this name twice.
    duplicate_name: Option<&'static str>,
    /// Answer matching requests with an HTTP status instead (see `Fail`).
    fail: Option<Fail>,
    /// Drive: after this many content answers, every file is trashed.
    trash_after: Option<u32>,
    /// Drive: files are trashed (metadata says so; listings, which ask for
    /// `trashed = false`, leave them out).
    trashed: bool,
    /// Drive: files are Google Docs (`application/vnd.google-apps.document`).
    native: bool,
}

/// A scripted HTTP failure: `status` (with a Drive `error.errors[].reason` when given) on
/// every content request after the first `skip` (Drive: on listings instead, with `listing`).
#[derive(Clone, Copy)]
struct Fail {
    status: u16,
    reason: Option<&'static str>,
    skip: u32,
    listing: bool,
}

struct Sim {
    files: BTreeMap<String, SimFile>,
    script: Script,
    log: Vec<Seen>,
    content_answers: u32,
    /// Payload bytes sent in content answers (206 or 200 bodies).
    bytes_sent: u64,
}

/// The scripted service: one simulated file store rendered as Dropbox, WebDAV or Drive.
struct Scripted {
    kind: Kind,
    sim: Mutex<Sim>,
}

fn find<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap()
}

/// A Drive id for a service path ("" is My Drive's `root`): the path's bytes in hex, so an id
/// never needs quoting in `q` or encoding in a URL, whatever the name holds.
fn drive_id(path: &str) -> String {
    if path.is_empty() {
        "root".into()
    } else {
        let hex: String = path.bytes().map(|b| format!("{b:02x}")).collect();
        format!("n{hex}")
    }
}

fn drive_path(id: &str) -> String {
    if id == "root" {
        return String::new();
    }
    let hex = id.trim_start_matches('n');
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    String::from_utf8(bytes).unwrap()
}

/// A `q` string literal's text, from just after its opening `'`: up to the closing `'`, with
/// Drive's `\'` and `\\` escapes undone. Returns the text and what follows the literal.
fn q_literal(text: &str) -> (String, &str) {
    let mut out = String::new();
    let mut chars = text.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => out.push(chars.next().expect("an escape has a character").1),
            '\'' => return (out, &text[i + 1..]),
            other => out.push(other),
        }
    }
    panic!("unterminated q literal: {text}");
}

fn json_response(status: u16, value: Value) -> Response {
    Response::new(status, value.to_string()).header("Content-Type", "application/json")
}

impl Scripted {
    fn new(kind: Kind) -> Arc<Scripted> {
        Arc::new(Scripted {
            kind,
            sim: Mutex::new(Sim {
                files: BTreeMap::new(),
                script: Script::default(),
                log: Vec::new(),
                content_answers: 0,
                bytes_sent: 0,
            }),
        })
    }
    fn put(&self, path: &str, bytes: Vec<u8>) {
        self.sim
            .lock()
            .unwrap()
            .files
            .insert(path.into(), SimFile { bytes, version: 7 });
    }
    fn put_game(&self, files: &[(&str, &[u8])]) {
        self.put(&remote_manifest_path(APP), manifest(APP, files));
        for (path, bytes) in files {
            self.put(&remote_file_path(APP, path), bytes.to_vec());
        }
    }
    fn script(&self, edit: impl FnOnce(&mut Script)) {
        edit(&mut self.sim.lock().unwrap().script);
    }
    fn revise(&self) {
        for file in self.sim.lock().unwrap().files.values_mut() {
            file.version += 1;
        }
    }
    fn log(&self) -> Vec<Seen> {
        self.sim.lock().unwrap().log.clone()
    }
    fn clear_log(&self) {
        self.sim.lock().unwrap().log.clear();
    }
    fn ranged(&self) -> Vec<Seen> {
        self.log()
            .into_iter()
            .filter(|seen| seen.content && seen.range.is_some())
            .collect()
    }
    fn bytes_sent(&self) -> u64 {
        self.sim.lock().unwrap().bytes_sent
    }

    fn identity(&self, file: &SimFile, path: &str, weak: bool) -> String {
        match self.kind {
            Kind::Dropbox => format!("rev{:09}", file.version),
            Kind::Nextcloud if weak => format!("W/\"etag-{}\"", file.version),
            Kind::Nextcloud => format!("\"etag-{}\"", file.version),
            Kind::Drive => format!("{}:{}", drive_id(path), file.version),
        }
    }

    /// A content answer for `path` (the whole file without `range`), with the identity header
    /// each service sends.
    fn content(&self, sim: &mut Sim, path: &str, request: &Request) -> Response {
        let Some(file) = sim.files.get(path).cloned() else {
            return self.not_found();
        };
        if self.kind == Kind::Nextcloud {
            if let Some(want) = find(&request.headers, "If-Match") {
                if want != self.identity(&file, path, false) {
                    return Response::new(412, "precondition failed");
                }
            }
        }
        let range = find(&request.headers, "Range").and_then(|r| {
            let (a, b) = r.strip_prefix("bytes=")?.split_once('-')?;
            Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
        });
        let size = file.bytes.len();
        let script = &sim.script;
        let mut response = match range {
            Some(_) if script.refuse_range => {
                Response::new(416, "").header("Content-Range", &format!("bytes */{size}"))
            }
            Some((a, _)) if a >= size => {
                Response::new(416, "").header("Content-Range", &format!("bytes */{size}"))
            }
            Some((a, b)) if !script.ignore_range => {
                let mut b = b.min(size - 1);
                if let Some(short) = script.short {
                    b = b.min(a + short - 1);
                }
                let total = script.total.unwrap_or(size as u64);
                let content_range = script
                    .content_range
                    .clone()
                    .unwrap_or_else(|| format!("bytes {a}-{b}/{total}"));
                let mut answer = Response::new(206, file.bytes[a..=b].to_vec())
                    .header("Content-Range", &content_range);
                answer = if script.multipart {
                    answer.header("Content-Type", "multipart/byteranges; boundary=x")
                } else {
                    answer.header("Content-Type", "application/octet-stream")
                };
                answer
            }
            _ => Response::new(200, file.bytes.clone())
                .header("Content-Type", "application/octet-stream"),
        };
        sim.bytes_sent += response.body.len() as u64;
        let identity = self.identity(&file, path, script.weak_etag);
        response = match self.kind {
            Kind::Dropbox => response.header(
                "Dropbox-API-Result",
                &json!({"rev": identity, "size": size}).to_string(),
            ),
            Kind::Nextcloud => response.header("ETag", &identity),
            Kind::Drive => response,
        };
        sim.content_answers += 1;
        if sim
            .script
            .trash_after
            .is_some_and(|after| sim.content_answers >= after)
        {
            sim.script.trash_after = None;
            sim.script.trashed = true;
        }
        if sim
            .script
            .revise_after
            .is_some_and(|after| sim.content_answers >= after)
        {
            sim.script.revise_after = None;
            for file in sim.files.values_mut() {
                file.version += 1;
            }
        }
        response
    }

    /// The answer each service gives for a scripted failure status.
    fn failure(&self, fail: Fail) -> Response {
        match self.kind {
            Kind::Dropbox => json_response(
                fail.status,
                json!({"error_summary": "too_many_requests/..", "error": {".tag": "too_many_requests"}}),
            ),
            Kind::Nextcloud => Response::new(fail.status, "busy"),
            Kind::Drive => {
                let errors: Vec<Value> = fail
                    .reason
                    .map(|reason| json!({"domain": "global", "reason": reason}))
                    .into_iter()
                    .collect();
                json_response(
                    fail.status,
                    json!({"error": {"code": fail.status, "errors": errors, "message": "scripted"}}),
                )
            }
        }
    }

    fn not_found(&self) -> Response {
        match self.kind {
            Kind::Dropbox => json_response(409, json!({"error_summary": "path/not_found/.."})),
            Kind::Nextcloud => Response::new(404, "not found"),
            Kind::Drive => json_response(404, json!({"error": {"code": 404}})),
        }
    }

    fn drive_meta(&self, sim: &Sim, path: &str, name: &str) -> Value {
        let folder = !sim.files.contains_key(path);
        let mut value = json!({
            "id": drive_id(path),
            "name": name,
            "mimeType": if folder {
                "application/vnd.google-apps.folder"
            } else if sim.script.native {
                "application/vnd.google-apps.document"
            } else {
                "application/octet-stream"
            },
            "createdTime": "2026-01-01T00:00:00Z",
            "trashed": !folder && sim.script.trashed,
        });
        if let Some(file) = sim.files.get(path) {
            value["size"] = json!(file.bytes.len().to_string());
            if !sim.script.no_version {
                value["version"] = json!(file.version.to_string());
            }
        } else if !sim.script.no_version {
            value["version"] = json!("1");
        }
        value
    }

    fn drive(&self, sim: &mut Sim, request: &Request) -> (Response, Option<String>, bool) {
        let rest = request.url.strip_prefix(&format!("{API}/files")).unwrap();
        if let Some(query) = rest.strip_prefix('?') {
            // files.list: "'<parent>' in parents and trashed = false and name = '<name>'".
            if sim.script.endless_pages {
                return (
                    json_response(200, json!({"files": [], "nextPageToken": "more"})),
                    None,
                    false,
                );
            }
            let q = query
                .split('&')
                .find_map(|pair| pair.strip_prefix("q="))
                .map(decode)
                .unwrap();
            let (parent, rest) = q_literal(q.strip_prefix('\'').unwrap());
            let rest = rest
                .strip_prefix(" in parents and trashed = false and name = '")
                .unwrap_or_else(|| panic!("unexpected q: {q}"));
            let (name, rest) = q_literal(rest);
            assert!(rest.is_empty(), "unexpected q: {q}");
            let name = name.as_str();
            let prefix = drive_path(&parent);
            let child = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            let exists = (sim.files.contains_key(&child) && !sim.script.trashed)
                || sim
                    .files
                    .keys()
                    .any(|k| k.starts_with(&format!("{child}/")));
            let mut files = Vec::new();
            if exists {
                files.push(self.drive_meta(sim, &child, name));
                if sim.script.duplicates || sim.script.duplicate_name == Some(name) {
                    let mut twin = self.drive_meta(sim, &child, name);
                    twin["id"] = json!(format!("{}-twin", drive_id(&child)));
                    files.push(twin);
                }
            }
            return (json_response(200, json!({ "files": files })), None, false);
        }
        let rest = rest.strip_prefix('/').unwrap();
        let (id, query) = rest.split_once('?').unwrap();
        let path = drive_path(id);
        if query == "alt=media" {
            return (self.content(sim, &path, request), Some(path), true);
        }
        if !sim.files.contains_key(&path) {
            return (self.not_found(), Some(path), false);
        }
        let name = path.rsplit('/').next().unwrap().to_string();
        (
            json_response(200, self.drive_meta(sim, &path, &name)),
            Some(path),
            false,
        )
    }
}

impl Transport for Scripted {
    fn send(&self, request: &Request) -> save_relay::Result<Response> {
        let mut sim = self.sim.lock().unwrap();
        if request.url.contains("/oauth2/token") || request.url.contains("oauth2.googleapis.com") {
            return Ok(json_response(
                200,
                json!({"access_token": "scripted-access", "expires_in": 3600}),
            ));
        }
        let header = |name: &str| find(&request.headers, name).map(str::to_string);
        let mut seen = Seen {
            url: request.url.clone(),
            path: None,
            range: header("Range"),
            if_match: header("If-Match"),
            accept_encoding: header("Accept-Encoding"),
            max: request.max_response_bytes,
            content: false,
        };
        let content_request = match self.kind {
            Kind::Dropbox => request.url == format!("{CONTENT}/files/download"),
            Kind::Nextcloud => request.method == "GET",
            Kind::Drive => request.url.ends_with("?alt=media"),
        };
        let listing = self.kind == Kind::Drive && request.url.starts_with(&format!("{API}/files?"));
        if let Some(fail) = sim.script.fail {
            let matches = if fail.listing {
                listing
            } else {
                content_request
            };
            if matches && fail.skip > 0 {
                sim.script.fail.as_mut().unwrap().skip -= 1;
            } else if matches {
                seen.content = content_request;
                sim.log.push(seen);
                return Ok(self.failure(fail));
            }
        }
        if content_request && sim.script.drop_next > 0 {
            sim.script.drop_next -= 1;
            seen.content = true;
            sim.log.push(seen);
            return Err(Error::Unreachable("scripted drop".into()));
        }
        let response = if sim.script.unauthorized {
            Response::new(401, "unauthorized")
        } else {
            match self.kind {
                Kind::Dropbox => {
                    let arg: Value =
                        serde_json::from_str(&header("Dropbox-API-Arg").unwrap()).unwrap();
                    let path = arg["path"].as_str().unwrap().trim_start_matches('/');
                    seen.path = Some(path.to_string());
                    seen.content = true;
                    self.content(&mut sim, path, request)
                }
                Kind::Nextcloud => {
                    let base = files_url(NC_SERVER, NC_USER).unwrap();
                    let path = decode(request.url.strip_prefix(&base).unwrap());
                    seen.path = Some(path.clone());
                    seen.content = true;
                    self.content(&mut sim, &path, request)
                }
                Kind::Drive => {
                    let (response, path, content) = self.drive(&mut sim, request);
                    seen.path = path;
                    seen.content = content;
                    response
                }
            }
        };
        sim.log.push(seen);
        // The real transport's bound (http::UreqTransport): never more than asked for.
        if let Some(limit) = request.max_response_bytes {
            if response.body.len() > limit {
                return Err(Error::Server(format!(
                    "HTTP response body exceeds the {limit}-byte limit"
                )));
            }
        }
        Ok(response)
    }
}

fn client() -> Client {
    Client {
        id: "contract-test",
        secret: Some("not-a-secret"),
    }
}

fn source(kind: Kind, transport: &Arc<Scripted>) -> Box<dyn GameSource> {
    let transport: Arc<dyn Transport> = transport.clone();
    match kind {
        Kind::Dropbox => {
            let tokens = TokenSource::new(
                &DROPBOX,
                client(),
                Secret::new("refresh"),
                transport.clone(),
                || 1000,
            );
            Box::new(Dropbox::with_sleep(tokens, transport, |_| {}))
        }
        Kind::Nextcloud => Box::new(WebDav::new(
            &files_url(NC_SERVER, NC_USER).unwrap(),
            NC_USER,
            Secret::new("app-password"),
            transport,
        )),
        Kind::Drive => {
            let tokens = TokenSource::new(
                &GOOGLE,
                client(),
                Secret::new("refresh"),
                transport.clone(),
                || 1000,
            );
            Box::new(Drive::new(tokens, transport, || 1000).with_wait(Arc::new(|_| {})))
        }
    }
}

fn options() -> Options {
    Options {
        chunk: CHUNK,
        retries: 3,
        sleep: Arc::new(|_| {}),
        ..Options::default()
    }
}

struct Case {
    kind: Kind,
    service: Arc<Scripted>,
    source: Box<dyn GameSource>,
    dest: PathBuf,
    state: PathBuf,
}

/// A fresh service of `kind` holding a game of one 200 000-byte file and one small file.
fn setup(kind: Kind, name: &str) -> (Case, Vec<u8>, Vec<u8>) {
    let big = body(3, 200_000);
    let small = body(4, 5_000);
    let service = Scripted::new(kind);
    service.put_game(&[("data/big.bin", &big), ("small.bin", &small)]);
    let root = folder(&format!("providers-{name}-{kind:?}"));
    let source = source(kind, &service);
    (
        Case {
            kind,
            service,
            source,
            dest: root.join("game"),
            state: root.join("state"),
        },
        big,
        small,
    )
}

impl Case {
    fn run(&self) -> save_relay::Result<save_relay::gamefiles::Progress> {
        self.run_with(&AtomicBool::new(false), &mut |_| {})
    }
    fn run_with(
        &self,
        stop: &AtomicBool,
        observe: &mut dyn FnMut(&save_relay::gamefiles::Progress),
    ) -> save_relay::Result<save_relay::gamefiles::Progress> {
        engine::run(
            self.source.as_ref(),
            APP,
            &self.dest,
            &self.state,
            &options(),
            stop,
            observe,
        )
    }
    fn installed(&self, rel: &str) -> Option<Vec<u8>> {
        fs::read(self.dest.join(rel)).ok()
    }
    /// Every range request is bounded: a `bytes=` Range, identity encoding, and a response
    /// bound of at most `max(chunk, RANGE_RESPONSE_FLOOR)`.
    fn assert_bounded(&self) {
        let floor = (CHUNK as usize).max(RANGE_RESPONSE_FLOOR);
        for seen in self.service.log() {
            if !seen.content {
                assert!(
                    seen.max.is_some() || self.kind != Kind::Drive,
                    "{:?}: unbounded Drive request {}",
                    self.kind,
                    seen.url
                );
                continue;
            }
            assert_eq!(
                seen.accept_encoding.as_deref(),
                Some("identity"),
                "{:?}: {seen:?}",
                self.kind
            );
            let max = seen.max.expect("every content request is bounded");
            match &seen.range {
                Some(range) => {
                    assert!(range.starts_with("bytes="), "{range}");
                    assert!(max <= floor, "{:?}: bound {max} > {floor}", self.kind);
                }
                None => assert!(max <= MANIFEST_MAX_BYTES, "{:?}: {max}", self.kind),
            }
        }
    }
}

fn code(result: save_relay::Result<save_relay::gamefiles::Progress>) -> (&'static str, Class) {
    let error = result.expect_err("the run should fail");
    (error.code(), error.class())
}

#[test]
fn every_provider_downloads_in_bounded_ranges_and_sends_each_byte_once() {
    for kind in KINDS {
        let (case, big, small) = setup(kind, "full");
        let done = case.run().unwrap();
        assert_eq!(done.state, State::Complete, "{kind:?}");
        assert_eq!(case.installed("data/big.bin").unwrap(), big, "{kind:?}");
        assert_eq!(case.installed("small.bin").unwrap(), small, "{kind:?}");
        let ranged = case.service.ranged();
        assert_eq!(
            ranged.len(),
            200_000usize.div_ceil(CHUNK as usize) + 1,
            "{kind:?}"
        );
        assert_eq!(
            ranged[0].range.as_deref(),
            Some("bytes=0-65535"),
            "{kind:?}"
        );
        let manifest_len = manifest(APP, &[("data/big.bin", &big), ("small.bin", &small)]).len();
        assert_eq!(
            case.service.bytes_sent() as usize,
            big.len() + small.len() + manifest_len,
            "{kind:?}: nothing is sent twice"
        );
        case.assert_bounded();
    }
}

#[test]
fn a_later_chunk_carries_the_identity_of_the_first_where_the_service_can_check_it() {
    let (case, ..) = setup(Kind::Nextcloud, "if-match");
    case.run().unwrap();
    let ranged = case.service.ranged();
    let big: Vec<_> = ranged
        .iter()
        .filter(|seen| seen.path.as_deref().is_some_and(|p| p.ends_with("big.bin")))
        .collect();
    assert_eq!(
        big[0].if_match, None,
        "the first chunk has nothing to match yet"
    );
    assert!(big[1..]
        .iter()
        .all(|seen| seen.if_match.as_deref() == Some("\"etag-7\"")));
}

#[test]
fn every_provider_resumes_after_a_dropped_connection_and_after_a_cancel() {
    for kind in KINDS {
        let (case, big, _) = setup(kind, "resume");
        case.service.script(|s| s.drop_next = 2);
        let stop = AtomicBool::new(false);
        let cancelled = case
            .run_with(&stop, &mut |p| {
                if p.bytes_done >= CHUNK {
                    stop.store(true, Ordering::SeqCst);
                }
            })
            .unwrap();
        assert_eq!(cancelled.state, State::Cancelled, "{kind:?}");
        let before = case.service.bytes_sent();
        let done = case.run().unwrap();
        assert_eq!(done.state, State::Complete, "{kind:?}");
        assert_eq!(case.installed("data/big.bin").unwrap(), big, "{kind:?}");
        // The resume asked only for what was missing (plus the manifest again).
        let fetched_after = case.service.bytes_sent() - before;
        assert!(
            fetched_after < 200_000 + 5_000 + 1_000,
            "{kind:?}: {fetched_after}"
        );
        case.assert_bounded();
    }
}

#[test]
fn every_provider_accepts_a_short_valid_206_and_asks_for_the_rest() {
    for kind in KINDS {
        let (case, big, _) = setup(kind, "short");
        case.service.script(|s| s.short = Some(10_000));
        assert_eq!(case.run().unwrap().state, State::Complete, "{kind:?}");
        assert_eq!(case.installed("data/big.bin").unwrap(), big, "{kind:?}");
        assert!(case.service.ranged().len() >= 20, "{kind:?}");
        case.assert_bounded();
    }
}

#[test]
fn a_200_to_a_range_is_range_unsupported_and_never_appended() {
    for kind in KINDS {
        let (case, ..) = setup(kind, "ignores");
        case.service.script(|s| s.ignore_range = true);
        let (code, class) = code(case.run());
        assert_eq!(code, "range_unsupported", "{kind:?}");
        assert_eq!(class, Class::Fatal, "{kind:?}");
        assert!(case.installed("data/big.bin").is_none(), "{kind:?}");
        assert!(case.installed("small.bin").is_none(), "{kind:?}");
        case.assert_bounded();
        // A small file's whole body fits the bound: it is still refused (200, not 206).
        let small = Scripted::new(kind);
        small.put_game(&[("tiny.bin", b"tiny file")]);
        small.script(|s| s.ignore_range = true);
        let root = folder(&format!("providers-ignores-small-{kind:?}"));
        let source = source(kind, &small);
        let error = engine::run(
            source.as_ref(),
            APP,
            &root.join("game"),
            &root.join("state"),
            &options(),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(error.code(), "range_unsupported", "{kind:?}: {error:?}");
        assert!(!root.join("game/tiny.bin").exists());
    }
}

#[test]
fn malformed_or_multipart_content_ranges_are_range_unsupported() {
    let bad = |s: &mut Script, which: u8| match which {
        0 => s.content_range = Some("bytes 0-65535".into()),
        1 => s.content_range = Some("bytes 5-65535/200000".into()),
        2 => s.content_range = Some("bytes 0-99/200000".into()),
        _ => s.multipart = true,
    };
    for kind in KINDS {
        for which in 0..4 {
            let (case, ..) = setup(kind, &format!("malformed-{which}"));
            case.service.script(|s| bad(s, which));
            let (code, class) = code(case.run());
            assert_eq!(code, "range_unsupported", "{kind:?} #{which}");
            assert_eq!(class, Class::Fatal);
            assert!(case.installed("data/big.bin").is_none());
        }
    }
}

#[test]
fn a_wrong_total_is_remote_changed_and_416_is_range_unsupported() {
    for kind in KINDS {
        let (case, ..) = setup(kind, "total");
        case.service.script(|s| s.total = Some(123));
        assert_eq!(code(case.run()).0, "remote_changed", "{kind:?}");
        let (case, ..) = setup(kind, "416");
        case.service.script(|s| s.refuse_range = true);
        assert_eq!(code(case.run()).0, "range_unsupported", "{kind:?}");
    }
}

#[test]
fn an_identity_change_mid_download_stops_the_file_and_a_retry_restarts_it() {
    for kind in KINDS {
        let (case, big, _) = setup(kind, "revised");
        // The manifest is the first content answer; the version moves after the first chunk.
        case.service.script(|s| s.revise_after = Some(2));
        let (code, class) = code(case.run());
        assert_eq!(code, "remote_changed", "{kind:?}"); // one contract code (docs/assets.md); class decides
        assert_eq!(class, Class::Retryable, "{kind:?}");
        assert!(case.installed("data/big.bin").is_none(), "{kind:?}");
        // The partial copy is gone: the next run starts the file at 0 and completes.
        case.service.clear_log();
        assert_eq!(case.run().unwrap().state, State::Complete, "{kind:?}");
        assert_eq!(case.installed("data/big.bin").unwrap(), big, "{kind:?}");
        let first = &case.service.ranged()[0];
        assert_eq!(first.range.as_deref(), Some("bytes=0-65535"), "{kind:?}");
        assert_eq!(first.if_match, None, "{kind:?}");
        case.assert_bounded();
    }
}

#[test]
fn an_identity_change_across_a_restart_is_caught_before_the_resumed_bytes_are_used() {
    for kind in KINDS {
        let (case, big, _) = setup(kind, "revised-restart");
        let stop = AtomicBool::new(false);
        let cancelled = case
            .run_with(&stop, &mut |p| {
                if p.bytes_done >= CHUNK {
                    stop.store(true, Ordering::SeqCst);
                }
            })
            .unwrap();
        assert_eq!(cancelled.state, State::Cancelled, "{kind:?}");
        case.service.revise();
        case.service.clear_log();
        let (code, class) = code(case.run());
        assert_eq!(code, "remote_changed", "{kind:?}"); // one contract code (docs/assets.md); class decides
        assert_eq!(class, Class::Retryable);
        let resumed: Vec<_> = case.service.ranged();
        match kind {
            // The server refused the old ETag (412), once: a refusal is not retried.
            Kind::Nextcloud => {
                assert_eq!(resumed.len(), 1, "{resumed:?}");
                assert_eq!(resumed[0].if_match.as_deref(), Some("\"etag-7\""));
            }
            // Drive's version is checked before any media request.
            Kind::Drive => assert!(resumed.is_empty(), "{resumed:?}"),
            // Dropbox's new rev is compared when the answer arrives; its bytes are discarded.
            Kind::Dropbox => assert_eq!(resumed.len(), 1),
        }
        assert_eq!(case.run().unwrap().state, State::Complete, "{kind:?}");
        assert_eq!(case.installed("data/big.bin").unwrap(), big, "{kind:?}");
    }
}

#[test]
fn an_auth_failure_is_reauth_needed() {
    for kind in KINDS {
        let (case, ..) = setup(kind, "auth");
        case.service.script(|s| s.unauthorized = true);
        let error = case.run().unwrap_err();
        assert_eq!(error.class(), Class::ReauthNeeded, "{kind:?}: {error:?}");
        let expected = if kind == Kind::Nextcloud {
            "auth_failed"
        } else {
            "reauth_needed"
        };
        assert_eq!(error.code(), expected, "{kind:?}");
        assert!(!error.to_string().contains("scripted-access"));
        assert!(!error.to_string().contains("app-password"));
    }
}

#[test]
fn a_missing_manifest_or_file_is_not_yet_uploaded() {
    for kind in KINDS {
        let service = Scripted::new(kind);
        let root = folder(&format!("providers-missing-{kind:?}"));
        let source = source(kind, &service);
        let go = || {
            engine::run(
                source.as_ref(),
                APP,
                &root.join("game"),
                &root.join("state"),
                &options(),
                &AtomicBool::new(false),
                &mut |_| {},
            )
        };
        let error = go().unwrap_err();
        assert_eq!(error.class(), Class::NotYetUploaded, "{kind:?}: {error:?}");
        // The manifest lists a file that is not there yet.
        service.put(
            &remote_manifest_path(APP),
            manifest(APP, &[("later.bin", b"later")]),
        );
        let error = go().unwrap_err();
        assert_eq!(error.code(), "not_yet_uploaded", "{kind:?}: {error:?}");
        assert!(error.to_string().contains("later.bin"), "{error}");
        // Only on Drive may the app be unable to see a file that is there (drive.file).
        assert_eq!(
            error.to_string().contains("Drive hides it"),
            kind == Kind::Drive,
            "{kind:?}: {error}"
        );
        assert!(!error.to_string().contains("  "), "{kind:?}: {error}");
    }
}

#[test]
fn an_oversize_manifest_is_refused_within_its_bound() {
    for kind in KINDS {
        let service = Scripted::new(kind);
        service.put(
            &remote_manifest_path(APP),
            vec![b' '; MANIFEST_MAX_BYTES + 1],
        );
        let source = source(kind, &service);
        let error = source
            .read_bounded(&remote_manifest_path(APP), MANIFEST_MAX_BYTES)
            .unwrap_err();
        assert_eq!(error.code(), "manifest_invalid", "{kind:?}: {error:?}");
        let seen = service.log();
        let read = seen.iter().find(|s| s.content).unwrap();
        assert_eq!(read.max, Some(MANIFEST_MAX_BYTES), "{kind:?}");
        assert_eq!(read.range, None);
    }
}

#[test]
fn nextcloud_never_uses_a_weak_etag_as_identity() {
    let (case, big, _) = setup(Kind::Nextcloud, "weak");
    case.service.script(|s| s.weak_etag = true);
    assert_eq!(case.run().unwrap().state, State::Complete);
    assert_eq!(case.installed("data/big.bin").unwrap(), big);
    assert!(case.service.ranged().iter().all(|s| s.if_match.is_none()));
}

#[test]
fn drive_refuses_duplicate_names_missing_versions_folders_and_endless_listings() {
    let (case, ..) = setup(Kind::Drive, "dupes");
    case.service.script(|s| s.duplicates = true);
    let error = case.run().unwrap_err();
    assert_eq!(error.class(), Class::Fatal, "{error:?}");
    assert!(error.to_string().contains("more than one"), "{error}");

    let (case, ..) = setup(Kind::Drive, "no-version");
    case.service.script(|s| s.no_version = true);
    let error = case.run().unwrap_err();
    assert!(error.to_string().contains("no version"), "{error}");
    assert_eq!(error.class(), Class::Fatal);

    // A folder where a file is listed is not a file.
    let (case, ..) = setup(Kind::Drive, "folder");
    let error = case
        .source
        .fetch(&format!("game-files/{APP}/files/data"), 0, 10)
        .unwrap_err();
    assert!(matches!(error, Error::NotFound(_)), "{error:?}");

    let (case, ..) = setup(Kind::Drive, "endless");
    case.service.script(|s| s.endless_pages = true);
    let error = case.run().unwrap_err();
    assert!(
        error.to_string().contains("too many listing pages"),
        "{error}"
    );
    assert!(
        case.service.log().len() <= 16,
        "{}",
        case.service.log().len()
    );
    assert!(case
        .service
        .log()
        .iter()
        .all(|s| s.max.is_some_and(|m| m <= 256 * 1024)));
}

#[test]
fn sources_name_their_service() {
    let names: Vec<_> = KINDS
        .iter()
        .map(|kind| source(*kind, &Scripted::new(*kind)).service())
        .collect();
    assert_eq!(names, ["dropbox", "nextcloud", "gdrive"]);
}

/// A game of one file at `rel` on a fresh service of `kind`.
fn setup_one(kind: Kind, name: &str, rel: &str, bytes: &[u8]) -> Case {
    let service = Scripted::new(kind);
    service.put_game(&[(rel, bytes)]);
    let root = folder(&format!("providers-{name}-{kind:?}"));
    let source = source(kind, &service);
    Case {
        kind,
        service,
        source,
        dest: root.join("game"),
        state: root.join("state"),
    }
}

fn fail(status: u16, reason: Option<&'static str>, skip: u32) -> Option<Fail> {
    Some(Fail {
        status,
        reason,
        skip,
        listing: false,
    })
}

#[test]
fn every_provider_classes_429_and_503_as_retryable_on_the_manifest_and_on_a_chunk() {
    for kind in KINDS {
        for status in [429, 503] {
            // skip 0: the manifest read; skip 1: the first payload chunk.
            for skip in [0, 1] {
                let (case, ..) = setup(kind, &format!("status-{status}-{skip}"));
                case.service.script(|s| s.fail = fail(status, None, skip));
                let error = case.run().unwrap_err();
                assert_eq!(
                    error.class(),
                    Class::Retryable,
                    "{kind:?} {status} skip {skip}: {error:?}"
                );
                assert_eq!(error.code(), "server_error", "{kind:?}: {error:?}");
                assert!(case.installed("data/big.bin").is_none());
            }
        }
        // Once the service recovers, the next run completes.
        let (case, big, _) = setup(kind, "status-recovers");
        case.service.script(|s| s.fail = fail(503, None, 1));
        assert!(case.run().is_err());
        case.service.script(|s| s.fail = None);
        assert_eq!(case.run().unwrap().state, State::Complete, "{kind:?}");
        assert_eq!(case.installed("data/big.bin").unwrap(), big, "{kind:?}");
    }
}

#[test]
fn drive_rate_limit_403s_stay_retryable_and_other_403s_name_their_reason() {
    for reason in ["rateLimitExceeded", "userRateLimitExceeded"] {
        for listing in [false, true] {
            let (case, ..) = setup(Kind::Drive, &format!("rate-{reason}-{listing}"));
            case.service.script(|s| {
                s.fail = Some(Fail {
                    status: 403,
                    reason: Some(reason),
                    skip: 1,
                    listing,
                })
            });
            let error = case.run().unwrap_err();
            assert_eq!(
                error.class(),
                Class::Retryable,
                "{reason} {listing}: {error:?}"
            );
            assert!(error.to_string().contains("429 (rate limited)"), "{error}");
            assert!(!error.to_string().contains("drive.file"), "{error}");
        }
    }
    let cases = [
        (None, "drive.file"),
        (Some("appNotAuthorizedToFile"), "drive.file"),
        (Some("insufficientFilePermissions"), "drive.file"),
        (Some("cannotDownloadAbusiveFile"), "malware"),
        (Some("fileNotDownloadable"), "Google Docs"),
        (Some("downloadQuotaExceeded"), "quota"),
    ];
    for (reason, says) in cases {
        for skip in [0, 1] {
            let (case, ..) = setup(
                Kind::Drive,
                &format!("403-{}-{skip}", reason.unwrap_or("none")),
            );
            case.service.script(|s| s.fail = fail(403, reason, skip));
            let error = case.run().unwrap_err();
            assert_eq!(error.class(), Class::Fatal, "{reason:?}: {error:?}");
            let text = error.to_string();
            assert!(text.contains(says), "{reason:?}: {text}");
            assert_eq!(
                text.contains("drive.file"),
                says == "drive.file",
                "{reason:?}: {text}"
            );
            assert!(text.contains(&format!("game-files/{APP}/")), "{text}");
        }
    }
}

#[test]
fn every_provider_fetches_names_with_spaces_unicode_percent_and_apostrophes() {
    let rel = "Tony's Game/disc 1 ü 100%/it's here.bin";
    for kind in KINDS {
        let bytes = body(9, 150_000);
        let case = setup_one(kind, "names", rel, &bytes);
        let done = case.run().unwrap();
        assert_eq!(done.state, State::Complete, "{kind:?}");
        assert_eq!(case.installed(rel).unwrap(), bytes, "{kind:?}");
        let named: Vec<_> = case
            .service
            .ranged()
            .into_iter()
            .filter(|seen| seen.path.as_deref() == Some(remote_file_path(APP, rel).as_str()))
            .collect();
        assert_eq!(
            named.len(),
            150_000usize.div_ceil(CHUNK as usize),
            "{kind:?}"
        );
        if kind == Kind::Nextcloud {
            let url = &named[0].url;
            for raw in [" ", "'", "ü", "%/"] {
                assert!(!url.contains(raw), "{url} has a raw {raw:?}");
            }
            assert!(
                url.contains("Tony%27s%20Game/disc%201%20%C3%BC%20100%25/"),
                "{url}"
            );
        }
        case.assert_bounded();
    }
}

#[test]
fn drive_errors_whose_names_look_like_a_status_stay_fatal() {
    // Two items named "500.bin": the duplicate refusal must not read the 500 as a status.
    let case = setup_one(Kind::Drive, "dupe-500", "503/500.bin", b"five hundred");
    case.service.script(|s| s.duplicate_name = Some("500.bin"));
    let error = case.run().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("more than one item is named '500.bin'"),
        "{error}"
    );
    assert_eq!(error.class(), Class::Fatal, "{error:?}");

    let rel = "track 429/disc 503.bin";
    let case = setup_one(Kind::Drive, "no-version-429", rel, b"x");
    case.service.script(|s| s.no_version = true);
    let error = case
        .source
        .fetch(&remote_file_path(APP, rel), 0, 1)
        .unwrap_err();
    assert!(error.to_string().contains("no version"), "{error}");
    assert_eq!(error.class(), Class::Fatal, "{error:?}");
}

#[test]
fn a_drive_file_trashed_mid_download_stops_before_its_chunk_is_appended() {
    let (case, ..) = setup(Kind::Drive, "trashed");
    // The manifest is the first content answer; the files are trashed after the first chunk.
    case.service.script(|s| s.trash_after = Some(2));
    let error = case.run().unwrap_err();
    assert_eq!(error.code(), "not_yet_uploaded", "{error:?}");
    assert!(error.to_string().contains("data/big.bin"), "{error}");
    assert!(case.installed("data/big.bin").is_none());
    assert_eq!(case.service.ranged().len(), 1, "no chunk after the trash");
}

#[test]
fn a_google_docs_file_on_drive_is_refused_before_any_media_request() {
    let (case, ..) = setup(Kind::Drive, "native");
    case.service.script(|s| s.native = true);
    let error = case.run().unwrap_err();
    assert_eq!(error.class(), Class::Fatal, "{error:?}");
    assert!(error.to_string().contains("Google Docs"), "{error}");
    assert!(
        case.service.log().iter().all(|seen| !seen.content),
        "no alt=media request"
    );
}

#[test]
fn a_drive_503_on_a_chunk_costs_retries_plus_one_requests_and_no_inner_waits() {
    let (mut case, ..) = setup(Kind::Drive, "no-inner-waits");
    let waits = Arc::new(Mutex::new(Vec::<u64>::new()));
    let seen = waits.clone();
    let transport: Arc<dyn Transport> = case.service.clone();
    let tokens = TokenSource::new(
        &GOOGLE,
        client(),
        Secret::new("refresh"),
        transport.clone(),
        || 1000,
    );
    case.source = Box::new(
        Drive::new(tokens, transport, || 1000)
            .with_wait(Arc::new(move |seconds| seen.lock().unwrap().push(seconds))),
    );
    // The manifest read passes; every later content request answers 503 with a Retry-After.
    case.service.script(|s| s.fail = fail(503, None, 1));
    let error = case.run().unwrap_err();
    assert_eq!(error.class(), Class::Retryable, "{error:?}");
    assert!(
        waits.lock().unwrap().is_empty(),
        "the client slept inside a request: {:?}",
        waits.lock().unwrap()
    );
    // options().retries is 3: one try and three retries, all made by the engine.
    let media = case
        .service
        .log()
        .iter()
        .filter(|seen| seen.content)
        .count();
    assert_eq!(media, 1 + 4, "the manifest read plus 4 chunk attempts");
}
