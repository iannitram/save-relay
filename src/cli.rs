//! The `save-relay` command line (SPEC.md M8). Human text by default; `--json` prints exactly
//! one envelope line (`api`) for launchers.
//!
//! ```text
//! save-relay pull    <game> [--manifest <file>] [--saves-dir <dir>] [--resolve keep-server|keep-local] [--json]
//! save-relay push    <game> [--manifest <file>] [--saves-dir <dir>] [--resolve keep-server|keep-local] [--json]
//! save-relay status  <game> [--manifest <file>] [--saves-dir <dir>] [--json]
//! save-relay history <game> [--json]
//! save-relay restore <game> <entry> [--manifest <file>] [--saves-dir <dir>] [--json]
//! save-relay setup   [--service nextcloud] --server <url> --user <name> [--device <name>] [--root <folder>] [--json]
//!                    (reads the app password from the first line of standard input)
//! save-relay setup   --service gdrive|dropbox [--no-browser | --finish] [--device <name>] [--root <folder>] [--json]
//!                    (signs in through the browser; --finish reads Dropbox's code from standard input)
//! save-relay setup   [--device <name>] [--root <folder>]
//!                    (no --service, --server or --user: asks which service on standard input)
//! save-relay help | --help | -h
//! save-relay version | --version | -V
//! ```

use crate::error::{not_built, Result};
use crate::service::Service;
use crate::sync::Resolve;
use std::io::{Read, Write};
use std::path::PathBuf;

/// The default server folder that holds every game.
pub const DEFAULT_ROOT: &str = "save-relay";

/// A game named on the command line. With `--manifest`, the manifest's id must equal `id`;
/// without it, the manifest remembered on this machine is used (`local::GameRecord`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameArg {
    pub id: String,
    pub manifest: Option<PathBuf>,
    pub saves_dir: Option<PathBuf>,
}

/// How `setup` signs in to Google Drive or Dropbox (`oauth`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInStep {
    /// Open the browser here and wait for the loopback redirect (the default outside Game Mode).
    Browser,
    /// `--no-browser`: print the sign-in to finish on any device (`api::SignInPrompt`).
    Code,
    /// `--finish`: complete the sign-in a `--no-browser` run started.
    Finish,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Pull {
        game: GameArg,
        resolve: Option<Resolve>,
    },
    Push {
        game: GameArg,
        resolve: Option<Resolve>,
    },
    Status {
        game: GameArg,
    },
    History {
        game: String,
    },
    Restore {
        game: GameArg,
        entry: String,
    },
    /// Nextcloud: `--server` and `--user` (with or without `--service nextcloud`).
    Setup {
        server: String,
        user: String,
        device: Option<String>,
        root: String,
    },
    /// Google Drive or Dropbox (`--service gdrive|dropbox`), or `service: None` when setup got
    /// none of `--service`, `--server`, `--user`: `run` then asks which service on standard input.
    SetupService {
        service: Option<Service>,
        device: Option<String>,
        root: String,
        step: SignInStep,
    },
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub command: Command,
    pub json: bool,
}

/// Parse the arguments after the program name. Flags may come in any order after the command;
/// `--json` anywhere. A flag's value is the next argument (`--flag=value` is not accepted).
/// No arguments is `Command::Help`. Errors are `Error::Usage` with one line saying what is
/// wrong: an unknown command or flag, a missing `<game>` or `<entry>`, an extra positional, a
/// flag given twice or without its value, a `--resolve` other than `keep-server`/`keep-local`,
/// `--manifest`/`--saves-dir`/`--resolve` on a command that does not take them, `setup` without
/// `--server` or `--user`, and `--password` (the message says the password is read from
/// standard input, so it never shows in a process list). `--root` defaults to `DEFAULT_ROOT`.
///
/// `setup` and the service (docs/code-map.md § API shape): `--service nextcloud`, or no
/// `--service` with `--server` or `--user`, is `Command::Setup` (both needed, as above);
/// `--service gdrive|dropbox` is `Command::SetupService` (`--server`/`--user` with it, a
/// `--service` other than the three keys, `--no-browser` or `--finish` without an OAuth
/// service, and `--no-browser` with `--finish` are each `Error::Usage` naming the flag); no
/// `--service`, `--server` or `--user` is `SetupService { service: None, .. }`, except with
/// `--json`, where it is `Error::Usage` naming `--service` (a launcher must say which).
pub fn parse(args: &[String]) -> Result<Args> {
    not_built("cli::parse")
}

/// The command's name as it appears in the envelope: "pull", "push", "status", "history",
/// "restore", "setup" (both setup forms), "help", "version".
pub fn command_name(command: &Command) -> &'static str {
    crate::not_built!("cli::command_name")
}

/// The help text printed by `help`.
pub fn usage() -> &'static str {
    crate::not_built!("cli::usage")
}

/// Run one command end to end: parse, load the config, the credential store and the game,
/// connect, call `sync`, print text or the `--json` envelope to `stdout`, and return the exit
/// code (`api::exit_code_for`, `api::exit_code`). A parse error with `--json` among the
/// arguments is still printed as an envelope (command "unknown" when none was recognised).
pub fn run(args: &[String], stdin: &mut dyn Read, stdout: &mut dyn Write) -> i32 {
    let _ = writeln!(stdout, "save-relay: not built yet");
    1
}
