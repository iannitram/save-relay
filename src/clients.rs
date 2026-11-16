//! The app registrations save-relay signs in with on Google and Dropbox. They are public by
//! design: a desktop app cannot keep a secret, so PKCE (`oauth`) protects each sign-in instead.
//! Google calls its value for installed apps a "client secret" but documents that it is not
//! treated as one for desktop clients; Dropbox's PKCE flow needs only the app key.
//!
//! The registrations are made under the project owner's accounts (#36, "Register the save-relay
//! app with Google and Dropbox"). The two ids and the Dropbox app key are pasted here as the
//! fallbacks. The two Google secrets are not: GitHub's secret scanning refuses them in the
//! repository (owner ruling, 2026-11-16), so they come only from the repository's Actions secrets
//! `SAVE_RELAY_GOOGLE_DESKTOP_SECRET` and `SAVE_RELAY_GOOGLE_DEVICE_SECRET` at build time, and a
//! build without them has Dropbox and Nextcloud but no Google Drive. A fork can build with its
//! own registration by setting the `SAVE_RELAY_GOOGLE_*` / `SAVE_RELAY_DROPBOX_APP_KEY`
//! variables; an empty variable keeps the value here. A `Client` with an empty id, or with an
//! empty secret where it needs one, is not registered: `setup` for that service says so
//! (`Error::Usage`).

use crate::error::{Error, Result};

/// A registration: its id, and the secret when the service needs one sent (Google).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Client {
    pub id: &'static str,
    pub secret: Option<&'static str>,
}

/// The build-time value when it is set and not empty, else the registration pasted here. A CI
/// variable that is not defined still reaches cargo as an empty string, so empty falls back too.
const fn or(value: Option<&'static str>, fallback: &'static str) -> &'static str {
    match value {
        Some(v) if !v.is_empty() => v,
        _ => fallback,
    }
}

/// Google Cloud OAuth client of type "Desktop app" (browser sign-in, loopback redirect).
pub const GOOGLE_DESKTOP: Client = Client {
    id: or(
        option_env!("SAVE_RELAY_GOOGLE_DESKTOP_ID"),
        "388452691247-j3m8hdsqv7nn4l047huauji97oetokds.apps.googleusercontent.com",
    ),
    secret: Some(or(option_env!("SAVE_RELAY_GOOGLE_DESKTOP_SECRET"), "")),
};

/// Google Cloud OAuth client of type "TVs and Limited Input devices" (the code shown on screen
/// and typed on a phone, for Steam's Game Mode).
pub const GOOGLE_DEVICE: Client = Client {
    id: or(
        option_env!("SAVE_RELAY_GOOGLE_DEVICE_ID"),
        "388452691247-fdli5t7tbrtikr9s7bvgu394874vas29.apps.googleusercontent.com",
    ),
    secret: Some(or(option_env!("SAVE_RELAY_GOOGLE_DEVICE_SECRET"), "")),
};

/// Dropbox app key (App Console, a scoped app with App folder access, public PKCE clients on).
pub const DROPBOX: Client = Client {
    id: or(option_env!("SAVE_RELAY_DROPBOX_APP_KEY"), "p8xhxihp8s3gb33"),
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

/// `client` when it has an app id and, where it needs one, a secret; else `Error::Usage` saying
/// the app is not registered yet, so `setup` never starts a sign-in it cannot finish. `label` is
/// the service's name.
pub fn registered(client: Client, label: &str) -> Result<Client> {
    if client.id.is_empty() || client.secret.is_some_and(str::is_empty) {
        return Err(Error::Usage(format!(
            "{label} is not registered yet: the save-relay app has no app id or secret in this build"
        )));
    }
    Ok(client)
}

/// The loopback redirect URI registered for Dropbox. Google desktop clients accept any
/// `http://127.0.0.1:<port>` and need no registered URI; Dropbox needs this exact one.
pub const DROPBOX_REDIRECT: &str = "http://127.0.0.1:53710/";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_missing_override_keeps_the_pasted_registration() {
        assert_eq!(or(None, "pasted"), "pasted");
        assert_eq!(or(Some(""), "pasted"), "pasted");
        assert_eq!(or(Some("fork"), "pasted"), "fork");
    }

    #[test]
    fn the_shipped_build_has_the_pasted_ids_and_dropbox_is_registered() {
        let shipped = ClientSet::shipped();
        assert!(!shipped.google_desktop.id.is_empty());
        assert!(!shipped.google_device.id.is_empty());
        assert!(registered(shipped.dropbox, "Dropbox").is_ok());
    }

    #[test]
    fn a_google_client_with_an_empty_secret_is_not_registered() {
        let client = Client {
            id: "an-id",
            secret: Some(""),
        };
        let error = registered(client, "Google Drive").unwrap_err();
        assert!(error.to_string().contains("not registered yet"), "{error}");
        let with_secret = Client {
            id: "an-id",
            secret: Some("a-secret"),
        };
        assert!(registered(with_secret, "Google Drive").is_ok());
    }
}
