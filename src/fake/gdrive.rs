//! `FakeDrive`: the Drive v3 endpoints `gdrive::Drive` uses, over a file tree in memory.

use crate::error::{Error, Result};
use crate::fake::REFRESH_TOKEN;
use crate::gdrive::FOLDER_MIME;
use crate::http::{Request, Response, Transport};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};

/// One page of `files.list`, so tests have to follow `nextPageToken`.
const PAGE: usize = 2;
/// Access tokens last an hour, as Google's do; `expire_access_tokens` retires them early.
const TOKEN_LIFETIME: u64 = 3600;
/// The clock's start: 2026-10-31T19:51:00Z.
const CLOCK_START: u64 = 1793476260;
/// The account `about` reports.
const ACCOUNT_EMAIL: &str = "fake@example.com";
/// Drive's token endpoint (`oauth::GOOGLE.token_url`).
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// A file with no `mimeType` in its metadata is treated as this.
const DEFAULT_MIME: &str = "application/octet-stream";
/// Days and months for the text forms Drive uses.
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

// Assumptions about Drive this fake models, checked by F46's real round trip:
// - `files.create` with a client id (from `files.generateIds`) succeeds only while the id is
//   unused; every later create with it, even after the file is deleted, is 409. The M9 guard
//   rests on that, so a used id is never freed (Drive keeps a trashed file's id too).
// - a successful create is 201; a successful update or copy is 200; a delete is 204.
// - `version` is a per-file counter starting at 1 and growing on every update (content, name,
//   parents, `appProperties`).
// - `createdTime` and the `Date` header come from the fake's clock, which `advance` moves.
// - `files.list` returns children in creation order. Drive promises no order; only a stable
//   one is needed here, because `gdrive::pick` chooses the oldest by `createdTime` then id.
// - `appProperties` on update replaces the whole map, as Drive documents.
// - omitting `parents` on create puts the file in My Drive's root.
// - deleting a folder removes everything below it.
// - `copy` copies a file only; a folder copy is refused, as Drive has no folder copy.
// - a folder cannot move under itself or under its own subtree: `files.update` answers 400 and
//   changes nothing, so no cycle forms and no subtree drops out of My Drive.
// - access tokens are retired only by `expire_access_tokens` or `revoke`; `advance` exists for
//   lease ages, not token expiry.

/// A hook run before the fake handles a request, with the request; it may call `send` on the
/// same fake (the lock is not held), so a test can slip a second machine's push between two
/// steps of the guard.
pub type Hook = Box<dyn FnMut(&Request)>;

/// One file or folder in My Drive. Folders have no bytes and `size` 0.
#[derive(Debug, Clone)]
struct Node {
    id: String,
    name: String,
    mime_type: String,
    is_dir: bool,
    bytes: Vec<u8>,
    parents: Vec<String>,
    version: u64,
    created: u64,
    modified: u64,
    app_properties: BTreeMap<String, String>,
    /// Creation order, so a list is stable even when the clock does not move.
    seq: u64,
}

/// An access token and whether `expire_access_tokens` has retired it.
#[derive(Debug)]
struct IssuedToken {
    expired: bool,
}

/// Where the next page of one `files.list` q starts.
#[derive(Debug, Clone)]
struct Cursor {
    parent: String,
    name: Option<String>,
    offset: usize,
}

struct Inner {
    nodes: BTreeMap<String, Node>,
    /// Every id ever used by a create or copy; Drive does not free an id when its file goes.
    used_ids: BTreeSet<String>,
    requests: Vec<String>,
    tokens: BTreeMap<String, IssuedToken>,
    cursors: BTreeMap<String, Cursor>,
    next_file: u64,
    next_generated: u64,
    next_token: u64,
    next_seq: u64,
    next_cursor: u64,
    clock: u64,
    reachable: bool,
    revoked: bool,
    hook: Option<Hook>,
    hook_busy: bool,
    hook_generation: u64,
}

impl Default for Inner {
    fn default() -> Inner {
        Inner {
            nodes: BTreeMap::new(),
            used_ids: BTreeSet::new(),
            requests: Vec::new(),
            tokens: BTreeMap::new(),
            cursors: BTreeMap::new(),
            next_file: 1,
            next_generated: 1,
            next_token: 1,
            next_seq: 1,
            next_cursor: 1,
            clock: CLOCK_START,
            reachable: true,
            revoked: false,
            hook: None,
            hook_busy: false,
            hook_generation: 0,
        }
    }
}

/// The civil date for a count of days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { y + 1 } else { y }, month as u32, day as u32)
}

/// RFC 3339 UTC with a `Z`, as `createdTime` is sent: `1793476260` ->
/// `"2026-10-31T19:51:00Z"`.
fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        (rest % 3_600) / 60,
        rest % 60
    )
}

/// An HTTP-date for the `Date` header: `1793476260` ->
/// `"Sat, 31 Oct 2026 19:51:00 GMT"`.
fn http_date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    let weekday = WEEKDAYS[(days + 4).rem_euclid(7) as usize];
    let month = MONTHS[(month - 1) as usize];
    format!(
        "{weekday}, {day:02} {month} {year:04} {:02}:{:02}:{:02} GMT",
        rest / 3_600,
        (rest % 3_600) / 60,
        rest % 60
    )
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

/// The part of a Drive URL after `/drive/v3` or `/upload/drive/v3`.
fn drive_route(url: &str) -> Option<&str> {
    let path = url_path(url);
    path.strip_prefix("/drive/v3")
        .or_else(|| path.strip_prefix("/upload/drive/v3"))
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

fn form_fields(text: &str) -> BTreeMap<String, String> {
    text.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (form_decode(key), form_decode(value))
        })
        .collect()
}

fn query_fields(url: &str) -> BTreeMap<String, String> {
    form_fields(url.split_once('?').map(|(_, query)| query).unwrap_or(""))
}

/// The text between one opening `'` and its closing `'`, with `escape_q` undone.
fn read_quoted(text: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut chars = text.char_indices();
    while let Some((index, character)) = chars.next() {
        match character {
            '\\' => out.push(chars.next()?.1),
            '\'' => return Some((out, &text[index + 1..])),
            _ => out.push(character),
        }
    }
    None
}

/// Exactly the two `q` forms `gdrive::children_query` makes:
/// `'<parent>' in parents and trashed = false` and that plus ` and name = '<name>'`.
fn parse_children_query(query: &str) -> Option<(String, Option<String>)> {
    let rest = query.strip_prefix('\'')?;
    let (parent, rest) = read_quoted(rest)?;
    let rest = rest.strip_prefix(" in parents and trashed = false")?;
    if rest.is_empty() {
        return Some((parent, None));
    }
    let rest = rest.strip_prefix(" and name = '")?;
    let (name, rest) = read_quoted(rest)?;
    if rest.is_empty() {
        Some((parent, Some(name)))
    } else {
        None
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

fn boundary_of(request: &Request) -> Option<String> {
    let content_type = request.header_value("Content-Type")?;
    let lower = content_type.to_ascii_lowercase();
    let index = lower.find("boundary=")? + "boundary=".len();
    let rest = content_type[index..].trim_start();
    if let Some(rest) = rest.strip_prefix('"') {
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    } else {
        rest.split(';').next().map(|value| value.trim().to_string())
    }
}

fn split_headers(part: &[u8]) -> (&[u8], &[u8]) {
    if let Some(index) = find_bytes(part, b"\r\n\r\n", 0) {
        (&part[..index], &part[index + 4..])
    } else if let Some(index) = find_bytes(part, b"\n\n", 0) {
        (&part[..index], &part[index + 2..])
    } else {
        (&[], part)
    }
}

fn part_is_json(headers: &[u8]) -> bool {
    String::from_utf8_lossy(headers)
        .to_ascii_lowercase()
        .contains("application/json")
}

type Answer<T> = std::result::Result<T, Response>;

/// The metadata part and the content part of a `multipart/related` upload.
fn parse_multipart(request: &Request) -> Answer<(Value, Vec<u8>)> {
    let boundary = boundary_of(request)
        .ok_or_else(|| bad_request("fake drive: multipart upload needs a boundary"))?;
    let delimiter = format!("--{boundary}").into_bytes();
    let body = &request.body;
    let mut parts: Vec<(bool, Vec<u8>)> = Vec::new();
    let mut position = 0;
    while let Some(start) = find_bytes(body, &delimiter, position) {
        let after = start + delimiter.len();
        if body
            .get(after..)
            .map(|rest| rest.starts_with(b"--"))
            .unwrap_or(false)
        {
            break;
        }
        let mut part_start = after;
        if body
            .get(part_start..)
            .map(|rest| rest.starts_with(b"\r\n"))
            .unwrap_or(false)
        {
            part_start += 2;
        } else if body
            .get(part_start..)
            .map(|rest| rest.starts_with(b"\n"))
            .unwrap_or(false)
        {
            part_start += 1;
        }
        let next = find_bytes(body, &delimiter, part_start).unwrap_or(body.len());
        let mut part = &body[part_start..next];
        if part.ends_with(b"\r\n") {
            part = &part[..part.len() - 2];
        } else if part.ends_with(b"\n") {
            part = &part[..part.len() - 1];
        }
        if !part.is_empty() {
            let (headers, bytes) = split_headers(part);
            let is_json = part_is_json(headers) || bytes.starts_with(b"{");
            parts.push((is_json, bytes.to_vec()));
        }
        position = next;
        if next >= body.len() {
            break;
        }
    }
    if parts.is_empty() {
        return Err(bad_request("fake drive: multipart upload has no parts"));
    }
    let metadata_index = parts.iter().position(|(is_json, _)| *is_json).unwrap_or(0);
    let metadata_text = std::str::from_utf8(&parts[metadata_index].1)
        .map_err(|_| bad_request("fake drive: the metadata part is not UTF-8"))?;
    let metadata: Value = serde_json::from_str(metadata_text)
        .map_err(|_| bad_request("fake drive: the metadata part is not JSON"))?;
    if !metadata.is_object() {
        return Err(bad_request(
            "fake drive: the metadata part is not a JSON object",
        ));
    }
    let mut content = Vec::new();
    for (index, (_, bytes)) in parts.iter().enumerate() {
        if index != metadata_index {
            content = bytes.clone();
            break;
        }
    }
    Ok((metadata, content))
}

fn parse_json_body(request: &Request) -> Answer<Value> {
    if request.body.is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_slice(&request.body)
        .map_err(|_| bad_request("fake drive: the request body is not JSON"))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(bad_request(
            "fake drive: the request body is not a JSON object",
        ))
    }
}

fn parse_parents(value: &Value) -> Answer<Vec<String>> {
    let array = value
        .as_array()
        .ok_or_else(|| bad_request("fake drive: parents must be an array"))?;
    let mut parents: Vec<String> = Vec::new();
    for item in array {
        let parent = item
            .as_str()
            .ok_or_else(|| bad_request("fake drive: a parent must be a string"))?;
        if !parents.iter().any(|known| known.as_str() == parent) {
            parents.push(parent.to_string());
        }
    }
    Ok(parents)
}

fn parse_properties(value: &Value) -> Answer<BTreeMap<String, String>> {
    match value {
        Value::Null => Ok(BTreeMap::new()),
        Value::Object(object) => {
            let mut properties = BTreeMap::new();
            for (key, value) in object {
                // Drive deletes a property whose value is null; the fake simply leaves it out.
                if value.is_null() {
                    continue;
                }
                let text = value.as_str().ok_or_else(|| {
                    bad_request("fake drive: appProperties values must be strings")
                })?;
                properties.insert(key.clone(), text.to_string());
            }
            Ok(properties)
        }
        _ => Err(bad_request("fake drive: appProperties must be an object")),
    }
}

fn node_json(node: &Node) -> Value {
    let mut object = json!({
        "kind": "drive#file",
        "id": &node.id,
        "name": &node.name,
        "mimeType": &node.mime_type,
        "parents": &node.parents,
        "version": node.version.to_string(),
        "createdTime": rfc3339(node.created),
        "modifiedTime": rfc3339(node.modified),
        "size": if node.is_dir { "0".to_string() } else { node.bytes.len().to_string() },
    });
    if !node.app_properties.is_empty() {
        object["appProperties"] = json!(&node.app_properties);
    }
    object
}

fn error_response(status: u16, reason: &str, message: &str) -> Response {
    Response::new(
        status,
        json!({
            "error": {
                "code": status,
                "message": message,
                "errors": [{
                    "message": message,
                    "domain": "global",
                    "reason": reason,
                }],
            }
        })
        .to_string(),
    )
}

fn bad_request(message: &str) -> Response {
    error_response(400, "badRequest", message)
}

fn not_found(id: &str) -> Response {
    error_response(404, "notFound", &format!("File not found: {id}."))
}

fn conflict_id(id: &str) -> Response {
    error_response(
        409,
        "duplicate",
        &format!("A file with the same ID already exists: {id}"),
    )
}

fn unauthorized() -> Response {
    error_response(401, "authError", "Invalid Credentials")
}

fn method_not_allowed() -> Response {
    error_response(405, "methodNotAllowed", "Method not allowed")
}

fn invalid_grant() -> Response {
    Response::new(
        400,
        json!({
            "error": "invalid_grant",
            "error_description": "Token has been expired or revoked."
        })
        .to_string(),
    )
}

fn unsupported_grant() -> Response {
    Response::new(
        400,
        json!({
            "error": "unsupported_grant_type",
            "error_description": "grant_type must be refresh_token"
        })
        .to_string(),
    )
}

impl Inner {
    fn next_file_id(&mut self) -> String {
        loop {
            let id = format!("fake-file-{}", self.next_file);
            self.next_file += 1;
            if !self.used_ids.contains(&id) {
                return id;
            }
        }
    }

    fn next_generated_id(&mut self) -> String {
        loop {
            let id = format!("fake-generated-{}", self.next_generated);
            self.next_generated += 1;
            if !self.used_ids.contains(&id) {
                return id;
            }
        }
    }

    fn issue_token(&mut self) -> String {
        let access = format!("fake-access-{}", self.next_token);
        self.next_token += 1;
        self.tokens
            .insert(access.clone(), IssuedToken { expired: false });
        access
    }

    fn auth_error(&self, request: &Request) -> Option<Response> {
        let token = request
            .header_value("Authorization")
            .and_then(|value| value.strip_prefix("Bearer "));
        let token = match token {
            Some(token) => token,
            None => return Some(unauthorized()),
        };
        if self.revoked {
            return Some(unauthorized());
        }
        match self.tokens.get(token) {
            None => Some(unauthorized()),
            Some(issued) if issued.expired => Some(unauthorized()),
            Some(_) => None,
        }
    }

    fn validate_parents(&self, parents: &[String]) -> Option<Response> {
        for parent in parents {
            if parent == "root" {
                continue;
            }
            match self.nodes.get(parent) {
                Some(node) if node.is_dir => {}
                Some(_) => {
                    return Some(bad_request(&format!(
                        "fake drive: parent is not a folder: {parent}"
                    )));
                }
                None => return Some(not_found(parent)),
            }
        }
        None
    }

    /// True when `id` is `folder` or lies below it, following parents upward from `id`.
    fn is_at_or_below(&self, id: &str, folder: &str) -> bool {
        let mut seen = BTreeSet::new();
        let mut pending = vec![id.to_string()];
        while let Some(current) = pending.pop() {
            if current == folder {
                return true;
            }
            if !seen.insert(current.clone()) {
                continue;
            }
            if let Some(node) = self.nodes.get(&current) {
                pending.extend(node.parents.iter().cloned());
            }
        }
        false
    }

    fn children(&self, parent: &str, name: Option<&str>) -> Vec<String> {
        let mut children: Vec<&Node> = self
            .nodes
            .values()
            .filter(|node| node.parents.iter().any(|known| known == parent))
            .filter(|node| match name {
                Some(want) => node.name == want,
                None => true,
            })
            .collect();
        children.sort_by_key(|node| node.seq);
        children.into_iter().map(|node| node.id.clone()).collect()
    }

    fn handle_token(&mut self, request: &Request) -> Response {
        if self.revoked {
            return invalid_grant();
        }
        let body = String::from_utf8_lossy(&request.body);
        let fields = form_fields(&body);
        if fields.get("grant_type").map(String::as_str) != Some("refresh_token") {
            return unsupported_grant();
        }
        if fields.get("refresh_token").map(String::as_str) != Some(REFRESH_TOKEN) {
            return invalid_grant();
        }
        let access = self.issue_token();
        Response::new(
            200,
            json!({
                "access_token": access,
                "expires_in": TOKEN_LIFETIME,
                "token_type": "Bearer",
            })
            .to_string(),
        )
    }

    fn about(&self) -> Response {
        Response::new(
            200,
            json!({
                "kind": "drive#about",
                "user": {
                    "kind": "drive#user",
                    "displayName": "Fake User",
                    "emailAddress": ACCOUNT_EMAIL,
                    "permissionId": "fake-permission",
                    "me": true,
                }
            })
            .to_string(),
        )
    }

    fn generate_ids(&mut self, request: &Request) -> Response {
        let query = query_fields(&request.url);
        let count = query
            .get("count")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(10)
            .min(1_000);
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            ids.push(self.next_generated_id());
        }
        Response::new(
            200,
            json!({
                "kind": "drive#generatedIds",
                "space": "drive",
                "ids": ids,
            })
            .to_string(),
        )
    }

    fn list(&mut self, request: &Request) -> Response {
        let query = query_fields(&request.url);
        let q = match query.get("q") {
            Some(q) => q.as_str(),
            None => return bad_request("fake drive: files.list needs a q"),
        };
        let (parent, name) = match parse_children_query(q) {
            Some(parts) => parts,
            None => return bad_request(&format!("fake drive: unsupported q: {q}")),
        };
        let mut offset = 0;
        if let Some(token) = query.get("pageToken") {
            match self.cursors.get(token) {
                Some(cursor) if cursor.parent == parent && cursor.name == name => {
                    offset = cursor.offset;
                }
                _ => {
                    return bad_request(&format!("fake drive: invalid pageToken: {token}"));
                }
            }
        }
        let children = self.children(&parent, name.as_deref());
        let page: Vec<String> = children.iter().skip(offset).take(PAGE).cloned().collect();
        let next_offset = offset + page.len();
        let files: Vec<Value> = page
            .iter()
            .map(|id| node_json(self.nodes.get(id).expect("listed child exists")))
            .collect();
        let mut answer = json!({
            "kind": "drive#fileList",
            "files": files,
        });
        if next_offset < children.len() {
            let token = format!("page-{}", self.next_cursor);
            self.next_cursor += 1;
            self.cursors.insert(
                token.clone(),
                Cursor {
                    parent,
                    name,
                    offset: next_offset,
                },
            );
            answer["nextPageToken"] = json!(token);
        }
        Response::new(200, answer.to_string())
    }

    fn create(&mut self, request: &Request) -> Response {
        let content_type = request
            .header_value("Content-Type")
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        let upload_type = query_fields(&request.url)
            .get("uploadType")
            .map(String::as_str)
            .map(str::to_string);
        let multipart =
            content_type.starts_with("multipart/") || upload_type.as_deref() == Some("multipart");
        let (metadata, content) = if multipart {
            match parse_multipart(request) {
                Ok(parts) => parts,
                Err(response) => return response,
            }
        } else {
            match parse_json_body(request) {
                Ok(metadata) => (metadata, Vec::new()),
                Err(response) => return response,
            }
        };
        let id = match metadata.get("id") {
            Some(value) => match value.as_str() {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => return bad_request("fake drive: id must be a non-empty string"),
            },
            None => self.next_file_id(),
        };
        if self.used_ids.contains(&id) {
            return conflict_id(&id);
        }
        let name = match metadata.get("name") {
            Some(value) => match value.as_str() {
                Some(name) => name.to_string(),
                None => return bad_request("fake drive: name must be a string"),
            },
            None => return bad_request("fake drive: a file needs a name"),
        };
        let mime_type = match metadata.get("mimeType") {
            Some(value) => match value.as_str() {
                Some(mime) => mime.to_string(),
                None => return bad_request("fake drive: mimeType must be a string"),
            },
            None => DEFAULT_MIME.to_string(),
        };
        let is_dir = mime_type == FOLDER_MIME;
        let parents = match metadata.get("parents") {
            Some(value) => match parse_parents(value) {
                Ok(parents) => parents,
                Err(response) => return response,
            },
            None => vec!["root".to_string()],
        };
        if let Some(response) = self.validate_parents(&parents) {
            return response;
        }
        let app_properties = match metadata.get("appProperties") {
            Some(value) => match parse_properties(value) {
                Ok(properties) => properties,
                Err(response) => return response,
            },
            None => BTreeMap::new(),
        };
        if is_dir && !content.is_empty() {
            return bad_request("fake drive: a folder cannot have content");
        }
        let node = Node {
            id: id.clone(),
            name,
            mime_type,
            is_dir,
            bytes: if is_dir { Vec::new() } else { content },
            parents,
            version: 1,
            created: self.clock,
            modified: self.clock,
            app_properties,
            seq: self.next_seq,
        };
        self.next_seq += 1;
        self.used_ids.insert(id.clone());
        self.nodes.insert(id.clone(), node);
        Response::new(
            201,
            node_json(self.nodes.get(&id).expect("just created")).to_string(),
        )
    }

    fn get(&self, id: &str, request: &Request) -> Response {
        let node = match self.nodes.get(id) {
            Some(node) => node,
            None => return not_found(id),
        };
        let query = query_fields(&request.url);
        if query.get("alt").map(String::as_str) == Some("media") {
            if node.is_dir {
                return bad_request("fake drive: a folder has no media");
            }
            return Response::new(200, node.bytes.clone()).header("Content-Type", &node.mime_type);
        }
        Response::new(200, node_json(node).to_string())
    }

    fn update(&mut self, id: &str, request: &Request) -> Response {
        let mut node = match self.nodes.get(id) {
            Some(node) => node.clone(),
            None => return not_found(id),
        };
        let content_type = request
            .header_value("Content-Type")
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        let query = query_fields(&request.url);
        let upload_type = query.get("uploadType").map(String::as_str);
        let multipart = content_type.starts_with("multipart/") || upload_type == Some("multipart");
        let (metadata, content) = if multipart {
            match parse_multipart(request) {
                Ok((metadata, content)) => (metadata, Some(content)),
                Err(response) => return response,
            }
        } else if upload_type == Some("media") {
            (json!({}), Some(request.body.clone()))
        } else {
            match parse_json_body(request) {
                Ok(metadata) => (metadata, None),
                Err(response) => return response,
            }
        };
        if let Some(value) = metadata.get("name") {
            match value.as_str() {
                Some(name) => node.name = name.to_string(),
                None => return bad_request("fake drive: name must be a string"),
            }
        }
        if let Some(value) = metadata.get("mimeType") {
            match value.as_str() {
                Some(mime) => {
                    node.mime_type = mime.to_string();
                    node.is_dir = mime == FOLDER_MIME;
                    if node.is_dir {
                        node.bytes.clear();
                    }
                }
                None => return bad_request("fake drive: mimeType must be a string"),
            }
        }
        if let Some(value) = metadata.get("appProperties") {
            match parse_properties(value) {
                Ok(properties) => node.app_properties = properties,
                Err(response) => return response,
            }
        }
        if let Some(value) = metadata.get("parents") {
            match parse_parents(value) {
                Ok(parents) => node.parents = parents,
                Err(response) => return response,
            }
        }
        if let Some(add) = query.get("addParents") {
            for parent in add.split(',').filter(|parent| !parent.is_empty()) {
                if !node.parents.iter().any(|known| known == parent) {
                    node.parents.push(parent.to_string());
                }
            }
        }
        if let Some(remove) = query.get("removeParents") {
            for parent in remove.split(',').filter(|parent| !parent.is_empty()) {
                node.parents.retain(|known| known != parent);
            }
        }
        if let Some(response) = self.validate_parents(&node.parents) {
            return response;
        }
        let inside_itself = node.parents.iter().any(|parent| self.is_at_or_below(parent, id));
        if inside_itself {
            return bad_request("fake drive: a folder cannot move into itself or its subtree");
        }
        if let Some(bytes) = content {
            if node.is_dir {
                return bad_request("fake drive: a folder cannot have content");
            }
            node.bytes = bytes;
        }
        node.version += 1;
        node.modified = self.clock;
        self.nodes.insert(id.to_string(), node);
        Response::new(
            200,
            node_json(self.nodes.get(id).expect("just updated")).to_string(),
        )
    }

    fn copy(&mut self, source_id: &str, request: &Request) -> Response {
        let source = match self.nodes.get(source_id) {
            Some(node) if node.is_dir => {
                return bad_request("fake drive: a folder cannot be copied");
            }
            Some(node) => node.clone(),
            None => return not_found(source_id),
        };
        let metadata = match parse_json_body(request) {
            Ok(metadata) => metadata,
            Err(response) => return response,
        };
        let id = match metadata.get("id") {
            Some(value) => match value.as_str() {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => return bad_request("fake drive: id must be a non-empty string"),
            },
            None => self.next_file_id(),
        };
        if self.used_ids.contains(&id) {
            return conflict_id(&id);
        }
        let name = match metadata.get("name") {
            Some(value) => match value.as_str() {
                Some(name) => name.to_string(),
                None => return bad_request("fake drive: name must be a string"),
            },
            None => source.name.clone(),
        };
        let parents = match metadata.get("parents") {
            Some(value) => match parse_parents(value) {
                Ok(parents) => parents,
                Err(response) => return response,
            },
            None => source.parents.clone(),
        };
        if let Some(response) = self.validate_parents(&parents) {
            return response;
        }
        let app_properties = match metadata.get("appProperties") {
            Some(value) => match parse_properties(value) {
                Ok(properties) => properties,
                Err(response) => return response,
            },
            None => source.app_properties.clone(),
        };
        let node = Node {
            id: id.clone(),
            name,
            mime_type: source.mime_type,
            is_dir: false,
            bytes: source.bytes,
            parents,
            version: 1,
            created: self.clock,
            modified: self.clock,
            app_properties,
            seq: self.next_seq,
        };
        self.next_seq += 1;
        self.used_ids.insert(id.clone());
        self.nodes.insert(id.clone(), node);
        Response::new(
            200,
            node_json(self.nodes.get(&id).expect("just copied")).to_string(),
        )
    }

    fn delete(&mut self, id: &str) -> Response {
        if id == "root" {
            return bad_request("fake drive: the root cannot be deleted");
        }
        if !self.nodes.contains_key(id) {
            return not_found(id);
        }
        let mut doomed = BTreeSet::new();
        doomed.insert(id.to_string());
        loop {
            let children: Vec<String> = self
                .nodes
                .values()
                .filter(|node| {
                    !doomed.contains(&node.id)
                        && node.parents.iter().any(|parent| doomed.contains(parent))
                })
                .map(|node| node.id.clone())
                .collect();
            let before = doomed.len();
            for child in children {
                doomed.insert(child);
            }
            if doomed.len() == before {
                break;
            }
        }
        for id in &doomed {
            self.nodes.remove(id);
        }
        Response::new(204, Vec::<u8>::new())
    }

    fn file_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        let mut visited = BTreeSet::new();
        self.collect_paths("root", "", &mut visited, &mut paths);
        paths.sort();
        paths
    }

    fn collect_paths(
        &self,
        parent: &str,
        prefix: &str,
        visited: &mut BTreeSet<String>,
        out: &mut Vec<String>,
    ) {
        for node in self
            .nodes
            .values()
            .filter(|node| node.parents.iter().any(|known| known == parent))
        {
            if !visited.insert(node.id.clone()) {
                continue;
            }
            let path = if prefix.is_empty() {
                node.name.clone()
            } else {
                format!("{prefix}/{}", node.name)
            };
            if node.is_dir {
                self.collect_paths(&node.id, &path, visited, out);
            } else {
                out.push(path);
            }
            visited.remove(&node.id);
        }
    }

    fn with_date(&self, mut response: Response) -> Response {
        response
            .headers
            .push(("Date".to_string(), http_date(self.clock)));
        response
    }

    fn handle(&mut self, request: &Request) -> Response {
        let base = request.url.split('?').next().unwrap_or("");
        if base == TOKEN_URL {
            return self.handle_token(request);
        }
        if let Some(response) = self.auth_error(request) {
            return response;
        }
        let route = match drive_route(&request.url) {
            Some(route) => route,
            None => return not_found("the request"),
        };
        let method = request.method.as_str();
        if route == "/files/generateIds" {
            if method == "GET" || method == "POST" {
                return self.generate_ids(request);
            }
            return method_not_allowed();
        }
        if route == "/about" {
            if method == "GET" {
                return self.about();
            }
            return method_not_allowed();
        }
        if route == "/files" {
            return match method {
                "GET" => self.list(request),
                "POST" => self.create(request),
                _ => method_not_allowed(),
            };
        }
        if let Some(rest) = route.strip_prefix("/files/") {
            if let Some(id) = rest.strip_suffix("/copy") {
                if method == "POST" {
                    return self.copy(id, request);
                }
                return method_not_allowed();
            }
            if rest.is_empty() || rest.contains('/') {
                return not_found(rest);
            }
            return match method {
                "GET" => self.get(rest, request),
                "PATCH" | "PUT" => self.update(rest, request),
                "DELETE" => self.delete(rest),
                _ => method_not_allowed(),
            };
        }
        not_found(route)
    }
}

/// Answers `files.list` (the `q` forms `gdrive::children_query` makes; pages of 2),
/// `files.get` (metadata and `alt=media`), `files.create` (with a client id from
/// `files.generateIds`: 409 when that id is in use, as Drive does), `files.update` (content,
/// name, `addParents`/`removeParents`, `appProperties`), `files.copy`, `files.delete`,
/// `files.generateIds`, `about`, and the Google token endpoint (refresh). Files have ids,
/// parents, `version` (grows on every change), `createdTime` from its own clock, and may share
/// a name, as on Drive. Every answer carries a `Date` header from that clock.
/// Interior mutability (a `Mutex`).
#[derive(Default)]
pub struct FakeDrive {
    inner: Mutex<Inner>,
}

impl FakeDrive {
    /// Its clock starts at `stamp` 1793476260 (2026-10-31T19:51:00Z).
    pub fn new() -> FakeDrive {
        FakeDrive::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("fake drive lock poisoned")
    }

    /// While `false`, `send` returns `Error::Unreachable("fake drive is offline")`.
    pub fn set_reachable(&self, reachable: bool) {
        self.lock().reachable = reachable;
    }

    /// Move its clock forward (lease expiry tests).
    pub fn advance(&self, secs: u64) {
        let mut inner = self.lock();
        inner.clock = inner.clock.saturating_add(secs);
    }

    /// From now on the token endpoint answers `invalid_grant` and every access token is refused.
    pub fn revoke(&self) {
        self.lock().revoked = true;
    }

    /// Every access token handed out so far gets 401.
    pub fn expire_access_tokens(&self) {
        for token in self.lock().tokens.values_mut() {
            token.expired = true;
        }
    }

    /// Run `hook` before each request until it is replaced or cleared with `None`.
    pub fn set_hook(&self, hook: Option<Hook>) {
        let mut inner = self.lock();
        inner.hook = hook;
        inner.hook_generation = inner.hook_generation.wrapping_add(1);
    }

    /// Every file (not folder) as a `/`-joined path of names from My Drive, sorted; files
    /// with the same path appear once per copy. Works while unreachable.
    pub fn file_paths(&self) -> Vec<String> {
        self.lock().file_paths()
    }

    /// `"<METHOD> <url>"` of every request so far, in order (no headers, no tokens).
    pub fn requests(&self) -> Vec<String> {
        self.lock().requests.clone()
    }
}

impl Transport for FakeDrive {
    fn send(&self, request: &Request) -> Result<Response> {
        {
            let mut inner = self.lock();
            inner
                .requests
                .push(format!("{} {}", request.method, request.url));
            if !inner.reachable {
                return Err(Error::Unreachable("fake drive is offline".to_string()));
            }
        }

        // Take the hook out while it runs: a re-entrant `send` from the hook must not run it
        // again (that would recurse), and the lock must not be held across the call.
        let hook = {
            let mut inner = self.lock();
            if inner.hook_busy {
                None
            } else {
                match inner.hook.take() {
                    Some(hook) => {
                        inner.hook_busy = true;
                        Some((hook, inner.hook_generation))
                    }
                    None => None,
                }
            }
        };
        if let Some((mut hook, generation)) = hook {
            hook(request);
            let mut inner = self.lock();
            inner.hook_busy = false;
            if inner.hook_generation == generation && inner.hook.is_none() {
                inner.hook = Some(hook);
            }
        }

        let mut inner = self.lock();
        let response = inner.handle(request);
        Ok(inner.with_date(response))
    }
}
