//! The `save-relay` command line (SPEC.md M8). Human text by default; `--json` prints exactly
//! one envelope line (`api`) for launchers.
//!
//! ```text
//! save-relay pull    <game> [--manifest <file>] [--saves-dir <dir>] [--resolve keep-server|keep-local] [--json]
//! save-relay push    <game> [--manifest <file>] [--saves-dir <dir>] [--resolve keep-server|keep-local] [--json]
//! save-relay status  <game> [--manifest <file>] [--saves-dir <dir>] [--json]
//! save-relay history <game> [--json]
//! save-relay restore <game> <entry> [--manifest <file>] [--saves-dir <dir>] [--json]
//! save-relay pull    <game> --anyway [...]       (play past another machine's open-elsewhere mark)
//! save-relay wrap    [--game <id>] [--manifest <file>] [--saves-dir <dir>] -- <command>...
//!                    (Steam launch option `save-relay wrap -- %command%`: pull, run, push)
//! save-relay which   <appid> [--json]
//! save-relay install-wrap (<appid>... | --all-known) [--remove] [--dry-run] [--json]
//! save-relay answer  <game> keep-server|keep-local|play-anyway|wait [--json]
//! save-relay where   [--json]
//! save-relay picture <game> <entry> --out <file> [--json]
//! save-relay panel   [--port <n>] [--no-open] [--json]
//! save-relay setup   [--service nextcloud] --server <url> --user <name> [--device <name>] [--root <folder>] [--json]
//!                    (reads the app password from the first line of standard input)
//! save-relay setup   --service gdrive|dropbox [--no-browser | --finish] [--device <name>] [--root <folder>] [--json]
//!                    (signs in through the browser; --finish reads Dropbox's code from standard input)
//! save-relay setup   [--device <name>] [--root <folder>]
//!                    (no --service, --server or --user: asks which service on standard input)
//! save-relay help | --help | -h
//! save-relay version | --version | -V
//! ```

use crate::error::{Error, Result};
use crate::pending::Answer;
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
    /// `pull --anyway` (M11): a sibling of `Pull`, so #10's struct literals stay valid.
    PullAnyway {
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
    /// M12: `wrap [--game <id>] [--manifest <file>] [--saves-dir <dir>] -- <command>...`.
    /// No `--game` is `steam-<appid>` from the environment (`wrap::game_id`); `command` is
    /// everything after `--` and may not be empty.
    Wrap {
        game: Option<String>,
        manifest: Option<PathBuf>,
        saves_dir: Option<PathBuf>,
        command: Vec<String>,
    },
    /// M13: `which <appid>`.
    Which {
        app_id: u32,
    },
    /// M12: `install-wrap <appid>...` or `--all-known` (every app id in the ludusavi data that
    /// is installed); not both, and one of them.
    InstallWrap {
        app_ids: Vec<u32>,
        all_known: bool,
        remove: bool,
        dry_run: bool,
    },
    /// M12: `answer <game> <answer>` (`pending::Answer::parse`).
    Answer {
        game: String,
        answer: Answer,
    },
    /// M11: `where`.
    Where,
    /// M14: `picture <game> <entry> --out <file>`.
    Picture {
        game: String,
        entry: String,
        out: PathBuf,
    },
    /// M15: `panel [--port <n>] [--no-open]`; `open` is false with `--no-open`. With `--json`
    /// it prints `api::PanelData` as one envelope line once it listens, then serves.
    Panel {
        port: Option<u16>,
        open: bool,
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
    if args.iter().any(|arg| arg == "--password") {
        return Err(Error::Usage(
            "the password is read from standard input, not from --password".to_string(),
        ));
    }
    let json = args.iter().any(|arg| arg == "--json");
    let args: Vec<&str> = args
        .iter()
        .filter(|arg| arg.as_str() != "--json")
        .map(String::as_str)
        .collect();
    if args.is_empty() {
        return Ok(Args {
            command: Command::Help,
            json,
        });
    }
    let command = match args[0] {
        "help" | "--help" | "-h" => Command::Help,
        "version" | "--version" | "-V" => Command::Version,
        "pull" => {
            let flags = split(&args[1..], &["--manifest", "--saves-dir", "--resolve"])?;
            require(&flags.positionals, 1, &["<game>"])?;
            Command::Pull {
                game: game_arg(flags.positionals[0], &flags),
                resolve: resolve(&flags)?,
            }
        }
        "push" => {
            let flags = split(&args[1..], &["--manifest", "--saves-dir", "--resolve"])?;
            require(&flags.positionals, 1, &["<game>"])?;
            Command::Push {
                game: game_arg(flags.positionals[0], &flags),
                resolve: resolve(&flags)?,
            }
        }
        "status" => {
            let flags = split(&args[1..], &["--manifest", "--saves-dir"])?;
            require(&flags.positionals, 1, &["<game>"])?;
            Command::Status {
                game: game_arg(flags.positionals[0], &flags),
            }
        }
        "history" => {
            let flags = split(&args[1..], &[])?;
            require(&flags.positionals, 1, &["<game>"])?;
            Command::History {
                game: flags.positionals[0].to_string(),
            }
        }
        "restore" => {
            let flags = split(&args[1..], &["--manifest", "--saves-dir"])?;
            require(&flags.positionals, 2, &["<game>", "<entry>"])?;
            Command::Restore {
                game: game_arg(flags.positionals[0], &flags),
                entry: flags.positionals[1].to_string(),
            }
        }
        "setup" => {
            let flags = split(&args[1..], &["--server", "--user", "--device", "--root"])?;
            require(&flags.positionals, 0, &[])?;
            let server = match flags.flag("--server") {
                Some(value) => value.to_string(),
                None => return missing("--server"),
            };
            let user = match flags.flag("--user") {
                Some(value) => value.to_string(),
                None => return missing("--user"),
            };
            Command::Setup {
                server,
                user,
                device: flags.flag("--device").map(String::from),
                root: flags.flag("--root").unwrap_or(DEFAULT_ROOT).to_string(),
            }
        }
        other => {
            return Err(Error::Usage(format!("unknown command '{other}'")));
        }
    };
    Ok(Args { command, json })
}

struct Flags<'a> {
    positionals: Vec<&'a str>,
    values: Vec<(&'a str, &'a str)>,
}

impl<'a> Flags<'a> {
    fn flag(&self, name: &str) -> Option<&'a str> {
        for (flag, value) in &self.values {
            if *flag == name {
                return Some(*value);
            }
        }
        None
    }
}

fn split<'a>(args: &[&'a str], allowed: &[&str]) -> Result<Flags<'a>> {
    let mut flags = Flags {
        positionals: Vec::new(),
        values: Vec::new(),
    };
    let mut index = 0;
    while index < args.len() {
        let arg = args[index];
        if arg.starts_with('-') && arg != "-" {
            if !allowed.contains(&arg) {
                return Err(Error::Usage(format!("unknown flag '{arg}'")));
            }
            if flags.values.iter().any(|(name, _)| *name == arg) {
                return Err(Error::Usage(format!("flag '{arg}' given twice")));
            }
            let value = match args.get(index + 1) {
                Some(value) => *value,
                None => return Err(Error::Usage(format!("flag '{arg}' needs a value"))),
            };
            flags.values.push((arg, value));
            index += 2;
        } else {
            flags.positionals.push(arg);
            index += 1;
        }
    }
    Ok(flags)
}

fn require(positionals: &[&str], count: usize, missing: &[&str]) -> Result<()> {
    if positionals.len() < count {
        return missing_value(missing[positionals.len()]);
    }
    if positionals.len() > count {
        return Err(Error::Usage(format!(
            "unexpected argument '{}'",
            positionals[count]
        )));
    }
    Ok(())
}

fn missing<T>(what: &str) -> Result<T> {
    missing_value(what)
}

fn missing_value<T>(what: &str) -> Result<T> {
    Err(Error::Usage(format!("missing {what}")))
}

fn game_arg(id: &str, flags: &Flags<'_>) -> GameArg {
    GameArg {
        id: id.to_string(),
        manifest: flags.flag("--manifest").map(PathBuf::from),
        saves_dir: flags.flag("--saves-dir").map(PathBuf::from),
    }
}

fn resolve(flags: &Flags<'_>) -> Result<Option<Resolve>> {
    match flags.flag("--resolve") {
        None => Ok(None),
        Some("keep-server") => Ok(Some(Resolve::KeepServer)),
        Some("keep-local") => Ok(Some(Resolve::KeepLocal)),
        Some(other) => Err(Error::Usage(format!(
            "unknown --resolve '{other}': use keep-server or keep-local"
        ))),
    }
}

/// The command's name as it appears in the envelope: "pull" (`Pull` and `PullAnyway`), "push",
/// "status", "history", "restore", "setup" (both setup forms), "wrap", "which",
/// "install-wrap", "answer", "where", "picture", "panel", "help", "version".
pub fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Pull { .. } | Command::PullAnyway { .. } => "pull",
        Command::Push { .. } => "push",
        Command::Status { .. } => "status",
        Command::History { .. } => "history",
        Command::Restore { .. } => "restore",
        Command::Setup { .. } | Command::SetupService { .. } => "setup",
        Command::Wrap { .. } => "wrap",
        Command::Which { .. } => "which",
        Command::InstallWrap { .. } => "install-wrap",
        Command::Answer { .. } => "answer",
        Command::Where => "where",
        Command::Picture { .. } => "picture",
        Command::Panel { .. } => "panel",
        Command::Help => "help",
        Command::Version => "version",
    }
}

/// The help text printed by `help`.
pub fn usage() -> &'static str {
    "save-relay <command> [--json]

Commands:
  pull    <game> [--manifest <file>] [--saves-dir <dir>] [--resolve keep-server|keep-local]
  push    <game> [--manifest <file>] [--saves-dir <dir>] [--resolve keep-server|keep-local]
  status  <game> [--manifest <file>] [--saves-dir <dir>]
  history <game>
  restore <game> <entry> [--manifest <file>] [--saves-dir <dir>]
  setup   --server <url> --user <name> [--device <name>] [--root <folder>]
          (reads the app password from the first line of standard input)
  help | --help | -h
  version | --version | -V

--json prints exactly one envelope line for launchers.
"
}

/// Run one command end to end: parse, load the config, the credential store and the game,
/// connect, call `sync`, print text or the `--json` envelope to `stdout`, and return the exit
/// code (`api::exit_code_for`, `api::exit_code`). A parse error with `--json` among the
/// arguments is still printed as an envelope (command "unknown" when none was recognised).
pub fn run(args: &[String], stdin: &mut dyn Read, stdout: &mut dyn Write) -> i32 {
    let _ = writeln!(stdout, "save-relay: not built yet");
    1
}
