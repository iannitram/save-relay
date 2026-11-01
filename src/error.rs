//! The one error type. Every variant has a stable `code()`, which is the `error.code` of the
//! `--json` envelope (docs/code-map.md, API shape). Built in the skeleton: every module uses it.

use std::fmt;

/// What a stub returns, so tests and callers can tell "not built yet" apart.
/// The string names the function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotBuilt(pub &'static str);

impl fmt::Display for NotBuilt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not built yet: {}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A stub (skeleton only).
    NotBuilt(NotBuilt),
    /// The command line was wrong.
    Usage(String),
    /// A manifest failed to parse or validate.
    ManifestInvalid(String),
    /// A game id with no `--manifest` and no remembered manifest on this machine.
    GameUnknown(String),
    /// `save-relay setup` has not been run on this machine.
    NotSetUp,
    /// The server refused the user name or app password (HTTP 401/403).
    AuthFailed,
    /// The server could not be reached (DNS, connect, TLS, timeout).
    Unreachable(String),
    /// The server answered, but not as WebDAV should (unexpected status or body).
    Server(String),
    /// A state file (the server's `state.json` or a last-synced record) is unreadable.
    StateInvalid(String),
    /// The server's state changed between read and write (HTTP 412): another machine pushed.
    Changed,
    /// `restore` named a history entry that does not exist.
    NoSuchEntry(String),
    /// A remote path does not exist.
    NotFound(String),
    /// The system credential store failed or holds no password for this server.
    Credentials(String),
    /// A local file operation failed.
    Io(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// The stable code for `--json` output. Launchers match on these; never rename one.
    pub fn code(&self) -> &'static str {
        match self {
            Error::NotBuilt(_) => "not_built",
            Error::Usage(_) => "usage",
            Error::ManifestInvalid(_) => "manifest_invalid",
            Error::GameUnknown(_) => "game_unknown",
            Error::NotSetUp => "not_set_up",
            Error::AuthFailed => "auth_failed",
            Error::Unreachable(_) => "server_unreachable",
            Error::Server(_) => "server_error",
            Error::StateInvalid(_) => "state_invalid",
            Error::Changed => "changed_on_server",
            Error::NoSuchEntry(_) => "no_such_entry",
            Error::NotFound(_) => "not_found",
            Error::Credentials(_) => "credentials",
            Error::Io(_) => "io",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotBuilt(n) => write!(f, "{n}"),
            Error::Usage(m) => write!(f, "{m}"),
            Error::ManifestInvalid(m) => write!(f, "manifest: {m}"),
            Error::GameUnknown(g) => write!(
                f,
                "no manifest known for game '{g}' on this machine: pass --manifest <file>"
            ),
            Error::NotSetUp => write!(f, "this machine is not set up: run `save-relay setup`"),
            Error::AuthFailed => write!(f, "the server refused the user name or app password"),
            Error::Unreachable(m) => write!(f, "the server is unreachable: {m}"),
            Error::Server(m) => write!(f, "server error: {m}"),
            Error::StateInvalid(m) => write!(f, "state file: {m}"),
            Error::Changed => write!(
                f,
                "another machine changed the saves on the server meanwhile"
            ),
            Error::NoSuchEntry(e) => write!(f, "no history entry '{e}'"),
            Error::NotFound(p) => write!(f, "not found on the server: {p}"),
            Error::Credentials(m) => write!(f, "credential store: {m}"),
            Error::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<NotBuilt> for Error {
    fn from(n: NotBuilt) -> Error {
        Error::NotBuilt(n)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e.to_string())
    }
}

/// `Err(Error::NotBuilt(..))` for a stub.
pub(crate) fn not_built<T>(what: &'static str) -> Result<T> {
    Err(Error::NotBuilt(NotBuilt(what)))
}
