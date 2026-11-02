//! Signing in to Google Drive and Dropbox (SPEC.md M6, M10; docs/adr/0002-storage-backends.md).
//!
//! OAuth 2.0 for a desktop app with PKCE (RFC 7636, `S256`): no secret is relied on, and the
//! narrow scope is part of every sign-in request (M10). Three ways to finish a sign-in:
//!
//! - **Browser** (the default): open the system browser on `authorize_url`, wait on a loopback
//!   redirect (`http://127.0.0.1:<port>/`, std `TcpListener`), exchange the code.
//! - **Code, Dropbox**: no redirect; Dropbox shows a code that the person types back
//!   (`setup --no-browser`, then `setup --finish` with the code on standard input).
//! - **Device, Google**: a short code shown on screen and typed at google.com/device on a
//!   phone (RFC 8628), polled by `setup --finish`. This is the Game Mode path.
//!
//! The refresh token goes to the credential store (`service::token_account`) and nowhere
//! else; access tokens live in memory only (`TokenSource`). The pure parts (PKCE, URLs, the
//! redirect, token answers, expiry) are separate functions so each is tested on its own.

use crate::clients::Client;
use crate::creds::Secret;
use crate::error::Result;
use crate::http::{Request, Transport};
use crate::service::Service;
use crate::stamp::Clock;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{BufRead, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Refresh an access token this many seconds before it expires.
pub const REFRESH_MARGIN: u64 = 60;

/// How long `sign_in_browser` waits for the redirect before `Error::SignInFailed`.
pub const SIGN_IN_TIMEOUT: u64 = 300;

/// One service's OAuth endpoints and the narrow access it asks for (M10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provider {
    pub service: Service,
    pub authorize_url: &'static str,
    pub token_url: &'static str,
    /// The device-code endpoint (Google only).
    pub device_url: Option<&'static str>,
    /// Space-separated scopes, sent in every sign-in request.
    pub scope: &'static str,
    /// Extra query pairs for `authorize_url`, appended in this order.
    pub extra: &'static [(&'static str, &'static str)],
}

/// Google: `drive.file` only (files this app created; never the rest of the Drive).
/// `access_type=offline` and `prompt=consent` make Google return a refresh token every time.
pub const GOOGLE: Provider = Provider {
    service: Service::Gdrive,
    authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
    token_url: "https://oauth2.googleapis.com/token",
    device_url: Some("https://oauth2.googleapis.com/device/code"),
    scope: "https://www.googleapis.com/auth/drive.file",
    extra: &[("access_type", "offline"), ("prompt", "consent")],
};

/// Dropbox: an App folder app, so these scopes reach only `Apps/<app>/`.
/// `token_access_type=offline` makes Dropbox return a refresh token.
pub const DROPBOX: Provider = Provider {
    service: Service::Dropbox,
    authorize_url: "https://www.dropbox.com/oauth2/authorize",
    token_url: "https://api.dropboxapi.com/oauth2/token",
    device_url: None,
    scope: "account_info.read files.metadata.read files.metadata.write files.content.read files.content.write",
    extra: &[("token_access_type", "offline")],
};

/// The provider for a service; `None` for Nextcloud.
pub fn provider(service: Service) -> Option<&'static Provider> {
    match service {
        Service::Gdrive => Some(&GOOGLE),
        Service::Dropbox => Some(&DROPBOX),
        Service::Nextcloud => None,
    }
}

/// Base64url without padding (RFC 4648 §5): `b"hi?"` -> `"aGk_"`; `[0xfb, 0xff]` -> `"-_8"`.
pub fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 0x3f) as usize] as char);
        }
    }
    out
}

/// A PKCE code verifier from 32 random bytes: their base64url, 43 characters.
pub fn verifier_from_bytes(bytes: &[u8; 32]) -> String {
    base64url(bytes)
}

/// The PKCE `S256` challenge: base64url of the SHA-256 of the verifier's ASCII. RFC 7636
/// appendix B: `"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"` ->
/// `"E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"`.
pub fn challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    base64url(&Sha256::digest(verifier.as_bytes()))
}

/// 32 bytes from the OS's secure random source (the `getrandom` crate, pure Rust, added by
/// its issue). Failure is `Error::SignInFailed("no secure random source")`.
pub fn random_bytes() -> Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| crate::error::Error::SignInFailed("no secure random source".into()))?;
    Ok(bytes)
}

/// `application/x-www-form-urlencoded` pairs joined with `&`, in the given order: keep
/// `A-Z a-z 0-9 - . _ ~`, every other UTF-8 byte as `%XX` (uppercase hex; a space is `%20`).
/// `[("a", "b c"), ("x", "1&2")]` -> `"a=b%20c&x=1%262"`.
pub fn form_encode(pairs: &[(&str, &str)]) -> String {
    fn push_encoded(out: &mut String, s: &str) {
        for &b in s.as_bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    out.push(b as char);
                }
                _ => {
                    let high = char::from_digit((b >> 4) as u32, 16).unwrap();
                    let low = char::from_digit((b & 0x0f) as u32, 16).unwrap();
                    out.push('%');
                    out.push(high.to_ascii_uppercase());
                    out.push(low.to_ascii_uppercase());
                }
            }
        }
    }

    let mut out = String::new();
    for (i, (key, value)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        push_encoded(&mut out, key);
        out.push('=');
        push_encoded(&mut out, value);
    }
    out
}

/// The sign-in URL: `<authorize_url>?` + `form_encode` of, in this order, `client_id`,
/// `redirect_uri` (left out when `redirect_uri` is empty: Dropbox's code flow),
/// `response_type=code`, `scope`, `code_challenge`, `code_challenge_method=S256`, `state`,
/// then `provider.extra`.
pub fn authorize_url(
    provider: &Provider,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> String {
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    pairs.push(("client_id", client_id));
    if !redirect_uri.is_empty() {
        pairs.push(("redirect_uri", redirect_uri));
    }
    pairs.push(("response_type", "code"));
    pairs.push(("scope", provider.scope));
    pairs.push(("code_challenge", challenge));
    pairs.push(("code_challenge_method", "S256"));
    pairs.push(("state", state));
    pairs.extend(provider.extra.iter().copied());
    format!("{}?{}", provider.authorize_url, form_encode(&pairs))
}

/// The code from the loopback redirect's request target (`"/?code=..&state=..&scope=.."`;
/// query values percent-decoded, `+` read as a space). `state` must equal `want_state`, else
/// `Error::SignInFailed("the answer did not match this sign-in")`. An `error` pair (the person
/// refused: `error=access_denied`) is `Error::SignInFailed("<error value>")`, checked before
/// the state. No `code` is `Error::SignInFailed("no code in the answer")`.
pub fn parse_redirect(target: &str, want_state: &str) -> Result<String> {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }

    fn decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'+' => {
                    out.push(b' ');
                    i += 1;
                }
                b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(high), Some(low)) => {
                        out.push(high * 16 + low);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                },
                b => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    let mut error: Option<String> = None;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = decode(key);
        let value = decode(value);
        match key.as_str() {
            "code" => code = Some(value),
            "state" => state = Some(value),
            "error" => error = Some(value),
            _ => {}
        }
    }

    if let Some(error) = error {
        return Err(crate::error::Error::SignInFailed(error));
    }
    match state {
        Some(state) if state == want_state => {}
        _ => {
            return Err(crate::error::Error::SignInFailed(
                "the answer did not match this sign-in".into(),
            ));
        }
    }
    match code {
        Some(code) => Ok(code),
        None => Err(crate::error::Error::SignInFailed(
            "no code in the answer".into(),
        )),
    }
}

/// Tokens from a token endpoint. `Debug` shows `***` for both secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokens {
    pub access: Secret,
    /// Unix seconds: `now + expires_in` (`expires_in` 3600 when the answer leaves it out).
    pub expires_at: u64,
    /// Present on a sign-in; on a refresh only when the service rotated it.
    pub refresh: Option<Secret>,
}

/// Read a token endpoint's answer (a code exchange or a refresh), checked in this order:
///
/// 1. status 500-599, or a body that is not JSON: `Error::Server("token endpoint: <status>")`;
/// 2. `"error": "invalid_grant"` (any status: the refresh token expired or was withdrawn):
///    `Error::ReauthNeeded(<service label>)`;
/// 3. any other `error`: `Error::SignInFailed("<error>: <error_description>")`, or just
///    `"<error>"` with no description;
/// 4. status 200 with a string `access_token`: `Tokens` (`expires_in` a JSON number, 3600
///    when left out; `refresh_token` when present);
/// 5. anything else: `Error::Server("token endpoint: <status>")`.
pub fn parse_token_response(service: Service, status: u16, body: &str, now: u64) -> Result<Tokens> {
    let server_error = || crate::Error::Server(format!("token endpoint: {status}"));
    if (500..=599).contains(&status) {
        return Err(server_error());
    }
    let answer: serde_json::Value = serde_json::from_str(body).map_err(|_| server_error())?;
    match answer.get("error").and_then(serde_json::Value::as_str) {
        Some("invalid_grant") => {
            return Err(crate::Error::ReauthNeeded(service_label(service).into()));
        }
        Some(error) => {
            let message = match answer
                .get("error_description")
                .and_then(serde_json::Value::as_str)
            {
                Some(description) => format!("{error}: {description}"),
                None => error.to_string(),
            };
            return Err(crate::Error::SignInFailed(message));
        }
        None => {}
    }
    if status == 200 {
        if let Some(access) = answer
            .get("access_token")
            .and_then(serde_json::Value::as_str)
        {
            let expires_in = answer
                .get("expires_in")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(3600);
            let refresh = answer
                .get("refresh_token")
                .and_then(serde_json::Value::as_str)
                .map(Secret::new);
            return Ok(Tokens {
                access: Secret::new(access),
                expires_at: now.saturating_add(expires_in),
                refresh,
            });
        }
    }
    Err(server_error())
}

/// The name a service is shown by, for a message about its sign-in.
fn service_label(service: Service) -> &'static str {
    match service {
        Service::Gdrive => "Google Drive",
        Service::Dropbox => "Dropbox",
        Service::Nextcloud => "Nextcloud",
    }
}

/// True when an access token expiring at `expires_at` should be refreshed at `now`:
/// `now + REFRESH_MARGIN >= expires_at`.
pub fn needs_refresh(expires_at: u64, now: u64) -> bool {
    now.saturating_add(REFRESH_MARGIN) >= expires_at
}

/// Access tokens for one service, refreshed from the stored refresh token as needed.
/// Interior mutability (a `Mutex`), used through `&self` by a backend.
pub struct TokenSource {
    provider: &'static Provider,
    client: Client,
    transport: Arc<dyn Transport>,
    clock: Clock,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    cached: Option<Tokens>,
    refresh: Secret,
    rotated: Option<Secret>,
}

impl fmt::Debug for TokenSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenSource")
            .field("service", &self.provider.service)
            .finish()
    }
}

impl TokenSource {
    /// `client` from `clients`; `refresh` from the credential store.
    pub fn new(
        provider: &'static Provider,
        client: Client,
        refresh: Secret,
        transport: Arc<dyn Transport>,
        clock: Clock,
    ) -> TokenSource {
        TokenSource {
            provider,
            client,
            transport,
            clock,
            state: Mutex::new(State {
                cached: None,
                refresh,
                rotated: None,
            }),
        }
    }

    /// A valid access token: the cached one, or a new one from the token endpoint when none is
    /// cached or `needs_refresh`. `invalid_grant` is `Error::ReauthNeeded`.
    pub fn bearer(&self) -> Result<Secret> {
        let mut state = self.state.lock().expect("token source lock poisoned");
        let now = (self.clock)();
        if let Some(ref tokens) = state.cached {
            if !needs_refresh(tokens.expires_at, now) {
                return Ok(tokens.access.clone());
            }
        }

        let refresh_token = state.refresh.expose();
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        pairs.push(("client_id", self.client.id));
        if self.provider.service == Service::Gdrive {
            pairs.push(("client_secret", self.client.secret.unwrap_or("")));
        }
        pairs.push(("refresh_token", refresh_token));
        pairs.push(("grant_type", "refresh_token"));

        let body = form_encode(&pairs);
        let request = Request::new("POST", self.provider.token_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body.into_bytes());

        let response = self.transport.send(&request)?;
        let tokens = parse_token_response(
            self.provider.service,
            response.status,
            &response.text(),
            now,
        )?;

        if let Some(ref new_refresh) = tokens.refresh {
            state.refresh = new_refresh.clone();
            state.rotated = Some(new_refresh.clone());
        }

        let access = tokens.access.clone();
        state.cached = Some(tokens);
        Ok(access)
    }

    /// Forget the cached access token (a backend calls this after a 401, then retries once).
    pub fn invalidate(&self) {
        let mut state = self.state.lock().expect("token source lock poisoned");
        state.cached = None;
    }

    /// A new refresh token the service sent on a refresh, which the caller must store in place
    /// of the old one; `None` when it did not rotate.
    pub fn rotated(&self) -> Option<Secret> {
        let mut state = self.state.lock().expect("token source lock poisoned");
        state.rotated.take()
    }
}

/// A sign-in waiting for its `setup --finish`: kept in the credential store (as JSON in a
/// `Secret`, account `"pending:<service key>"`) between the two commands, never in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub service: Service,
    /// The page the person opens (Dropbox: the sign-in URL; Google: the device page).
    pub url: String,
    /// The code the person types on that page (Google); `None` for Dropbox.
    pub user_code: Option<String>,
    /// Unix seconds after which the sign-in has to start again.
    pub expires_at: u64,
    /// Seconds between polls (Google); 0 for Dropbox.
    pub interval: u64,
    /// The PKCE verifier (Dropbox) or the device code (Google).
    pub secret: Secret,
}

#[derive(Serialize, Deserialize)]
struct PendingRecord {
    service: Service,
    url: String,
    user_code: Option<String>,
    expires_at: u64,
    interval: u64,
    secret: String,
}

impl Pending {
    pub fn to_secret(&self) -> Secret {
        let record = PendingRecord {
            service: self.service,
            url: self.url.clone(),
            user_code: self.user_code.clone(),
            expires_at: self.expires_at,
            interval: self.interval,
            secret: self.secret.expose().to_string(),
        };
        let json = serde_json::to_string(&record).expect("pending sign-in serializes to JSON");
        Secret::new(json)
    }

    pub fn from_secret(secret: &Secret) -> Result<Pending> {
        let record: PendingRecord = serde_json::from_str(secret.expose()).map_err(|e| {
            crate::error::Error::Credentials(format!("invalid pending secret: {e}"))
        })?;
        Ok(Pending {
            service: record.service,
            url: record.url,
            user_code: record.user_code,
            expires_at: record.expires_at,
            interval: record.interval,
            secret: Secret::new(record.secret),
        })
    }
}

/// The browser sign-in: a PKCE verifier and a random `state`, a loopback listener
/// (`127.0.0.1`, any free port for Google, `clients::DROPBOX_REDIRECT`'s port for Dropbox),
/// `open(url)`, one redirect answered with a short "signed in, you can close this tab" page,
/// then the code exchange. No redirect within `SIGN_IN_TIMEOUT` is `Error::SignInFailed`.
pub fn sign_in_browser(
    provider: &'static Provider,
    client: Client,
    transport: &dyn Transport,
    open: &dyn Fn(&str) -> Result<()>,
    now: Clock,
) -> Result<Tokens> {
    sign_in_browser_timeout(
        provider,
        client,
        transport,
        open,
        now,
        Duration::from_secs(SIGN_IN_TIMEOUT),
    )
}

/// A version of `sign_in_browser` taking an explicit redirect timeout (for tests).
pub fn sign_in_browser_timeout(
    provider: &'static Provider,
    client: Client,
    transport: &dyn Transport,
    open: &dyn Fn(&str) -> Result<()>,
    now: Clock,
    timeout: Duration,
) -> Result<Tokens> {
    let (listener, redirect_uri) = match provider.service {
        Service::Gdrive => {
            let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| {
                crate::error::Error::SignInFailed(format!("could not bind loopback socket: {e}"))
            })?;
            let port = listener
                .local_addr()
                .map_err(|e| {
                    crate::error::Error::SignInFailed(format!("could not get local address: {e}"))
                })?
                .port();
            (listener, format!("http://127.0.0.1:{port}/"))
        }
        Service::Dropbox => {
            let listener = TcpListener::bind("127.0.0.1:53710").map_err(|_| {
                crate::error::Error::SignInFailed("port 53710 is in use: use --no-browser".into())
            })?;
            (listener, crate::clients::DROPBOX_REDIRECT.to_string())
        }
        Service::Nextcloud => {
            return Err(crate::error::Error::Usage(
                "Nextcloud does not use OAuth".into(),
            ));
        }
    };

    listener.set_nonblocking(true).map_err(|e| {
        crate::error::Error::SignInFailed(format!("could not set nonblocking socket: {e}"))
    })?;

    let rand = random_bytes()?;
    let verifier = verifier_from_bytes(&rand);
    let state_bytes = random_bytes()?;
    let state = base64url(&state_bytes);
    let ch = challenge(&verifier);
    let auth_url = authorize_url(provider, client.id, &redirect_uri, &ch, &state);

    open(&auth_url)?;

    let start = std::time::Instant::now();
    let code = loop {
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if start.elapsed() >= timeout {
                        return Err(crate::error::Error::SignInFailed(
                            "timed out waiting for sign-in in browser".into(),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    return Err(crate::error::Error::SignInFailed(format!(
                        "listener accept failed: {e}"
                    )));
                }
            }
        };

        // Some platforms (BSD, macOS) hand out accepted sockets in the listener's non-blocking
        // mode, which would make the request read below fail before the browser has sent it.
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut reader = std::io::BufReader::new(&mut stream);
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = request_line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let target = parts[1];
        if target.starts_with("/favicon.ico") {
            let resp = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(resp);
            let _ = stream.flush();
            continue;
        }

        // Check the redirect before answering, so the page never claims a sign-in that failed.
        let code = match parse_redirect(target, &state) {
            Ok(code) => code,
            Err(e) => {
                let html = "<!DOCTYPE html><html><body><p>The sign-in did not complete. Run setup again.</p></body></html>";
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(),
                    html
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
                return Err(e);
            }
        };
        let html = "<!DOCTYPE html><html><body><p>Signed in to save-relay. You can close this tab.</p></body></html>";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
        break code;
    };

    let mut pairs = vec![
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &redirect_uri),
        ("code_verifier", &verifier),
        ("client_id", client.id),
    ];
    if provider.service == Service::Gdrive {
        // Like the device flow, send the secret only when one is registered (never `client_secret=`).
        if let Some(secret) = client.secret.filter(|s| !s.is_empty()) {
            pairs.push(("client_secret", secret));
        }
    }

    let body = form_encode(&pairs);
    let req = Request::new("POST", provider.token_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body.into_bytes());
    let resp = transport.send(&req)?;
    parse_token_response(provider.service, resp.status, &resp.text(), (now)())
}

/// Start a sign-in with no browser on this machine: Dropbox's code flow (the URL with no
/// `redirect_uri`) or Google's device flow (`device_url`). The caller shows `url` and
/// `user_code` and stores the `Pending`.
pub fn start_code(
    provider: &'static Provider,
    client: Client,
    transport: &dyn Transport,
    now: u64,
) -> Result<Pending> {
    match provider.service {
        Service::Dropbox => {
            let rand = random_bytes()?;
            let verifier = verifier_from_bytes(&rand);
            let state_bytes = random_bytes()?;
            let state = base64url(&state_bytes);
            let ch = challenge(&verifier);
            let url = authorize_url(provider, client.id, "", &ch, &state);
            Ok(Pending {
                service: Service::Dropbox,
                url,
                user_code: None,
                expires_at: now.saturating_add(SIGN_IN_TIMEOUT),
                interval: 0,
                secret: Secret::new(verifier),
            })
        }
        Service::Gdrive => {
            let device_url = provider.device_url.ok_or_else(|| {
                crate::error::Error::Usage("Google provider missing device_url".into())
            })?;
            let body = form_encode(&[("client_id", client.id), ("scope", provider.scope)]);
            let req = Request::new("POST", device_url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body.into_bytes());
            let resp = transport.send(&req)?;
            let status = resp.status;
            let text = resp.text();
            if (500..=599).contains(&status) {
                return Err(crate::error::Error::Server(format!(
                    "device endpoint: {status}"
                )));
            }
            let val: serde_json::Value = serde_json::from_str(&text)
                .map_err(|_| crate::error::Error::Server(format!("device endpoint: {status}")))?;
            if status != 200 {
                let err = val
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown error");
                let msg = match val
                    .get("error_description")
                    .and_then(serde_json::Value::as_str)
                {
                    Some(desc) => format!("{err}: {desc}"),
                    None => err.to_string(),
                };
                return Err(crate::error::Error::SignInFailed(msg));
            }
            let device_code = val
                .get("device_code")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    crate::error::Error::Server("device endpoint: missing device_code".into())
                })?;
            let user_code = val
                .get("user_code")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    crate::error::Error::Server("device endpoint: missing user_code".into())
                })?;
            let url = val
                .get("verification_url")
                .or_else(|| val.get("verification_uri"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    crate::error::Error::Server("device endpoint: missing verification_url".into())
                })?;
            let expires_in = val
                .get("expires_in")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(1800);
            let interval = val
                .get("interval")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(5);
            Ok(Pending {
                service: Service::Gdrive,
                url: url.to_string(),
                user_code: Some(user_code.to_string()),
                expires_at: now.saturating_add(expires_in),
                interval,
                secret: Secret::new(device_code),
            })
        }
        Service::Nextcloud => Err(crate::error::Error::Usage(
            "Nextcloud does not use OAuth".into(),
        )),
    }
}

/// Finish a `Pending` sign-in: Dropbox exchanges `typed_code` (the first line of standard
/// input); Google polls the token endpoint every `interval` seconds until the person
/// finished, refused (`Error::SignInFailed("access_denied")`) or the code expired
/// (`Error::SignInFailed("the code expired: run setup again")`).
pub fn finish_code(
    provider: &'static Provider,
    client: Client,
    pending: &Pending,
    typed_code: Option<&str>,
    transport: &dyn Transport,
    now: Clock,
) -> Result<Tokens> {
    finish_code_with_sleep(
        provider,
        client,
        pending,
        typed_code,
        transport,
        now,
        &|secs| {
            if secs > 0 {
                std::thread::sleep(Duration::from_secs(secs));
            }
        },
    )
}

/// A version of `finish_code` taking an explicit sleep callback (for tests).
pub fn finish_code_with_sleep(
    provider: &'static Provider,
    client: Client,
    pending: &Pending,
    typed_code: Option<&str>,
    transport: &dyn Transport,
    now: Clock,
    sleep: &dyn Fn(u64),
) -> Result<Tokens> {
    match pending.service {
        Service::Dropbox => {
            let code = typed_code
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| crate::error::Error::SignInFailed("no code provided".into()))?;

            if (now)() >= pending.expires_at {
                return Err(crate::error::Error::SignInFailed(
                    "the code expired: run setup again".into(),
                ));
            }

            let pairs = vec![
                ("grant_type", "authorization_code"),
                ("code", code),
                ("client_id", client.id),
                ("code_verifier", pending.secret.expose()),
            ];

            let body = form_encode(&pairs);
            let request = Request::new("POST", provider.token_url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body.into_bytes());

            let response = transport.send(&request)?;
            parse_token_response(Service::Dropbox, response.status, &response.text(), (now)())
        }
        Service::Gdrive => {
            // A server interval of 0 would poll with no pause: floor it at one second.
            let mut interval = pending.interval.max(1);
            loop {
                if (now)() >= pending.expires_at {
                    return Err(crate::error::Error::SignInFailed(
                        "the code expired: run setup again".into(),
                    ));
                }

                let mut pairs = vec![
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", pending.secret.expose()),
                    ("client_id", client.id),
                ];
                if let Some(secret) = client.secret {
                    if !secret.is_empty() {
                        pairs.push(("client_secret", secret));
                    }
                }

                let body = form_encode(&pairs);
                let request = Request::new("POST", provider.token_url)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(body.into_bytes());

                let response = transport.send(&request)?;
                let status = response.status;
                let text = response.text();

                if (500..=599).contains(&status) {
                    return Err(crate::error::Error::Server(format!(
                        "token endpoint: {status}"
                    )));
                }

                if status == 200 {
                    return parse_token_response(Service::Gdrive, status, &text, (now)());
                }

                let answer: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
                    crate::error::Error::Server(format!("token endpoint: {status}"))
                })?;

                match answer.get("error").and_then(serde_json::Value::as_str) {
                    Some("authorization_pending") => {
                        sleep(interval);
                    }
                    Some("slow_down") => {
                        interval = interval.saturating_add(5);
                        sleep(interval);
                    }
                    Some("access_denied") => {
                        return Err(crate::error::Error::SignInFailed("access_denied".into()));
                    }
                    Some("expired_token") => {
                        return Err(crate::error::Error::SignInFailed(
                            "the code expired: run setup again".into(),
                        ));
                    }
                    Some(err) => {
                        let message = match answer
                            .get("error_description")
                            .and_then(serde_json::Value::as_str)
                        {
                            Some(desc) => format!("{err}: {desc}"),
                            None => err.to_string(),
                        };
                        return Err(crate::error::Error::SignInFailed(message));
                    }
                    None => {
                        return Err(crate::error::Error::Server(format!(
                            "token endpoint: {status}"
                        )));
                    }
                }
            }
        }
        Service::Nextcloud => Err(crate::error::Error::Usage(
            "Nextcloud does not use OAuth".into(),
        )),
    }
}

/// True when this looks like Steam's Game Mode on Linux (`XDG_CURRENT_DESKTOP` is
/// `gamescope`, or `SteamGamepadUI` is set), read through `var` like `platform::PathVars`.
/// There a desktop browser may never appear, so setup defaults to the code flows.
pub fn in_game_mode(var: &dyn Fn(&str) -> Option<String>) -> bool {
    if let Some(desktop) = var("XDG_CURRENT_DESKTOP") {
        if desktop == "gamescope" {
            return true;
        }
    }
    if let Some(ui) = var("SteamGamepadUI") {
        if !ui.is_empty() {
            return true;
        }
    }
    false
}

/// Open `url` in the system browser: `xdg-open` on Linux, `rundll32 url.dll,FileProtocolHandler`
/// on Windows (no `cmd` quoting of `&`), `open` on macOS. Failing to start one is
/// `Error::SignInFailed("could not open a browser: use --no-browser")`.
pub fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };

    cmd.spawn().map(|_| ()).map_err(|_| {
        crate::error::Error::SignInFailed("could not open a browser: use --no-browser".into())
    })
}
