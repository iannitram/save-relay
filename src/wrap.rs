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

use crate::api::{Action, SyncReport};
use crate::error::{Error, Result};
use crate::local::LocalStore;
use crate::pending::{self, Answer, ChoiceKind, PendingChoice, ANSWER_WAIT_SECS, POLL_SECS};
use crate::stamp::{self, Clock};
use crate::steam;
use crate::sync::Resolve;
use std::process::{Child, Command, ExitStatus, Stdio};

/// The game id: `explicit` when given, else `steam-<app id>` from the environment; neither is
/// `Error::Usage("wrap: no --game and no Steam app id in the environment")`.
pub fn game_id(explicit: Option<&str>, var: &dyn Fn(&str) -> Option<String>) -> Result<String> {
    if let Some(id) = explicit {
        if id.is_empty() {
            return Err(Error::Usage("wrap: --game is empty".into()));
        }
        return Ok(id.to_string());
    }
    match steam::app_id_from_env(var) {
        Some(app_id) => Ok(steam::game_id(app_id)),
        None => Err(Error::Usage(
            "wrap: no --game and no Steam app id in the environment".into(),
        )),
    }
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
    let time = stamp::rfc3339(now);
    match report.sync.action {
        Action::Conflict => Some(PendingChoice {
            format: pending::FORMAT,
            game: game.to_string(),
            kind: ChoiceKind::Conflict,
            server: report.sync.server.clone(),
            local: report.sync.local.clone(),
            server_card: report.server_card.clone(),
            local_card: report.local_card.clone(),
            held: None,
            time,
            answer: None,
        }),
        Action::Held => Some(PendingChoice {
            format: pending::FORMAT,
            game: game.to_string(),
            kind: ChoiceKind::Held,
            server: None,
            local: None,
            server_card: None,
            local_card: None,
            held: report.held.clone(),
            time,
            answer: None,
        }),
        _ => None,
    }
}

/// What an answer means before Play: `KeepServer`/`KeepLocal` call `pull` again with that
/// resolve; `PlayAnyway` calls `pull` with `anyway`; `Wait` reads the mark again
/// (`pull` once more, plain) until it is free or the wait ends. Pure.
pub fn resolve_for(kind: ChoiceKind, answer: Answer) -> (Option<Resolve>, bool) {
    match (kind, answer) {
        (ChoiceKind::Conflict, Answer::KeepServer) => (Some(Resolve::KeepServer), false),
        (ChoiceKind::Conflict, Answer::KeepLocal) => (Some(Resolve::KeepLocal), false),
        (ChoiceKind::Held, Answer::PlayAnyway) => (None, true),
        _ => (None, false),
    }
}

/// Start `command` (program and arguments, no shell) with inherited standard streams and wait.
/// Its exit code; killed by a signal is 128 + the signal on Unix, 1 elsewhere. A program that
/// cannot start is `Error::Io` naming it.
pub fn run_child(command: &[String]) -> Result<i32> {
    let mut child = spawn_child(command)?;
    let status = child
        .wait()
        .map_err(|error| Error::Io(format!("cannot wait for the game: {error}")))?;
    Ok(exit_code(status))
}

/// `command` started with inherited standard streams; the same errors as `run_child`.
fn spawn_child(command: &[String]) -> Result<Child> {
    let Some((program, arguments)) = command.split_first() else {
        return Err(Error::Usage("wrap: no command after --".into()));
    };
    Command::new(program)
        .args(arguments)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| Error::Io(format!("cannot start {program}: {error}")))
}

/// How the game ended: on its own, or stopped because `wrap` received a termination signal.
#[derive(Debug, PartialEq, Eq)]
enum Played {
    Exited(i32),
    /// Only Unix forwards a signal; elsewhere this is never built.
    #[cfg_attr(not(unix), allow(dead_code))]
    Signalled(i32),
}

/// Start the game and wait for it, or for a termination signal (F80, audit S3). On a signal the
/// signal is forwarded to the game and the wait ends there: the game is not waited for, so the
/// exit push can run at once. Unix only; elsewhere the game is waited for.
fn play(command: &[String]) -> Result<Played> {
    // Before the game starts: a signal sent once the game runs must find the handlers in place.
    #[cfg(unix)]
    signals::install();
    let mut child = spawn_child(command)?;
    #[cfg(unix)]
    {
        loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| Error::Io(format!("cannot wait for the game: {error}")))?
            {
                return Ok(Played::Exited(exit_code(status)));
            }
            if let Some(signal) = signals::received() {
                signals::forward(child.id(), signal);
                return Ok(Played::Signalled(signal));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    #[cfg(not(unix))]
    {
        let status = child
            .wait()
            .map_err(|error| Error::Io(format!("cannot wait for the game: {error}")))?;
        Ok(Played::Exited(exit_code(status)))
    }
}

/// Unix termination signals: SIGHUP and SIGTERM (Steam's force exit, a logout, a shutdown). The
/// first is recorded for the wait loop in `play`; any later one ends `wrap` at once with
/// `_exit`, which leaves `playing.json` for the next launch to handle.
#[cfg(unix)]
mod signals {
    use std::sync::atomic::{AtomicI32, Ordering};

    const SIGHUP: i32 = 1;
    const SIGTERM: i32 = 15;
    /// The first signal received; 0 while none has come.
    static FIRST: AtomicI32 = AtomicI32::new(0);

    extern "C" {
        fn signal(sig: i32, handler: usize) -> usize;
        fn kill(pid: i32, sig: i32) -> i32;
        fn _exit(status: i32) -> !;
    }

    extern "C" fn on_signal(sig: i32) {
        if FIRST
            .compare_exchange(0, sig, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            // SAFETY: `_exit` is async-signal-safe and never returns.
            unsafe { _exit(128 + sig) }
        }
    }

    /// Route SIGHUP and SIGTERM to `on_signal`.
    pub fn install() {
        let handler = on_signal as extern "C" fn(i32) as usize;
        for sig in [SIGHUP, SIGTERM] {
            // SAFETY: `handler` is an `extern "C" fn(i32)` that only touches an atomic and `_exit`.
            unsafe {
                signal(sig, handler);
            }
        }
    }

    /// The first signal received, if any.
    pub fn received() -> Option<i32> {
        match FIRST.load(Ordering::SeqCst) {
            0 => None,
            sig => Some(sig),
        }
    }

    /// Send `sig` to the game `pid`. A game that has already gone is not an error here.
    pub fn forward(pid: u32, sig: i32) {
        // SAFETY: `kill` takes a pid and a signal number; no memory is shared.
        unsafe {
            kill(pid as i32, sig);
        }
    }
}

fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}

/// The whole wrap: `pull`; on a choice, write it (`store.save_pending`) and
/// `pending::wait_for_answer`, then act on the answer or, with none, play the local saves;
/// `play` (a termination signal ends the wait and is forwarded to the game, F80); `push` (a
/// choice there is written and left pending, not waited for); remove a pending choice that was
/// settled. Errors from the engine are notes on standard error, never
/// the exit code. Returns the child's exit code (`run_child`), or 1 when it could not start.
pub fn run(
    engine: &dyn Engine,
    store: &LocalStore,
    command: &[String],
    now: Clock,
    sleep: &dyn Fn(u64),
) -> i32 {
    let game = engine.game().to_string();
    before_play(engine, store, &game, now, sleep);
    let code = match play(command) {
        Ok(Played::Exited(code)) => code,
        Ok(Played::Signalled(signal)) => {
            note(format!(
                "signal {signal}: the game was stopped; pushing what there is"
            ));
            128 + signal
        }
        Err(error) => {
            note(error);
            1
        }
    };
    after_play(engine, store, &game, now());
    code
}

/// `pull` before Play, and what the person's answer to a choice makes of it.
fn before_play(
    engine: &dyn Engine,
    store: &LocalStore,
    game: &str,
    now: Clock,
    sleep: &dyn Fn(u64),
) {
    let report = match engine.pull(None, false) {
        Ok(report) => report,
        Err(error) => {
            note(format!("{error}; starting on this machine's saves"));
            return;
        }
    };
    let Some(choice) = choice_for(game, &report, now()) else {
        report_note(&report);
        return;
    };
    if let Err(error) = store.save_pending(&choice) {
        note(format!("{error}; starting on this machine's saves"));
        return;
    }
    if let Err(error) = store.set_waiting(game, true) {
        note(format!("{error}; starting on this machine's saves"));
        clear_pending(store, game);
        return;
    }
    note(choice_line(game, &choice));
    // The whole wait, including a `Wait` answer's re-reads, ends this long after the hold.
    let deadline = now() + ANSWER_WAIT_SECS;
    let answer = pending::wait_for_answer(store, game, ANSWER_WAIT_SECS, now, sleep);
    match answer {
        Ok(Some(answer)) => act_on(
            engine,
            store,
            game,
            choice.kind,
            answer,
            now,
            deadline,
            sleep,
        ),
        Ok(None) => {
            note(format!(
                "no answer in {ANSWER_WAIT_SECS} s; starting on this machine's saves"
            ));
            clear_pending(store, game);
        }
        Err(error) => {
            note(format!("{error}; starting on this machine's saves"));
            clear_pending(store, game);
        }
    }
    if let Err(error) = store.set_waiting(game, false) {
        note(error);
    }
}

#[allow(clippy::too_many_arguments)]
fn act_on(
    engine: &dyn Engine,
    store: &LocalStore,
    game: &str,
    kind: ChoiceKind,
    answer: Answer,
    now: Clock,
    deadline: u64,
    sleep: &dyn Fn(u64),
) {
    if answer == Answer::Wait {
        wait_until_free(engine, store, game, now, deadline, sleep);
        return;
    }
    let (resolve, anyway) = resolve_for(kind, answer);
    match engine.pull(resolve, anyway) {
        Ok(report) => {
            report_note(&report);
            settle(store, game, &report, now());
        }
        Err(error) => note(error),
    }
}

/// `Wait`: `pull` again, plainly, until another machine no longer has the game open, or
/// `deadline` (the hold's report plus `ANSWER_WAIT_SECS`, set once in `before_play`) has passed.
/// Still open at the end: start on the local saves.
fn wait_until_free(
    engine: &dyn Engine,
    store: &LocalStore,
    game: &str,
    now: Clock,
    deadline: u64,
    sleep: &dyn Fn(u64),
) {
    // A second answer during the snooze window is a new answer, not the `Wait` already consumed.
    match store.load_pending(game) {
        Ok(Some(mut choice)) => {
            choice.answer = None;
            if let Err(error) = store.save_pending(&choice) {
                note(error);
                return;
            }
        }
        Ok(None) => return,
        Err(error) => {
            note(error);
            return;
        }
    }
    loop {
        match store.load_pending(game) {
            Ok(Some(choice)) => match choice.answer {
                Some(Answer::PlayAnyway) => {
                    match engine.pull(None, true) {
                        Ok(report) => {
                            report_note(&report);
                            settle(store, game, &report, now());
                        }
                        Err(error) => {
                            note(error);
                            clear_answer(store, game);
                        }
                    }
                    return;
                }
                Some(Answer::Wait) => {
                    let mut choice = choice;
                    choice.answer = None;
                    if let Err(error) = store.save_pending(&choice) {
                        note(error);
                        return;
                    }
                }
                Some(Answer::KeepServer | Answer::KeepLocal) => {
                    note("answer does not fit a held choice");
                    return;
                }
                None => {}
            },
            Ok(None) => return,
            Err(error) => {
                note(error);
                clear_answer(store, game);
                return;
            }
        }
        match engine.pull(None, false) {
            Ok(report) if report.sync.action == Action::Held => {
                if now() >= deadline {
                    note(format!(
                        "still open elsewhere after {ANSWER_WAIT_SECS} s; starting on this machine's saves"
                    ));
                    return;
                }
                sleep(POLL_SECS);
            }
            Ok(report) => {
                report_note(&report);
                settle(store, game, &report, now());
                return;
            }
            Err(error) => {
                note(error);
                clear_answer(store, game);
                return;
            }
        }
    }
}

fn clear_answer(store: &LocalStore, game: &str) {
    match store.load_pending(game) {
        Ok(Some(mut choice)) => {
            choice.answer = None;
            if let Err(error) = store.save_pending(&choice) {
                note(error);
            }
        }
        Ok(None) => {}
        Err(error) => note(error),
    }
}

/// After the game quits: `push`. A choice it finds is written and left pending.
fn after_play(engine: &dyn Engine, store: &LocalStore, game: &str, now: u64) {
    match engine.push(None) {
        Ok(report) => {
            report_note(&report);
            settle(store, game, &report, now);
        }
        Err(error) => note(error),
    }
}

/// A report that is not a choice clears any pending choice for `game` (it is settled); a
/// report that is one is written, left pending, and named on standard error.
fn settle(store: &LocalStore, game: &str, report: &SyncReport, now: u64) {
    match choice_for(game, report, now) {
        Some(choice) => match store.save_pending(&choice) {
            Ok(()) => note(choice_line(game, &choice)),
            Err(error) => note(error),
        },
        None => clear_pending(store, game),
    }
}

fn clear_pending(store: &LocalStore, game: &str) {
    if let Err(error) = store.clear_pending(game) {
        note(error);
    }
}

/// The one line that names a choice and how to answer it. The panel lists it too.
fn choice_line(game: &str, choice: &PendingChoice) -> String {
    match choice.kind {
        ChoiceKind::Conflict => format!(
            "{game}: saves changed on this machine and on the server; answer keep-server or keep-local (the panel lists it), or the game starts on this machine's saves"
        ),
        ChoiceKind::Held => {
            let device = choice
                .held
                .as_ref()
                .map_or("another machine", |held| held.device.as_str());
            format!(
                "{game}: open on {device}; answer play-anyway or wait (the panel lists it), or the game starts on this machine's saves"
            )
        }
    }
}

/// A report's message as one note. A conflict or a hold has its own line (`choice_line`).
fn report_note(report: &SyncReport) {
    if matches!(report.sync.action, Action::Conflict | Action::Held)
        || report.sync.message.is_empty()
    {
        return;
    }
    note(&report.sync.message);
}

fn note(text: impl std::fmt::Display) {
    eprintln!("save-relay: {text}");
}
