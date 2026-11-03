//! Which service holds the saves: chosen once at setup, the same on every machine (SPEC.md M6),
//! recorded in `local::Config`, and named by `status`. Every service is a `Remote`, so the sync
//! engine never knows which one it talks to (M9, docs/adr/0002-storage-backends.md).
//!
//! The config keeps format 1's shape: its `server` field holds the Nextcloud address
//! (`https://...`), or the service key `gdrive` or `dropbox` for those services.

use crate::clients::{Client, ClientSet};
use crate::creds::{self, CredentialStore, Secret};
use crate::dropbox::Dropbox;
use crate::error::{Error, Result};
use crate::gdrive::Drive;
use crate::http::Transport;
use crate::local::Config;
use crate::oauth::{self, TokenSource};
use crate::remote::Remote;
use crate::stamp::Clock;
use crate::webdav::{self, WebDav};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    /// The person's own Nextcloud, over WebDAV, with an app password.
    Nextcloud,
    /// Google Drive, signed in through the browser, `drive.file` access only (M10).
    Gdrive,
    /// Dropbox, signed in through the browser, an App folder only (M10).
    Dropbox,
}

impl Service {
    /// Every service, in the order setup offers them.
    pub const ALL: [Service; 3] = [Service::Nextcloud, Service::Gdrive, Service::Dropbox];

    /// The key used in `--service`, the config and `--json`: "nextcloud", "gdrive", "dropbox".
    pub fn key(self) -> &'static str {
        match self {
            Service::Nextcloud => "nextcloud",
            Service::Gdrive => "gdrive",
            Service::Dropbox => "dropbox",
        }
    }

    /// The name shown to a person: "Nextcloud", "Google Drive", "Dropbox".
    pub fn label(self) -> &'static str {
        match self {
            Service::Nextcloud => "Nextcloud",
            Service::Gdrive => "Google Drive",
            Service::Dropbox => "Dropbox",
        }
    }

    /// The service for a `--service` value: exactly a key above. Anything else is
    /// `Error::Usage("unknown service '<text>': use nextcloud, gdrive or dropbox")`.
    pub fn parse(text: &str) -> Result<Service> {
        match text {
            "nextcloud" => Ok(Service::Nextcloud),
            "gdrive" => Ok(Service::Gdrive),
            "dropbox" => Ok(Service::Dropbox),
            _ => Err(crate::Error::Usage(format!(
                "unknown service '{text}': use nextcloud, gdrive or dropbox"
            ))),
        }
    }

    /// The service a config's `server` field names: `http://` or `https://` at the start is
    /// Nextcloud; exactly `gdrive` or `dropbox` is that service; anything else is
    /// `Error::StateInvalid("config: unknown server '<server>'")`.
    pub fn from_server(server: &str) -> Result<Service> {
        match server {
            s if s.starts_with("http://") || s.starts_with("https://") => Ok(Service::Nextcloud),
            "gdrive" => Ok(Service::Gdrive),
            "dropbox" => Ok(Service::Dropbox),
            _ => Err(crate::Error::StateInvalid(format!(
                "config: unknown server '{server}'"
            ))),
        }
    }

    /// Signs in through the browser (Google Drive, Dropbox) rather than with an app password.
    pub fn uses_oauth(self) -> bool {
        matches!(self, Service::Gdrive | Service::Dropbox)
    }
}

/// The credential-store account a service's refresh token is filed under:
/// `"<key>:<user>"`, e.g. `token_account(Service::Gdrive, "me@example.org")` ->
/// `"gdrive:me@example.org"`. Nextcloud keeps an app password under `creds::account` instead,
/// so `Service::Nextcloud` is `Error::Usage`; an empty `user` is `Error::Usage` too.
pub fn token_account(service: Service, user: &str) -> Result<String> {
    if service == Service::Nextcloud {
        return Err(crate::Error::Usage(
            "Nextcloud uses an app password, not a refresh token".into(),
        ));
    }
    if user.is_empty() {
        return Err(crate::Error::Usage("user must not be empty".into()));
    }
    Ok(format!("{}:{user}", service.key()))
}

/// A backend opened for a command, and the token source behind it (Google Drive and Dropbox
/// only). After the command, the caller stores `tokens.rotated()` when it is `Some`.
pub struct Opened {
    pub remote: Box<dyn Remote>,
    pub tokens: Option<TokenSource>,
}

/// The credential-store account a service's pending sign-in (`oauth::Pending`) waits under
/// between `setup --no-browser` and `setup --finish`: `"pending:<key>"`.
pub fn pending_account(service: Service) -> String {
    format!("pending:{}", service.key())
}

/// The account that records which Google client a refresh token was issued to: `"gdrive-client:<user>"`,
/// holding `"device"` (the TVs and limited-input client) or `"desktop"`. A refresh token only works
/// with the client that issued it, so `open` needs this to pick the right id.
pub fn google_client_account(user: &str) -> String {
    format!("gdrive-client:{user}")
}

/// Every credential-store account the config's sign-in lives under: the Nextcloud app password,
/// or a Google Drive or Dropbox refresh token with its bookkeeping (and any pending sign-in).
/// Switching services deletes these (docs/adr/0002-storage-backends.md: switching starts fresh).
pub fn credential_accounts(config: &Config) -> Result<Vec<String>> {
    let service = Service::from_server(&config.server)?;
    Ok(match service {
        Service::Nextcloud => vec![creds::account(&config.server, &config.user)?],
        Service::Gdrive => vec![
            token_account(service, &config.user)?,
            google_client_account(&config.user),
            pending_account(service),
        ],
        Service::Dropbox => vec![
            token_account(service, &config.user)?,
            pending_account(service),
        ],
    })
}

/// Open the service the config names, as a `Remote`: `webdav::WebDav` with the app password,
/// or `dropbox::Dropbox` / `gdrive::Drive` with a `oauth::TokenSource` over the stored
/// refresh token. No credential stored is `Error::NotSetUp`. The clients are this build's
/// (`ClientSet::shipped`); `open_with` takes others.
pub fn open(
    config: &Config,
    store: &dyn CredentialStore,
    transport: Arc<dyn Transport>,
    clock: Clock,
) -> Result<Box<dyn Remote>> {
    open_with(config, store, transport, clock, &ClientSet::shipped()).map(|opened| opened.remote)
}

/// `open` with the client ids passed in, and the token source returned for `rotated()`.
pub fn open_with(
    config: &Config,
    store: &dyn CredentialStore,
    transport: Arc<dyn Transport>,
    clock: Clock,
    clients: &ClientSet,
) -> Result<Opened> {
    let service = Service::from_server(&config.server)?;
    if !service.uses_oauth() {
        let url = webdav::files_url(&config.server, &config.user)?;
        let account = creds::account(&config.server, &config.user)?;
        let password = store.get(&account)?.ok_or(Error::NotSetUp)?;
        let remote = WebDav::new(&url, &config.user, password, transport);
        return Ok(Opened {
            remote: Box::new(remote),
            tokens: None,
        });
    }

    let refresh = store
        .get(&token_account(service, &config.user)?)?
        .ok_or(Error::NotSetUp)?;
    let kind = match service {
        Service::Gdrive => store.get(&google_client_account(&config.user))?,
        _ => None,
    };
    let client = client_for(service, clients, kind.as_ref().map(Secret::expose))?;
    let provider =
        oauth::provider(service).ok_or_else(|| Error::Usage("no OAuth provider".into()))?;
    let tokens = TokenSource::new(provider, client, refresh, transport.clone(), clock);
    let handle = tokens.clone();
    Ok(Opened {
        remote: backend(service, tokens, transport, clock)?,
        tokens: Some(handle),
    })
}

/// The `Remote` of an OAuth service over `tokens`.
fn backend(
    service: Service,
    tokens: TokenSource,
    transport: Arc<dyn Transport>,
    clock: Clock,
) -> Result<Box<dyn Remote>> {
    match service {
        Service::Gdrive => Ok(Box::new(Drive::new(tokens, transport, clock))),
        Service::Dropbox => Ok(Box::new(Dropbox::new(tokens, transport))),
        Service::Nextcloud => Err(Error::Usage("Nextcloud does not use OAuth".into())),
    }
}

/// The signed-in account's e-mail address, and the backend over `tokens`, straight after a
/// sign-in: `setup` needs the address for the config and the account's folder for `ensure_dir`.
pub fn sign_in_account(
    service: Service,
    tokens: TokenSource,
    transport: Arc<dyn Transport>,
    clock: Clock,
) -> Result<(String, Box<dyn Remote>)> {
    match service {
        Service::Gdrive => {
            let drive = Drive::new(tokens, transport, clock);
            let email = drive.account_email()?;
            Ok((email, Box::new(drive)))
        }
        Service::Dropbox => {
            let dropbox = Dropbox::new(tokens, transport);
            let email = dropbox.account_email()?;
            Ok((email, Box::new(dropbox)))
        }
        Service::Nextcloud => Err(Error::Usage("Nextcloud does not use OAuth".into())),
    }
}

/// The client a service's tokens are issued to: Dropbox's, or for Google Drive the device one
/// when the sign-in recorded `"device"` (`google_client_account`), else the desktop one.
pub fn client_for(
    service: Service,
    clients: &ClientSet,
    google_kind: Option<&str>,
) -> Result<Client> {
    match service {
        Service::Dropbox => Ok(clients.dropbox),
        Service::Gdrive if google_kind == Some("device") => Ok(clients.google_device),
        Service::Gdrive => Ok(clients.google_desktop),
        Service::Nextcloud => Err(Error::Usage("Nextcloud does not use OAuth".into())),
    }
}
