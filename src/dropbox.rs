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

use crate::error::{Error, Result};
use crate::http::{Request, Response, Transport};
use crate::oauth::TokenSource;
use crate::remote::{Precondition, Remote, RemoteEntry};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

/// RPC endpoints (`files/list_folder`, `files/create_folder_v2`, ...).
pub const API: &str = "https://api.dropboxapi.com/2";
/// Content endpoints (`files/upload`, `files/download`).
pub const CONTENT: &str = "https://content.dropboxapi.com/2";

/// A `Remote` path as a Dropbox path: `""` -> `""` (the App folder's root, as `list_folder`
/// wants it); `"a/b c"` -> `"/a/b c"`.
pub fn api_path(path: &str) -> String {
    if path.is_empty() {
        String::new()
    } else {
        format!("/{path}")
    }
}

/// `value` as compact JSON safe for an HTTP header (`Dropbox-API-Arg`): every character
/// above U+007F written as `\uXXXX` (lowercase hex; two escapes, a UTF-16 pair, above U+FFFF),
/// so the header is ASCII and parses back to the same value.
pub fn header_json(value: &serde_json::Value) -> String {
    let text = value.to_string();
    let mut out = String::with_capacity(text.len());
    let mut units = [0u16; 2];
    for ch in text.chars() {
        if ch.is_ascii() {
            out.push(ch);
        } else {
            for unit in ch.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

/// The `Dropbox-API-Arg` of an upload, through `header_json`:
/// `{"path": api_path(path), "mode": M, "autorename": false, "mute": true,
/// "strict_conflict": true}` where `M` is `"overwrite"` for `Precondition::Any`, `"add"` for
/// `Absent`, `{".tag": "update", "update": <rev>}` for `Match(rev)`.
pub fn upload_arg(path: &str, pre: &Precondition) -> String {
    let mode = match pre {
        Precondition::Any => serde_json::json!("overwrite"),
        Precondition::Absent => serde_json::json!("add"),
        Precondition::Match(rev) => serde_json::json!({".tag": "update", "update": rev}),
    };
    let value = serde_json::json!({
        "path": api_path(path),
        "mode": mode,
        "autorename": false,
        "mute": true,
        "strict_conflict": true,
    });
    header_json(&value)
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
    let summary = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error_summary")
                .and_then(|s| s.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();
    match status {
        200 => return Outcome::Ok,
        401 => return Outcome::Refresh,
        429 | 500..=599 => return Outcome::Retry,
        409 => {}
        _ => {}
    }
    if status == 409 {
        if summary.starts_with("path/conflict/folder") && op == Op::Mkcol {
            return Outcome::Ok;
        }
        if summary.starts_with("path/conflict") && op == Op::Put {
            return Outcome::Err(Error::Changed);
        }
        if summary.starts_with("path/conflict") && op == Op::Mkcol {
            return Outcome::Err(Error::Server(format!("a file is there: {path}")));
        }
        if summary.starts_with("to/conflict") && matches!(op, Op::Copy | Op::Move) {
            return Outcome::Err(Error::Server(format!("exists: {summary}")));
        }
        if summary.starts_with("path/not_found")
            || summary.starts_with("path_lookup/not_found")
            || summary.starts_with("from_lookup/not_found")
        {
            return Outcome::Err(Error::NotFound(path.into()));
        }
        if matches!(op, Op::Get | Op::List)
            && (summary.starts_with("path/not_file") || summary.starts_with("path/not_folder"))
        {
            return Outcome::Err(Error::NotFound(path.into()));
        }
    }
    let what = if summary.is_empty() { body } else { &summary };
    Outcome::Err(Error::Server(format!("dropbox {status}: {what}")))
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
    let bad = || Error::Server("dropbox: bad list answer".into());
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| bad())?;
    let object = value.as_object().ok_or_else(bad)?;
    let entries = object
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(bad)?;
    let mut parsed = Vec::new();
    for entry in entries {
        let entry = entry.as_object().ok_or_else(bad)?;
        let tag = entry
            .get(".tag")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(bad)?;
        let name = entry
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(bad)?;
        match tag {
            "file" => parsed.push(RemoteEntry {
                name: name.into(),
                is_dir: false,
                size: entry
                    .get("size")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                etag: entry
                    .get("rev")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
            }),
            "folder" => parsed.push(RemoteEntry {
                name: name.into(),
                is_dir: true,
                size: 0,
                etag: None,
            }),
            _ => {}
        }
    }
    parsed.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(ListPage {
        entries: parsed,
        cursor: object
            .get("cursor")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .into(),
        has_more: object
            .get("has_more")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

/// The `rev` of a file metadata answer (an upload's reply, or the `Dropbox-API-Result` header
/// of a download). Missing, or not JSON, is `Error::Server("dropbox: no rev in the answer")`.
pub fn parse_rev(body: &str) -> Result<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("rev")?.as_str().map(str::to_string))
        .ok_or_else(|| Error::Server("dropbox: no rev in the answer".into()))
}

/// The real Dropbox, one App folder. Dropbox creates missing parent folders on its own, so
/// `put`, `mkcol`, `copy` and `move_to` check the parent first, keeping the `Remote`
/// contract's `NotFound(parent)`. Never logs a token.
pub struct Dropbox {
    tokens: TokenSource,
    transport: Arc<dyn Transport>,
    sleep: Box<dyn Fn(Duration)>,
}

impl Dropbox {
    pub fn new(tokens: TokenSource, transport: Arc<dyn Transport>) -> Dropbox {
        Self::with_sleep(tokens, transport, std::thread::sleep)
    }

    /// Inject the retry wait, so fake-backed tests never need to sleep.
    pub fn with_sleep(
        tokens: TokenSource,
        transport: Arc<dyn Transport>,
        sleep: impl Fn(Duration) + 'static,
    ) -> Dropbox {
        Dropbox {
            tokens,
            transport,
            sleep: Box::new(sleep),
        }
    }

    fn send(&self, request: Request, op: Op, path: &str) -> Result<Response> {
        let mut refreshed = false;
        let mut retries = 0;
        loop {
            let token = self.tokens.bearer()?;
            let outgoing = request
                .clone()
                .header("Authorization", &format!("Bearer {}", token.expose()));
            let response = self.transport.send(&outgoing)?;
            match classify(op, response.status, &response.text(), path) {
                Outcome::Ok => return Ok(response),
                Outcome::Refresh if !refreshed => {
                    self.tokens.invalidate();
                    refreshed = true;
                }
                Outcome::Refresh => return Err(Error::ReauthNeeded("Dropbox".into())),
                Outcome::Retry if retries < 3 => {
                    let seconds = response
                        .header_value("Retry-After")
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(1 << retries);
                    (self.sleep)(Duration::from_secs(seconds));
                    retries += 1;
                }
                // Never echo service-controlled text, which may contain a token.
                Outcome::Retry | Outcome::Err(Error::Server(_)) => {
                    return Err(Error::Server(format!("dropbox {}", response.status)));
                }
                Outcome::Err(error) => return Err(error),
            }
        }
    }

    fn rpc(&self, endpoint: &str, body: Value, op: Op, path: &str) -> Result<Response> {
        self.send(
            Request::new("POST", &format!("{API}/{endpoint}"))
                .header("Content-Type", "application/json")
                .body(body.to_string().into_bytes()),
            op,
            path,
        )
    }

    fn parent(&self, path: &str) -> Result<()> {
        let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
        if parent.is_empty() {
            return Ok(());
        }
        // Resolve once per operation: a folder cached across calls could have been deleted
        // by another machine, letting Dropbox silently recreate it during an upload.
        let response = self.rpc(
            "files/get_metadata",
            json!({"path": api_path(parent)}),
            Op::List,
            parent,
        )?;
        let metadata: Value = serde_json::from_slice(&response.body)
            .map_err(|_| Error::Server("dropbox: bad metadata answer".into()))?;
        if metadata[".tag"] == "folder" {
            Ok(())
        } else {
            Err(Error::NotFound(parent.into()))
        }
    }

    fn relocate(&self, from: &str, to: &str, op: Op) -> Result<()> {
        self.parent(to)?;
        let endpoint = if op == Op::Copy {
            "files/copy_v2"
        } else {
            "files/move_v2"
        };
        self.rpc(
            endpoint,
            json!({"from_path": api_path(from), "to_path": api_path(to), "autorename": false}),
            op,
            from,
        )?;
        Ok(())
    }

    /// The signed-in account's e-mail address (`users/get_current_account`).
    pub fn account_email(&self) -> Result<String> {
        let response = self.rpc("users/get_current_account", Value::Null, Op::Get, "")?;
        serde_json::from_slice::<Value>(&response.body)
            .ok()
            .and_then(|v| v["email"].as_str().map(str::to_string))
            .ok_or_else(|| Error::Server("dropbox: no email in the answer".into()))
    }
}

impl Remote for Dropbox {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        let mut response = self.rpc(
            "files/list_folder",
            json!({"path": api_path(dir)}),
            Op::List,
            dir,
        )?;
        let mut entries = Vec::new();
        loop {
            let page = parse_list(&response.text())?;
            entries.extend(page.entries);
            if !page.has_more {
                break;
            }
            if page.cursor.is_empty() {
                return Err(Error::Server("dropbox: missing cursor".into()));
            }
            response = self.rpc(
                "files/list_folder/continue",
                json!({"cursor": page.cursor}),
                Op::List,
                dir,
            )?;
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }
    fn get(&self, path: &str) -> Result<(Vec<u8>, String)> {
        let response = self.send(
            Request::new("POST", &format!("{CONTENT}/files/download")).header(
                "Dropbox-API-Arg",
                &header_json(&json!({"path": api_path(path)})),
            ),
            Op::Get,
            path,
        )?;
        let rev = parse_rev(response.header_value("Dropbox-API-Result").unwrap_or(""))?;
        Ok((response.body, rev))
    }
    fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> Result<String> {
        if path.is_empty() {
            return Err(Error::Server("cannot write the files root".into()));
        }
        self.parent(path)?;
        let response = self.send(
            Request::new("POST", &format!("{CONTENT}/files/upload"))
                .header("Content-Type", "application/octet-stream")
                .header("Dropbox-API-Arg", &upload_arg(path, pre))
                .body(bytes.to_vec()),
            Op::Put,
            path,
        );
        match response {
            Err(Error::NotFound(_)) if matches!(pre, Precondition::Match(_)) => Err(Error::Changed),
            Err(Error::Changed) if matches!(pre, Precondition::Any) => {
                Err(Error::Server("dropbox: a folder is there".into()))
            }
            other => parse_rev(&other?.text()),
        }
    }
    fn mkcol(&self, dir: &str) -> Result<()> {
        self.parent(dir)?;
        self.rpc(
            "files/create_folder_v2",
            json!({"path": api_path(dir), "autorename": false}),
            Op::Mkcol,
            dir,
        )?;
        Ok(())
    }
    fn copy(&self, from: &str, to: &str) -> Result<()> {
        self.relocate(from, to, Op::Copy)
    }
    fn move_to(&self, from: &str, to: &str) -> Result<()> {
        self.relocate(from, to, Op::Move)
    }
    fn delete(&self, path: &str) -> Result<()> {
        self.rpc(
            "files/delete_v2",
            json!({"path": api_path(path)}),
            Op::Delete,
            path,
        )?;
        Ok(())
    }
}
