//! The launcher contract: the `--json` envelope, each command's `data`, and exit codes.
//! The full shape, with examples, is `docs/code-map.md` § API shape. Field names and codes here
//! are a public contract: add fields, never rename or remove one.
//!
//! ```text
//! {"ok":true,"command":"pull","data":{...}}
//! {"ok":false,"command":"pull","error":{"code":"server_error","message":"..."}}
//! ```

use crate::card::Card;
use crate::error::Error;
use crate::pending::{Answer, ChoiceKind, PendingChoice};
use crate::plan::Decision;
use crate::transfer::Counts;
use serde::{Deserialize, Serialize};

/// One side of a sync: who made the saves and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Side {
    pub device: String,
    /// RFC 3339 UTC. For the local side: the newest save file's modification time.
    pub time: String,
    /// The server revision; `null` for local saves never pushed.
    pub revision: Option<u64>,
}

/// What `pull` or `push` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Already in sync.
    Noop,
    /// Same files on both sides; only the local record was updated.
    Adopted,
    /// The server's saves were downloaded.
    Pulled,
    /// The local saves were uploaded as a new revision.
    Pushed,
    /// `pull` with the server unreachable: start the game with local saves.
    Offline,
    /// `push` with the server unreachable: the saves stay local and go up next time.
    Deferred,
    /// Both sides changed: nothing was touched; ask the player, then call again with `--resolve`.
    Conflict,
    /// `pull`: another machine has the game open (its open-elsewhere mark, M11); nothing was
    /// touched. Ask the player: Wait (poll `status` until `held` is `null`), or Play anyway
    /// (`pull --anyway`). Exit code 6.
    Held,
}

/// Who has a game open, from its open-elsewhere mark (`presence::Mark`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    /// The other machine.
    pub device: String,
    /// RFC 3339 UTC: when its `pull` ran.
    pub since: String,
}

/// `data` of `pull` and `push`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncData {
    pub game: String,
    pub action: Action,
    /// The server revision the local saves now match; `null` when offline, deferred or in conflict.
    pub revision: Option<u64>,
    /// The server's current saves; `null` when unreachable or empty.
    pub server: Option<Side>,
    /// The local saves before the command; `null` when there are none.
    pub local: Option<Side>,
    /// The history entry that keeps the side not chosen in a `--resolve`; `null` otherwise.
    pub backup: Option<String>,
    pub counts: Counts,
    /// One line for a person, e.g. "Pulled revision 7 from steam-deck (2026-10-31T19:51:00Z)."
    pub message: String,
}

/// `data` of `pull` and `push` as the CLI prints it: `SyncData`'s fields, flattened, plus the
/// open-elsewhere mark and the two sides' save cards. These are siblings, not fields of
/// `SyncData` and `Side`, so the struct literals in #9's locked tests stay valid; in JSON they
/// are plain fields of `data` (`"held"`, `"server_card"`, `"local_card"`, each `null` when none).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    #[serde(flatten)]
    pub sync: SyncData,
    /// `Action::Held`: who has the game open. `null` otherwise.
    #[serde(default)]
    pub held: Option<Held>,
    /// The card of the server's current entry; `null` when it has none (M14).
    #[serde(default)]
    pub server_card: Option<Card>,
    /// The card the local saves would carry (their place and picture from the sidecar);
    /// `null` when there are no local saves.
    #[serde(default)]
    pub local_card: Option<Card>,
}

/// `data` of `status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusData {
    pub game: String,
    /// The service this machine was set up with: "nextcloud", "gdrive", "dropbox" (M6).
    pub service: String,
    pub reachable: bool,
    /// What `pull` would do now.
    pub would: Decision,
    pub server: Option<Side>,
    pub local: Option<Side>,
    /// The revision this machine last synced; `null` when never.
    pub last_synced_revision: Option<u64>,
    /// Local files differ from the last sync (or exist with no sync yet).
    pub local_changed: bool,
    pub message: String,
    /// Another machine's open-elsewhere mark, `null` when none (or this machine's own). A
    /// launcher that chose Wait polls `status` until this is `null` (M11).
    #[serde(default)]
    pub held: Option<Held>,
    /// A choice waiting for an answer on this machine (`wrap`, M12), `null` when none.
    #[serde(default)]
    pub pending: Option<PendingChoice>,
}

/// One row of `history`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// The entry name, which `restore` takes.
    pub entry: String,
    pub device: String,
    /// RFC 3339 UTC.
    pub time: String,
    /// It holds the server's current revision.
    pub current: bool,
    /// The entry's save card; `null` for entries pushed before cards (M14).
    #[serde(default)]
    pub card: Option<Card>,
}

/// `data` of `history`: newest first, at most `retention::KEEP` plus any conflict backups not
/// yet dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryData {
    pub game: String,
    pub current: Option<String>,
    pub entries: Vec<HistoryEntry>,
}

/// `data` of `restore`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreData {
    pub game: String,
    /// The entry restored from.
    pub entry: String,
    /// The new server revision, a copy of `entry`.
    pub revision: u64,
    /// The new entry holding that copy.
    pub new_entry: String,
    /// Where unsynced local changes were kept first; `null` when there were none.
    pub backup: Option<String>,
    pub counts: Counts,
    pub message: String,
}

/// `data` of `setup`. Never the password or a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupData {
    /// "nextcloud", "gdrive", "dropbox".
    pub service: String,
    /// The Nextcloud address; for Google Drive and Dropbox the service key again.
    pub server: String,
    /// The Nextcloud user; for Google Drive and Dropbox the account's e-mail address.
    pub user: String,
    pub device: String,
    pub root: String,
    /// `CredentialStore::kind` of the store the password or refresh token went into.
    pub credential_store: String,
}

/// `data` of `setup --no-browser` (Google Drive, Dropbox): the sign-in to finish on any
/// device, then `setup --service <s> --finish`. A launcher shows `url` and `code` (as text or
/// a QR code); nothing is set up yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignInPrompt {
    pub service: String,
    /// The page to open.
    pub url: String,
    /// Google: the code to type on that page. Dropbox: `null` (Dropbox shows a code to type
    /// back into `setup --finish` instead).
    pub code: Option<String>,
    /// RFC 3339 UTC; after it, start again.
    pub expires_at: String,
    pub message: String,
}

/// One row of `where`: a game this machine remembers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhereGame {
    pub game: String,
    /// The machine that has it open (its mark, this machine's own included); `null` when none.
    pub held: Option<Held>,
    /// The server's current saves; `null` when nothing was pushed yet.
    pub server: Option<Side>,
}

/// `data` of `where`: every game this machine remembers, by id. Unreachable is an error (exit 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhereData {
    pub games: Vec<WhereGame>,
}

/// Where a wrapped game's saves folder came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhichSource {
    /// A manifest of the person's own (always wins).
    Manifest,
    /// The ludusavi-manifest data.
    Ludusavi,
}

/// `data` of `which <appid>`. No clear folder is the error `save_location_unclear` (exit 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhichData {
    pub app_id: u32,
    /// `steam-<appid>`.
    pub game: String,
    /// The game's name in the data or the manifest.
    pub name: String,
    pub saves_dir: String,
    /// The files that count as saves, relative to `saves_dir`.
    pub include: Vec<String>,
    pub source: WhichSource,
    pub message: String,
}

/// One launch option `install-wrap` changed (or would, with `--dry-run`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallWrapApp {
    pub app_id: u32,
    /// The Steam user whose `localconfig.vdf` holds it.
    pub user_id: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// `data` of `install-wrap`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallWrapData {
    pub dry_run: bool,
    pub changed: Vec<InstallWrapApp>,
    /// The backup files written (none with `--dry-run`).
    pub backups: Vec<String>,
    pub message: String,
}

/// `data` of `answer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerData {
    pub game: String,
    pub kind: ChoiceKind,
    pub answer: Answer,
    pub message: String,
}

/// `data` of `picture <game> <entry> --out <file>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PictureData {
    /// The file written.
    pub path: String,
    pub bytes: u64,
}

/// `data` of `panel`: printed once, when it is listening.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelData {
    /// `http://127.0.0.1:<port>/?token=<token>`.
    pub url: String,
    pub port: u16,
}

/// `{"ok":true,"command":<command>,"data":<data>}` on one line, no trailing newline.
pub fn success<T: Serialize>(command: &str, data: &T) -> String {
    #[derive(Serialize)]
    struct SuccessEnvelope<'a, T> {
        ok: bool,
        command: &'a str,
        data: &'a T,
    }

    serde_json::to_string(&SuccessEnvelope {
        ok: true,
        command,
        data,
    })
    .expect("serializing API success envelope should not fail")
}

/// `{"ok":false,"command":<command>,"error":{"code":<error.code()>,"message":<error's Display>}}`
/// on one line, no trailing newline.
pub fn failure(command: &str, error: &Error) -> String {
    #[derive(Serialize)]
    struct FailureEnvelope<'a> {
        ok: bool,
        command: &'a str,
        error: ErrorDetail,
    }

    #[derive(Serialize)]
    struct ErrorDetail {
        code: &'static str,
        message: String,
    }

    serde_json::to_string(&FailureEnvelope {
        ok: false,
        command,
        error: ErrorDetail {
            code: error.code(),
            message: error.to_string(),
        },
    })
    .expect("serializing API failure envelope should not fail")
}

/// The exit code of a command that succeeded: 3 for `Action::Conflict`, 6 for `Action::Held`,
/// else 0.
pub fn exit_code_for(action: Action) -> i32 {
    if action == Action::Conflict {
        3
    } else {
        0
    }
}

/// The exit code of a failed command:
/// 2 `usage`, `manifest_invalid`, `game_unknown`, `no_such_entry`, `save_location_unclear`,
/// `steam_not_found`, `steam_running`, `no_pending_choice`;
/// 4 `not_set_up`, `auth_failed`, `credentials`, `reauth_needed`, `sign_in_failed`;
/// 5 `server_unreachable`, `server_error`, `state_invalid`, `changed_on_server`, `not_found`;
/// 1 anything else (`io`, `not_built`).
pub fn exit_code(error: &Error) -> i32 {
    match error {
        Error::Usage(_)
        | Error::ManifestInvalid(_)
        | Error::GameUnknown(_)
        | Error::NoSuchEntry(_)
        | Error::SaveLocationUnclear(_)
        | Error::SteamNotFound(_)
        | Error::SteamRunning
        | Error::NoPendingChoice(_) => 2,
        Error::NotSetUp
        | Error::AuthFailed
        | Error::Credentials(_)
        | Error::ReauthNeeded(_)
        | Error::SignInFailed(_) => 4,
        Error::Unreachable(_)
        | Error::Server(_)
        | Error::StateInvalid(_)
        | Error::Changed
        | Error::NotFound(_) => 5,
        Error::Io(_) | Error::NotBuilt(_) => 1,
    }
}
