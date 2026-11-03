//! The app registrations save-relay signs in with on Google and Dropbox. They are public by
//! design: a desktop app cannot keep a secret, so PKCE (`oauth`) protects each sign-in instead.
//! Google calls its value for installed apps a "client secret" but documents that it is not
//! treated as one for desktop clients; Dropbox's PKCE flow needs only the app key.
//!
//! The registrations are made under the project owner's accounts (the issue labelled
//! `needs-owner-device`, "Register the save-relay app with Google and Dropbox"), and the values
//! are pasted here. Empty means not registered yet: `setup` for that service says so
//! (`Error::Usage`). A fork can build with its own registration by setting the
//! `SAVE_RELAY_GOOGLE_*` / `SAVE_RELAY_DROPBOX_APP_KEY` variables at build time.

use crate::error::{Error, Result};

/// A registration: its id, and the secret when the service needs one sent (Google).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Client {
    pub id: &'static str,
    pub secret: Option<&'static str>,
}

const fn or(value: Option<&'static str>, fallback: &'static str) -> &'static str {
    match value {
        Some(v) => v,
        None => fallback,
    }
}

/// Google Cloud OAuth client of type "Desktop app" (browser sign-in, loopback redirect).
pub const GOOGLE_DESKTOP: Client = Client {
    id: or(option_env!("SAVE_RELAY_GOOGLE_DESKTOP_ID"), ""),
    secret: Some(or(option_env!("SAVE_RELAY_GOOGLE_DESKTOP_SECRET"), "")),
};

/// Google Cloud OAuth client of type "TVs and Limited Input devices" (the code shown on screen
/// and typed on a phone, for Steam's Game Mode).
pub const GOOGLE_DEVICE: Client = Client {
    id: or(option_env!("SAVE_RELAY_GOOGLE_DEVICE_ID"), ""),
    secret: Some(or(option_env!("SAVE_RELAY_GOOGLE_DEVICE_SECRET"), "")),
};

/// Dropbox app key (App Console, a scoped app with App folder access, public PKCE clients on).
pub const DROPBOX: Client = Client {
    id: or(option_env!("SAVE_RELAY_DROPBOX_APP_KEY"), ""),
    secret: None,
};

/// The three registrations as one value, so `cli::Net` can carry them: `shipped()` is the build's
/// own, and tests pass ids of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientSet {
    pub google_desktop: Client,
    pub google_device: Client,
    pub dropbox: Client,
}

impl ClientSet {
    /// The registrations this build was made with (`GOOGLE_DESKTOP`, `GOOGLE_DEVICE`, `DROPBOX`).
    pub const fn shipped() -> ClientSet {
        ClientSet {
            google_desktop: GOOGLE_DESKTOP,
            google_device: GOOGLE_DEVICE,
            dropbox: DROPBOX,
        }
    }
}

/// `client` when it has an app id; else `Error::Usage` saying the app is not registered yet, so
/// `setup` never starts a sign-in it cannot finish. `label` is the service's name.
pub fn registered(client: Client, label: &str) -> Result<Client> {
    if client.id.is_empty() {
        return Err(Error::Usage(format!(
            "{label} is not registered yet: the save-relay app has no app id in this build"
        )));
    }
    Ok(client)
}

/// The loopback redirect URI registered for Dropbox. Google desktop clients accept any
/// `http://127.0.0.1:<port>` and need no registered URI; Dropbox needs this exact one.
pub const DROPBOX_REDIRECT: &str = "http://127.0.0.1:53710/";
