//! `save-relay panel [--port N] [--no-open]`: the Save Relay panel in the browser (SPEC.md M15;
//! docs/adr/0003-baton-wrap-cards-panel.md). The same views as the Game Mode panel (`decky/`),
//! from the same front end (`panel/`), served on `127.0.0.1` by a tiny HTTP/1.1 server on
//! `std::net` (no new dependency).
//!
//! - `GET /` and the front end's files: embedded in the binary at build time from
//!   `panel/dist` with the `panel-dist` feature (after `pnpm --dir panel build`, which writes
//!   fixed names: `index.html`, `app.js`, `app.css`); without it, the placeholder page
//!   `panel/placeholder/index.html`, so `cargo build` never needs Node. No build script.
//! - `POST /api/<command>` with a JSON object body: runs the same code as the command line
//!   (`api_args`, then `cli::run`) and answers its `--json` envelope.
//! - `GET /picture/<game>/<entry>`: a save card's picture.
//!
//! A random token is in the URL that is opened (`/?token=...`) and must come with every
//! request (`X-Save-Relay-Token` header or `token` query), so other pages in the browser cannot
//! call it; a missing or wrong token is 403. The browser opens like `oauth::open_browser`.

use crate::error::{not_built, Result};

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

/// The commands `POST /api/<command>` may run. `setup` only for Google Drive and Dropbox with
/// `no_browser` or `finish` (the QR code sign-in); never `wrap`, `install-wrap`, `panel` or a
/// Nextcloud setup (its password stays on standard input).
pub const API_COMMANDS: [&str; 10] = [
    "status", "history", "restore", "pull", "push", "answer", "where", "which", "picture", "setup",
];

/// The token from 32 random bytes (`oauth::random_bytes`): their base64url, 43 characters.
pub fn token_from_bytes(bytes: &[u8; 32]) -> String {
    crate::not_built!("panel::token_from_bytes")
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

/// Read one HTTP/1.1 request from the bytes received (request line, headers, a
/// `Content-Length` body; at most 1 MiB). `Ok(None)` while incomplete; malformed is
/// `Error::Usage`.
pub fn parse_request(bytes: &[u8]) -> Result<Option<Request>> {
    not_built("panel::parse_request")
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

/// Route `request`: the token first, then the path. `/` is `index.html`. Pure.
pub fn route(request: &Request, token: &str) -> Route {
    crate::not_built!("panel::route")
}

/// The command-line arguments for an API call: `<command>` with its fields as flags
/// (`{"game":"g","entry":"e"}` for `restore` -> `["restore","g","e","--json"]`), always ending
/// in `--json`. A field the command does not take, or a missing one, is `Error::Usage`.
pub fn api_args(command: &str, body: &[u8]) -> Result<Vec<String>> {
    not_built("panel::api_args")
}

/// One HTTP/1.1 answer: status line, `Content-Type`, `Content-Length`,
/// `Cache-Control: no-store`, `Connection: close`, then the body.
pub fn response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    crate::not_built!("panel::response")
}

/// Runs one command line (the arguments after the program name) and returns its exit code and
/// stdout; `cli::run` behind a closure.
pub type RunCommand = dyn Fn(&[String]) -> (i32, Vec<u8>);

/// Listen on `127.0.0.1:<port>` (any free port when `None`), print the URL with its token
/// (`api::PanelData` with `--json`) through `announce`, open it unless `open` is false, and
/// answer requests one at a time until the process is stopped. `run` runs one command line and
/// returns (exit code, its stdout); `cli::run` passes itself.
pub fn serve(
    port: Option<u16>,
    open: bool,
    announce: &dyn Fn(&str),
    run: &RunCommand,
) -> Result<()> {
    not_built("panel::serve")
}
