//! `FakeDropbox`: the Dropbox endpoints `dropbox::Dropbox` uses, over one App folder in memory.

use crate::error::{Error, Result};
use crate::fake::REFRESH_TOKEN;
use crate::http::{Request, Response, Transport};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// One page of `files/list_folder`, so tests have to follow `/continue`.
const PAGE: usize = 2;
/// Access tokens last four hours on the fake's own clock.
const TOKEN_LIFETIME: Duration = Duration::from_secs(4 * 60 * 60);
/// The account `users/get_current_account` reports.
const ACCOUNT_EMAIL: &str = "fake@example.com";
/// Every metadata answer uses the same server time; nothing here advances a clock.
const SERVER_MODIFIED: &str = "2026-10-31T19:51:00Z";

/// One file or folder in the App folder. A folder has no bytes and no rev, as on Dropbox.
#[derive(Debug, Clone)]
struct Node {
    is_dir: bool,
    bytes: Vec<u8>,
    rev: String,
    id: String,
}

impl Node {
    fn dir(id: String) -> Node {
        Node {
            is_dir: true,
            bytes: Vec::new(),
            rev: String::new(),
            id,
        }
    }

    fn file(id: String, rev: String, bytes: Vec<u8>) -> Node {
        Node {
            is_dir: false,
            bytes,
            rev,
            id,
        }
    }
}

/// An access token and whether `expire_access_tokens` has retired it.
#[derive(Debug)]
struct IssuedToken {
    issued: Instant,
    expired: bool,
}

/// The rest of a listing: where the next page starts.
#[derive(Debug, Clone)]
struct Cursor {
    dir: String,
    offset: usize,
}

#[derive(Debug)]
struct Inner {
    nodes: BTreeMap<String, Node>,
    requests: Vec<String>,
    next_id: u64,
    next_rev: u64,
    next_token: u64,
    tokens: BTreeMap<String, IssuedToken>,
    cursors: BTreeMap<String, Cursor>,
    next_cursor: u64,
    revoked: bool,
    reachable: bool,
}

impl Default for Inner {
    fn default() -> Inner {
        Inner {
            nodes: BTreeMap::new(),
            requests: Vec::new(),
            next_id: 1,
            next_rev: 1,
            next_token: 1,
            tokens: BTreeMap::new(),
            cursors: BTreeMap::new(),
            next_cursor: 1,
            revoked: false,
            reachable: true,
        }
    }
}

/// `"a//b/"` -> `"a/b"`; the App folder's root is `""`.
fn normalize(path: &str) -> String {
    path.split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn parent_of(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((parent, _)) => parent,
        None => "",
    }
}

fn name_of(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((_, name)) => name,
        None => path,
    }
}

fn url_path(url: &str) -> &str {
    let without_query = url.split('?').next().unwrap_or(url);
    let after_scheme = without_query
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(without_query);
    after_scheme
        .find('/')
        .map(|index| &after_scheme[index..])
        .unwrap_or("")
}

fn form_decode(text: &str) -> String {
    fn hex(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => match (hex(bytes[index + 1]), hex(bytes[index + 2]))
            {
                (Some(high), Some(low)) => {
                    out.push(high * 16 + low);
                    index += 3;
                }
                _ => {
                    out.push(b'%');
                    index += 1;
                }
            },
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn form_fields(body: &str) -> BTreeMap<String, String> {
    body.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (form_decode(key), form_decode(value))
        })
        .collect()
}

fn error_response(status: u16, summary: &str, error: Value) -> Response {
    Response::new(
        status,
        json!({"error_summary": summary, "error": error}).to_string(),
    )
}

fn path_not_found() -> Response {
    error_response(
        409,
        "path/not_found/..",
        json!({".tag": "path", "path": {".tag": "not_found"}}),
    )
}

fn path_not_file() -> Response {
    error_response(
        409,
        "path/not_file/..",
        json!({".tag": "path", "path": {".tag": "not_file"}}),
    )
}

fn path_not_folder() -> Response {
    error_response(
        409,
        "path/not_folder/..",
        json!({".tag": "path", "path": {".tag": "not_folder"}}),
    )
}

fn path_conflict_file() -> Response {
    error_response(
        409,
        "path/conflict/file/..",
        json!({".tag": "path", "path": {".tag": "conflict", "conflict": {".tag": "file"}}}),
    )
}

fn path_conflict_folder() -> Response {
    error_response(
        409,
        "path/conflict/folder/..",
        json!({".tag": "path", "path": {".tag": "conflict", "conflict": {".tag": "folder"}}}),
    )
}

fn to_conflict(is_dir: bool) -> Response {
    let tag = if is_dir { "folder" } else { "file" };
    error_response(
        409,
        &format!("to/conflict/{tag}/.."),
        json!({".tag": "to", "to": {".tag": "conflict", "conflict": {".tag": tag}}}),
    )
}

fn from_lookup_not_found() -> Response {
    error_response(
        409,
        "from_lookup/not_found/..",
        json!({".tag": "from_lookup", "from_lookup": {".tag": "not_found"}}),
    )
}

fn path_lookup_not_found() -> Response {
    error_response(
        409,
        "path_lookup/not_found/..",
        json!({".tag": "path_lookup", "path_lookup": {".tag": "not_found"}}),
    )
}

fn expired_access_token() -> Response {
    error_response(
        401,
        "expired_access_token/",
        json!({".tag": "expired_access_token"}),
    )
}

fn invalid_access_token() -> Response {
    error_response(
        401,
        "invalid_access_token/",
        json!({".tag": "invalid_access_token"}),
    )
}

fn bad_request() -> Response {
    error_response(400, "bad_request/..", json!({".tag": "bad_request"}))
}

fn invalid_grant() -> Response {
    Response::new(
        400,
        json!({
            "error": "invalid_grant",
            "error_description": "refresh token is invalid or has been revoked"
        })
        .to_string(),
    )
}

/// `files/upload`'s `mode`, in the shapes `dropbox::upload_arg` sends.
enum UploadMode {
    Overwrite,
    Add,
    Update(String),
}

fn upload_mode(mode: Option<&Value>) -> Option<UploadMode> {
    match mode {
        None => Some(UploadMode::Add),
        Some(Value::String(text)) => match text.as_str() {
            "overwrite" => Some(UploadMode::Overwrite),
            "add" => Some(UploadMode::Add),
            _ => None,
        },
        Some(Value::Object(object)) => match object.get(".tag").and_then(Value::as_str) {
            Some("overwrite") => Some(UploadMode::Overwrite),
            Some("add") => Some(UploadMode::Add),
            Some("update") => object
                .get("update")
                .and_then(Value::as_str)
                .map(|rev| UploadMode::Update(rev.to_string())),
            _ => None,
        },
        Some(_) => None,
    }
}

impl Inner {
    fn next_id(&mut self) -> String {
        let id = format!("id:fake-{}", self.next_id);
        self.next_id += 1;
        id
    }

    fn next_rev(&mut self) -> String {
        let rev = format!("{:09x}", self.next_rev);
        self.next_rev += 1;
        rev
    }

    fn is_dir(&self, path: &str) -> bool {
        path.is_empty() || matches!(self.nodes.get(path), Some(node) if node.is_dir)
    }

    fn is_file(&self, path: &str) -> bool {
        matches!(self.nodes.get(path), Some(node) if !node.is_dir)
    }

    fn metadata(&self, path: &str) -> Option<Value> {
        if path.is_empty() {
            return Some(json!({
                ".tag": "folder",
                "name": "",
                "path_lower": "",
                "path_display": "/",
                "id": "id:root",
            }));
        }
        let node = self.nodes.get(path)?;
        let name = name_of(path);
        let display = format!("/{path}");
        let lower = display.to_lowercase();
        if node.is_dir {
            Some(json!({
                ".tag": "folder",
                "name": name,
                "path_lower": lower,
                "path_display": display,
                "id": node.id,
            }))
        } else {
            Some(json!({
                ".tag": "file",
                "name": name,
                "path_lower": lower,
                "path_display": display,
                "id": node.id,
                "client_modified": SERVER_MODIFIED,
                "server_modified": SERVER_MODIFIED,
                "rev": node.rev,
                "size": node.bytes.len(),
                "is_downloadable": true,
            }))
        }
    }

    fn write_file(&mut self, path: &str, bytes: &[u8]) -> String {
        let id = match self.nodes.get(path) {
            Some(node) if !node.is_dir => node.id.clone(),
            _ => self.next_id(),
        };
        let rev = self.next_rev();
        self.nodes.insert(
            path.to_string(),
            Node::file(id, rev.clone(), bytes.to_vec()),
        );
        rev
    }

    fn children(&self, dir: &str) -> Vec<String> {
        self.nodes
            .keys()
            .filter(|path| parent_of(path) == dir)
            .cloned()
            .collect()
    }

    fn tree_paths(&self, from: &str) -> Vec<String> {
        let prefix = format!("{from}/");
        self.nodes
            .keys()
            .filter(|path| *path == from || path.starts_with(&prefix))
            .cloned()
            .collect()
    }

    fn remove_tree(&mut self, path: &str) {
        let prefix = format!("{path}/");
        self.nodes
            .retain(|key, _| key != path && !key.starts_with(&prefix));
    }

    /// Make every folder above `path`; an upload creates what it needs, as Dropbox does.
    fn create_parents(&mut self, path: &str) -> std::result::Result<(), Response> {
        let parts: Vec<&str> = path.split('/').collect();
        let mut current = String::new();
        for part in parts.iter().take(parts.len().saturating_sub(1)) {
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(part);
            let existing = self.nodes.get(&current).map(|node| node.is_dir);
            match existing {
                Some(true) => {}
                Some(false) => return Err(path_conflict_file()),
                None => {
                    let id = self.next_id();
                    self.nodes.insert(current.clone(), Node::dir(id));
                }
            }
        }
        Ok(())
    }

    fn clone_tree(&mut self, from: &str, to: &str, fresh: bool) {
        let tree: Vec<(String, Node)> = self
            .tree_paths(from)
            .into_iter()
            .map(|path| {
                let node = self.nodes.get(&path).cloned().expect("tree path exists");
                (path, node)
            })
            .collect();
        let mut inserts = Vec::new();
        for (path, mut node) in tree {
            let rest = &path[from.len()..];
            let new_path = format!("{to}{rest}");
            if fresh {
                node.id = self.next_id();
                if !node.is_dir {
                    node.rev = self.next_rev();
                }
            }
            inserts.push((new_path, node));
        }
        for (path, node) in inserts {
            self.nodes.insert(path, node);
        }
    }

    fn list_page(&mut self, dir: &str, offset: usize) -> Response {
        let children = self.children(dir);
        let paths: Vec<String> = children.iter().skip(offset).take(PAGE).cloned().collect();
        let next = offset + paths.len();
        let has_more = next < children.len();
        let cursor = format!("cursor-{}", self.next_cursor);
        self.next_cursor += 1;
        self.cursors.insert(
            cursor.clone(),
            Cursor {
                dir: dir.to_string(),
                offset: next,
            },
        );
        let entries: Vec<Value> = paths
            .iter()
            .filter_map(|path| self.metadata(path))
            .collect();
        Response::new(
            200,
            json!({"entries": entries, "cursor": cursor, "has_more": has_more}).to_string(),
        )
    }

    fn handle_token(&mut self, request: &Request) -> Response {
        if self.revoked {
            return invalid_grant();
        }
        let body = String::from_utf8_lossy(&request.body);
        let fields = form_fields(&body);
        if fields.get("grant_type").map(String::as_str) != Some("refresh_token") {
            return Response::new(
                400,
                json!({
                    "error": "unsupported_grant_type",
                    "error_description": "grant_type must be refresh_token"
                })
                .to_string(),
            );
        }
        if fields.get("refresh_token").map(String::as_str) != Some(REFRESH_TOKEN) {
            return invalid_grant();
        }
        let access = format!("fake-access-{}", self.next_token);
        self.next_token += 1;
        self.tokens.insert(
            access.clone(),
            IssuedToken {
                issued: Instant::now(),
                expired: false,
            },
        );
        Response::new(
            200,
            json!({
                "access_token": access,
                "token_type": "bearer",
                "expires_in": TOKEN_LIFETIME.as_secs(),
            })
            .to_string(),
        )
    }

    fn auth_error(&self, request: &Request) -> Option<Response> {
        let token = request
            .header_value("Authorization")
            .and_then(|value| value.strip_prefix("Bearer "));
        let token = match token {
            Some(token) => token,
            None => return Some(invalid_access_token()),
        };
        match self.tokens.get(token) {
            None => Some(invalid_access_token()),
            Some(_) if self.revoked => Some(invalid_access_token()),
            Some(issued) => {
                if issued.expired || issued.issued.elapsed() >= TOKEN_LIFETIME {
                    Some(expired_access_token())
                } else {
                    None
                }
            }
        }
    }

    fn body_json(request: &Request) -> std::result::Result<Value, Response> {
        serde_json::from_slice(&request.body).map_err(|_| bad_request())
    }

    fn header_arg(request: &Request) -> std::result::Result<Value, Response> {
        let text = request
            .header_value("Dropbox-API-Arg")
            .ok_or_else(bad_request)?;
        serde_json::from_str(text).map_err(|_| bad_request())
    }

    fn list_folder(&mut self, request: &Request) -> Response {
        let body = match Self::body_json(request) {
            Ok(body) => body,
            Err(response) => return response,
        };
        let path = normalize(body.get("path").and_then(Value::as_str).unwrap_or(""));
        if self.is_file(&path) {
            return path_not_folder();
        }
        if !self.is_dir(&path) {
            return path_not_found();
        }
        self.list_page(&path, 0)
    }

    fn list_continue(&mut self, request: &Request) -> Response {
        let body = match Self::body_json(request) {
            Ok(body) => body,
            Err(response) => return response,
        };
        let cursor = body.get("cursor").and_then(Value::as_str).unwrap_or("");
        match self.cursors.remove(cursor) {
            Some(cursor) => self.list_page(&cursor.dir, cursor.offset),
            None => path_not_found(),
        }
    }

    fn get_metadata(&mut self, request: &Request) -> Response {
        let body = match Self::body_json(request) {
            Ok(body) => body,
            Err(response) => return response,
        };
        let path = normalize(body.get("path").and_then(Value::as_str).unwrap_or(""));
        match self.metadata(&path) {
            Some(metadata) => Response::new(200, metadata.to_string()),
            None => path_not_found(),
        }
    }

    fn upload(&mut self, request: &Request) -> Response {
        let arg = match Self::header_arg(request) {
            Ok(arg) => arg,
            Err(response) => return response,
        };
        let path = normalize(arg.get("path").and_then(Value::as_str).unwrap_or(""));
        if path.is_empty() {
            return path_conflict_file();
        }
        let mode = match upload_mode(arg.get("mode")) {
            Some(mode) => mode,
            None => return bad_request(),
        };
        if let Err(response) = self.create_parents(&path) {
            return response;
        }
        // The backend always sends `autorename: false` and `strict_conflict: true`; the fake
        // treats every existing or mismatched target as a conflict rather than reproducing
        // Dropbox's autorename.
        match &mode {
            UploadMode::Add => match self.nodes.get(&path) {
                Some(node) if node.is_dir => return path_conflict_folder(),
                Some(_) => return path_conflict_file(),
                None => {}
            },
            UploadMode::Update(rev) => match self.nodes.get(&path) {
                Some(node) if node.is_dir => return path_conflict_folder(),
                Some(node) if node.rev != rev.as_str() => return path_conflict_file(),
                Some(_) => {}
                None => return path_not_found(),
            },
            UploadMode::Overwrite => match self.nodes.get(&path) {
                Some(node) if node.is_dir => return path_conflict_folder(),
                _ => {}
            },
        }
        self.write_file(&path, &request.body);
        match self.metadata(&path) {
            Some(metadata) => Response::new(200, metadata.to_string()),
            None => path_not_found(),
        }
    }

    fn download(&mut self, request: &Request) -> Response {
        let arg = match Self::header_arg(request) {
            Ok(arg) => arg,
            Err(response) => return response,
        };
        let path = normalize(arg.get("path").and_then(Value::as_str).unwrap_or(""));
        let node = match self.nodes.get(&path) {
            Some(node) => node.clone(),
            None => return path_not_found(),
        };
        if node.is_dir {
            return path_not_file();
        }
        match self.metadata(&path) {
            Some(metadata) => Response::new(200, node.bytes).header(
                "Dropbox-API-Result",
                &crate::dropbox::header_json(&metadata),
            ),
            None => path_not_found(),
        }
    }

    fn create_folder(&mut self, request: &Request) -> Response {
        let body = match Self::body_json(request) {
            Ok(body) => body,
            Err(response) => return response,
        };
        let path = normalize(body.get("path").and_then(Value::as_str).unwrap_or(""));
        if path.is_empty() {
            return path_conflict_folder();
        }
        match self.nodes.get(&path) {
            Some(node) if node.is_dir => return path_conflict_folder(),
            Some(_) => return path_conflict_file(),
            None => {}
        }
        let parent = parent_of(&path);
        if self.is_file(parent) {
            return path_conflict_file();
        }
        if !self.is_dir(parent) {
            return path_not_found();
        }
        let id = self.next_id();
        self.nodes.insert(path.clone(), Node::dir(id));
        match self.metadata(&path) {
            Some(metadata) => Response::new(200, json!({"metadata": metadata}).to_string()),
            None => path_not_found(),
        }
    }

    fn copy_or_move(&mut self, request: &Request, moving: bool) -> Response {
        let body = match Self::body_json(request) {
            Ok(body) => body,
            Err(response) => return response,
        };
        let from = normalize(body.get("from_path").and_then(Value::as_str).unwrap_or(""));
        let to = normalize(body.get("to_path").and_then(Value::as_str).unwrap_or(""));
        if !self.nodes.contains_key(&from) {
            return from_lookup_not_found();
        }
        if to.is_empty() {
            return to_conflict(true);
        }
        if let Some(target) = self.nodes.get(&to) {
            return to_conflict(target.is_dir);
        }
        let parent = parent_of(&to);
        if self.is_file(parent) {
            return path_conflict_file();
        }
        if !self.is_dir(parent) {
            return path_not_found();
        }
        if from != to && to.starts_with(&format!("{from}/")) {
            return path_conflict_folder();
        }
        self.clone_tree(&from, &to, !moving);
        if moving {
            self.remove_tree(&from);
        }
        match self.metadata(&to) {
            Some(metadata) => Response::new(200, json!({"metadata": metadata}).to_string()),
            None => path_not_found(),
        }
    }

    fn delete(&mut self, request: &Request) -> Response {
        let body = match Self::body_json(request) {
            Ok(body) => body,
            Err(response) => return response,
        };
        let path = normalize(body.get("path").and_then(Value::as_str).unwrap_or(""));
        if path.is_empty() || !self.nodes.contains_key(&path) {
            return path_lookup_not_found();
        }
        let metadata = match self.metadata(&path) {
            Some(metadata) => metadata,
            None => return path_lookup_not_found(),
        };
        self.remove_tree(&path);
        Response::new(200, json!({"metadata": metadata}).to_string())
    }

    fn current_account(&self) -> Response {
        Response::new(
            200,
            json!({
                "account_id": "dbid:fake",
                "name": {
                    "given_name": "Fake",
                    "surname": "User",
                    "familiar_name": "Fake",
                    "display_name": "Fake User",
                    "abbreviated_name": "FU"
                },
                "email": ACCOUNT_EMAIL,
                "email_verified": true,
                "disabled": false,
                "country": "US",
                "locale": "en",
                "referral_link": "https://www.dropbox.com/referrals/fake",
                "is_paired": false,
                "account_type": {".tag": "basic"}
            })
            .to_string(),
        )
    }

    fn handle(&mut self, request: &Request) -> Response {
        let route = url_path(&request.url);
        if route == "/oauth2/token" {
            return self.handle_token(request);
        }
        if let Some(response) = self.auth_error(request) {
            return response;
        }
        match route {
            "/2/files/list_folder" => self.list_folder(request),
            "/2/files/list_folder/continue" => self.list_continue(request),
            "/2/files/get_metadata" => self.get_metadata(request),
            "/2/files/upload" => self.upload(request),
            "/2/files/download" => self.download(request),
            "/2/files/create_folder_v2" => self.create_folder(request),
            "/2/files/copy_v2" => self.copy_or_move(request, false),
            "/2/files/move_v2" => self.copy_or_move(request, true),
            "/2/files/delete_v2" => self.delete(request),
            "/2/users/get_current_account" => self.current_account(),
            _ => Response::new(404, json!({"error_summary": "bad_request/.."}).to_string()),
        }
    }
}

/// Answers `files/list_folder` (+ `/continue`, pages of 2 so paging is exercised),
/// `files/get_metadata`, `files/upload` (modes `add`, `overwrite`, `update` with
/// `strict_conflict`), `files/download` (`Dropbox-API-Result` header), `files/create_folder_v2`,
/// `files/copy_v2`, `files/move_v2`, `files/delete_v2`, `users/get_current_account`, and
/// `oauth2/token` (refresh). Like real Dropbox it creates missing parent folders on upload;
/// every write gives a new `rev`. Interior mutability (a `Mutex`).
#[derive(Debug, Default)]
pub struct FakeDropbox {
    inner: Mutex<Inner>,
}

impl FakeDropbox {
    pub fn new() -> FakeDropbox {
        FakeDropbox::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("fake dropbox lock poisoned")
    }

    /// While `false`, `send` returns `Error::Unreachable("fake dropbox is offline")`.
    pub fn set_reachable(&self, reachable: bool) {
        self.lock().reachable = reachable;
    }

    /// From now on the token endpoint answers `invalid_grant` and every access token is refused.
    pub fn revoke(&self) {
        self.lock().revoked = true;
    }

    /// Every access token handed out so far gets 401 `expired_access_token`.
    pub fn expire_access_tokens(&self) {
        for token in self.lock().tokens.values_mut() {
            token.expired = true;
        }
    }

    /// Every file path in the App folder, `Remote` form, sorted. Works while unreachable.
    pub fn file_paths(&self) -> Vec<String> {
        self.lock()
            .nodes
            .iter()
            .filter_map(|(path, node)| {
                if node.is_dir {
                    None
                } else {
                    Some(path.clone())
                }
            })
            .collect()
    }

    /// `"<METHOD> <url>"` of every request so far, in order (no headers, no tokens).
    pub fn requests(&self) -> Vec<String> {
        self.lock().requests.clone()
    }
}

impl Transport for FakeDropbox {
    fn send(&self, request: &Request) -> Result<Response> {
        let mut inner = self.lock();
        inner
            .requests
            .push(format!("{} {}", request.method, request.url));
        if !inner.reachable {
            return Err(Error::Unreachable("fake dropbox is offline".to_string()));
        }
        Ok(inner.handle(request))
    }
}
