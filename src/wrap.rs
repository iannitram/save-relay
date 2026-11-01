//! `save-relay wrap [--game <id>] [--manifest <f>] [--saves-dir <d>] -- <command...>`: sync
//! any Steam game through one launch option, `save-relay wrap -- %command%` (SPEC.md M12;
//! docs/adr/0003-baton-wrap-cards-panel.md). It reuses the engine unchanged: `pull`, then the
//! game as a child process with this process's standard streams, then `push`, and it exits
//! with the CHILD's exit code, so Steam sees the game's own result.
//!
//! The game always starts, even when `pull` fails (the local saves are played, as in the
//! launcher's flow, M5). With no launcher to ask, a `conflict` or `held` becomes a pending
//! choice (`pending`), answered from the panel or with `save-relay answer`; with no answer
//! within `pending::ANSWER_WAIT_SECS` the game starts on the local saves and nothing on the
//! server is overwritten. wrap never guesses. Its own notes go to standard error, one line each.
//!
//! The game id is `--game`, else `steam::game_id` of the app id Steam passed
//! (`steam::app_id_from_env`); the saves folder is `--manifest` or a manifest remembered for
//! that id (the person's own always wins), else `ludusavi`.

use crate::api::SyncReport;
use crate::error::{not_built, Result};
use crate::local::LocalStore;
use crate::pending::{Answer, ChoiceKind, PendingChoice};
use crate::stamp::Clock;
use crate::sync::Resolve;

/// The game id: `explicit` when given, else `steam-<app id>` from the environment; neither is
/// `Error::Usage("wrap: no --game and no Steam app id in the environment")`.
pub fn game_id(explicit: Option<&str>, var: &dyn Fn(&str) -> Option<String>) -> Result<String> {
    not_built("wrap::game_id")
}

/// The engine as `wrap` uses it, so its flow is tested without a server or a game:
/// `cli::run` passes one built over `sync::pull` / `sync::push` for one game.
pub trait Engine {
    fn game(&self) -> &str;
    /// `sync::pull` with `resolve` and `anyway`.
    fn pull(&self, resolve: Option<Resolve>, anyway: bool) -> Result<SyncReport>;
    /// `sync::push` with `resolve`.
    fn push(&self, resolve: Option<Resolve>) -> Result<SyncReport>;
}

/// The pending choice a `pull` or `push` answer needs, if any: `Action::Conflict` is a
/// `Conflict` (sides and cards copied), `Action::Held` a `Held` (who has it open); anything
/// else is `None`. `now` is the choice's `time`. Pure.
pub fn choice_for(game: &str, report: &SyncReport, now: u64) -> Option<PendingChoice> {
    crate::not_built!("wrap::choice_for")
}

/// What an answer means before Play: `KeepServer`/`KeepLocal` call `pull` again with that
/// resolve; `PlayAnyway` calls `pull` with `anyway`; `Wait` reads the mark again
/// (`pull` once more, plain) until it is free or the wait ends. Pure.
pub fn resolve_for(kind: ChoiceKind, answer: Answer) -> (Option<Resolve>, bool) {
    crate::not_built!("wrap::resolve_for")
}

/// Start `command` (program and arguments, no shell) with inherited standard streams and wait.
/// Its exit code; killed by a signal is 128 + the signal on Unix, 1 elsewhere. A program that
/// cannot start is `Error::Io` naming it.
pub fn run_child(command: &[String]) -> Result<i32> {
    not_built("wrap::run_child")
}

/// The whole wrap: `pull`; on a choice, write it (`store.save_pending`) and
/// `pending::wait_for_answer`, then act on the answer or, with none, play the local saves;
/// `run_child`; `push` (a choice there is written and left pending, not waited for); remove a
/// pending choice that was settled. Errors from the engine are notes on standard error, never
/// the exit code. Returns the child's exit code (`run_child`), or 1 when it could not start.
pub fn run(
    engine: &dyn Engine,
    store: &LocalStore,
    command: &[String],
    now: Clock,
    sleep: &dyn Fn(u64),
) -> i32 {
    crate::not_built!("wrap::run")
}
