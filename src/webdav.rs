//! Nextcloud over WebDAV, one of the three services (docs/adr/0002-storage-backends.md). The
//! URL helpers and the PROPFIND parser are pure; `WebDav` sends its requests through a
//! `http::Transport` (the real one is behind the `http` feature), so the core builds without a
//! network stack. Its overwrite guard is the server's own `If-Match`.

use crate::error::{not_built, Result};
use crate::remote::RemoteEntry;

/// The WebDAV files URL for `user` on the Nextcloud at `server`, ending in `/`:
/// `files_url("https://cloud.example.org/", "me")` ->
/// `"https://cloud.example.org/remote.php/dav/files/me/"`. `server` may carry a path
/// (`https://example.org/nextcloud`). It must start with `http://` or `https://`, else
/// `Error::Usage`. The user name is percent-encoded like a path segment (`encode_segment`).
pub fn files_url(server: &str, user: &str) -> Result<String> {
    not_built("webdav::files_url")
}

/// Percent-encode one path segment: keep `A-Z a-z 0-9 - . _ ~`, encode every other byte of the
/// UTF-8 as `%XX` (uppercase hex). `"Uncharted 1"` -> `"Uncharted%201"`.
pub fn encode_segment(segment: &str) -> String {
    crate::not_built!("webdav::encode_segment")
}

/// `encode_segment` on each `/`-separated segment, joined with `/`.
pub fn encode_path(path: &str) -> String {
    crate::not_built!("webdav::encode_path")
}

/// Parse a `PROPFIND` (Depth: 1) `207 Multi-Status` body into the children of the folder whose
/// href path is `dir_href` (the folder's own response is left out). Reads `href`,
/// `resourcetype/collection`, `getcontentlength`, `getetag` (quotes kept as sent), in any
/// namespace prefix. Names are percent-decoded. Sorted by name. Malformed XML: `Error::Server`.
pub fn parse_propfind(xml: &str, dir_href: &str) -> Result<Vec<RemoteEntry>> {
    not_built("webdav::parse_propfind")
}

/// The real server, over any `Transport`. Every request carries Basic auth; 401/403 is `Error::AuthFailed`; a
/// connection, DNS, TLS or timeout failure is `Error::Unreachable`; 412 is `Error::Changed`;
/// 404 is `Error::NotFound`; 409 on PUT/MKCOL/MOVE/COPY is `Error::NotFound(parent)`.
/// The password is never logged or put in an error message.
pub struct WebDav {
    _private: (),
}

impl WebDav {
    /// `files_url` from `files_url()`; the password from the credential store; `transport`
    /// is `http::UreqTransport` outside tests.
    pub fn new(
        files_url: &str,
        user: &str,
        password: crate::creds::Secret,
        transport: std::sync::Arc<dyn crate::http::Transport>,
    ) -> WebDav {
        WebDav { _private: () }
    }
}

impl crate::remote::Remote for WebDav {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        crate::error::not_built("webdav::WebDav::list")
    }
    fn get(&self, path: &str) -> Result<(Vec<u8>, String)> {
        crate::error::not_built("webdav::WebDav::get")
    }
    fn put(&self, path: &str, bytes: &[u8], pre: &crate::remote::Precondition) -> Result<String> {
        crate::error::not_built("webdav::WebDav::put")
    }
    fn mkcol(&self, dir: &str) -> Result<()> {
        crate::error::not_built("webdav::WebDav::mkcol")
    }
    fn copy(&self, from: &str, to: &str) -> Result<()> {
        crate::error::not_built("webdav::WebDav::copy")
    }
    fn move_to(&self, from: &str, to: &str) -> Result<()> {
        crate::error::not_built("webdav::WebDav::move_to")
    }
    fn delete(&self, path: &str) -> Result<()> {
        crate::error::not_built("webdav::WebDav::delete")
    }
}
