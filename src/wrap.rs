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

/// How the game ended: on its own, or stopped because `wrap` received a termination request.
#[derive(Debug, PartialEq, Eq)]
enum Played {
    Exited(i32),
    /// The termination request: a signal on Unix, a console event on Windows.
    #[cfg_attr(not(any(unix, windows)), allow(dead_code))]
    Signalled(i32),
    /// Windows may close wrap while a GUI game continues writing its saves.
    #[cfg(windows)]
    StillRunning(i32),
}

/// Unix: give a signalled game this long to exit before killing it and pushing (F80).
#[cfg(not(windows))]
pub const STOP_WAIT_SECS: u64 = 3;
/// Windows: wait this long for the game to exit on its own. A game still running is left alone;
/// skip the push and leave the mark. Leave most of the console close window for cleanup.
#[cfg(windows)]
pub const STOP_WAIT_SECS: u64 = 1;

#[cfg(windows)]
use console as stop;
#[cfg(unix)]
use signals as stop;

/// Start the game and wait for it, or for a termination request (F80, audit S3; #264 on
/// Windows). On a request the game is asked to stop (`stop::forward`) and given `STOP_WAIT_SECS`
/// to exit (`stop_game`). Windows skips the push if the game is still running; Unix kills and
/// reaps it before pushing. Elsewhere the game is waited for.
fn play(command: &[String]) -> Result<Played> {
    // Before the game starts: a request sent once the game runs must find the handlers in place.
    #[cfg(unix)]
    stop::install();
    let mut child = spawn_child(command)?;
    #[cfg(any(unix, windows))]
    {
        loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| Error::Io(format!("cannot wait for the game: {error}")))?
            {
                return Ok(Played::Exited(exit_code(status)));
            }
            if let Some(signal) = stop::received() {
                stop::forward(child.id(), signal);
                let stopped = stop_game(&mut child)?;
                #[cfg(windows)]
                if !stopped {
                    return Ok(Played::StillRunning(signal));
                }
                #[cfg(unix)]
                debug_assert!(stopped);
                return Ok(Played::Signalled(signal));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let status = child
            .wait()
            .map_err(|error| Error::Io(format!("cannot wait for the game: {error}")))?;
        Ok(Played::Exited(exit_code(status)))
    }
}

/// After a stop request: wait up to `STOP_WAIT_SECS` for the game to exit. Unix then kills and
/// reaps it; Windows leaves it running and returns false to skip the push and keep the mark.
#[cfg(any(unix, windows))]
fn stop_game(child: &mut Child) -> Result<bool> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(STOP_WAIT_SECS);
    while std::time::Instant::now() < until {
        if child
            .try_wait()
            .map_err(|error| Error::Io(format!("cannot wait for the game: {error}")))?
            .is_some()
        {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    #[cfg(windows)]
    {
        Ok(false)
    }
    #[cfg(unix)]
    {
        note(format!(
            "the game did not stop within {STOP_WAIT_SECS} s; ending it"
        ));
        if let Err(error) = child.kill() {
            // The game may have exited between the last poll and the kill.
            if child.try_wait().ok().flatten().is_none() {
                return Err(Error::Io(format!("cannot stop the game: {error}")));
            }
        }
        child
            .wait()
            .map_err(|error| Error::Io(format!("cannot wait for the game: {error}")))?;
        Ok(true)
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

/// Windows console events: the console window closing (CTRL_CLOSE_EVENT), logoff and shutdown
/// (#264, split from #243). The console calls `on_event` on a thread of its own. The first event
/// is recorded for the wait loop in `play`, and the handler returns only once the exit push and
/// rotated-token storage have finished (`finished`): returning TRUE early lets Windows end
/// the process. A later event ends `wrap` at once with `_exit`, which leaves `playing.json`.
#[cfg(windows)]
mod console {
    use std::sync::{Mutex, Once};
    use std::time::{Duration, Instant};

    const CTRL_CLOSE_EVENT: u32 = 2;
    const CTRL_LOGOFF_EVENT: u32 = 5;
    const CTRL_SHUTDOWN_EVENT: u32 = 6;
    const FALSE: i32 = 0;
    const TRUE: i32 = 1;
    /// Stop, push and token storage share 4.5 s, before Windows' usual 5 s close timeout.
    /// If the service cannot finish in time, exit with the mark still present for recovery.
    pub const CLEANUP_BUDGET: Duration = Duration::from_millis(4500);

    struct State {
        first: u32,
        finished: bool,
    }
    // Publish requests, completion, and the start of a new run under the same lock. In
    // particular a previous run's completion must never release this run's handler.
    static STATE: Mutex<State> = Mutex::new(State {
        first: 0,
        finished: false,
    });
    static INSTALL: Once = Once::new();

    type Handler = unsafe extern "system" fn(u32) -> i32;

    extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<Handler>, add: i32) -> i32;
    }

    extern "C" {
        fn _exit(status: i32) -> !;
    }

    /// The console's handler. Not one of the three events: FALSE, the next handler decides.
    ///
    /// # Safety
    /// Called by the console only, as its handler routine.
    pub unsafe extern "system" fn on_event(event: u32) -> i32 {
        if !matches!(
            event,
            CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT
        ) {
            return FALSE;
        }
        let until = Instant::now() + CLEANUP_BUDGET;
        {
            let mut state = STATE.lock().unwrap_or_else(|poison| poison.into_inner());
            if state.first != 0 {
                // SAFETY: `_exit` ends the process and never returns.
                unsafe { _exit(128 + event as i32) }
            }
            state.first = event;
        }
        loop {
            if STATE
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .finished
            {
                break;
            }
            if Instant::now() >= until {
                // Do not leave an unfinished push running after the cleanup budget.
                // SAFETY: `_exit` ends the process and never returns.
                unsafe { _exit(128 + event as i32) }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        TRUE
    }

    /// Register `on_event` once (a second registration would run it twice per event).
    pub fn install() {
        *STATE.lock().unwrap_or_else(|poison| poison.into_inner()) = State {
            first: 0,
            finished: false,
        };
        INSTALL.call_once(|| {
            // SAFETY: `on_event` is an `extern "system"` routine that only locks the state,
            // sleeps and `_exit`.
            unsafe {
                SetConsoleCtrlHandler(Some(on_event), TRUE);
            }
        });
    }

    /// The first event received, if any.
    pub fn received() -> Option<i32> {
        match STATE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .first
        {
            0 => None,
            event => Some(event as i32),
        }
    }

    /// Nothing to forward: GUI games do not receive console events. Console games sharing
    /// this console receive the event directly; either kind may still be writing its saves.
    pub fn forward(_pid: u32, _event: i32) {}

    /// The exit push and rotated-token storage are done: release the handler.
    pub fn finished() {
        STATE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .finished = true;
    }
}

/// Called by the CLI only after `with_remote` has saved any rotated refresh token. The
/// console handler's original cleanup deadline also covers credential storage.
#[cfg(windows)]
pub(crate) fn finish_console_cleanup() {
    console::finished();
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
    // Initialize before pull as well: a later run must not expose stale completion to
    // an already registered console handler while it is preparing the game.
    #[cfg(windows)]
    console::install();
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
        #[cfg(windows)]
        Ok(Played::StillRunning(event)) => {
            note(format!(
                "the game did not stop within {STOP_WAIT_SECS} s; skipping the exit push and leaving the mark"
            ));
            return 128 + event;
        }
        Err(error) => {
            note(error);
            #[cfg(any(unix, windows))]
            if stop::received().is_some() {
                // A failed stop must not upload while the game might still be writing.
                return 1;
            }
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

/// Exercise the real push/mark flow in isolated Windows hosts: console handlers have
/// process-global state, and the second-event and deadline paths terminate their process.
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::cli::{run_with_net, Net};
    use crate::clients::{Client, ClientSet};
    use crate::creds::{CredentialStore, MemoryStore, Secret};
    use crate::fake::{FakeDropbox, REFRESH_TOKEN};
    use crate::http::{Request, Response, Transport};
    use crate::layout::Layout;
    use crate::manifest::Manifest;
    use crate::platform::{AppDirs, PathVars};
    use crate::remote::MemoryRemote;
    use crate::service::{token_account, Service};
    use crate::sync::{self, Game, Session};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    const DIR: &str = "SAVE_RELAY_CONSOLE_TEST_DIR";
    const MODE: &str = "SAVE_RELAY_CONSOLE_TEST_MODE";

    fn wait_for(path: &Path) {
        let until = Instant::now() + Duration::from_secs(15);
        while !path.exists() {
            assert!(
                Instant::now() < until,
                "timed out waiting for {}",
                path.display()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    struct TestEngine {
        remote: MemoryRemote,
        game: Game,
        store: LocalStore,
        dir: PathBuf,
        mode: String,
    }

    impl TestEngine {
        fn session(&self, now: u64) -> Session<'_> {
            Session {
                remote: &self.remote,
                layout: Layout::new("saves").unwrap(),
                device: "console-test".into(),
                now,
            }
        }
    }

    impl Engine for TestEngine {
        fn game(&self) -> &str {
            &self.game.manifest.id
        }
        fn pull(&self, resolve: Option<Resolve>, anyway: bool) -> Result<SyncReport> {
            let report = sync::pull(
                &self.session(1000),
                &self.game,
                &self.store,
                resolve,
                anyway,
            )?;
            let mark = self
                .remote
                .read("saves/console-game/playing.json")
                .expect("pull sets playing.json");
            fs::write(self.dir.join("playing.json"), mark).unwrap();
            Ok(report)
        }
        fn push(&self, resolve: Option<Resolve>) -> Result<SyncReport> {
            // SYNCHRONIZE and WaitForSingleObject prove the game is gone before any upload.
            extern "system" {
                fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
                fn WaitForSingleObject(handle: *mut std::ffi::c_void, millis: u32) -> u32;
                fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
            }
            let pid = fs::read_to_string(self.dir.join("pid"))
                .unwrap()
                .parse()
                .unwrap();
            // SAFETY: the handle is used only for a zero-time wait and closed once.
            unsafe {
                let handle = OpenProcess(0x00100000, 0, pid);
                if !handle.is_null() {
                    let status = WaitForSingleObject(handle, 0);
                    CloseHandle(handle);
                    assert_eq!(status, 0, "the game still runs during the exit push");
                }
            }
            fs::write(self.dir.join("pushing"), "").unwrap();
            if matches!(self.mode.as_str(), "second" | "timeout") {
                thread::sleep(Duration::from_secs(60));
            }
            // A nonzero push duration also detects early handler completion.
            thread::sleep(Duration::from_millis(500));
            let report = sync::push(&self.session(1001), &self.game, &self.store, resolve)?;
            assert!(self
                .remote
                .read("saves/console-game/playing.json")
                .is_none());
            assert!(self
                .remote
                .file_paths()
                .iter()
                .filter(|p| p.ends_with("slot.sav"))
                .any(|p| self.remote.read(p).as_deref() == Some(b"game save")));
            fs::remove_file(self.dir.join("playing.json")).unwrap();
            fs::write(self.dir.join("pushed"), "").unwrap();
            Ok(report)
        }
    }

    #[test]
    #[ignore = "spawned as the wrapped game"]
    fn console_game() {
        let Some(dir) = std::env::var_os(DIR).map(PathBuf::from) else {
            return;
        };
        fs::write(dir.join("saves/slot.sav"), "game save").unwrap();
        fs::write(dir.join("pid"), std::process::id().to_string()).unwrap();
        fs::write(dir.join("started"), "").unwrap();
        wait_for(&dir.join("exit-game"));
        fs::write(dir.join("exiting"), "").unwrap();
    }

    #[test]
    #[ignore = "spawned as the wrap host"]
    fn console_host() {
        let Some(dir) = std::env::var_os(DIR).map(PathBuf::from) else {
            return;
        };
        let mode = std::env::var(MODE).unwrap();
        let event: u32 = std::env::var("SAVE_RELAY_CONSOLE_TEST_EVENT")
            .unwrap()
            .parse()
            .unwrap();
        let vars = PathVars {
            home: dir.join("home"),
            data: dir.join("data"),
            config: dir.join("config"),
            documents: None,
        };
        let engine = TestEngine {
            remote: MemoryRemote::new(),
            game: Game {
                manifest: Manifest::from_json(r#"{"format":1,"id":"console-game","name":"Console game","saves_from":"launcher","saves":{},"include":["*.sav"]}"#).unwrap(),
                saves_dir: dir.join("saves"),
            },
            store: LocalStore::new(AppDirs::from_vars(&vars)),
            dir: dir.clone(), mode: mode.clone(),
        };
        fs::create_dir_all(&engine.game.saves_dir).unwrap();
        fs::write(engine.game.saves_dir.join("slot.sav"), "before game").unwrap();
        // Simulate completion of an earlier wrap in this process. install must reset it.
        console::finished();
        assert_eq!(
            unsafe { console::on_event(0) },
            0,
            "unrelated event is unhandled"
        );
        let events = thread::spawn(move || {
            wait_for(&dir.join("started"));
            let began = Instant::now();
            let first = thread::spawn(move || unsafe { console::on_event(event) });
            while console::received().is_none() {
                thread::sleep(Duration::from_millis(10));
            }
            if mode != "running" {
                // Simulate a game finishing its save and exiting of its own accord.
                fs::write(dir.join("exit-game"), "").unwrap();
            }
            if mode == "second" {
                wait_for(&dir.join("pushing"));
                // SAFETY: emulate a second console handler thread during the exit push.
                unsafe {
                    console::on_event(event);
                }
                panic!("a second event must not return");
            }
            assert_eq!(first.join().unwrap(), 1);
            assert!(began.elapsed() < console::CLEANUP_BUDGET);
            if mode == "running" {
                assert!(!dir.join("pushing").exists());
                assert!(dir.join("playing.json").exists());
                // The game must survive wrap's stop wait, not merely skip its upload.
                assert_game_running(&dir);
                fs::write(dir.join("exit-game"), "").unwrap();
                wait_for(&dir.join("exiting"));
                return;
            }
            assert!(
                dir.join("pushed").exists(),
                "handler returned before push finished"
            );
            assert!(!dir.join("playing.json").exists());
        });
        let command = vec![
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "--exact".into(),
            "wrap::tests::console_game".into(),
            "--ignored".into(),
        ];
        let code = run(&engine, &engine.store, &command, || 1000, &|_| {});
        finish_console_cleanup();
        events.join().unwrap();
        // The game may exit before play observes the event; both paths must push safely.
        if engine.mode == "running" {
            assert_eq!(code, 128 + event as i32);
        } else {
            assert!(code == 0 || code == 128 + event as i32);
        }
    }

    fn scenario(event: u32, mode: &str) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/console-tests")
            .join(format!("{}-{event}-{mode}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut host = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "wrap::tests::console_host",
                "--ignored",
                "--nocapture",
            ])
            .env(DIR, &dir)
            .env(MODE, mode)
            .env("SAVE_RELAY_CONSOLE_TEST_EVENT", event.to_string())
            .spawn()
            .unwrap();
        let began = Instant::now();
        wait_for(&dir.join("started"));
        let started = Instant::now();
        let status = loop {
            if let Some(status) = host.try_wait().unwrap() {
                break status;
            }
            if began.elapsed() > Duration::from_secs(15) {
                host.kill().unwrap();
                host.wait().unwrap();
                panic!("console host did not exit");
            }
            thread::sleep(Duration::from_millis(10));
        };
        if mode == "running" {
            assert!(status.success());
            assert!(!dir.join("pushing").exists());
            assert!(dir.join("playing.json").exists());
        } else if mode == "normal" {
            assert!(dir.join("pushing").exists(), "exit push was not reached");
            assert!(status.success());
            assert!(dir.join("pushed").exists());
            assert!(!dir.join("playing.json").exists());
        } else {
            assert_eq!(status.code(), Some(128 + event as i32));
            assert!(dir.join("playing.json").exists());
            assert!(!dir.join("pushed").exists());
            if mode == "second" {
                assert!(
                    started.elapsed() < Duration::from_secs(4),
                    "second event did not exit at once"
                );
            } else {
                assert!(
                    started.elapsed() < Duration::from_secs(6),
                    "cleanup exceeded the close window"
                );
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn console_events_push_and_clear_playing_json() {
        for event in [2, 5, 6] {
            scenario(event, "normal");
        }
    }

    #[test]
    fn second_console_event_exits_at_once_and_leaves_playing_json() {
        scenario(2, "second");
    }

    #[test]
    fn console_cleanup_deadline_leaves_playing_json() {
        scenario(2, "timeout");
    }

    #[test]
    fn console_stop_leaves_a_running_game_and_skips_the_push() {
        for event in [2, 5, 6] {
            scenario(event, "running");
        }
    }

    fn assert_game_running(dir: &Path) {
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
            fn WaitForSingleObject(handle: *mut std::ffi::c_void, millis: u32) -> u32;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        let pid = fs::read_to_string(dir.join("pid"))
            .unwrap()
            .parse()
            .unwrap();
        // SAFETY: open a synchronization handle, poll once, and close it exactly once.
        unsafe {
            let handle = OpenProcess(0x00100000, 0, pid);
            assert!(!handle.is_null(), "the game must still exist");
            let status = WaitForSingleObject(handle, 0);
            CloseHandle(handle);
            assert_eq!(status, 0x00000102, "the game must still be running");
        }
    }

    const ROTATED: &str = "fake-rotated-token";

    /// Expire the pull's access token once the game starts, so the exit push rotates it.
    struct Rotating {
        fake: FakeDropbox,
        dir: PathBuf,
        expired: AtomicBool,
    }

    impl Transport for Rotating {
        fn send(&self, request: &Request) -> Result<Response> {
            if self.dir.join("started").exists() && !self.expired.swap(true, Ordering::SeqCst) {
                self.fake.expire_access_tokens();
            }
            let response = self.fake.send(request)?;
            if request.url.ends_with("/oauth2/token") && self.expired.load(Ordering::SeqCst) {
                assert_eq!(response.status, 200);
                let mut answer: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
                answer["refresh_token"] = ROTATED.into();
                return Ok(Response::new(200, answer.to_string()));
            }
            Ok(response)
        }
    }

    struct ObservedCredentials {
        inner: MemoryStore,
        handler: Mutex<Option<thread::JoinHandle<i32>>>,
    }

    impl CredentialStore for ObservedCredentials {
        fn get(&self, account: &str) -> Result<Option<Secret>> {
            self.inner.get(account)
        }
        fn set(&self, account: &str, secret: &Secret) -> Result<()> {
            if secret.expose() == ROTATED {
                // A close right between wrap::run returning and credential storage must
                // wait for this write. The former completion inside run fails this check.
                let handler = thread::spawn(|| unsafe { console::on_event(2) });
                while console::received().is_none() {
                    thread::sleep(Duration::from_millis(10));
                }
                thread::sleep(Duration::from_millis(150));
                assert!(
                    !handler.is_finished(),
                    "handler released before token storage"
                );
                *self.handler.lock().unwrap() = Some(handler);
            }
            self.inner.set(account, secret)
        }
        fn delete(&self, account: &str) -> Result<()> {
            self.inner.delete(account)
        }
        fn kind(&self) -> &'static str {
            "memory"
        }
    }

    #[test]
    #[ignore = "spawned as the CLI wrap host"]
    fn console_token_host() {
        let Some(dir) = std::env::var_os(DIR).map(PathBuf::from) else {
            return;
        };
        fs::create_dir_all(dir.join("saves")).unwrap();
        let vars = PathVars {
            home: dir.join("home"),
            data: dir.join("data"),
            config: dir.join("config"),
            documents: None,
        };
        let store = LocalStore::new(AppDirs::from_vars(&vars));
        store
            .save_config(&crate::local::Config {
                format: 1,
                server: "dropbox".into(),
                user: "test-account".into(),
                root: "saves".into(),
                device: "console-test".into(),
            })
            .unwrap();
        let manifest = dir.join("game.json");
        fs::write(&manifest, r#"{"format":1,"id":"console-game","name":"Console game","saves_from":"launcher","saves":{},"include":["*.sav"]}"#).unwrap();
        let creds = ObservedCredentials {
            inner: MemoryStore::new(),
            handler: Mutex::new(None),
        };
        let account = token_account(Service::Dropbox, "test-account").unwrap();
        creds
            .inner
            .set(&account, &Secret::new(REFRESH_TOKEN))
            .unwrap();
        let net = Net {
            transport: Arc::new(Rotating {
                fake: FakeDropbox::new(),
                dir: dir.clone(),
                expired: AtomicBool::new(false),
            }),
            open_browser: &|_| panic!("no browser needed"),
            clients: ClientSet {
                dropbox: Client {
                    id: "test-dropbox",
                    secret: None,
                },
                ..ClientSet::shipped()
            },
            game_mode: false,
            clock: || 1000,
        };
        // The game exits on its own; the close arrives during credential storage.
        fs::write(dir.join("exit-game"), "").unwrap();
        let args = vec![
            "wrap".into(),
            "--game".into(),
            "console-game".into(),
            "--manifest".into(),
            manifest.to_string_lossy().into_owned(),
            "--saves-dir".into(),
            dir.join("saves").to_string_lossy().into_owned(),
            "--".into(),
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "--exact".into(),
            "wrap::tests::console_game".into(),
            "--ignored".into(),
        ];
        assert_eq!(
            run_with_net(
                &args,
                &mut &b""[..],
                &mut Vec::new(),
                &MemoryRemote::new(),
                &creds,
                &store,
                &vars,
                1000,
                &net
            ),
            0
        );
        assert!(creds.inner.get(&account).unwrap().unwrap().expose() == ROTATED);
        let handler = creds
            .handler
            .lock()
            .unwrap()
            .take()
            .expect("token rotated during exit push");
        assert_eq!(handler.join().unwrap(), 1);
    }

    #[test]
    fn console_handler_waits_for_rotated_token_storage() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/console-tests")
            .join(format!("{}-token", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "wrap::tests::console_token_host",
                "--ignored",
                "--nocapture",
            ])
            .env(DIR, &dir)
            .status()
            .unwrap();
        assert!(status.success());
        fs::remove_dir_all(dir).unwrap();
    }
}
