//! `save-relay panel [--port N] [--no-open]`: the Save Relay panel in the browser (SPEC.md M15;
//! docs/adr/0003-baton-wrap-cards-panel.md). The same views as the Game Mode panel (`decky/`),
//! from the same front end (`panel/`), served on `127.0.0.1` by a tiny HTTP/1.1 server on
//! `std::net` (no new dependency).
//!
//! - `GET /` (also `/index.html`): the front end's page. Its `./app.js` and `./app.css`
//!   references carry the token (`served_page`), so the browser sends it for those files too.
//!   The files are embedded in the binary at build time from `panel/dist` with the `panel-dist`
//!   feature (after `pnpm --dir panel build`, which writes fixed names); without it, the
//!   placeholder page `panel/placeholder/index.html`, so `cargo build` never needs Node. No build
//!   script.
//! - `GET /<file>` (`/app.js`, `/app.css`): the front end's other files, with the token like
//!   every request.
//! - `POST /api/<command>` with a JSON object body: runs the same code as the command line
//!   (`api_args`, then `cli::run`) and answers its `--json` envelope with status 200, failures
//!   included. A refused request (an unknown or missing field, a command the panel does not run)
//!   is a usage envelope with status 400. A picture is not an API call: `POST /api/picture` is
//!   refused the same way.
//! - `GET /picture/<game>/<entry>` (or `.../<game>/<entry>/<file>`, which the front end sends):
//!   a save card's picture, as `image/jpeg` or `image/png` by its magic bytes.
//!
//! A random token is in the URL that is opened (`/?token=...`) and must come with every request
//! (`X-Save-Relay-Token` header or `token` query): the page, its files, the API and the pictures.
//! So other pages in the browser cannot call it; a missing or wrong token is 403. The browser
//! opens like `oauth::open_browser`. Requests are answered one at a time.

use crate::api::{self, PanelData};
use crate::card;
use crate::error::{Error, Result};
use crate::oauth;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

/// The front end, embedded: (path, bytes, content type).
pub mod assets {
    /// The built front end (`--features panel-dist`).
    #[cfg(feature = "panel-dist")]
    pub const ASSETS: &[(&str, &[u8], &str)] = &[
        (
            "index.html",
            include_bytes!("../panel/dist/index.html"),
            "text/html; charset=utf-8",
        ),
        (
            "app.js",
            include_bytes!("../panel/dist/app.js"),
            "text/javascript; charset=utf-8",
        ),
        (
            "app.css",
            include_bytes!("../panel/dist/app.css"),
            "text/css; charset=utf-8",
        ),
    ];

    /// The placeholder page: this build has no panel.
    #[cfg(not(feature = "panel-dist"))]
    pub const ASSETS: &[(&str, &[u8], &str)] = &[(
        "index.html",
        include_bytes!("../panel/placeholder/index.html"),
        "text/html; charset=utf-8",
    )];
}

/// The header that carries the token.
pub const TOKEN_HEADER: &str = "X-Save-Relay-Token";

/// The commands `POST /api/<command>` may run. `setup` only for Google Drive with `no-browser`
/// or `finish`, and for Dropbox with `no-browser` (its `finish` needs a terminal); never `wrap`,
/// `panel` or a Nextcloud setup (its password stays on standard input). Steam game commands are
/// available to the M16 panel, which delegates launch-option writes to Steam's own client.
pub const API_COMMANDS: [&str; 13] = [
    "status",
    "history",
    "restore",
    "pull",
    "push",
    "answer",
    "where",
    "which",
    "picture",
    "setup",
    "steam-games",
    "install-wrap",
    "launch-option",
];

/// Request headers larger than this are refused.
const MAX_HEAD: usize = 16 * 1024;
/// Request bodies larger than this are refused.
const MAX_BODY: usize = 1024 * 1024;
/// A connection that sends nothing for this long is dropped: requests are answered one at a time.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// The flags of `api_args` that are switches (`true` adds the flag, `false` leaves it out).
const SWITCHES: [&str; 7] = [
    "anyway",
    "no-browser",
    "finish",
    "no-steam-cloud",
    "refresh",
    "remove",
    "dry-run",
];
const TEXT: &str = "text/plain; charset=utf-8";
const JSON: &str = "application/json; charset=utf-8";

/// The token from 32 random bytes (`oauth::random_bytes`): their base64url, 43 characters.
pub fn token_from_bytes(bytes: &[u8; 32]) -> String {
    oauth::base64url(bytes)
}

/// One request, as `parse_request` read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// The path, percent-decoded, without the query.
    pub path: String,
    /// The query's pairs, percent-decoded.
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Read one HTTP/1.1 request from the bytes received so far: the request line, the headers and
/// a `Content-Length` body of at most 1 MiB. `Ok(None)` while it is incomplete. A malformed one
/// (a chunked body, an encoded slash in the path, a bad percent-escape) is `Error::Usage`.
pub fn parse_request(bytes: &[u8]) -> Result<Option<Request>> {
    let Some(head_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        if bytes.len() > MAX_HEAD {
            return Err(usage_error("the request headers are too large"));
        }
        return Ok(None);
    };
    if head_end > MAX_HEAD {
        return Err(usage_error("the request headers are too large"));
    }
    let head = std::str::from_utf8(&bytes[..head_end])
        .map_err(|_| usage_error("the request headers are not text"))?;
    let mut lines = head.split("\r\n");
    let (method, target) = request_line(lines.next().unwrap_or(""))?;
    let (raw_path, raw_query) = match target.split_once('?') {
        Some((path, query)) => (path, query),
        None => (target, ""),
    };
    if !raw_path.starts_with('/') || raw_path.to_ascii_lowercase().contains("%2f") {
        return Err(usage_error("the request target is not a path"));
    }
    let path = percent_decode(raw_path, false)?;
    let query = parse_query(raw_query)?;
    let mut headers = Vec::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(usage_error("a header line has no colon"));
        };
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(usage_error("a header name is not valid"));
        }
        headers.push((name.to_string(), value.trim().to_string()));
    }
    if headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("transfer-encoding"))
    {
        return Err(usage_error("chunked request bodies are not supported"));
    }
    let lengths: Vec<&str> = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.as_str())
        .collect();
    let length = match lengths.as_slice() {
        [] => 0,
        [one] if !one.is_empty() && one.bytes().all(|byte| byte.is_ascii_digit()) => one
            .parse::<usize>()
            .map_err(|_| usage_error("Content-Length is too large"))?,
        _ => return Err(usage_error("Content-Length must be one whole number")),
    };
    if length > MAX_BODY {
        return Err(usage_error("the request body is over 1 MiB"));
    }
    let start = head_end + 4;
    if bytes.len() < start + length {
        return Ok(None);
    }
    Ok(Some(Request {
        method,
        path,
        query,
        headers,
        body: bytes[start..start + length].to_vec(),
    }))
}

/// Where a request goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// An embedded file, by its path in `assets::ASSETS`.
    Asset(&'static str),
    /// `POST /api/<command>` with one of `API_COMMANDS`.
    Api(String),
    /// `GET /picture/<game>/<entry>`.
    Picture {
        game: String,
        entry: String,
    },
    /// No token, or a wrong one (compared in constant time).
    Forbidden,
    NotFound,
}

/// Route `request`. Every request is checked for the token first: the header when it is there,
/// else the `token` query. Then `/` and `/index.html` are the page, `GET /<file>` is one of the
/// front end's other files, `POST /api/<command>` runs a command and `GET /picture/<game>/<entry>`
/// (with or without a file segment) is a picture. Pure.
pub fn route(request: &Request, token: &str) -> Route {
    let given = header(request, TOKEN_HEADER).or_else(|| query_value(request, "token"));
    if !given.is_some_and(|given| same_token(given, token)) {
        return Route::Forbidden;
    }
    let path = request.path.as_str();
    match request.method.as_str() {
        "GET" if path == "/" || path == "/index.html" => Route::Asset("index.html"),
        "GET" => match asset_named(path) {
            Some(name) => Route::Asset(name),
            None => match path.strip_prefix("/picture/") {
                Some(rest) => picture_route(rest),
                None => Route::NotFound,
            },
        },
        "POST" => match path.strip_prefix("/api/") {
            Some(command) if API_COMMANDS.contains(&command) => Route::Api(command.to_string()),
            _ => Route::NotFound,
        },
        _ => Route::NotFound,
    }
}

/// The embedded file that `path` names (`/app.js`), other than the page itself.
fn asset_named(path: &str) -> Option<&'static str> {
    let name = path.strip_prefix('/')?;
    assets::ASSETS
        .iter()
        .find(|entry| entry.0 != "index.html" && entry.0 == name)
        .map(|entry| entry.0)
}

/// The page as it is served: its `./app.js` and `./app.css` references carry the token
/// (`./app.js?token=...`). A relative reference drops the page's own query, so without this the
/// browser would ask for the files with no token. The token is base64url, which needs no escaping
/// in an attribute. A page without those references (the placeholder) is returned unchanged.
pub fn served_page(token: &str, html: &[u8]) -> Vec<u8> {
    let mut page = String::from_utf8_lossy(html).into_owned();
    for file in ["app.js", "app.css"] {
        page = page.replace(
            &format!("\"./{file}\""),
            &format!("\"./{file}?token={token}\""),
        );
    }
    page.into_bytes()
}

/// The command-line arguments for an API call: `<command>` with its fields as flags
/// (`{"game":"g","entry":"e"}` for `restore` -> `["restore","g","e","--json"]`), always ending
/// in `--json`. An empty body is `{}`. A field the command does not take, or a missing one, is
/// `Error::Usage`; so is a Nextcloud setup, a setup without `no-browser` (Google Drive may use
/// `finish`), a Dropbox `finish`, and any command not in `API_COMMANDS`.
pub fn api_args(command: &str, body: &[u8]) -> Result<Vec<String>> {
    if !API_COMMANDS.contains(&command) {
        return Err(usage_error(format!("'{command}' is not a panel command")));
    }
    let fields = body_fields(body)?;
    let (positionals, flags) = fields_of(command);
    if let Some(key) = fields
        .keys()
        .find(|key| !positionals.contains(&key.as_str()) && !flags.contains(&key.as_str()))
    {
        return Err(usage_error(format!("{command} does not take '{key}'")));
    }
    if command == "setup" {
        check_setup(&fields)?;
    }
    let mut argv = vec![command.to_string()];
    for name in positionals {
        if *name == "app-ids" {
            if let Some(Value::Array(values)) = fields.get(*name) {
                for value in values {
                    let app_id = value
                        .as_u64()
                        .filter(|id| (1..=u64::from(u32::MAX)).contains(id))
                        .ok_or_else(|| {
                            usage_error("app-ids must contain only positive whole numbers")
                        })?;
                    argv.push(app_id.to_string());
                }
            } else if fields.contains_key(*name) {
                return Err(usage_error("app-ids must be an array of whole numbers"));
            }
            continue;
        }
        let text = match (*name, fields.get(*name)) {
            ("appid", Some(Value::String(_))) if command == "launch-option" => {
                return Err(usage_error("appid must be a whole number"));
            }
            ("appid", Some(Value::Number(number))) => number
                .as_u64()
                .filter(|appid| (1..=u64::from(u32::MAX)).contains(appid))
                .map(|appid| appid.to_string())
                .ok_or_else(|| usage_error("appid must be a whole number"))?,
            (_, Some(Value::String(text))) => text.clone(),
            _ => return Err(usage_error(format!("{command} needs {name}"))),
        };
        check_value(name, &text)?;
        argv.push(text);
    }
    for name in flags {
        let Some(value) = fields.get(*name) else {
            continue;
        };
        match value {
            Value::Bool(on) if SWITCHES.contains(name) => {
                if *on {
                    argv.push(format!("--{name}"));
                }
            }
            Value::String(text) if !SWITCHES.contains(name) => {
                if *name == "current" {
                    if text.contains('\n') || text.contains('\r') || text.contains('\0') {
                        return Err(usage_error(
                            "'current' must be single-line text without NUL",
                        ));
                    }
                    argv.push(format!("--current={text}"));
                } else {
                    check_value(name, text)?;
                    argv.push(format!("--{name}"));
                    argv.push(text.clone());
                }
            }
            _ => return Err(usage_error(format!("'{name}' has the wrong type"))),
        }
    }
    argv.push("--json".to_string());
    Ok(argv)
}

/// One HTTP/1.1 answer: status line, `Content-Type`, `Content-Length`,
/// `Cache-Control: no-store`, `Connection: close`, then the body.
pub fn response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut out = head.into_bytes();
    out.extend_from_slice(body);
    out
}

/// Runs one command line (the arguments after the program name) and returns its exit code and
/// stdout; `cli::run` behind a closure.
pub type RunCommand = dyn Fn(&[String]) -> (i32, Vec<u8>);

/// Listen on `127.0.0.1:<port>` (any free port when `None`), announce the URL with its token
/// (`api::PanelData`: printed as the `--json` envelope or as text by the caller) through
/// `announce`, open it unless `open` is false, and answer requests one at a time until the
/// process is stopped. `run` runs one command line and returns (exit code, its stdout);
/// `cli::run` passes itself. Only returns with an error (the port is taken, no random bytes).
pub fn serve(
    port: Option<u16>,
    open: bool,
    announce: &dyn Fn(&PanelData),
    run: &RunCommand,
) -> Result<()> {
    let wanted = port.unwrap_or(0);
    let listener = TcpListener::bind(("127.0.0.1", wanted))
        .map_err(|error| Error::Io(format!("cannot listen on 127.0.0.1:{wanted}: {error}")))?;
    let bound = listener
        .local_addr()
        .map_err(|error| Error::Io(error.to_string()))?
        .port();
    let token = token_from_bytes(&oauth::random_bytes()?);
    let url = format!("http://127.0.0.1:{bound}/?token={token}");
    announce(&PanelData {
        url: url.clone(),
        port: bound,
    });
    if open {
        // The URL is already printed, so a browser that does not open is not an error.
        let _ = oauth::open_browser(&url);
    }
    for mut stream in listener.incoming().flatten() {
        // A client that fails or goes away is dropped; the next one is served.
        let _ = handle(&mut stream, &token, run);
    }
    Ok(())
}

/// What a request is answered with: status, content type, body.
type Answer = (u16, &'static str, Vec<u8>);

/// Read one request from `stream`, answer it and close the connection.
fn handle(stream: &mut TcpStream, token: &str, run: &RunCommand) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut received = Vec::new();
    let mut chunk = [0u8; 8192];
    let request = loop {
        match parse_request(&received) {
            Ok(Some(request)) => break request,
            Ok(None) => {}
            Err(error) => {
                return write_answer(stream, (400, TEXT, error.to_string().into_bytes()));
            }
        }
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return write_answer(stream, (400, TEXT, b"the request ended early".to_vec()));
        }
        received.extend_from_slice(&chunk[..read]);
    };
    write_answer(stream, answer(&request, token, run))
}

fn write_answer(
    stream: &mut TcpStream,
    (status, content_type, body): Answer,
) -> std::io::Result<()> {
    stream.write_all(&response(status, content_type, &body))?;
    stream.flush()
}

fn answer(request: &Request, token: &str, run: &RunCommand) -> Answer {
    match route(request, token) {
        Route::Asset(name) => match assets::ASSETS.iter().find(|entry| entry.0 == name) {
            Some(entry) if name == "index.html" => (200, entry.2, served_page(token, entry.1)),
            Some(entry) => (200, entry.2, entry.1.to_vec()),
            None => (404, TEXT, b"not found".to_vec()),
        },
        Route::Api(command) => api_answer(&command, &request.body, run),
        Route::Picture { game, entry } => match picture_bytes(&game, &entry, run) {
            Some((kind, bytes)) => (200, kind.mime(), bytes),
            None => (404, TEXT, b"no picture".to_vec()),
        },
        Route::Forbidden => (403, TEXT, b"forbidden".to_vec()),
        Route::NotFound => (404, TEXT, b"not found".to_vec()),
    }
}

fn api_answer(command: &str, body: &[u8], run: &RunCommand) -> Answer {
    if command == "picture" {
        let error = usage_error("read a picture with GET /picture/<game>/<entry>");
        return (400, JSON, api::failure(command, &error).into_bytes());
    }
    let argv = match api_args(command, body) {
        Ok(argv) => argv,
        Err(error) => return (400, JSON, api::failure(command, &error).into_bytes()),
    };
    let (_, stdout) = run(&argv);
    if serde_json::from_slice::<Value>(&stdout).is_ok() {
        (200, JSON, stdout)
    } else {
        let error = Error::Server("the command did not print an envelope".into());
        (500, JSON, api::failure(command, &error).into_bytes())
    }
}

/// A save card's picture, when the command wrote one that `card::check_picture` accepts. The
/// command writes into a fresh folder under the temporary directory, which is removed here.
fn picture_bytes(
    game: &str,
    entry: &str,
    run: &RunCommand,
) -> Option<(card::PictureKind, Vec<u8>)> {
    let random = oauth::random_bytes().ok()?;
    let folder = std::env::temp_dir().join(format!(
        "save-relay-panel-{}",
        oauth::base64url(&random[..12])
    ));
    fs::create_dir(&folder).ok()?;
    let out = folder.join("picture");
    let body = serde_json::json!({ "game": game, "entry": entry }).to_string();
    let bytes = match api_args("picture", body.as_bytes()) {
        Ok(mut argv) => {
            argv.pop(); // `--json`, added again after `--out`
            argv.push("--out".to_string());
            argv.push(out.to_string_lossy().into_owned());
            argv.push("--json".to_string());
            let (code, _) = run(&argv);
            if code == 0 {
                fs::read(&out).ok()
            } else {
                None
            }
        }
        Err(_) => None,
    };
    let _ = fs::remove_dir_all(&folder);
    let bytes = bytes?;
    let kind = card::check_picture(&bytes).ok()?;
    Some((kind, bytes))
}

/// `<game>/<entry>` or `<game>/<entry>/<file>` after `/picture/`.
fn picture_route(rest: &str) -> Route {
    let parts: Vec<&str> = rest.split('/').collect();
    match parts.as_slice() {
        [game, entry] | [game, entry, _] if valid_id(game) && valid_id(entry) => Route::Picture {
            game: game.to_string(),
            entry: entry.to_string(),
        },
        _ => Route::NotFound,
    }
}

fn valid_id(text: &str) -> bool {
    !text.is_empty() && !text.starts_with('-') && !text.contains('\0')
}

fn usage_error(message: impl Into<String>) -> Error {
    Error::Usage(message.into())
}

fn request_line(line: &str) -> Result<(String, &str)> {
    let mut parts = line.split(' ');
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    let version = parts.next().unwrap_or("");
    if parts.next().is_some()
        || method.is_empty()
        || target.is_empty()
        || version != "HTTP/1.1"
        || !method.bytes().all(|byte| byte.is_ascii_uppercase())
    {
        return Err(usage_error("the request line is not HTTP/1.1"));
    }
    Ok((method.to_string(), target))
}

fn parse_query(text: &str) -> Result<Vec<(String, String)>> {
    text.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| -> Result<(String, String)> {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            Ok((percent_decode(key, true)?, percent_decode(value, true)?))
        })
        .collect()
}

/// Percent-decode a path (`plus_is_space` false) or a query part (true). A bad escape or bytes
/// that are not UTF-8 are `Error::Usage`.
fn percent_decode(text: &str, plus_is_space: bool) -> Result<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let high = bytes.get(index + 1).and_then(|byte| hex_value(*byte));
                let low = bytes.get(index + 2).and_then(|byte| hex_value(*byte));
                match (high, low) {
                    (Some(high), Some(low)) => out.push(high * 16 + low),
                    _ => return Err(usage_error("the request has a bad percent-escape")),
                }
                index += 3;
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| usage_error("the request is not UTF-8"))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn query_value<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .query
        .iter()
        .find(|(key, _)| key.as_str() == name)
        .map(|(_, value)| value.as_str())
}

/// Compares every byte, so the time taken does not show how much of a guess was right.
fn same_token(given: &str, token: &str) -> bool {
    let (given, token) = (given.as_bytes(), token.as_bytes());
    given.len() == token.len()
        && given
            .iter()
            .zip(token)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

fn body_fields(body: &[u8]) -> Result<serde_json::Map<String, Value>> {
    if body.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Ok(serde_json::Map::new());
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(fields)) => Ok(fields),
        Ok(_) => Err(usage_error("the body must be a JSON object")),
        Err(error) => Err(usage_error(format!("the body is not JSON: {error}"))),
    }
}

/// The positional fields and the flag fields of each command.
fn fields_of(command: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match command {
        "history" => (&["game"], &[]),
        "status" => (&["game"], &["manifest", "saves-dir"]),
        "pull" => (&["game"], &["manifest", "saves-dir", "resolve", "anyway"]),
        "push" => (&["game"], &["manifest", "saves-dir", "resolve"]),
        "restore" => (&["game", "entry"], &["manifest", "saves-dir"]),
        "picture" => (&["game", "entry"], &[]),
        "answer" => (&["game", "answer"], &[]),
        "which" => (&["appid"], &[]),
        "steam-games" => (&[], &["no-steam-cloud", "refresh"]),
        "install-wrap" => (&["app-ids"], &["no-steam-cloud", "remove", "dry-run"]),
        "launch-option" => (&["appid"], &["remove", "current"]),
        "setup" => (&[], &["service", "device", "root", "no-browser", "finish"]),
        _ => (&[], &[]),
    }
}

/// A panel setup signs in to Google Drive or Dropbox without a browser of its own: it needs
/// `no-browser`, or `finish` for Google Drive. A Dropbox `finish` reads its code from standard
/// input, which the panel cannot supply, so it is refused. The Nextcloud password is never sent
/// through the panel.
fn check_setup(fields: &serde_json::Map<String, Value>) -> Result<()> {
    let service = match fields.get("service").and_then(Value::as_str) {
        Some(service @ ("gdrive" | "dropbox")) => service,
        Some("nextcloud") => {
            return Err(usage_error(
                "the Nextcloud password is read from standard input: run `save-relay setup` in a terminal",
            ));
        }
        _ => return Err(usage_error("setup needs service gdrive or dropbox")),
    };
    if service == "dropbox" && fields.get("finish") == Some(&Value::Bool(true)) {
        return Err(usage_error(
            "setup from the panel cannot finish a Dropbox sign-in: run `save-relay setup --service dropbox --finish` in a terminal",
        ));
    }
    let code_sign_in = ["no-browser", "finish"]
        .iter()
        .any(|key| fields.get(*key) == Some(&Value::Bool(true)));
    if !code_sign_in {
        return Err(usage_error(
            "setup from the panel needs no-browser or finish: it never opens a browser itself",
        ));
    }
    Ok(())
}

fn check_value(name: &str, text: &str) -> Result<()> {
    if text.is_empty() || text.starts_with('-') || text.contains('\0') {
        return Err(usage_error(format!("'{name}' is not a valid value")));
    }
    Ok(())
}
