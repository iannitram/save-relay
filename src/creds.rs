//! The app password (Nextcloud) or refresh token (Google Drive, Dropbox), kept in the system's
//! credential store and nowhere else (SPEC.md M6): never in a file, a log, an error message or
//! `--json` output. A Nextcloud password is filed under `account`, a refresh token under
//! `service::token_account`, a sign-in waiting for `setup --finish` under
//! `"pending:<service key>"` (`oauth::Pending`). `KeyringStore` (Secret Service on
//! Linux, Credential Manager on Windows, Keychain on macOS) is behind the `keyring` feature;
//! `MemoryStore` is for tests; a mobile build brings its own `CredentialStore`.

use crate::error::{Error, Result};
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

/// The service name every stored password is filed under.
pub const SERVICE: &str = "save-relay";

/// A password or token. `Debug` and `Display` print `***`, so it cannot leak through a log line.
/// On purpose: the redaction must exist before anything can print one.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    /// The password itself, for the HTTP client only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

pub trait CredentialStore {
    /// The password for `account`, or `None` when none is stored.
    fn get(&self, account: &str) -> Result<Option<Secret>>;
    /// Store (or replace) the password for `account`.
    fn set(&self, account: &str, secret: &Secret) -> Result<()>;
    /// Remove it; removing a missing one is `Ok`.
    fn delete(&self, account: &str) -> Result<()>;
    /// A short name for `setup`'s output: "memory", "secret-service", "windows-credential-manager",
    /// "macos-keychain".
    fn kind(&self) -> &'static str;
}

/// The account a password is filed under: `"<user>@<host>"`, host lowercased and with any port
/// kept, from the server URL: `account("https://Cloud.Example.org:8443/nc", "me")` ->
/// `"me@cloud.example.org:8443"`. A URL without `http://` or `https://` is `Error::Usage`.
pub fn account(server: &str, user: &str) -> Result<String> {
    let host = server
        .strip_prefix("https://")
        .or_else(|| server.strip_prefix("http://"))
        .ok_or_else(|| Error::Usage("server URL must start with http:// or https://".into()))?
        .split('/')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    if host.is_empty() {
        return Err(Error::Usage("server URL must include a host".into()));
    }
    Ok(format!("{user}@{host}"))
}

/// Passwords in a map, for tests. Interior mutability, used through `&self`.
#[derive(Debug, Default)]
pub struct MemoryStore {
    secrets: Mutex<HashMap<String, Secret>>,
}

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }
}

impl CredentialStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<Secret>> {
        Ok(self
            .secrets
            .lock()
            .map_err(|_| Error::Credentials("memory store lock poisoned".into()))?
            .get(account)
            .cloned())
    }
    fn set(&self, account: &str, secret: &Secret) -> Result<()> {
        self.secrets
            .lock()
            .map_err(|_| Error::Credentials("memory store lock poisoned".into()))?
            .insert(account.to_owned(), secret.clone());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<()> {
        self.secrets
            .lock()
            .map_err(|_| Error::Credentials("memory store lock poisoned".into()))?
            .remove(account);
        Ok(())
    }
    fn kind(&self) -> &'static str {
        "memory"
    }
}

/// The variable the integration tests set (issue #23): a debug build uses its value as the
/// password for every account, so the built binary runs on a CI runner with no keyring. A
/// release build never reads it.
pub const TEST_PASSWORD_VAR: &str = "SAVE_RELAY_TEST_PASSWORD";

/// The password from `TEST_PASSWORD_VAR`, in a debug build only; `None` otherwise, and when the
/// variable is unset or empty.
#[cfg(debug_assertions)]
pub fn test_password_from_env(var: &dyn Fn(&str) -> Option<String>) -> Option<Secret> {
    var(TEST_PASSWORD_VAR)
        .filter(|value| !value.is_empty())
        .map(Secret::new)
}

/// A release build has no test override.
#[cfg(not(debug_assertions))]
pub fn test_password_from_env(_: &dyn Fn(&str) -> Option<String>) -> Option<Secret> {
    None
}

/// The test override as a store: every account has the one password, and `set` and `delete`
/// write nothing, because the environment is the store. Nothing reaches a file.
pub struct EnvPasswordStore {
    password: Secret,
}

impl EnvPasswordStore {
    pub fn new(password: Secret) -> EnvPasswordStore {
        EnvPasswordStore { password }
    }
}

impl CredentialStore for EnvPasswordStore {
    fn get(&self, _: &str) -> Result<Option<Secret>> {
        Ok(Some(self.password.clone()))
    }
    fn set(&self, _: &str, _: &Secret) -> Result<()> {
        Ok(())
    }
    fn delete(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn kind(&self) -> &'static str {
        "test-environment"
    }
}

/// The system credential store, through the `keyring` crate: Secret Service on Linux (pure-Rust
/// zbus backend), Credential Manager on Windows, Keychain on macOS. Each account is one entry
/// under the service name `SERVICE`.
#[cfg(feature = "keyring")]
#[derive(Debug, Default)]
pub struct KeyringStore {
    _private: (),
}

#[cfg(feature = "keyring")]
impl KeyringStore {
    pub fn new() -> KeyringStore {
        KeyringStore::default()
    }
}

/// An error for the user, built from the failure's kind only: never the crate's own text, which
/// could carry bytes of the stored value (`BadEncoding`).
#[cfg(feature = "keyring")]
fn credentials_error(what: &str, err: &keyring::Error) -> Error {
    use keyring::Error as K;
    let why = match err {
        K::NoDefaultStore => "no credential store is available on this machine",
        K::NoStorageAccess(_) => "the credential store is locked or cannot be reached",
        K::PlatformFailure(_) => "the credential store reported a failure",
        K::TooLong(..) => "the value is too long for the credential store",
        K::BadEncoding(_) | K::BadDataFormat(..) | K::BadStoreFormat(_) => {
            "the stored value is not readable"
        }
        K::Ambiguous(_) => "more than one entry matches this account",
        _ => "the credential store refused the request",
    };
    Error::Credentials(format!("{what}: {why}"))
}

#[cfg(feature = "keyring")]
fn entry(account: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, account).map_err(|e| credentials_error("open entry", &e))
}

#[cfg(feature = "keyring")]
impl CredentialStore for KeyringStore {
    fn get(&self, account: &str) -> Result<Option<Secret>> {
        match entry(account)?.get_password() {
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(credentials_error("read", &e)),
        }
    }
    fn set(&self, account: &str, secret: &Secret) -> Result<()> {
        entry(account)?
            .set_password(secret.expose())
            .map_err(|e| credentials_error("store", &e))
    }
    fn delete(&self, account: &str) -> Result<()> {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(credentials_error("delete", &e)),
        }
    }
    fn kind(&self) -> &'static str {
        if cfg!(target_os = "windows") {
            "windows-credential-manager"
        } else if cfg!(target_os = "macos") {
            "macos-keychain"
        } else {
            "secret-service"
        }
    }
}

#[cfg(all(test, feature = "keyring"))]
mod tests {
    use super::*;

    #[test]
    fn kind_names_the_platform_store() {
        let k = KeyringStore::new().kind();
        assert!([
            "secret-service",
            "windows-credential-manager",
            "macos-keychain"
        ]
        .contains(&k));
    }

    #[test]
    fn error_text_has_no_secret() {
        let e = credentials_error("read", &keyring::Error::BadEncoding(b"hunter2".to_vec()));
        assert!(!e.to_string().contains("hunter2"));
    }

    /// Needs a real, unlocked credential store: run by hand on each OS with
    /// `cargo test --lib -- --ignored keyring_round_trip`.
    #[test]
    #[ignore]
    fn keyring_round_trip() {
        let store = KeyringStore::new();
        let account = format!("save-relay-test-{}@throwaway.invalid", std::process::id());
        assert!(store.get(&account).unwrap().is_none());
        store.set(&account, &Secret::new("first")).unwrap();
        assert_eq!(store.get(&account).unwrap().unwrap().expose(), "first");
        store.set(&account, &Secret::new("second")).unwrap();
        assert_eq!(store.get(&account).unwrap().unwrap().expose(), "second");
        store.delete(&account).unwrap();
        assert!(store.get(&account).unwrap().is_none());
        store.delete(&account).unwrap();
    }
}
