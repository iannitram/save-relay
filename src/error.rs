//! The one error type. Every variant has a stable `code()`, which is the `error.code` of the
//! `--json` envelope (docs/code-map.md, API shape). The `Result` alias below returns it.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The command line was wrong.
    Usage(String),
    /// A manifest failed to parse or validate.
    ManifestInvalid(String),
    /// A game id with no `--manifest` and no remembered manifest on this machine.
    GameUnknown(String),
    /// `save-relay setup` has not been run on this machine.
    NotSetUp,
    /// The Nextcloud server refused the user name or app password (HTTP 401/403).
    AuthFailed,
    /// The server could not be reached (DNS, connect, TLS, timeout).
    Unreachable(String),
    /// The server answered, but not as its API should (unexpected status or body).
    Server(String),
    /// A state file (the server's `state.json` or a last-synced record) is unreadable.
    StateInvalid(String),
    /// The server's state changed between read and write (HTTP 412): another machine pushed.
    Changed,
    /// A refused write is associated with a stranded Nextcloud revision claim.
    ChangedRepair(String),
    /// Drive refused creation of the state-named claim (409), then confirmed it was absent (404).
    /// This is evidence for an explicit, local-only recovery flow; it is still a failed CAS.
    MissingDriveClaim {
        path: String,
        file_id: String,
        version: u64,
        claim: String,
    },
    /// `restore` named a history entry that does not exist.
    NoSuchEntry(String),
    /// A remote path does not exist.
    NotFound(String),
    /// The system credential store failed or holds no password for this server.
    Credentials(String),
    /// Google Drive or Dropbox refused the stored sign-in (the refresh token expired or was
    /// withdrawn): sign in again with `save-relay setup`. The string is the service's label.
    ReauthNeeded(String),
    /// A sign-in in the browser during `setup` did not finish: refused, timed out, or the
    /// answer did not match the request.
    SignInFailed(String),
    /// A wrapped game has no clear saves folder (M13): no entry for its Steam app id in the
    /// ludusavi data, an entry with no save paths for this OS, or more than one folder that
    /// could be it. Never guessed. The string says which, and what was looked at.
    SaveLocationUnclear(String),
    /// No Steam installation was found (`steam`); the string names the folders looked at.
    SteamNotFound(String),
    /// `install-wrap` found Steam running: Steam rewrites `localconfig.vdf` when it quits, so
    /// the change would be lost. save-relay never closes Steam itself.
    SteamRunning,
    /// `answer` named a game with no pending choice on this machine (`pending`).
    NoPendingChoice(String),
    /// A local file operation failed.
    Io(String),
    /// Game files (docs/assets.md): the service ignored or mangled a byte range, so the file
    /// cannot be fetched in resumable chunks.
    RangeUnsupported(String),
    /// Game files: the file on the service changed while it was being fetched (another
    /// revision answered). The partial copy is discarded; run again to restart that file.
    RemoteChanged(String),
    /// Game files: another revision answered between chunks. The partial copy is already
    /// discarded, so a plain retry restarts that file and succeeds: class `retryable`.
    RemoteRevised(String),
    /// Game files: the finished file's size or SHA-256 differs from the manifest. Never installed.
    IntegrityFailed(String),
    /// Game files: a file in the destination is not ours or is not what the manifest says;
    /// it is never overwritten.
    DestinationConflict(String),
    /// Game files: another download already holds this destination.
    Busy(String),
    /// Game files: the manifest or a file it lists is not on the service (yet). The owner may
    /// still be uploading it.
    NotYetUploaded(String),
}

/// What a caller should do about a failure (the `class` of a game-files error):
/// try again later, wait for the owner's upload, sign in again, or stop and show the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Retryable,
    NotYetUploaded,
    ReauthNeeded,
    Fatal,
}

impl Class {
    pub fn key(self) -> &'static str {
        match self {
            Class::Retryable => "retryable",
            Class::NotYetUploaded => "not_yet_uploaded",
            Class::ReauthNeeded => "reauth_needed",
            Class::Fatal => "fatal",
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// The stable code for `--json` output. Launchers match on these; never rename one.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Usage(_) => "usage",
            Error::ManifestInvalid(_) => "manifest_invalid",
            Error::GameUnknown(_) => "game_unknown",
            Error::NotSetUp => "not_set_up",
            Error::AuthFailed => "auth_failed",
            Error::Unreachable(_) => "server_unreachable",
            Error::Server(_) => "server_error",
            Error::StateInvalid(_) => "state_invalid",
            Error::Changed | Error::ChangedRepair(_) | Error::MissingDriveClaim { .. } => {
                "changed_on_server"
            }
            Error::NoSuchEntry(_) => "no_such_entry",
            Error::NotFound(_) => "not_found",
            Error::Credentials(_) => "credentials",
            Error::ReauthNeeded(_) => "reauth_needed",
            Error::SignInFailed(_) => "sign_in_failed",
            Error::SaveLocationUnclear(_) => "save_location_unclear",
            Error::SteamNotFound(_) => "steam_not_found",
            Error::SteamRunning => "steam_running",
            Error::NoPendingChoice(_) => "no_pending_choice",
            Error::Io(_) => "io",
            Error::RangeUnsupported(_) => "range_unsupported",
            Error::RemoteChanged(_) => "remote_changed",
            Error::RemoteRevised(_) => "remote_revised",
            Error::IntegrityFailed(_) => "integrity_failed",
            Error::DestinationConflict(_) => "destination_conflict",
            Error::Busy(_) => "busy",
            Error::NotYetUploaded(_) => "not_yet_uploaded",
        }
    }

    /// How a caller should treat this failure. A dropped connection, a busy service
    /// (`dropbox 429` or 5xx) and a held destination are `Retryable`; any other service
    /// answer (a 4xx the service will repeat) is `Fatal`.
    pub fn class(&self) -> Class {
        match self {
            Error::Unreachable(_) | Error::Busy(_) | Error::RemoteRevised(_) => Class::Retryable,
            Error::Server(message) if retryable_status(message) => Class::Retryable,
            Error::NotYetUploaded(_) => Class::NotYetUploaded,
            Error::ReauthNeeded(_) | Error::AuthFailed | Error::NotSetUp => Class::ReauthNeeded,
            _ => Class::Fatal,
        }
    }
}

/// `Server("dropbox 503")`-style messages: true for 429 and 5xx.
fn retryable_status(message: &str) -> bool {
    message
        .split(|c: char| !c.is_ascii_digit())
        .find(|part| part.len() == 3)
        .and_then(|part| part.parse::<u16>().ok())
        .is_some_and(|status| status == 429 || (500..=599).contains(&status))
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
            Error::ChangedRepair(message) => write!(f, "{message}"),
            Error::MissingDriveClaim { path, .. } => write!(
                f,
                "Google Drive refused a state write because its named claim is missing ({path}); no state was written. Stop sync on every other device, then inspect with `save-relay repair`."
            ),
            Error::Changed => write!(
                f,
                "another machine changed the saves on the server meanwhile"
            ),
            Error::NoSuchEntry(e) => write!(f, "no history entry '{e}'"),
            Error::NotFound(p) => write!(f, "not found on the server: {p}"),
            Error::Credentials(m) => write!(f, "credential store: {m}"),
            Error::ReauthNeeded(s) => write!(
                f,
                "the sign-in to {s} has expired or was withdrawn: run `save-relay setup` again"
            ),
            Error::SignInFailed(m) => write!(f, "sign-in failed: {m}"),
            Error::SaveLocationUnclear(m) => write!(
                f,
                "no clear saves folder: {m}; write a manifest for this game and pass --manifest"
            ),
            Error::SteamNotFound(m) => write!(
                f,
                "Steam was not found ({m}): set SAVE_RELAY_STEAM_DIR to its folder"
            ),
            Error::SteamRunning => write!(
                f,
                "Steam is running: quit Steam, then run install-wrap again"
            ),
            Error::NoPendingChoice(g) => write!(f, "no choice is waiting for game '{g}'"),
            Error::Io(m) => write!(f, "{m}"),
            Error::RangeUnsupported(m) => write!(f, "the service does not serve byte ranges: {m}"),
            Error::RemoteChanged(m) => write!(f, "the file changed on the server meanwhile: {m}"),
            Error::RemoteRevised(m) => write!(f, "the file was replaced on the server meanwhile: {m}"),
            Error::IntegrityFailed(m) => write!(f, "the downloaded file is not what the manifest lists: {m}"),
            Error::DestinationConflict(m) => write!(f, "{m}"),
            Error::Busy(m) => write!(f, "{m}"),
            Error::NotYetUploaded(m) => write!(f, "not on the server yet: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e.to_string())
    }
}
