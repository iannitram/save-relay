//! The app password, kept in the system's credential store and nowhere else (SPEC.md M6): never
//! in a file, a log, an error message or `--json` output. `KeyringStore` (Secret Service on
//! Linux, Credential Manager on Windows, Keychain on macOS) is behind the `keyring` feature;
//! `MemoryStore` is for tests; a mobile build brings its own `CredentialStore`.

use crate::error::{not_built, Result};
use std::fmt;

/// The service name every stored password is filed under.
pub const SERVICE: &str = "save-relay";

/// A password. `Debug` and `Display` print `***`, so it cannot leak through a log line.
/// Built in the skeleton, on purpose: the redaction must exist before anything can print one.
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
    not_built("creds::account")
}

/// Passwords in a map, for tests. Interior mutability, used through `&self`.
#[derive(Debug, Default)]
pub struct MemoryStore {
    _private: (),
}

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }
}

impl CredentialStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<Secret>> {
        not_built("creds::MemoryStore::get")
    }
    fn set(&self, account: &str, secret: &Secret) -> Result<()> {
        not_built("creds::MemoryStore::set")
    }
    fn delete(&self, account: &str) -> Result<()> {
        not_built("creds::MemoryStore::delete")
    }
    fn kind(&self) -> &'static str {
        "memory"
    }
}

/// The system credential store, through the `keyring` crate (added by its issue).
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

#[cfg(feature = "keyring")]
impl CredentialStore for KeyringStore {
    fn get(&self, account: &str) -> Result<Option<Secret>> {
        not_built("creds::KeyringStore::get")
    }
    fn set(&self, account: &str, secret: &Secret) -> Result<()> {
        not_built("creds::KeyringStore::set")
    }
    fn delete(&self, account: &str) -> Result<()> {
        not_built("creds::KeyringStore::delete")
    }
    fn kind(&self) -> &'static str {
        crate::not_built!("creds::KeyringStore::kind")
    }
}
