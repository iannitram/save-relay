//! The sync decision. Pure: given the local save set, this machine's last-synced record and what
//! the server holds, say what to do. Nothing is overwritten without a choice (SPEC.md M3).

use crate::presence::Hold;
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
    /// At launch, another machine has the game open (M11): touch nothing and ask (Wait or
    /// Play anyway). Only `decide_with_mark` answers this; `decide` never does.
    Held,
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
/// | At | some | local empty and server non-empty | Pull | Pull |
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
    match server {
        ServerView::Unreachable => Decision::Offline,
        ServerView::Empty => {
            if local.is_empty() {
                Decision::Noop
            } else {
                Decision::Push
            }
        }
        ServerView::At { revision, files } => match last {
            None => {
                if local.same_content(files) {
                    Decision::Adopt
                } else if local.is_empty() {
                    Decision::Pull
                } else {
                    Decision::Conflict
                }
            }
            Some(last) => {
                // An empty local set can mean the saves folder is missing (for example, a
                // removable drive or Proton prefix has not mounted). Never publish that as a
                // deletion of the server's saves during the automatic launch/exit flow.
                if local.is_empty() && !files.is_empty() {
                    return Decision::Pull;
                }
                let local_changed = !local.same_content(&last.save_set());
                let server_changed = *revision != last.revision;
                match (local_changed, server_changed) {
                    (false, false) => Decision::Noop,
                    (true, false) => Decision::Push,
                    (false, true) => match intent {
                        Intent::Launch => Decision::Pull,
                        Intent::Exit => Decision::Noop,
                    },
                    (true, true) if local.same_content(files) => Decision::Adopt,
                    (true, true) => Decision::Conflict,
                }
            }
        },
    }
}

/// Decide with the local folder identity. A record from a different known folder is treated as
/// absent, so its file list can never make this folder look like a local-only change.
pub fn decide_for_folder(
    intent: Intent,
    local: &SaveSet,
    last: Option<&LastSynced>,
    folder: &str,
    server: &ServerView,
) -> Decision {
    let matching = last.filter(|record| {
        record
            .saves_folder
            .as_deref()
            .is_none_or(|saved| saved == folder)
    });
    decide(intent, local, matching, server)
}

/// `decide` with the open-elsewhere mark (M11), one row added after `Unreachable`:
///
/// | server | hold | anyway | Launch | Exit |
/// |---|---|---|---|---|
/// | Unreachable | any | any | Offline | Offline |
/// | not Unreachable | Elsewhere | false | Held | (`decide`) |
/// | otherwise | | | (`decide`) | (`decide`) |
///
/// The mark is never checked at exit: a push goes ahead whoever has the game open, and M3
/// still guards it. `anyway` is `pull --anyway`.
pub fn decide_with_mark(
    intent: Intent,
    hold: &Hold,
    anyway: bool,
    local: &SaveSet,
    last: Option<&LastSynced>,
    server: &ServerView,
) -> Decision {
    if *server == ServerView::Unreachable {
        Decision::Offline
    } else if intent == Intent::Launch && !anyway && matches!(hold, Hold::Elsewhere { .. }) {
        Decision::Held
    } else {
        decide(intent, local, last, server)
    }
}

/// `decide_with_mark` with folder-aware last-sync matching.
pub fn decide_with_mark_for_folder(
    intent: Intent,
    hold: &Hold,
    anyway: bool,
    local: &SaveSet,
    last: Option<&LastSynced>,
    folder: &str,
    server: &ServerView,
) -> Decision {
    if *server == ServerView::Unreachable {
        Decision::Offline
    } else if intent == Intent::Launch && !anyway && matches!(hold, Hold::Elsewhere { .. }) {
        Decision::Held
    } else {
        decide_for_folder(intent, local, last, folder, server)
    }
}
