//! The sync decision. Pure: given the local save set, this machine's last-synced record and what
//! the server holds, say what to do. Nothing is overwritten without a choice (SPEC.md M3).

use crate::scan::SaveSet;
use crate::state::LastSynced;
use serde::{Deserialize, Serialize};

/// Why the launcher called: before Play (`pull`) or after the game quit (`push`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Launch,
    Exit,
}

/// What the server holds for the game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerView {
    /// The server could not be reached.
    Unreachable,
    /// No `state.json` yet: nothing was ever pushed for this game.
    Empty,
    /// The current revision and its files.
    At { revision: u64, files: SaveSet },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Nothing to do.
    Noop,
    /// Local and server already hold the same files; only record the server revision locally.
    Adopt,
    /// Download the server's revision.
    Pull,
    /// Upload the local files as a new revision.
    Push,
    /// Both sides changed since the last sync: ask (M3).
    Conflict,
    /// The server is unreachable: play with local saves, push later (M5).
    Offline,
}

/// The table (`local changed` = `!local.same_content(&last.save_set())`;
/// `server changed` = `revision != last.revision`):
///
/// | server | last synced | condition | Launch | Exit |
/// |---|---|---|---|---|
/// | Unreachable | any | | Offline | Offline |
/// | Empty | any | local empty | Noop | Noop |
/// | Empty | any | local not empty | Push | Push |
/// | At | none | local same content as server | Adopt | Adopt |
/// | At | none | local empty | Pull | Pull |
/// | At | none | otherwise | Conflict | Conflict |
/// | At | some | neither changed | Noop | Noop |
/// | At | some | only local changed | Push | Push |
/// | At | some | only server changed | Pull | Noop |
/// | At | some | both changed, local same content as server | Adopt | Adopt |
/// | At | some | both changed, otherwise | Conflict | Conflict |
///
/// Rows are checked top to bottom; the first that fits wins.
pub fn decide(
    intent: Intent,
    local: &SaveSet,
    last: Option<&LastSynced>,
    server: &ServerView,
) -> Decision {
    crate::not_built!("plan::decide")
}
