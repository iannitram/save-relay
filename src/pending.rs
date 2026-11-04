//! Pending choices (SPEC.md M12; docs/adr/0003-baton-wrap-cards-panel.md). A wrapped game has
//! no launcher to ask, so a `conflict` or a `held` at `wrap` time is written to
//! `<data>/save-relay/pending/<game>.json` (`local::LocalStore::save_pending`); `wrap` then
//! waits up to `ANSWER_WAIT_SECS` for `save-relay answer <game> <answer>`, which the panel
//! calls. With no answer the game starts on the local saves and nothing on the server is
//! overwritten; the push at exit meets the same conflict and leaves it pending again. Never
//! guessed. `status` and the panel show what is pending.

use crate::api::{Held, Side};
use crate::card::Card;
use crate::error::{Error, Result};
use crate::local::LocalStore;
use crate::stamp::Clock;
use serde::{Deserialize, Serialize};

/// The pending-choice format this build reads and writes.
pub const FORMAT: u32 = 1;

/// How long `wrap` waits for an answer before it starts the game on the local saves.
pub const ANSWER_WAIT_SECS: u64 = 120;

/// How often `wait_for_answer` reads the file again.
pub const POLL_SECS: u64 = 1;

/// What is waiting for a choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceKind {
    /// Both sides changed (M3), at launch or at exit.
    Conflict,
    /// Another machine has the game open (M11), at launch.
    Held,
}

/// The choices. In JSON snake case (`keep_server`); on the command line kebab case
/// (`keep-server`), like `--resolve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    /// Conflict: take the server's saves (`sync::Resolve::KeepServer`).
    KeepServer,
    /// Conflict: keep this machine's saves (`sync::Resolve::KeepLocal`).
    KeepLocal,
    /// Held: play past the other machine's mark (`pull --anyway`).
    PlayAnyway,
    /// Held: do not start yet; `wrap` reads the mark again until it is gone or the wait ends.
    Wait,
}

impl Answer {
    /// `keep-server`, `keep-local`, `play-anyway`, `wait`; anything else is `None`.
    pub fn parse(text: &str) -> Option<Answer> {
        match text {
            "keep-server" => Some(Answer::KeepServer),
            "keep-local" => Some(Answer::KeepLocal),
            "play-anyway" => Some(Answer::PlayAnyway),
            "wait" => Some(Answer::Wait),
            _ => None,
        }
    }

    /// The command-line form, the inverse of `parse`.
    pub fn key(self) -> &'static str {
        match self {
            Answer::KeepServer => "keep-server",
            Answer::KeepLocal => "keep-local",
            Answer::PlayAnyway => "play-anyway",
            Answer::Wait => "wait",
        }
    }

    /// True when the answer fits the kind: `KeepServer`/`KeepLocal` a conflict,
    /// `PlayAnyway`/`Wait` a held game.
    pub fn fits(self, kind: ChoiceKind) -> bool {
        matches!(
            (self, kind),
            (Answer::KeepServer | Answer::KeepLocal, ChoiceKind::Conflict)
                | (Answer::PlayAnyway | Answer::Wait, ChoiceKind::Held)
        )
    }
}

/// `<data>/save-relay/pending/<game>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingChoice {
    pub format: u32,
    pub game: String,
    pub kind: ChoiceKind,
    /// The server's side and card (conflict), as in `api::SyncReport`.
    pub server: Option<Side>,
    pub local: Option<Side>,
    #[serde(default)]
    pub server_card: Option<Card>,
    #[serde(default)]
    pub local_card: Option<Card>,
    /// Who has the game open (held).
    #[serde(default)]
    pub held: Option<Held>,
    /// When the choice was written, `stamp::rfc3339` form.
    pub time: String,
    /// `null` until `answer` writes one.
    #[serde(default)]
    pub answer: Option<Answer>,
}

impl PendingChoice {
    /// Parse and check: `format == FORMAT`, `game` not empty, `time` RFC 3339, an `answer`
    /// that `fits` the kind. Any failure is `Error::StateInvalid`.
    pub fn from_json(text: &str) -> Result<PendingChoice> {
        let choice: PendingChoice =
            serde_json::from_str(text).map_err(|error| Error::StateInvalid(error.to_string()))?;
        if choice.format != FORMAT {
            return Err(Error::StateInvalid(format!(
                "unsupported format {}; expected {FORMAT}",
                choice.format
            )));
        }
        if choice.game.is_empty() {
            return Err(Error::StateInvalid("game must not be empty".into()));
        }
        if crate::stamp::parse_rfc3339(&choice.time).is_none() {
            return Err(Error::StateInvalid("time must be RFC 3339 UTC".into()));
        }
        if choice
            .answer
            .is_some_and(|answer| !answer.fits(choice.kind))
        {
            return Err(Error::StateInvalid("answer does not fit the choice".into()));
        }
        Ok(choice)
    }

    /// Pretty JSON, fields in declaration order, ending in one newline.
    pub fn to_json(&self) -> String {
        format!(
            "{}\n",
            serde_json::to_string_pretty(self).expect("pending choice serialization cannot fail")
        )
    }
}

/// `save-relay answer <game> <answer>`: load the game's pending choice (none is
/// `Error::NoPendingChoice`), refuse a choice that names another game (`Error::StateInvalid`) or
/// an answer that does not `fit` it (`Error::Usage` naming both), write it with the answer set,
/// and return it. The write goes to the file of the game the choice names, so the check is what
/// keeps an answer from landing on another game's choice.
pub fn answer(store: &LocalStore, game: &str, answer: Answer) -> Result<PendingChoice> {
    let mut choice = store
        .load_pending(game)?
        .ok_or_else(|| Error::NoPendingChoice(game.into()))?;
    if choice.game != game {
        return Err(Error::StateInvalid(format!(
            "the pending file for '{game}' names game '{}'",
            choice.game
        )));
    }
    if !answer.fits(choice.kind) {
        let kind = match choice.kind {
            ChoiceKind::Conflict => "conflict",
            ChoiceKind::Held => "held",
        };
        return Err(Error::Usage(format!(
            "{} does not answer a {kind} choice",
            answer.key()
        )));
    }
    choice.answer = Some(answer);
    store.save_pending(&choice)?;
    Ok(choice)
}

/// Wait for an answer to the game's pending choice: read it every `POLL_SECS` (through
/// `sleep`, so tests do not wait) until it has an answer or `wait_secs` passed by `now`.
/// Returns the answer, or `None` on timeout or when the file is gone. Does not remove the file.
pub fn wait_for_answer(
    store: &LocalStore,
    game: &str,
    wait_secs: u64,
    now: Clock,
    sleep: &dyn Fn(u64),
) -> Result<Option<Answer>> {
    let start = now();
    loop {
        let Some(choice) = store.load_pending(game)? else {
            return Ok(None);
        };
        if let Some(answer) = choice.answer {
            return Ok(Some(answer));
        }
        if now().saturating_sub(start) >= wait_secs {
            return Ok(None);
        }
        sleep(POLL_SECS);
    }
}
