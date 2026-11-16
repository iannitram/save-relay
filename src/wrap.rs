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
}

/// How long a game that was asked to stop gets to exit on its own before `wrap` ends it (F80,
/// the #262 review). A game that ignores the stop would otherwise keep writing its saves while
/// the exit push runs, so the push waits for the game to be gone: after this many seconds the
/// game is killed and then pushed. Windows ends a process whose console handler has not returned
/// within its own window (about 5 s on close, 5 to 20 s on logoff and shutdown), so the stop, this
/// wait and the push together must fit in that window; a push Windows cuts short leaves the mark
/// for the next launch, as a second signal does.
pub const STOP_WAIT_SECS: u64 = 3;

#[cfg(windows)]
use console as stop;
#[cfg(unix)]
use signals as stop;

/// Start the game and wait for it, or for a termination request (F80, audit S3; #264 on
/// Windows). On a request the game is asked to stop (`stop::forward`) and given `STOP_WAIT_SECS`
/// to exit (`stop_game`); the exit push then runs. Unix and Windows only; elsewhere the game is
/// waited for.
fn play(command: &[String]) -> Result<Played> {
    // Before the game starts: a request sent once the game runs must find the handlers in place.
    #[cfg(any(unix, windows))]
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
                stop_game(&mut child);
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

/// After a stop request: wait up to `STOP_WAIT_SECS` for the game to exit, then end it and reap
/// it, so the exit push never runs beside a game that is still writing. A game that has gone is
/// the normal case; `kill` and `wait` on it are best effort.
#[cfg(any(unix, windows))]
fn stop_game(child: &mut Child) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(STOP_WAIT_SECS);
    while std::time::Instant::now() < until {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    note(format!(
        "the game did not stop within {STOP_WAIT_SECS} s; ending it"
    ));
    let _ = child.kill();
    let _ = child.wait();
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
/// is recorded for the wait loop in `play`, and the handler returns only once the exit push has
/// finished (`finished`) or `CONSOLE_WAIT_SECS` have passed: returning TRUE early lets Windows end
/// the process. A later event ends `wrap` at once with `_exit`, which leaves `playing.json`.
#[cfg(windows)]
mod console {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Once;
    use std::time::{Duration, Instant};

    const CTRL_CLOSE_EVENT: u32 = 2;
    const CTRL_LOGOFF_EVENT: u32 = 5;
    const CTRL_SHUTDOWN_EVENT: u32 = 6;
    const FALSE: i32 = 0;
    const TRUE: i32 = 1;
    /// The longest the handler waits for the exit push. Windows ends a process whose handler is
    /// still running after its own window (about 5 s on close, 5 to 20 s on logoff and shutdown),
    /// so this only keeps a hung push from holding the handler thread forever.
    pub const CONSOLE_WAIT_SECS: u64 = 20;

    /// The first event received; 0 while none has come.
    static FIRST: AtomicU32 = AtomicU32::new(0);
    /// Set once the exit push is done, releasing the handler.
    static FINISHED: AtomicBool = AtomicBool::new(false);
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
        if FIRST
            .compare_exchange(0, event, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            // SAFETY: `_exit` ends the process and never returns.
            unsafe { _exit(128 + event as i32) }
        }
        let until = Instant::now() + Duration::from_secs(CONSOLE_WAIT_SECS);
        while !FINISHED.load(Ordering::SeqCst) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        TRUE
    }

    /// Register `on_event` once (a second registration would run it twice per event).
    pub fn install() {
        INSTALL.call_once(|| {
            // SAFETY: `on_event` is an `extern "system"` routine that only touches atomics,
            // sleeps and `_exit`.
            unsafe {
                SetConsoleCtrlHandler(Some(on_event), TRUE);
            }
        });
    }

    /// The first event received, if any.
    pub fn received() -> Option<i32> {
        match FIRST.load(Ordering::SeqCst) {
            0 => None,
            event => Some(event as i32),
        }
    }

    /// Nothing to send: the console delivers the same event to the game, which shares it.
    pub fn forward(_pid: u32, _event: i32) {}

    /// The exit push is done: release the handler.
    pub fn finished() {
        FINISHED.store(true, Ordering::SeqCst);
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
    #[cfg(windows)]
    console::finished();
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

/// #264: a console close event on a Windows host. The console cannot be asked for a close event
/// from a test, so the event goes to the registered routine directly, on its own thread, as the
/// console would call it. The game is `ping` (it does not stop on its own); the handler must
/// return only after the exit push has run and cleared the mark.
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::platform::{AppDirs, PathVars};
    use std::fs;
    use std::path::PathBuf;
    use std::thread;

    const CTRL_CLOSE_EVENT: u32 = 2;
    const TRUE: i32 = 1;

    /// The scripted engine: `pull` fails (no server, the game starts at once); the exit `push`
    /// removes `mark` and writes `pushed`.
    struct Scripted {
        dir: PathBuf,
    }

    impl Engine for Scripted {
        fn game(&self) -> &str {
            "console-game"
        }
        fn pull(&self, _resolve: Option<Resolve>, _anyway: bool) -> Result<SyncReport> {
            Err(Error::Io("no server in this test".into()))
        }
        fn push(&self, _resolve: Option<Resolve>) -> Result<SyncReport> {
            let _ = fs::remove_file(self.dir.join("mark"));
            fs::write(self.dir.join("pushed"), "").expect("write pushed");
            Err(Error::Io("no server in this test".into()))
        }
    }

    fn clock() -> u64 {
        1_793_476_260
    }

    fn no_wait(_: u64) {}

    #[test]
    fn a_console_close_stops_the_game_runs_the_exit_push_and_clears_the_mark() {
        let dir = std::env::temp_dir().join(format!("save-relay-console-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("the scratch folder");
        fs::write(dir.join("mark"), "").expect("the mark");
        let vars = PathVars {
            home: dir.join("home"),
            data: dir.join("data"),
            config: dir.join("config"),
            documents: None,
        };
        let store = LocalStore::new(AppDirs::from_vars(&vars));
        let engine = Scripted { dir: dir.clone() };
        let command: Vec<String> = ["ping", "-n", "60", "127.0.0.1"]
            .into_iter()
            .map(String::from)
            .collect();

        let host = thread::spawn(move || run(&engine, &store, &command, clock, &no_wait));
        // SAFETY: the registered routine, called as the console calls it, on a thread of its own.
        let began = std::time::Instant::now();
        let handler = thread::spawn(|| unsafe { console::on_event(CTRL_CLOSE_EVENT) });
        let returned = handler.join().expect("the handler thread");
        // The handler returned: it must have waited for the push, which ran and cleared the mark,
        // and not given up at its bound.
        assert!(
            began.elapsed() < std::time::Duration::from_secs(console::CONSOLE_WAIT_SECS),
            "the handler gave up at its bound instead of waiting for the push"
        );
        assert!(
            dir.join("pushed").exists(),
            "the handler returned before the exit push ran"
        );
        assert!(!dir.join("mark").exists(), "playing.json mark is still set");
        assert_eq!(
            returned, TRUE,
            "the handler must return TRUE after the push"
        );
        let code = host.join().expect("the wrap thread");
        assert_eq!(
            code,
            128 + CTRL_CLOSE_EVENT as i32,
            "the exit code after a close event"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
