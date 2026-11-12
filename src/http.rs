//! The one door to the network. Every backend (`webdav`, `dropbox`, `gdrive`) and the sign-in
//! (`oauth`) build `Request`s and send them through a `Transport`; they never open a socket
//! themselves. The real transport (`UreqTransport`: ureq with rustls, no OpenSSL) is behind the
//! `http` feature, so the core builds with no network stack, the fakes in `fake` stand in for
//! Google Drive and Dropbox in tests, and a mobile build can bring its own (docs/adr/0002).
//!
//! A `Transport` returns every HTTP answer, 4xx and 5xx included, as a `Response`; only a
//! connection, DNS, TLS or timeout failure is an error (`Error::Unreachable`).

use crate::error::Result;
use std::fmt;
#[cfg(feature = "http")]
use std::io::Read;
#[cfg(feature = "http")]
use std::time::Duration;

/// One HTTP request. `Debug` prints `***` for the value of an `Authorization` header, so a
/// token or password cannot leak through a log line. On purpose: the redaction must exist
/// before anything can print one.
#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    /// `GET`, `PUT`, `POST`, `PROPFIND`, ...
    pub method: String,
    /// The full URL, query included.
    pub url: String,
    /// In the order added; names as given.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Optional bound for the response body; the transport must stop after this many bytes.
    pub max_response_bytes: Option<usize>,
}

impl Request {
    pub fn new(method: &str, url: &str) -> Request {
        Request {
            method: method.to_string(),
            url: url.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            max_response_bytes: None,
        }
    }

    /// Add a header (builder style).
    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Set the body (builder style).
    pub fn body(mut self, body: Vec<u8>) -> Request {
        self.body = body;
        self
    }

    /// Limit how many response body bytes a transport may read.
    pub fn max_response_bytes(mut self, bytes: usize) -> Request {
        self.max_response_bytes = Some(bytes);
        self
    }

    /// The first header called `name`, compared without case.
    pub fn header_value(&self, name: &str) -> Option<&str> {
        find(&self.headers, name)
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(k, v)| {
                if k.eq_ignore_ascii_case("authorization") {
                    (k.as_str(), "***")
                } else {
                    (k.as_str(), v.as_str())
                }
            })
            .collect();
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &headers)
            .field("body_len", &self.body.len())
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
}

/// One HTTP answer, whatever its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    /// Add a header (builder style).
    pub fn header(mut self, name: &str, value: &str) -> Response {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// The first header called `name`, compared without case.
    pub fn header_value(&self, name: &str) -> Option<&str> {
        find(&self.headers, name)
    }

    /// The body as text (lossy UTF-8).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn find<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

pub trait Transport {
    /// Send one request and return the answer, whatever its status. A connection, DNS, TLS
    /// or timeout failure is `Error::Unreachable(<short reason>)`, never naming a header value.
    /// If `request.max_response_bytes` is set, implementations must cap bytes read from the
    /// response stream and fail when the response is larger than that bound.
    fn send(&self, request: &Request) -> Result<Response>;
}

/// The most request body bytes a transport sends. A body is held in memory whole, so this is
/// the bound on that memory: a larger one is refused before any connection is made.
pub const MAX_REQUEST_BODY: usize = 256 * 1024 * 1024;

/// The default idle timeout: each read or write step may wait this long.
#[cfg(feature = "http")]
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// The real transport: ureq with rustls (pure Rust TLS). The connect timeout is 5 s; reads and
/// writes each time out after 60 s of no progress, so a slow upload of a large save finishes
/// while a stalled connection still ends (M5) as `Unreachable`. There is no total deadline for
/// a whole request. No retries here: each backend decides what to retry. Added by its issue
/// (docs/code-map.md).
#[cfg(feature = "http")]
#[derive(Debug)]
pub struct UreqTransport {
    agent: ureq::Agent,
}

#[cfg(feature = "http")]
impl UreqTransport {
    pub fn new() -> UreqTransport {
        UreqTransport::with_idle_timeout(IDLE_TIMEOUT)
    }

    /// A transport whose reads and writes each time out after `idle` without progress.
    pub fn with_idle_timeout(idle: Duration) -> UreqTransport {
        UreqTransport {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(5))
                .timeout_read(idle)
                .timeout_write(idle)
                .redirects(0)
                .build(),
        }
    }
}

#[cfg(feature = "http")]
impl Default for UreqTransport {
    fn default() -> UreqTransport {
        UreqTransport::new()
    }
}

#[cfg(feature = "http")]
impl Transport for UreqTransport {
    fn send(&self, request: &Request) -> Result<Response> {
        if request.body.len() > MAX_REQUEST_BODY {
            return Err(crate::error::Error::Server(format!(
                "request body exceeds the {MAX_REQUEST_BODY}-byte limit"
            )));
        }
        let host = host_of(&request.url);
        let mut outgoing = self.agent.request(&request.method, &request.url);
        for (name, value) in &request.headers {
            outgoing = outgoing.set(name, value);
        }

        let response = match outgoing.send_bytes(&request.body) {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => response,
            Err(ureq::Error::Transport(error)) => {
                return Err(crate::error::Error::Unreachable(format!(
                    "{host}: {:?}",
                    error.kind()
                )));
            }
        };

        let status = response.status();
        let mut headers = Vec::new();
        for name in response.headers_names() {
            for value in response.all(&name) {
                headers.push((name.clone(), value.to_string()));
            }
        }
        let mut body = Vec::new();
        if let Some(limit) = request.max_response_bytes {
            response
                .into_reader()
                .take(limit.saturating_add(1) as u64)
                .read_to_end(&mut body)
                .map_err(|error| {
                    crate::error::Error::Unreachable(format!("{host}: reading the answer: {error}"))
                })?;
            if body.len() > limit {
                return Err(crate::error::Error::Server(format!(
                    "HTTP response body exceeds the {limit}-byte limit"
                )));
            }
        } else {
            response
                .into_reader()
                .read_to_end(&mut body)
                .map_err(|error| {
                    crate::error::Error::Unreachable(format!("{host}: reading the answer: {error}"))
                })?;
        }

        Ok(Response {
            status,
            headers,
            body,
        })
    }
}

/// The host part of `url`, so an error can name the server without ever repeating a header.
#[cfg(feature = "http")]
fn host_of(url: &str) -> &str {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}
