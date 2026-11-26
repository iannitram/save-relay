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
use crate::gamefiles::{GameSource, RangeAnswer};
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
    let claim_key = if name == format!(".claim-{id}") {
        guard::NEXT_PROPERTY
    } else {
        guard::CLAIM_PROPERTY
    };
    let claim = object
        .get("appProperties")
        .and_then(serde_json::Value::as_object)
        .and_then(|properties| properties.get(claim_key))
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

/// Retries (with waits) of one save-sync request after a 429 or 5xx.
const SYNC_RETRIES: u32 = 3;

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
        self.send_inner(request, path, held_since, &[], SYNC_RETRIES)
    }

    /// A game-file request: no retries or waits in here. The engine owns the retry budget and
    /// waits in stop-aware slices, so a cancel is never stuck behind this loop's sleeps and the
    /// budgets do not multiply. A 429/5xx comes back at once (with its `Retry-After`).
    fn send_game(&self, request: Request, path: &str, pass: &[u16]) -> Result<Response> {
        self.send_inner(request, path, None, pass, 0)
    }

    // `pass` lists statuses handed back raw: 409 for the claim step, 416 for a game-file range.
    // Never include response bodies in errors: an API or proxy can echo credentials. Recheck a
    // guarded upload's deadline on every retry.
    fn send_inner(
        &self,
        request: Request,
        path: &str,
        held_since: Option<u64>,
        pass: &[u16],
        max_retries: u32,
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
            let reasons = if response.status == 403 {
                error_reasons(&response.body)
            } else {
                Vec::new()
            };
            let rate_limited = reasons.iter().any(|reason| {
                matches!(
                    reason.as_str(),
                    "rateLimitExceeded" | "userRateLimitExceeded"
                )
            });
            if response.status == 429
                || response.status >= 500 && response.status <= 599
                || rate_limited
            {
                if retries >= max_retries {
                    // A rate-limited 403 that outlasts the retries is still a rate limit:
                    // say 429, so it stays retryable and is never read as a refusal.
                    let message = if rate_limited {
                        "drive: HTTP 429 (rate limited)".to_string()
                    } else {
                        format!("drive: HTTP {}", response.status)
                    };
                    return Err(
                        match response
                            .header_value("Retry-After")
                            .and_then(|value| value.trim().parse::<u64>().ok())
                        {
                            Some(seconds) => Error::RateLimited {
                                message,
                                retry_after_secs: Some(seconds),
                            },
                            None => Error::Server(message),
                        },
                    );
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
                status if pass.contains(&status) => Ok(response),
                404 => Err(Error::NotFound(path.into())),
                403 => Err(Error::Server(match reasons.first() {
                    Some(reason) => format!("drive: HTTP 403 ({reason})"),
                    None => "drive: HTTP 403".into(),
                })),
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
        path: &str,
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
                &[409],
                SYNC_RETRIES,
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
                        Error::NotFound(_) => match self.metadata(&file.id, path) {
                            Ok(current)
                                if current.version == expected
                                    && current.claim.as_deref() == Some(&claim_id) =>
                            {
                                Error::MissingDriveClaim {
                                    path: path.to_string(),
                                    file_id: file.id.clone(),
                                    version: expected,
                                    claim: claim_id.clone(),
                                }
                            }
                            Ok(_) | Err(Error::NotFound(_)) => Error::Changed,
                            Err(other) => other,
                        },
                        other => other,
                    })?;
                let value: Value = serde_json::from_slice(&taken.body)
                    .map_err(|_| Error::Server("drive: bad claim answer".into()))?;
                // The 409 can be our own claim: an earlier attempt committed it and answered
                // 5xx, and this retry met it. `next` is a fresh id this attempt generated.
                if value["appProperties"][guard::NEXT_PROPERTY].as_str() == Some(next.as_str()) {
                    guard::Claim::Won
                } else {
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
        let files = self.children(parent, None)?;
        // `state.json`'s etag names the first claim in a guard chain. Keep every old claim
        // reachable through NEXT_PROPERTY: deleting any active id consumes it permanently and
        // a later conditional write can no longer follow the chain.
        let mut active_claims = std::collections::BTreeSet::new();
        let mut next_claim = files
            .iter()
            .find(|file| file.name == "state.json" && !file.is_dir)
            .and_then(|file| file.claim.clone());
        for _ in 0..=guard::MAX_HOPS {
            let Some(claim_id) = next_claim else {
                break;
            };
            if !active_claims.insert(claim_id.clone()) {
                break;
            }
            next_claim = files
                .iter()
                .find(|file| file.id == claim_id && is_claim(file))
                .and_then(|file| file.claim.clone());
        }
        for file in files {
            if is_claim(&file)
                && !active_claims.contains(&file.id)
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

/// The `error.errors[].reason` values of a Drive error body, in order. Only short ASCII-letter
/// reasons are kept, so no other service text reaches an error message.
fn error_reasons(body: &[u8]) -> Vec<String> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|body| body["error"]["errors"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|error| error["reason"].as_str())
        .filter(|reason| {
            !reason.is_empty()
                && reason.len() <= 64
                && reason.bytes().all(|byte| byte.is_ascii_alphabetic())
        })
        .map(str::to_owned)
        .collect()
}

/// The ids in a `files.list` page whose `mimeType` is a Google Workspace type other than a
/// folder (`application/vnd.google-apps.*`): files with no stored bytes.
fn google_native_ids(body: &[u8]) -> Vec<String> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|page| page["files"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter(|file| {
            file["mimeType"].as_str().is_some_and(|mime| {
                mime.starts_with("application/vnd.google-apps.") && mime != FOLDER_MIME
            })
        })
        .filter_map(|file| file["id"].as_str().map(str::to_owned))
        .collect()
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

/// The most `files.list` pages one path segment of a game file may take to resolve.
const GAME_LOOKUP_PAGES: usize = 16;
/// The longest `files.list` page a game-file lookup reads.
const GAME_LOOKUP_BYTES: usize = 256 * 1024;
/// The longest `files.get` metadata answer a game-file check reads.
const GAME_METADATA_BYTES: usize = 64 * 1024;
/// The fields a game-file metadata check asks for.
const GAME_FIELDS: &str = "id,name,mimeType,size,version,trashed";

/// A 403 that is not a rate limit, on a game file, by Drive's reason. Only an access reason
/// names `drive.file` (the app reads only files it created or was given; the scope is never
/// widened); a file Drive flags as abusive, a Google Docs-type file and a download policy each
/// say so instead. `acknowledgeAbuse` is never set.
fn no_access(error: Error, path: &str) -> Error {
    let Error::Server(message) = &error else {
        return error;
    };
    let Some(rest) = message.strip_prefix("drive: HTTP 403") else {
        return error;
    };
    let reason = rest
        .strip_prefix(" (")
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or("");
    let why = match reason {
        "" | "appNotAuthorizedToFile" | "insufficientFilePermissions" | "forbidden" => {
            "this app may read only files it created or was given (drive.file)"
        }
        "cannotDownloadAbusiveFile" => {
            "Drive flagged this file as malware or abuse and will not serve it; replace the file"
        }
        "fileNotDownloadable" => {
            "this is a Google Docs, Sheets or Slides file, not a stored file; upload the real file"
        }
        "downloadQuotaExceeded" => {
            "Drive's download quota for this file is used up; Drive lifts it on its own schedule, and this download is not retried automatically"
        }
        "domainPolicy" | "cannotDownloadFile" => {
            "a Drive or domain policy does not allow downloading this file"
        }
        _ => "Drive refused the download",
    };
    let shown = if reason.is_empty() {
        String::new()
    } else {
        format!(" ({reason})")
    };
    // A fixed 403 status first, then the path: digits in the path never decide the class.
    Error::Server(format!("drive: HTTP 403{shown} for {path}: {why}"))
}

/// A lookup or metadata answer past its bound is a service error, not a range problem.
fn too_long(error: Error, path: &str, what: &str) -> Error {
    match crate::gamefiles::over_bound(error, path) {
        Error::RangeUnsupported(_) => Error::Server(format!("drive: {what} is too long")),
        other => no_access(other, path),
    }
}

impl Drive {
    /// The one child called `name` of folder `parent`, for the game file `path`: a bounded,
    /// paginated `files.list` (at most `GAME_LOOKUP_PAGES` pages of `GAME_LOOKUP_BYTES`).
    /// None is `NotFound(path)` (under `drive.file` that includes a file the app may not see);
    /// two or more is an error naming the path, never a pick, unlike save sync's oldest-wins.
    fn game_child(&self, parent: &DriveFile, name: &str, path: &str) -> Result<(DriveFile, bool)> {
        let base = format!(
            "{API}/files?q={}&fields={LIST_FIELDS}&spaces=drive&pageSize=100",
            encode_segment(&children_query(&parent.id, Some(name)))
        );
        let mut url = base.clone();
        let mut found: Option<(DriveFile, bool)> = None;
        for _ in 0..GAME_LOOKUP_PAGES {
            let response = self
                .send_game(
                    Request::new("GET", &url).max_response_bytes(GAME_LOOKUP_BYTES),
                    path,
                    &[],
                )
                .map_err(|error| too_long(error, path, "a folder listing page"))?;
            let page = parse_files(&response.text())?;
            let native = google_native_ids(&response.body);
            for file in page.files.into_iter().filter(|file| file.name == name) {
                if found.is_some() {
                    return Err(Error::Server(format!(
                        "drive: more than one item is named '{name}' on the way to {path}; remove or rename the extra one"
                    )));
                }
                let is_native = native.contains(&file.id);
                found = Some((file, is_native));
            }
            match page.next_page {
                None => return found.ok_or_else(|| Error::NotFound(path.into())),
                Some(next) => url = format!("{base}&pageToken={}", encode_segment(&next)),
            }
        }
        Err(Error::Server(
            "drive: too many listing pages looking up a game file".into(),
        ))
    }

    /// The game file at `path`, resolved afresh from My Drive's root one segment at a time. A
    /// folder, or a missing segment, is `NotFound(path)`; a file without a version (or version
    /// 0) is an error, never taken as version 0.
    fn game_file(&self, path: &str) -> Result<DriveFile> {
        crate::gamefiles::check_file_path(path)?;
        let mut current = root();
        let mut native = false;
        for part in path.split('/') {
            if !current.is_dir {
                return Err(Error::NotFound(path.into()));
            }
            (current, native) = self.game_child(&current, part, path)?;
        }
        if current.is_dir {
            return Err(Error::NotFound(path.into()));
        }
        if native {
            // Docs, Sheets and Slides have no stored bytes to fetch in ranges; never ask.
            return Err(Error::Server(format!(
                "drive: {path} is a Google Docs, Sheets or Slides file, not a stored file; upload the real file"
            )));
        }
        if current.version == 0 {
            return Err(Error::Server(format!("drive: no version for {path}")));
        }
        Ok(current)
    }

    /// `file`'s current metadata; trashed is `NotFound(path)`.
    fn game_metadata(&self, file: &DriveFile, path: &str) -> Result<DriveFile> {
        let response = self
            .send_game(
                Request::new(
                    "GET",
                    &format!(
                        "{API}/files/{}?fields={GAME_FIELDS}",
                        encode_segment(&file.id)
                    ),
                )
                .max_response_bytes(GAME_METADATA_BYTES),
                path,
                &[],
            )
            .map_err(|error| too_long(error, path, "a metadata answer"))?;
        let trashed = serde_json::from_slice::<Value>(&response.body)
            .ok()
            .and_then(|value| value["trashed"].as_bool())
            .unwrap_or(false);
        if trashed {
            return Err(Error::NotFound(path.into()));
        }
        parse_file(&response.text())
    }

    fn media_url(file: &DriveFile) -> String {
        format!("{API}/files/{}?alt=media", encode_segment(&file.id))
    }
}

/// Game files from Drive under `drive.file` (docs/assets.md). Each request resolves the path
/// afresh (bounded lookups, duplicates refused), so a chunk costs one `files.list` per path
/// segment, the media request and one metadata check. The identity is `<file id>:<version>`:
/// a resumed or later chunk whose file now has another id or version is `remote_changed`
/// before any bytes are asked for, and a version or size that changed while a chunk was in
/// flight is `remote_changed` after it. These checks do not bind the media bytes atomically:
/// only the final size and SHA-256 against the manifest allow publication.
impl GameSource for Drive {
    fn service(&self) -> &'static str {
        "gdrive"
    }

    fn read_bounded(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        let max = max.min(crate::gamefiles::MANIFEST_MAX_BYTES);
        let file = self.game_file(path)?;
        let response = self
            .send_game(
                Request::new("GET", &Self::media_url(&file))
                    .header("Accept-Encoding", "identity")
                    .max_response_bytes(max),
                path,
                &[],
            )
            .map_err(|error| {
                no_access(
                    crate::gamefiles::over_manifest_bound(error, path, max),
                    path,
                )
            })?;
        Ok(response.body)
    }

    fn fetch(&self, path: &str, start: u64, len: u64) -> Result<RangeAnswer> {
        self.fetch_matching(path, start, len, None)
    }

    fn fetch_matching(
        &self,
        path: &str,
        start: u64,
        len: u64,
        expected: Option<&str>,
    ) -> Result<RangeAnswer> {
        let end = crate::gamefiles::range_end(start, len)?;
        let file = self.game_file(path)?;
        let identity = format!("{}:{}", file.id, file.version);
        if expected.is_some_and(|want| want != identity) {
            return Err(Error::RemoteRevised(format!(
                "{path} is another file or version on Drive now"
            )));
        }
        let response = self
            .send_game(
                Request::new("GET", &Self::media_url(&file))
                    .header("Range", &format!("bytes={start}-{end}"))
                    .header("Accept-Encoding", "identity")
                    .max_response_bytes(crate::gamefiles::range_response_bound(len)),
                path,
                &[416],
            )
            .map_err(|error| no_access(crate::gamefiles::over_bound(error, path), path))?;
        let after = self.game_metadata(&file, path)?;
        if after.version != file.version || after.size != file.size {
            return Err(Error::RemoteRevised(format!(
                "{path} changed on Drive while a chunk was read"
            )));
        }
        let header = |name: &str| response.header_value(name).map(str::to_string);
        Ok(RangeAnswer {
            status: response.status,
            content_range: header("Content-Range"),
            content_encoding: header("Content-Encoding"),
            content_type: header("Content-Type"),
            identity: Some(identity),
            body: response.body,
        })
    }
}

impl Remote for Drive {
    fn repair_missing_state_claim(
        &self,
        path: &str,
        evidence: Option<&crate::local::DriveClaimEvidence>,
        apply: bool,
    ) -> Result<Option<String>> {
        let Some(evidence) = evidence else {
            return Ok(None);
        };
        let state = self.resolve(path)?;
        if state.is_dir || evidence.state_path != path || evidence.file_id != state.id {
            return Ok(Some(
                "saved Drive claim evidence does not match this state file".into(),
            ));
        }
        let (bytes, etag) = self.get(path)?;
        let (version, claim) = guard::parse_etag(&etag).ok_or(Error::Changed)?;
        if version != evidence.version || claim != evidence.claim {
            return Ok(Some(
                "saved Drive claim evidence is stale; state metadata has changed".into(),
            ));
        }
        if claim.is_empty() {
            return Ok(Some(
                "saved Drive claim evidence has no claim id; cannot repair automatically".into(),
            ));
        }
        match self.metadata(&claim, path) {
            Ok(_) => {
                return Ok(Some(
                    "saved Drive claim evidence is stale; the named claim now exists".into(),
                ));
            }
            Err(Error::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
        if !apply {
            return Ok(Some(format!(
                "missing Drive claim {claim} was confirmed by a guarded 409/404; recovery is available. Stop sync on every other device before applying"
            )));
        }

        // Drive v3 exposes no conditional metadata update here. Require a single writer,
        // revalidate immediately before PATCH, and prove the state bytes did not change after.
        let (before_bytes, before_etag) = self.get(path)?;
        if before_etag != etag || before_bytes != bytes {
            return Err(Error::Changed);
        }
        let before = self.metadata(&state.id, path)?;
        if before.version != version || before.claim.as_deref() != Some(&claim) {
            return Err(Error::Changed);
        }
        match self.metadata(&claim, path) {
            Ok(_) => return Err(Error::Changed),
            Err(Error::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
        let fresh = self.generate_id()?;
        self.json(
            "PATCH",
            &format!(
                "{API}/files/{}?fields={FILE_FIELDS}",
                encode_segment(&state.id)
            ),
            &json!({"appProperties": {guard::CLAIM_PROPERTY: fresh}}),
            path,
        )?;
        let after = self.metadata(&state.id, path)?;
        if after.version <= version || after.claim.as_deref() != Some(&fresh) {
            return Err(Error::Changed);
        }
        let (after_bytes, after_etag) = self.get(path)?;
        if after_bytes != bytes
            || guard::parse_etag(&after_etag).map(|(v, _)| v) != Some(after.version)
        {
            return Err(Error::Changed);
        }
        Ok(Some(format!(
            "rotated missing Drive claim {claim} to {fresh}; state bytes were preserved"
        )))
    }

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
                return self.guarded_put(path, &file, &parent.id, bytes, etag);
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
