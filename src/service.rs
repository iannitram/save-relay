//! Which service holds the saves: chosen once at setup, the same on every machine (SPEC.md M6),
//! recorded in `local::Config`, and named by `status`. Every service is a `Remote`, so the sync
//! engine never knows which one it talks to (M9, docs/adr/0002-storage-backends.md).
//!
//! The config keeps format 1's shape: its `server` field holds the Nextcloud address
//! (`https://...`), or the service key `gdrive` or `dropbox` for those services.

use crate::creds::CredentialStore;
use crate::error::{not_built, Result};
use crate::http::Transport;
use crate::local::Config;
use crate::remote::Remote;
use crate::stamp::Clock;
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
        crate::not_built!("service::Service::key")
    }

    /// The name shown to a person: "Nextcloud", "Google Drive", "Dropbox".
    pub fn label(self) -> &'static str {
        crate::not_built!("service::Service::label")
    }

    /// The service for a `--service` value: exactly a key above. Anything else is
    /// `Error::Usage("unknown service '<text>': use nextcloud, gdrive or dropbox")`.
    pub fn parse(text: &str) -> Result<Service> {
        not_built("service::Service::parse")
    }

    /// The service a config's `server` field names: `http://` or `https://` at the start is
    /// Nextcloud; exactly `gdrive` or `dropbox` is that service; anything else is
    /// `Error::StateInvalid("config: unknown server '<server>'")`.
    pub fn from_server(server: &str) -> Result<Service> {
        not_built("service::Service::from_server")
    }

    /// Signs in through the browser (Google Drive, Dropbox) rather than with an app password.
    pub fn uses_oauth(self) -> bool {
        crate::not_built!("service::Service::uses_oauth")
    }
}

/// The credential-store account a service's refresh token is filed under:
/// `"<key>:<user>"`, e.g. `token_account(Service::Gdrive, "me@example.org")` ->
/// `"gdrive:me@example.org"`. Nextcloud keeps an app password under `creds::account` instead,
/// so `Service::Nextcloud` is `Error::Usage`; an empty `user` is `Error::Usage` too.
pub fn token_account(service: Service, user: &str) -> Result<String> {
    not_built("service::token_account")
}

/// Open the service the config names, as a `Remote`: `webdav::WebDav` with the app password,
/// or `dropbox::Dropbox` / `gdrive::Drive` with a `oauth::TokenSource` over the stored
/// refresh token. No credential stored is `Error::NotSetUp`.
pub fn open(
    config: &Config,
    store: &dyn CredentialStore,
    transport: Arc<dyn Transport>,
    clock: Clock,
) -> Result<Box<dyn Remote>> {
    not_built("service::open")
}
