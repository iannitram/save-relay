//! Google Drive (REST API v3) as a `Remote`, with `drive.file` access only: save-relay sees the
//! files it created and nothing else in the Drive (SPEC.md M9, M10;
//! docs/adr/0002-storage-backends.md). Its folder (`<root>`, default `save-relay`) is a normal
//! folder in My Drive that the person can open.
//!
//! Drive is not path-based: every file has an id and parents, and two files may share a name.
//! `Drive` resolves a `Remote` path one segment at a time from `root` (Drive's alias for My
//! Drive), caching ids for the life of the value; when two children share a name, the oldest
//! (`createdTime`, then id) is the one (`pick`), everywhere, on every machine.
//!
//! Drive v3 has no `If-Match` on an upload, so a `put` with `Precondition::Match` goes through
//! the guard in `guard` (a claim file per state version) to keep M9's "a push never replaces
//! saves another machine pushed meanwhile". A file's etag is `guard::make_etag(version, claim)`.

pub mod guard;

use crate::error::{not_built, Result};
use crate::http::Transport;
use crate::oauth::TokenSource;
use crate::remote::{Precondition, Remote, RemoteEntry};
use crate::stamp::Clock;
use std::sync::Arc;

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
        crate::not_built!("gdrive::DriveFile::entry")
    }
}

/// Quote text for a Drive query string literal: `\` -> `\\`, `'` -> `\'`.
/// `"it's"` -> `"it\'s"`.
pub fn escape_q(text: &str) -> String {
    crate::not_built!("gdrive::escape_q")
}

/// The `q` for the children of folder `parent_id`, optionally with one name:
/// `"'<parent>' in parents and trashed = false"`, plus `" and name = '<name>'"`, both through
/// `escape_q`.
pub fn children_query(parent_id: &str, name: Option<&str>) -> String {
    crate::not_built!("gdrive::children_query")
}

/// One page of `files.list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePage {
    /// In the order Drive sent them.
    pub files: Vec<DriveFile>,
    pub next_page: Option<String>,
}

/// Parse a `files.list` answer (`files`, `nextPageToken`). Not JSON, a file without `id` or
/// `name`, or a `version`/`size` that is not a whole number is
/// `Error::Server("drive: bad list answer")`.
pub fn parse_files(body: &str) -> Result<FilePage> {
    not_built("gdrive::parse_files")
}

/// Parse one file's metadata (the answer of `files.get`, `files.create`, `files.update`),
/// with the same rules as one entry of `parse_files`; else `Error::Server("drive: bad file answer")`.
pub fn parse_file(body: &str) -> Result<DriveFile> {
    not_built("gdrive::parse_file")
}

/// The file called `name` among `files`: the oldest by `created`, then by `id` when two have
/// the same time. `None` when no file has that name.
pub fn pick<'a>(files: &'a [DriveFile], name: &str) -> Option<&'a DriveFile> {
    crate::not_built!("gdrive::pick")
}

/// A `multipart/related` upload body: the metadata JSON part, then the content part
/// (`application/octet-stream`), with `boundary`; each part's headers then a blank line, CRLF
/// line ends, closed by `--<boundary>--`.
pub fn multipart_body(metadata: &serde_json::Value, bytes: &[u8], boundary: &str) -> Vec<u8> {
    crate::not_built!("gdrive::multipart_body")
}

/// The real Google Drive. `put` with `Any` updates or creates; with `Absent` creates, then
/// lists the name and keeps only the oldest (`pick`), deleting its own copy and returning
/// `Error::Changed` if another machine's is older; with `Match` runs the guard. `move_to` is
/// one metadata update (new parent and name), so a folder appears in `history/` whole.
/// `copy` of a folder copies file by file (Drive copies files only): not atomic, which is why
/// callers copy into `incoming/` and then `move_to` (docs/adr/0002). Retries 429, 5xx and
/// 403 `rateLimitExceeded` with backoff; refreshes the token once on 401. Never logs a token.
pub struct Drive {
    _private: (),
}

impl Drive {
    pub fn new(tokens: TokenSource, transport: Arc<dyn Transport>, clock: Clock) -> Drive {
        Drive { _private: () }
    }

    /// The signed-in account's e-mail address (`about?fields=user`), for the config and
    /// `status`.
    pub fn account_email(&self) -> Result<String> {
        not_built("gdrive::Drive::account_email")
    }
}

impl Remote for Drive {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        not_built("gdrive::Drive::list")
    }
    fn get(&self, path: &str) -> Result<(Vec<u8>, String)> {
        not_built("gdrive::Drive::get")
    }
    fn put(&self, path: &str, bytes: &[u8], pre: &Precondition) -> Result<String> {
        not_built("gdrive::Drive::put")
    }
    fn mkcol(&self, dir: &str) -> Result<()> {
        not_built("gdrive::Drive::mkcol")
    }
    fn copy(&self, from: &str, to: &str) -> Result<()> {
        not_built("gdrive::Drive::copy")
    }
    fn move_to(&self, from: &str, to: &str) -> Result<()> {
        not_built("gdrive::Drive::move_to")
    }
    fn delete(&self, path: &str) -> Result<()> {
        not_built("gdrive::Drive::delete")
    }
}
