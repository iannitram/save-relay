//! Dropbox (HTTP API v2) as a `Remote`, inside the app's App folder only (SPEC.md M9, M10;
//! docs/adr/0002-storage-backends.md). Paths map one to one: `"a/b"` is `"/a/b"` in the App
//! folder and `""` is its root. A file's etag is its Dropbox `rev`.
//!
//! The overwrite guard that M9 needs is Dropbox's own: an upload with
//! `mode: {".tag": "update", "update": <rev>}` and `strict_conflict: true` succeeds only while
//! the file is still at that rev, and `mode: add` only while it does not exist.
//!
//! The request builders and answer parsers are pure; `Dropbox` sends them through a
//! `http::Transport` with a bearer token from `oauth::TokenSource`.

use crate::error::{not_built, Error, Result};
use crate::http::Transport;
use crate::oauth::TokenSource;
use crate::remote::{Precondition, Remote, RemoteEntry};
use std::sync::Arc;

/// RPC endpoints (`files/list_folder`, `files/create_folder_v2`, ...).
pub const API: &str = "https://api.dropboxapi.com/2";
/// Content endpoints (`files/upload`, `files/download`).
pub const CONTENT: &str = "https://content.dropboxapi.com/2";

/// A `Remote` path as a Dropbox path: `""` -> `""` (the App folder's root, as `list_folder`
/// wants it); `"a/b c"` -> `"/a/b c"`.
pub fn api_path(path: &str) -> String {
    crate::not_built!("dropbox::api_path")
}

/// `value` as compact JSON safe for an HTTP header (`Dropbox-API-Arg`): every character
/// above U+007F written as `\uXXXX` (lowercase hex; two escapes, a UTF-16 pair, above U+FFFF),
/// so the header is ASCII and parses back to the same value.
pub fn header_json(value: &serde_json::Value) -> String {
    crate::not_built!("dropbox::header_json")
}

/// The `Dropbox-API-Arg` of an upload, through `header_json`:
/// `{"path": api_path(path), "mode": M, "autorename": false, "mute": true,
/// "strict_conflict": true}` where `M` is `"overwrite"` for `Precondition::Any`, `"add"` for
/// `Absent`, `{".tag": "update", "update": <rev>}` for `Match(rev)`.
pub fn upload_arg(path: &str, pre: &Precondition) -> String {
    crate::not_built!("dropbox::upload_arg")
}

/// Which `Remote` call an answer belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    List,
    Get,
    Put,
    Mkcol,
    Copy,
    Move,
    Delete,
}

/// What a backend does with an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Use the answer.
    Ok,
    /// Get a new access token (`TokenSource::invalidate`) and send once more.
    Refresh,
    /// Wait (the `Retry-After` header, else 1 s, 2 s, 4 s) and send again, at most 3 times;
    /// then `Error::Server`.
    Retry,
    Err(Error),
}

/// Sort a Dropbox answer by its status and, for 409, its `error_summary` (a `/`-separated
/// tag path such as `"path/not_found/.."`). `path` is the `Remote` path of the call (`from`
/// for Copy and Move).
///
/// | status | summary starts with | op | outcome |
/// |---|---|---|---|
/// | 200 | | any | Ok |
/// | 401 | | any | Refresh |
/// | 429, 500-599 | | any | Retry |
/// | 409 | `path/conflict/folder` | Mkcol | Ok (already a folder) |
/// | 409 | `path/conflict` | Put | Err(Changed) |
/// | 409 | `path/conflict` | Mkcol | Err(Server("a file is there: <path>")) |
/// | 409 | `to/conflict` | Copy, Move | Err(Server("exists: <summary>")) |
/// | 409 | `path/not_found`, `path_lookup/not_found`, `from_lookup/not_found` | any | Err(NotFound(path)) |
/// | 409 | `path/not_file`, `path/not_folder` | Get, List | Err(NotFound(path)) |
/// | other | | any | Err(Server("dropbox <status>: <summary or body>")) |
///
/// Rows are checked top to bottom; the first that fits wins. A body that is not JSON counts
/// as an empty summary.
pub fn classify(op: Op, status: u16, body: &str, path: &str) -> Outcome {
    crate::not_built!("dropbox::classify")
}

/// One page of `files/list_folder` (or `/continue`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPage {
    /// `"file"` entries (size, etag = rev) and `"folder"` entries (size 0, etag `None`);
    /// `"deleted"` entries left out. Sorted by name.
    pub entries: Vec<RemoteEntry>,
    pub cursor: String,
    pub has_more: bool,
}

/// Parse a `list_folder` answer. Not JSON, or an entry without `.tag`/`name`, is
/// `Error::Server("dropbox: bad list answer")`.
pub fn parse_list(body: &str) -> Result<ListPage> {
    not_built("dropbox::parse_list")
}

/// The `rev` of a file metadata answer (an upload's reply, or the `Dropbox-API-Result` header
/// of a download). Missing, or not JSON, is `Error::Server("dropbox: no rev in the answer")`.
pub fn parse_rev(body: &str) -> Result<String> {
    not_built("dropbox::parse_rev")
}

/// The real Dropbox, one App folder. Dropbox creates missing parent folders on its own, so
/// `put`, `mkcol`, `copy` and `move_to` check the parent first (and remember the folders they
/// have seen), keeping the `Remote` contract's `NotFound(parent)`. Never logs a token.
pub struct Dropbox {
    _private: (),
}

impl Dropbox {
    pub fn new(tokens: TokenSource, transport: Arc<dyn Transport>) -> Dropbox {
        Dropbox { _private: () }
    }

    /// The signed-in account's e-mail address (`users/get_current_account`), for the config
    /// and `status`.
    pub fn account_email(&self) -> Result<String> {
        not_built("dropbox::Dropbox::account_email")
    }
}

impl Remote for Dropbox {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        not_built("dropbox::Dropbox::list")
    }
    fn get(&self, path: &str) -> Result<(Vec<u8>, String)> {
        not_built("dropbox::Dropbox::get")
    }
    fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> Result<String> {
        not_built("dropbox::Dropbox::put")
    }
    fn mkcol(&self, dir: &str) -> Result<()> {
        not_built("dropbox::Dropbox::mkcol")
    }
    fn copy(&self, from: &str, to: &str) -> Result<()> {
        not_built("dropbox::Dropbox::copy")
    }
    fn move_to(&self, from: &str, to: &str) -> Result<()> {
        not_built("dropbox::Dropbox::move_to")
    }
    fn delete(&self, path: &str) -> Result<()> {
        not_built("dropbox::Dropbox::delete")
    }
}
