//! Nextcloud over WebDAV, one of the three services (docs/adr/0002-storage-backends.md). The
//! URL helpers and the PROPFIND parser are pure; `WebDav` sends its requests through a
//! `http::Transport` (the real one is behind the `http` feature), so the core builds without a
//! network stack. Its overwrite guard is the server's own `If-Match`.

use crate::error::{Error, Result};
use crate::remote::RemoteEntry;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;

/// The WebDAV files URL for `user` on the Nextcloud at `server`, ending in `/`:
/// `files_url("https://cloud.example.org/", "me")` ->
/// `"https://cloud.example.org/remote.php/dav/files/me/"`. `server` may carry a path
/// (`https://example.org/nextcloud`). It must start with `http://` or `https://`, else
/// `Error::Usage`. The user name is percent-encoded like a path segment (`encode_segment`).
pub fn files_url(server: &str, user: &str) -> Result<String> {
    if !server.starts_with("http://") && !server.starts_with("https://") {
        return Err(Error::Usage(
            "the WebDAV server must start with http:// or https://".into(),
        ));
    }
    Ok(format!(
        "{}/remote.php/dav/files/{}/",
        server.trim_end_matches('/'),
        encode_segment(user)
    ))
}

/// Percent-encode one path segment: keep `A-Z a-z 0-9 - . _ ~`, encode every other byte of the
/// UTF-8 as `%XX` (uppercase hex). `"Uncharted 1"` -> `"Uncharted%201"`.
pub fn encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write;
            write!(encoded, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
    encoded
}

/// `encode_segment` on each `/`-separated segment, joined with `/`.
pub fn encode_path(path: &str) -> String {
    path.split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// Parse a `PROPFIND` (Depth: 1) `207 Multi-Status` body into the children of the folder whose
/// href path is `dir_href` (the folder's own response is left out). Reads `href`,
/// `resourcetype/collection`, `getcontentlength`, `getetag` (quotes kept as sent), in any
/// namespace prefix. Names are percent-decoded. Sorted by name. Malformed XML: `Error::Server`.
pub fn parse_propfind(xml: &str, dir_href: &str) -> Result<Vec<RemoteEntry>> {
    let mut reader = NsReader::from_str(xml);
    let mut entries = Vec::new();
    let mut current: Option<Response> = None;
    let mut field: Option<(Field, String)> = None;
    let mut depth = 0usize;
    let own = dir_href.trim_end_matches('/');

    loop {
        let (ns, event) = reader.read_resolved_event().map_err(xml_error)?;
        match event {
            Event::Start(start) => {
                depth += 1;
                if !is_dav(&ns) {
                    continue;
                }
                match start.local_name().as_ref() {
                    b"response" => {
                        current = Some(Response::default());
                        field = None;
                    }
                    b"href" => field = Some((Field::Href, String::new())),
                    b"getcontentlength" => field = Some((Field::Length, String::new())),
                    b"getetag" => field = Some((Field::Etag, String::new())),
                    b"collection" => {
                        if let Some(response) = current.as_mut() {
                            response.is_dir = true;
                        }
                    }
                    _ => {}
                }
            }
            Event::Empty(start) => {
                if !is_dav(&ns) {
                    continue;
                }
                match start.local_name().as_ref() {
                    b"href" => {
                        if let Some(response) = current.as_mut() {
                            response.href = Some(String::new());
                        }
                    }
                    b"collection" => {
                        if let Some(response) = current.as_mut() {
                            response.is_dir = true;
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(text) => {
                if let Some((_, buffer)) = field.as_mut() {
                    buffer.push_str(&text.unescape().map_err(xml_error)?);
                }
            }
            Event::CData(text) => {
                if let Some((_, buffer)) = field.as_mut() {
                    buffer.push_str(&String::from_utf8_lossy(&text.into_inner()));
                }
            }
            Event::End(end) => {
                depth = depth.saturating_sub(1);
                if !is_dav(&ns) {
                    continue;
                }
                match end.local_name().as_ref() {
                    b"href" | b"getcontentlength" | b"getetag" => {
                        if let Some((kind, text)) = field.take() {
                            if let Some(response) = current.as_mut() {
                                response.set(kind, &text)?;
                            }
                        }
                    }
                    b"response" => {
                        field = None;
                        if let Some(response) = current.take() {
                            if let Some(entry) = response.into_entry(own) {
                                entries.push(entry);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if depth != 0 || current.is_some() {
        return Err(Error::Server(
            "the PROPFIND body ended before the XML did".into(),
        ));
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

#[derive(Debug, Default)]
struct Response {
    href: Option<String>,
    is_dir: bool,
    size: Option<u64>,
    etag: Option<String>,
}

impl Response {
    fn set(&mut self, kind: Field, text: &str) -> Result<()> {
        let text = text.trim();
        match kind {
            Field::Href => self.href = Some(text.to_string()),
            Field::Length if text.is_empty() => self.size = None,
            Field::Length => {
                self.size = Some(text.parse().map_err(|_| {
                    Error::Server(format!("the PROPFIND body has a bad size: '{text}'"))
                })?);
            }
            Field::Etag if text.is_empty() => self.etag = None,
            Field::Etag => self.etag = Some(text.to_string()),
        }
        Ok(())
    }

    fn into_entry(self, own: &str) -> Option<RemoteEntry> {
        let href = self.href?;
        let path = href.split(['?', '#']).next().unwrap_or(&href);
        let path = path.trim_end_matches('/');
        if path == own {
            return None;
        }
        let segment = path.rsplit('/').next().unwrap_or("");
        if segment.is_empty() {
            return None;
        }
        Some(RemoteEntry {
            name: decode_percent(segment),
            is_dir: self.is_dir,
            size: if self.is_dir {
                0
            } else {
                self.size.unwrap_or(0)
            },
            etag: self.etag,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum Field {
    Href,
    Length,
    Etag,
}

fn is_dav(ns: &ResolveResult<'_>) -> bool {
    matches!(ns, ResolveResult::Bound(namespace) if namespace.0 == b"DAV:")
}

fn xml_error(error: impl std::fmt::Display) -> Error {
    Error::Server(format!("the PROPFIND body is not valid XML: {error}"))
}

fn decode_percent(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                decoded.push(high * 16 + low);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// The real server, over any `Transport`. Every request carries Basic auth; 401/403 is `Error::AuthFailed`; a
/// connection, DNS, TLS or timeout failure is `Error::Unreachable`; 412 on PUT is `Error::Changed`
/// (on COPY/MOVE it is `Error::Server("exists")`); 404 is `Error::NotFound`; 409 on
/// PUT/MKCOL/MOVE/COPY is `Error::NotFound(parent)`.
/// The password is never logged or put in an error message.
pub struct WebDav {
    files_url: String,
    auth: String,
    transport: std::sync::Arc<dyn crate::http::Transport>,
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
        WebDav {
            files_url: files_url.to_string(),
            auth: "Basic ".to_string()
                + &Self::b64(format!("{}:{}", user, password.expose()).as_bytes()),
            transport,
        }
    }

    fn b64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut result = String::new();
        let mut i = 0;

        while i < bytes.len() {
            let mut bits = 0u32;
            let mut bit_count = 0;

            // Take up to 3 bytes (224 bits) and shift them into the bits variable
            for j in 0..3 {
                if i + j < bytes.len() {
                    bits |= (bytes[i + j] as u32) << (16 - j * 8);
                    bit_count += 8;
                }
            }

            // Process 4 characters at a time
            for j in 0..4 {
                if bit_count >= (j * 6) {
                    let index = (bits >> (18 - j * 6)) & 0x3F;
                    result.push(ALPHABET[index as usize] as char);
                } else {
                    result.push('=');
                }
            }

            i += 3;
        }

        result
    }

    fn url(&self, path: &str) -> String {
        let encoded = encode_path(path);
        format!("{}{}", self.files_url, encoded)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, String)],
        body: Vec<u8>,
    ) -> Result<crate::http::Response> {
        let url = self.url(path);
        let mut request = crate::http::Request::new(method, &url);
        request = request.header("Authorization", &self.auth);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        if !body.is_empty() {
            request = request.body(body);
        }
        self.transport.send(&request)
    }

    /// COPY or MOVE `from` to `to` with `Overwrite: F`. Only the answers that differ from the
    /// common mapping in `send` are special here: 412 (the destination exists) and 409 (the
    /// parent of the destination is missing).
    fn transfer(&self, method: &str, from: &str, to: &str) -> Result<()> {
        let headers = [
            ("Destination", self.url(to)),
            ("Overwrite", "F".to_string()),
        ];
        let response = self.request(method, from, &headers, vec![])?;
        match response.status {
            201 | 204 => Ok(()),
            412 => Err(Error::Server("exists".into())),
            409 => Err(Error::NotFound(match to.rfind('/') {
                Some(pos) => to[..pos].to_string(),
                None => String::new(),
            })),
            404 => Err(Error::NotFound(from.to_string())),
            401 | 403 => Err(Error::AuthFailed),
            status => Err(Error::Server(format!("{} {}: {}", method, from, status))),
        }
    }

    fn send(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, String)],
        body: Vec<u8>,
        ok: &[u16],
    ) -> Result<crate::http::Response> {
        let response = self.request(method, path, headers, body)?;
        if ok.contains(&response.status) {
            Ok(response)
        } else {
            match response.status {
                404 => Err(Error::NotFound(path.to_string())),
                409 => match path.rfind('/') {
                    Some(pos) => Err(Error::NotFound(path[..pos].to_string())),
                    None => Err(Error::NotFound(String::new())),
                },
                412 if method == "PUT" => Err(Error::Changed),
                401 | 403 => Err(Error::AuthFailed),
                _ => Err(Error::Server(format!(
                    "{} {}: {}",
                    method, path, response.status
                ))),
            }
        }
    }
}

impl crate::remote::Remote for WebDav {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        let response = self.send("PROPFIND", dir, &[("Depth", "1".into())], vec![], &[207])?;
        // The href should be the path portion of the request URL (without the scheme and host)
        let url = self.url(dir);
        let href = if let Some(pos) = url.find("/remote.php/dav/files/") {
            // Extract everything from /remote.php/dav/files/ onwards
            &url[pos..]
        } else {
            // Fallback to just the path part
            let path_start = url.rfind('/').map(|i| i + 1).unwrap_or(0);
            &url[path_start..]
        };

        let entries = parse_propfind(&response.text(), href)?;
        Ok(entries)
    }

    fn get(&self, path: &str) -> Result<(Vec<u8>, String)> {
        let response = self.send("GET", path, &[], vec![], &[200])?;
        let etag = response
            .header_value("ETag")
            .ok_or_else(|| Error::Server(format!("GET {}: no ETag", path)))?
            .to_string();
        Ok((response.body, etag))
    }

    fn put(&self, path: &str, bytes: &[u8], pre: &crate::remote::Precondition) -> Result<String> {
        let headers: Vec<(&str, String)> = match pre {
            crate::remote::Precondition::Any => vec![],
            crate::remote::Precondition::Match(etag) => vec![("If-Match", etag.clone())],
            crate::remote::Precondition::Absent => vec![("If-None-Match", "*".to_string())],
        };
        let response = self.send("PUT", path, &headers, bytes.to_vec(), &[200, 201, 204])?;
        response
            .header_value("ETag")
            .map(|etag| etag.to_string())
            .ok_or_else(|| Error::Server(format!("PUT {path}: no ETag")))
    }
    fn mkcol(&self, dir: &str) -> Result<()> {
        self.send("MKCOL", dir, &[], vec![], &[201, 405])?;
        Ok(())
    }
    fn copy(&self, from: &str, to: &str) -> Result<()> {
        self.transfer("COPY", from, to)
    }
    fn move_to(&self, from: &str, to: &str) -> Result<()> {
        self.transfer("MOVE", from, to)
    }
    fn delete(&self, path: &str) -> Result<()> {
        self.send("DELETE", path, &[], vec![], &[200, 204])?;
        Ok(())
    }
}
