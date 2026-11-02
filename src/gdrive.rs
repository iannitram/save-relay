//! Google Drive (REST API v3) as a `Remote`, with `drive.file` access only: save-relay sees the
//! files it created and nothing else in the Drive (SPEC.md M9, M10;
//! docs/adr/0002-storage-backends.md). Its folder (`<root>`, default `save-relay`) is a normal
//! folder in My Drive that the person can open.
//!
//! Drive is not path-based: every file has an id and parents, and two files may share a name.
//! `Drive` resolves a `Remote` path one segment at a time from `root` (Drive's alias for My
//! Drive); when two children share a name, the oldest
//! (`createdTime`, then id) is the one (`pick`), everywhere, on every machine.
//!
//! Drive v3 has no `If-Match` on an upload, so a `put` with `Precondition::Match` goes through
//! the guard in `guard` (a claim file per state version) to keep M9's "a push never replaces
//! saves another machine pushed meanwhile". A file's etag is `guard::make_etag(version, claim)`.

pub mod guard;

use crate::error::{Error, Result};
use crate::http::{Request, Response, Transport};
use crate::oauth::TokenSource;
use crate::remote::{Precondition, Remote, RemoteEntry};
use crate::stamp::Clock;
use crate::webdav::encode_segment;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// Metadata endpoints (`files`, `files/<id>`, `files/generateIds`, `about`).
pub const API: &str = "https://www.googleapis.com/drive/v3";
/// Content upload endpoint (`files?uploadType=multipart`, `files/<id>?uploadType=multipart`).
pub const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3";
/// A folder's `mimeType`.
pub const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
/// The `fields` every list asks for.
pub const LIST_FIELDS: &str =
    "nextPageToken,files(id,name,mimeType,size,version,createdTime,appProperties)";

/// One Drive file or folder, as save-relay needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    /// `mimeType` is `FOLDER_MIME`.
    pub is_dir: bool,
    /// Bytes (`size`, sent as a string); 0 for a folder or when absent.
    pub size: u64,
    /// Drive's change counter (`version`, sent as a string); grows on every change.
    pub version: u64,
    /// `createdTime`, RFC 3339 as Drive sends it (sorting the text sorts by time).
    pub created: String,
    /// `appProperties[guard::CLAIM_PROPERTY]`: the claim id for the next write (`guard`).
    pub claim: Option<String>,
}

impl DriveFile {
    /// As a `Remote` listing row: name, `is_dir`, size, and etag
    /// `guard::make_etag(version, claim or "")` for a file, `None` for a folder.
    pub fn entry(&self) -> RemoteEntry {
        RemoteEntry {
            name: self.name.clone(),
            is_dir: self.is_dir,
            size: if self.is_dir { 0 } else { self.size },
            etag: (!self.is_dir)
                .then(|| guard::make_etag(self.version, self.claim.as_deref().unwrap_or(""))),
        }
    }
}

/// Quote text for a Drive query string literal: `\` -> `\\`, `'` -> `\'`.
/// `"it's"` -> `"it\'s"`.
pub fn escape_q(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\'', "\\'")
}

/// The `q` for the children of folder `parent_id`, optionally with one name:
/// `"'<parent>' in parents and trashed = false"`, plus `" and name = '<name>'"`, both through
/// `escape_q`.
pub fn children_query(parent_id: &str, name: Option<&str>) -> String {
    let mut query = format!("'{}' in parents and trashed = false", escape_q(parent_id));
    if let Some(name) = name {
        query.push_str(&format!(" and name = '{}'", escape_q(name)));
    }
    query
}

/// One page of `files.list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePage {
    /// In the order Drive sent them.
    pub files: Vec<DriveFile>,
    pub next_page: Option<String>,
}

/// A non-empty run of ASCII digits: `size` and `version` as Drive writes them (no sign).
fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn parse_drive_file(value: &serde_json::Value) -> Option<DriveFile> {
    fn number(value: Option<&serde_json::Value>) -> Option<u64> {
        match value {
            None => Some(0),
            Some(serde_json::Value::String(s)) if is_digits(s) => s.parse().ok(),
            Some(serde_json::Value::Number(n)) => n.as_u64(),
            _ => None,
        }
    }

    let object = value.as_object()?;
    let id = object.get("id")?.as_str()?.to_owned();
    let name = object.get("name")?.as_str()?.to_owned();
    let size = number(object.get("size"))?;
    let version = number(object.get("version"))?;
    let mime_type = object.get("mimeType").and_then(serde_json::Value::as_str);
    let created = object
        .get("createdTime")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned();
    let claim = object
        .get("appProperties")
        .and_then(serde_json::Value::as_object)
        .and_then(|properties| properties.get(guard::CLAIM_PROPERTY))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Some(DriveFile {
        id,
        name,
        is_dir: mime_type == Some(FOLDER_MIME),
        size,
        version,
        created,
        claim,
    })
}

/// Parse a `files.list` answer (`files`, `nextPageToken`). Not JSON, a file without `id` or
/// `name`, or a `version`/`size` that is not a whole number is
/// `Error::Server("drive: bad list answer")`.
pub fn parse_files(body: &str) -> Result<FilePage> {
    fn bad() -> crate::Error {
        crate::Error::Server("drive: bad list answer".into())
    }

    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| bad())?;
    let object = value.as_object().ok_or_else(bad)?;
    let files = match object.get("files") {
        None => Vec::new(),
        Some(value) => value
            .as_array()
            .ok_or_else(bad)?
            .iter()
            .map(|file| parse_drive_file(file).ok_or_else(bad))
            .collect::<Result<Vec<_>>>()?,
    };
    let next_page = object
        .get("nextPageToken")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Ok(FilePage { files, next_page })
}

/// Parse one file's metadata (the answer of `files.get`, `files.create`, `files.update`),
/// with the same rules as one entry of `parse_files`; else `Error::Server("drive: bad file answer")`.
pub fn parse_file(body: &str) -> Result<DriveFile> {
    fn bad() -> crate::Error {
        crate::Error::Server("drive: bad file answer".into())
    }

    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| bad())?;
    parse_drive_file(&value).ok_or_else(bad)
}

/// The file called `name` among `files`: the oldest by `created`, then by `id` when two have
/// the same time. `None` when no file has that name.
pub fn pick<'a>(files: &'a [DriveFile], name: &str) -> Option<&'a DriveFile> {
    files
        .iter()
        .filter(|file| file.name == name)
        .min_by(|a, b| (&a.created, &a.id).cmp(&(&b.created, &b.id)))
}

/// A `multipart/related` upload body: the metadata JSON part, then the content part
/// (`application/octet-stream`), with `boundary`; each part's headers then a blank line, CRLF
/// line ends, closed by `--<boundary>--`.
pub fn multipart_body(metadata: &serde_json::Value, bytes: &[u8], boundary: &str) -> Vec<u8> {
    let mut body = format!(
        "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n\
         --{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// The real Google Drive. `put` with `Any` updates or creates; with `Absent` creates, then
/// lists the name and keeps only the oldest (`pick`), deleting its own copy and returning
/// `Error::Changed` if another machine's is older; with `Match` runs the guard. `move_to` is
/// one metadata update (new parent and name), so a folder appears in `history/` whole.
/// `copy` of a folder copies file by file (Drive copies files only): not atomic, which is why
/// callers copy into `incoming/` and then `move_to` (docs/adr/0002). Retries 429, 5xx and
/// 403 `rateLimitExceeded` with backoff; refreshes the token once on 401. Never logs a token.
pub struct Drive {
    tokens: TokenSource,
    transport: Arc<dyn Transport>,
    clock: Clock,
    wait: Arc<dyn Fn(u64)>,
}

const FILE_FIELDS: &str = "id,name,mimeType,size,version,createdTime,appProperties";

fn split_path(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

fn root() -> DriveFile {
    DriveFile {
        id: "root".into(),
        name: String::new(),
        is_dir: true,
        size: 0,
        version: 0,
        created: String::new(),
        claim: None,
    }
}

// Drive timestamps may include fractional seconds. Lease ages use whole server seconds.
fn created_secs(text: &str) -> Option<u64> {
    if let Some((seconds, fraction)) = text.split_once('.') {
        if !fraction.ends_with('Z') {
            return None;
        }
        crate::stamp::parse_rfc3339(&format!("{seconds}Z"))
    } else {
        crate::stamp::parse_rfc3339(text)
    }
}

fn server_secs(response: &Response) -> Option<u64> {
    let fields: Vec<_> = response.header_value("Date")?.split_whitespace().collect();
    if fields.len() != 6 || fields[5] != "GMT" {
        return None;
    }
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|month| *month == fields[2])?
        + 1;
    crate::stamp::parse_rfc3339(&format!(
        "{}-{month:02}-{}T{}Z",
        fields[3], fields[1], fields[4]
    ))
}

fn is_claim(file: &DriveFile) -> bool {
    file.name == format!(".claim-{}", file.id)
}

fn too_slow() -> Error {
    Error::Unreachable("drive: the write took too long".into())
}

impl Drive {
    pub fn new(tokens: TokenSource, transport: Arc<dyn Transport>, clock: Clock) -> Drive {
        Drive {
            tokens,
            transport,
            clock,
            wait: Arc::new(|seconds| std::thread::sleep(Duration::from_secs(seconds))),
        }
    }

    /// Replace retry waits (seconds); tests can record them without sleeping.
    pub fn with_wait(mut self, wait: Arc<dyn Fn(u64)>) -> Drive {
        self.wait = wait;
        self
    }

    fn send(&self, request: Request, path: &str, held_since: Option<u64>) -> Result<Response> {
        self.send_inner(request, path, held_since, false)
    }

    // Keep raw 409 only for the claim step. Never include response bodies in errors: an
    // API or proxy can echo credentials. Recheck a guarded upload's deadline on every retry.
    fn send_inner(
        &self,
        request: Request,
        path: &str,
        held_since: Option<u64>,
        claiming: bool,
    ) -> Result<Response> {
        let mut refreshed = false;
        let mut retries = 0;
        loop {
            let token = self.tokens.bearer()?;
            if held_since
                .is_some_and(|start| (self.clock)().saturating_sub(start) >= guard::SLOW_SECS)
            {
                return Err(too_slow());
            }
            let response = self.transport.send(
                &request
                    .clone()
                    .header("Authorization", &format!("Bearer {}", token.expose())),
            )?;
            if response.status == 401 {
                if refreshed {
                    return Err(Error::ReauthNeeded("Google Drive".into()));
                }
                self.tokens.invalidate();
                refreshed = true;
                continue;
            }
            let rate_limited = response.status == 403
                && serde_json::from_slice::<Value>(&response.body)
                    .ok()
                    .and_then(|body| body["error"]["errors"].as_array().cloned())
                    .is_some_and(|errors| {
                        errors.iter().any(|error| {
                            matches!(
                                error["reason"].as_str(),
                                Some("rateLimitExceeded" | "userRateLimitExceeded")
                            )
                        })
                    });
            if response.status == 429
                || response.status >= 500 && response.status <= 599
                || rate_limited
            {
                if retries == 3 {
                    return Err(Error::Server(format!("drive: HTTP {}", response.status)));
                }
                let seconds = response
                    .header_value("Retry-After")
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(1 << retries)
                    .min(60);
                (self.wait)(seconds);
                retries += 1;
                continue;
            }
            return match response.status {
                200..=299 => Ok(response),
                409 if claiming => Ok(response),
                404 => Err(Error::NotFound(path.into())),
                status => Err(Error::Server(format!("drive: HTTP {status}"))),
            };
        }
    }

    fn json(&self, method: &str, url: &str, body: &Value, path: &str) -> Result<Response> {
        self.send(
            Request::new(method, url)
                .header("Content-Type", "application/json")
                .body(body.to_string().into_bytes()),
            path,
            None,
        )
    }

    fn metadata(&self, id: &str, path: &str) -> Result<DriveFile> {
        let response = self.send(
            Request::new(
                "GET",
                &format!("{API}/files/{}?fields={FILE_FIELDS}", encode_segment(id)),
            ),
            path,
            None,
        )?;
        parse_file(&response.text())
    }

    fn children(&self, parent: &str, name: Option<&str>) -> Result<Vec<DriveFile>> {
        let base = format!(
            "{API}/files?q={}&fields={LIST_FIELDS}&spaces=drive",
            encode_segment(&children_query(parent, name))
        );
        let mut url = base.clone();
        let mut files = Vec::new();
        loop {
            let response = self.send(Request::new("GET", &url), parent, None)?;
            let page = parse_files(&response.text())?;
            files.extend(page.files);
            match page.next_page {
                Some(next) => url = format!("{base}&pageToken={}", encode_segment(&next)),
                None => return Ok(files),
            }
        }
    }

    // Resolve afresh so renames, deletion and concurrently-created duplicate folders cannot
    // leave a machine using a different tree. All callers use the same oldest-wins rule.
    fn resolve(&self, path: &str) -> Result<DriveFile> {
        let mut current = root();
        if !path.is_empty() {
            for part in path.split('/') {
                if !current.is_dir {
                    return Err(Error::NotFound(path.into()));
                }
                current = pick(&self.children(&current.id, Some(part))?, part)
                    .cloned()
                    .ok_or_else(|| Error::NotFound(path.into()))?;
            }
        }
        Ok(current)
    }

    fn folder(&self, path: &str) -> Result<DriveFile> {
        let file = self.resolve(path)?;
        if file.is_dir {
            Ok(file)
        } else {
            Err(Error::NotFound(path.into()))
        }
    }

    fn generate_id(&self) -> Result<String> {
        let response = self.send(
            Request::new(
                "GET",
                &format!("{API}/files/generateIds?count=1&space=drive&type=files"),
            ),
            "",
            None,
        )?;
        serde_json::from_slice::<Value>(&response.body)
            .ok()
            .and_then(|value| value["ids"][0].as_str().map(str::to_owned))
            .filter(|id| !id.is_empty())
            .ok_or_else(|| Error::Server("drive: bad generated id answer".into()))
    }

    fn upload(
        &self,
        file: Option<&DriveFile>,
        metadata: &Value,
        bytes: &[u8],
        held_since: Option<u64>,
    ) -> Result<(DriveFile, Option<u64>)> {
        // A generated id is also a unique boundary; extend it if content contains it.
        let mut boundary = format!(
            "save-relay-{}",
            metadata["appProperties"][guard::CLAIM_PROPERTY]
                .as_str()
                .unwrap_or("upload")
        );
        while bytes
            .windows(boundary.len())
            .any(|part| part == boundary.as_bytes())
            || metadata.to_string().contains(&boundary)
        {
            boundary.push('x');
        }
        let (method, endpoint) = match file {
            Some(file) => ("PATCH", format!("files/{}", encode_segment(&file.id))),
            None => ("POST", "files".into()),
        };
        let response = self.send(
            Request::new(
                method,
                &format!("{UPLOAD}/{endpoint}?uploadType=multipart&fields={FILE_FIELDS}"),
            )
            .header(
                "Content-Type",
                &format!("multipart/related; boundary={boundary}"),
            )
            .body(multipart_body(metadata, bytes, &boundary)),
            file.map_or("", |file| file.name.as_str()),
            held_since,
        )?;
        Ok((parse_file(&response.text())?, server_secs(&response)))
    }

    fn guarded_put(
        &self,
        file: &DriveFile,
        parent: &str,
        bytes: &[u8],
        etag: &str,
    ) -> Result<String> {
        let (expected, mut claim_id) = guard::parse_etag(etag).ok_or(Error::Changed)?;
        if claim_id.is_empty() {
            return Err(Error::Changed);
        }
        let initial = self.metadata(&file.id, &file.name)?;
        if initial.version != expected || initial.claim.as_deref() != Some(&claim_id) {
            return Err(Error::Changed);
        }
        let mut hops = 0;
        loop {
            let next = self.generate_id()?;
            let fresh = self.generate_id()?;
            let started = (self.clock)();
            let response = self.send_inner(
                Request::new("POST", &format!("{API}/files?fields={FILE_FIELDS}"))
                    .header("Content-Type", "application/json")
                    .body(
                        json!({"id": claim_id, "name": format!(".claim-{claim_id}"),
                        "parents": [parent], "appProperties": {guard::NEXT_PROPERTY: next}})
                        .to_string()
                        .into_bytes(),
                    ),
                &file.name,
                None,
                true,
            )?;
            let claim = if response.status == 409 {
                let taken = self
                    .send(
                        Request::new(
                            "GET",
                            &format!(
                                "{API}/files/{}?fields={FILE_FIELDS}",
                                encode_segment(&claim_id)
                            ),
                        ),
                        &file.name,
                        None,
                    )
                    .map_err(|error| match error {
                        Error::NotFound(_) => Error::Changed,
                        other => other,
                    })?;
                let value: Value = serde_json::from_slice(&taken.body)
                    .map_err(|_| Error::Server("drive: bad claim answer".into()))?;
                let created = value["createdTime"].as_str().and_then(created_secs);
                let age_secs = server_secs(&taken)
                    .zip(created)
                    .map(|(now, created)| now.saturating_sub(created))
                    .ok_or_else(|| Error::Server("drive: missing claim time".into()))?;
                guard::Claim::Taken {
                    age_secs,
                    next: value["appProperties"][guard::NEXT_PROPERTY]
                        .as_str()
                        .map(str::to_owned),
                }
            } else {
                guard::Claim::Won
            };
            let current = self.metadata(&file.id, &file.name)?;
            match guard::decide(
                expected,
                current.version,
                &claim,
                (self.clock)().saturating_sub(started),
                hops,
            ) {
                guard::Step::Changed => return Err(Error::Changed),
                guard::Step::TooSlow => return Err(too_slow()),
                guard::Step::Break(next) => {
                    claim_id = next;
                    hops += 1;
                }
                guard::Step::Write => {
                    let (written, now) = self.upload(
                        Some(file),
                        &json!({"appProperties": {guard::CLAIM_PROPERTY: fresh}}),
                        bytes,
                        Some(started),
                    )?;
                    let seen = self.metadata(&file.id, &file.name)?;
                    guard::check_written(
                        written.version,
                        &fresh,
                        seen.version,
                        seen.claim.as_deref(),
                    )?;
                    self.clean_claims(parent, now)?;
                    return Ok(guard::make_etag(written.version, &fresh));
                }
            }
        }
    }

    fn clean_claims(&self, parent: &str, now: Option<u64>) -> Result<()> {
        let Some(now) = now else {
            return Ok(());
        };
        for file in self.children(parent, None)? {
            if is_claim(&file)
                && created_secs(&file.created)
                    .is_some_and(|created| now.saturating_sub(created) >= 86400)
            {
                self.send(
                    Request::new(
                        "DELETE",
                        &format!("{API}/files/{}", encode_segment(&file.id)),
                    ),
                    &file.name,
                    None,
                )?;
            }
        }
        Ok(())
    }

    fn copy_file(&self, file: &DriveFile, parent: &str, name: &str) -> Result<()> {
        if file.is_dir {
            let response = self.json(
                "POST",
                &format!("{API}/files?fields={FILE_FIELDS}"),
                &json!({"name": name, "mimeType": FOLDER_MIME, "parents": [parent]}),
                name,
            )?;
            let folder = parse_file(&response.text())?;
            for child in oldest(self.children(&file.id, None)?)
                .into_iter()
                .filter(|file| !is_claim(file))
            {
                self.copy_file(&child, &folder.id, &child.name)?;
            }
        } else {
            let fresh = self.generate_id()?;
            self.json("POST", &format!("{API}/files/{}/copy", encode_segment(&file.id)),
                &json!({"name": name, "parents": [parent], "appProperties": {guard::CLAIM_PROPERTY: fresh}}), name)?;
        }
        Ok(())
    }

    fn destination(&self, to: &str) -> Result<(DriveFile, String)> {
        if to.is_empty() {
            return Err(Error::Server("drive: destination exists".into()));
        }
        let (parent, name) = split_path(to);
        let parent = self.folder(parent)?;
        if pick(&self.children(&parent.id, Some(name))?, name).is_some() {
            return Err(Error::Server("drive: destination exists".into()));
        }
        Ok((parent, name.into()))
    }

    /// The signed-in account's e-mail address (`about?fields=user`), for the config and
    /// `status`.
    pub fn account_email(&self) -> Result<String> {
        let response = self.send(
            Request::new("GET", &format!("{API}/about?fields=user")),
            "",
            None,
        )?;
        serde_json::from_slice::<Value>(&response.body)
            .ok()
            .and_then(|value| value["user"]["emailAddress"].as_str().map(str::to_owned))
            .ok_or_else(|| Error::Server("drive: bad account answer".into()))
    }
}

fn oldest(files: Vec<DriveFile>) -> Vec<DriveFile> {
    let mut by_name = BTreeMap::<String, DriveFile>::new();
    for file in files {
        let current = by_name
            .entry(file.name.clone())
            .or_insert_with(|| file.clone());
        if (&file.created, &file.id) < (&current.created, &current.id) {
            *current = file;
        }
    }
    by_name.into_values().collect()
}

impl Remote for Drive {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        let folder = self.folder(dir)?;
        Ok(oldest(self.children(&folder.id, None)?)
            .iter()
            .filter(|file| !is_claim(file))
            .map(DriveFile::entry)
            .collect())
    }

    fn get(&self, path: &str) -> Result<(Vec<u8>, String)> {
        let file = self.resolve(path)?;
        if file.is_dir {
            return Err(Error::NotFound(path.into()));
        }
        let response = self.send(
            Request::new(
                "GET",
                &format!("{API}/files/{}?alt=media", encode_segment(&file.id)),
            ),
            path,
            None,
        )?;
        // A concurrent writer must not make us return newer bytes with an older etag.
        let seen = self.metadata(&file.id, path)?;
        if file.version != seen.version || file.claim != seen.claim {
            return Err(Error::Changed);
        }
        Ok((
            response.body,
            guard::make_etag(file.version, file.claim.as_deref().unwrap_or("")),
        ))
    }

    fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> Result<String> {
        if path.is_empty() {
            return Err(Error::Server("drive: cannot write the files root".into()));
        }
        let (parent, name) = split_path(path);
        let parent = self.folder(parent)?;
        let existing = pick(&self.children(&parent.id, Some(name))?, name).cloned();
        match pre {
            Precondition::Absent if existing.is_some() => return Err(Error::Changed),
            Precondition::Match(etag) => {
                let file = existing.filter(|file| !file.is_dir).ok_or(Error::Changed)?;
                return self.guarded_put(&file, &parent.id, bytes, etag);
            }
            _ => {}
        }
        if existing.as_ref().is_some_and(|file| file.is_dir) {
            return Err(Error::Server("drive: a folder is there".into()));
        }
        let fresh = self.generate_id()?;
        let mut metadata = json!({"appProperties": {guard::CLAIM_PROPERTY: fresh}});
        if existing.is_none() {
            metadata["name"] = json!(name);
            metadata["parents"] = json!([parent.id]);
        }
        let (written, now) = self.upload(existing.as_ref(), &metadata, bytes, None)?;
        if existing.is_none() {
            let files = self.children(&parent.id, Some(name))?;
            if pick(&files, name).map(|file| &file.id) != Some(&written.id) {
                self.send(
                    Request::new(
                        "DELETE",
                        &format!("{API}/files/{}", encode_segment(&written.id)),
                    ),
                    path,
                    None,
                )?;
                return Err(Error::Changed);
            }
        }
        self.clean_claims(&parent.id, now)?;
        Ok(guard::make_etag(written.version, &fresh))
    }

    fn mkcol(&self, dir: &str) -> Result<()> {
        if dir.is_empty() {
            // Even an idempotent root operation observes an offline transport.
            self.list("")?;
            return Ok(());
        }
        let (parent, name) = split_path(dir);
        let parent = self.folder(parent)?;
        if let Some(file) = pick(&self.children(&parent.id, Some(name))?, name) {
            return if file.is_dir {
                Ok(())
            } else {
                Err(Error::Server("drive: a file is there".into()))
            };
        }
        self.json(
            "POST",
            &format!("{API}/files"),
            &json!({"name": name, "mimeType": FOLDER_MIME, "parents": [parent.id]}),
            dir,
        )?;
        Ok(())
    }

    fn copy(&self, from: &str, to: &str) -> Result<()> {
        let file = self.resolve(from)?;
        if file.is_dir && (to == from || to.starts_with(&format!("{from}/")) || from.is_empty()) {
            return Err(Error::Server(
                "drive: cannot copy a folder into itself".into(),
            ));
        }
        let (parent, name) = self.destination(to)?;
        self.copy_file(&file, &parent.id, &name)
    }

    fn move_to(&self, from: &str, to: &str) -> Result<()> {
        let file = self.resolve(from)?;
        if file.is_dir && (to == from || to.starts_with(&format!("{from}/")) || from.is_empty()) {
            return Err(Error::Server(
                "drive: cannot move a folder into itself".into(),
            ));
        }
        let old_parent = self.folder(split_path(from).0)?;
        let (parent, name) = self.destination(to)?;
        // Drive rejects adding and removing the same parent. A rename needs only a name.
        let parents = if parent.id == old_parent.id {
            String::new()
        } else {
            format!(
                "?addParents={}&removeParents={}",
                encode_segment(&parent.id),
                encode_segment(&old_parent.id)
            )
        };
        self.json(
            "PATCH",
            &format!("{API}/files/{}{parents}", encode_segment(&file.id)),
            &json!({"name": name}),
            from,
        )?;
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<()> {
        let file = self.resolve(path)?;
        if path.is_empty() {
            for child in self.children("root", None)? {
                self.send(
                    Request::new(
                        "DELETE",
                        &format!("{API}/files/{}", encode_segment(&child.id)),
                    ),
                    path,
                    None,
                )?;
            }
        } else {
            self.send(
                Request::new(
                    "DELETE",
                    &format!("{API}/files/{}", encode_segment(&file.id)),
                ),
                path,
                None,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::parse_file;

    #[test]
    fn size_and_version_are_digits_only() {
        assert!(parse_file(r#"{"id":"a","name":"a","size":"+5"}"#).is_err());
        assert!(parse_file(r#"{"id":"a","name":"a","version":"+5"}"#).is_err());
        assert!(parse_file(r#"{"id":"a","name":"a","size":""}"#).is_err());
        assert!(parse_file(r#"{"id":"a","name":"a","size":"5"}"#).is_ok());
    }
}
