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

use crate::api::{self, Action, HistoryData, SetupData, Side, SyncReport, WhereData};
use crate::creds::{self, CredentialStore, Secret};
use crate::error::{Error, NotBuilt, Result};
use crate::layout::{self, Layout};
use crate::local::{self, Config, GameRecord, LocalStore};
use crate::manifest::Manifest;
use crate::pending::Answer;
use crate::platform::{AppDirs, PathVars, Platform};
use crate::remote::{self, Precondition, Remote, RemoteEntry};
use crate::service::Service;
use crate::stamp;
use crate::sync::{self, Game, Resolve, Session};
use crate::webdav;
#[cfg(feature = "http")]
use crate::webdav::WebDav;
use serde::Serialize;
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

impl GameArg {
    /// The manifest and saves folder this command works on, read without writing anything, so
    /// a refusal comes before any server call or file write. The manifest is `--manifest` (its
    /// id must equal `id`) or the one remembered here (`Error::GameUnknown` when neither). The
    /// folder is `--saves-dir` verbatim; else, for a `saves_from` game, `Error::Usage` naming
    /// `--saves-dir` (never a remembered or guessed folder, #48); else the remembered
    /// `--saves-dir`, else the manifest's template for `platform`.
    pub fn load(&self, store: &LocalStore, platform: Platform, vars: &PathVars) -> Result<Game> {
        let record = store.load_game(&self.id)?;
        let manifest = match &self.manifest {
            Some(path) => {
                let text = std::fs::read_to_string(path).map_err(|error| {
                    Error::Io(format!("cannot read manifest {}: {error}", path.display()))
                })?;
                let manifest = Manifest::from_json(&text)?;
                if manifest.id != self.id {
                    return Err(Error::Usage(format!(
                        "--manifest is for '{}', not '{}'",
                        manifest.id, self.id
                    )));
                }
                manifest
            }
            None => match &record {
                Some(record) => record.manifest.clone(),
                None => return Err(Error::GameUnknown(self.id.clone())),
            },
        };
        let saves_dir = match &self.saves_dir {
            Some(folder) => folder.clone(),
            None if manifest.saves_from.is_some() => {
                return Err(crate::manifest::saves_dir_required(&manifest.id))
            }
            None => match record.and_then(|record| record.saves_dir) {
                Some(folder) => folder,
                None => manifest.saves_dir(platform, vars)?,
            },
        };
        Ok(Game {
            manifest,
            saves_dir,
        })
    }
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
    let first_non_json = args
        .iter()
        .find(|arg| arg.as_str() != "--json")
        .map(String::as_str);

    if first_non_json == Some("wrap") {
        let dash_pos = args.iter().position(|arg| arg.as_str() == "--");
        let before_dash = match dash_pos {
            Some(idx) => &args[..idx],
            None => args,
        };
        if before_dash.iter().any(|arg| arg.as_str() == "--password") {
            return Err(Error::Usage(
                "the password is read from standard input, not from --password".to_string(),
            ));
        }
        let dash_idx = match dash_pos {
            Some(idx) => idx,
            None => return missing("--"),
        };
        let command: Vec<String> = args[dash_idx + 1..].to_vec();
        if command.is_empty() {
            return missing("<command>");
        }
        let json = args[..dash_idx].iter().any(|arg| arg.as_str() == "--json");
        let before: Vec<&str> = args[..dash_idx]
            .iter()
            .filter(|arg| arg.as_str() != "--json")
            .map(String::as_str)
            .collect();
        let flags = split(&before[1..], &["--game", "--manifest", "--saves-dir"], &[])?;
        require(&flags.positionals, 0, &[])?;
        let game = flags.flag("--game").map(String::from);
        let manifest = flags.flag("--manifest").map(PathBuf::from);
        let saves_dir = flags.flag("--saves-dir").map(PathBuf::from);
        return Ok(Args {
            command: Command::Wrap {
                game,
                manifest,
                saves_dir,
                command,
            },
            json,
        });
    }

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
            let flags = split(
                &args[1..],
                &["--manifest", "--saves-dir", "--resolve"],
                &["--anyway"],
            )?;
            require(&flags.positionals, 1, &["<game>"])?;
            let game = game_arg(flags.positionals[0], &flags);
            let resolve = resolve(&flags)?;
            if flags.has("--anyway") {
                Command::PullAnyway { game, resolve }
            } else {
                Command::Pull { game, resolve }
            }
        }
        "push" => {
            let flags = split(&args[1..], &["--manifest", "--saves-dir", "--resolve"], &[])?;
            require(&flags.positionals, 1, &["<game>"])?;
            Command::Push {
                game: game_arg(flags.positionals[0], &flags),
                resolve: resolve(&flags)?,
            }
        }
        "status" => {
            let flags = split(&args[1..], &["--manifest", "--saves-dir"], &[])?;
            require(&flags.positionals, 1, &["<game>"])?;
            Command::Status {
                game: game_arg(flags.positionals[0], &flags),
            }
        }
        "history" => {
            let flags = split(&args[1..], &[], &[])?;
            require(&flags.positionals, 1, &["<game>"])?;
            Command::History {
                game: flags.positionals[0].to_string(),
            }
        }
        "restore" => {
            let flags = split(&args[1..], &["--manifest", "--saves-dir"], &[])?;
            require(&flags.positionals, 2, &["<game>", "<entry>"])?;
            Command::Restore {
                game: game_arg(flags.positionals[0], &flags),
                entry: flags.positionals[1].to_string(),
            }
        }
        "setup" => {
            let flags = split(
                &args[1..],
                &["--service", "--server", "--user", "--device", "--root"],
                &["--no-browser", "--finish"],
            )?;
            require(&flags.positionals, 0, &[])?;
            let server = flags.flag("--server");
            let user = flags.flag("--user");
            let device = flags.flag("--device").map(String::from);
            let root = flags.flag("--root").unwrap_or(DEFAULT_ROOT).to_string();
            let no_browser = flags.has("--no-browser");
            let finish = flags.has("--finish");
            if no_browser && finish {
                return Err(Error::Usage(
                    "--no-browser and --finish cannot be used together".to_string(),
                ));
            }
            let step = if no_browser {
                SignInStep::Code
            } else if finish {
                SignInStep::Finish
            } else {
                SignInStep::Browser
            };
            let service = match flags.flag("--service") {
                Some("nextcloud") => Some(Service::Nextcloud),
                Some("gdrive") => Some(Service::Gdrive),
                Some("dropbox") => Some(Service::Dropbox),
                Some(value) => {
                    return Err(Error::Usage(format!(
                        "unknown service '{value}': use nextcloud, gdrive or dropbox"
                    )));
                }
                None => None,
            };
            match service {
                Some(Service::Nextcloud) => {
                    if no_browser {
                        return Err(Error::Usage(
                            "--no-browser requires an OAuth service".to_string(),
                        ));
                    }
                    if finish {
                        return Err(Error::Usage(
                            "--finish requires an OAuth service".to_string(),
                        ));
                    }
                    let server = match server {
                        Some(value) => value.to_string(),
                        None => return missing("--server"),
                    };
                    let user = match user {
                        Some(value) => value.to_string(),
                        None => return missing("--user"),
                    };
                    Command::Setup {
                        server,
                        user,
                        device,
                        root,
                    }
                }
                None if server.is_some() || user.is_some() => {
                    if no_browser {
                        return Err(Error::Usage(
                            "--no-browser requires an OAuth service".to_string(),
                        ));
                    }
                    if finish {
                        return Err(Error::Usage(
                            "--finish requires an OAuth service".to_string(),
                        ));
                    }
                    let server = match server {
                        Some(value) => value.to_string(),
                        None => return missing("--server"),
                    };
                    let user = match user {
                        Some(value) => value.to_string(),
                        None => return missing("--user"),
                    };
                    Command::Setup {
                        server,
                        user,
                        device,
                        root,
                    }
                }
                Some(service @ (Service::Gdrive | Service::Dropbox)) => {
                    if server.is_some() {
                        return Err(Error::Usage(
                            "--server cannot be used with this service".to_string(),
                        ));
                    }
                    if user.is_some() {
                        return Err(Error::Usage(
                            "--user cannot be used with this service".to_string(),
                        ));
                    }
                    Command::SetupService {
                        service: Some(service),
                        device,
                        root,
                        step,
                    }
                }
                None => {
                    if no_browser {
                        return Err(Error::Usage(
                            "--no-browser requires an OAuth service".to_string(),
                        ));
                    }
                    if finish {
                        return Err(Error::Usage(
                            "--finish requires an OAuth service".to_string(),
                        ));
                    }
                    if json {
                        return Err(Error::Usage("--json setup requires --service".to_string()));
                    }
                    Command::SetupService {
                        service: None,
                        device,
                        root,
                        step: SignInStep::Browser,
                    }
                }
            }
        }
        "which" => {
            let flags = split(&args[1..], &[], &[])?;
            require(&flags.positionals, 1, &["<appid>"])?;
            let app_id = parse_app_id(flags.positionals[0])?;
            Command::Which { app_id }
        }
        "install-wrap" => {
            let flags = split(&args[1..], &[], &["--all-known", "--remove", "--dry-run"])?;
            let all_known = flags.has("--all-known");
            let remove = flags.has("--remove");
            let dry_run = flags.has("--dry-run");
            if all_known && !flags.positionals.is_empty() {
                return Err(Error::Usage(
                    "cannot use both <appid> and --all-known".to_string(),
                ));
            }
            if !all_known && flags.positionals.is_empty() {
                return Err(Error::Usage("missing <appid> or --all-known".to_string()));
            }
            let mut app_ids = Vec::with_capacity(flags.positionals.len());
            for pos in &flags.positionals {
                app_ids.push(parse_app_id(pos)?);
            }
            app_ids.sort_unstable();
            app_ids.dedup();
            Command::InstallWrap {
                app_ids,
                all_known,
                remove,
                dry_run,
            }
        }
        "answer" => {
            let flags = split(&args[1..], &[], &[])?;
            require(&flags.positionals, 2, &["<game>", "<answer>"])?;
            let answer = match Answer::parse(flags.positionals[1]) {
                Some(a) => a,
                None => {
                    return Err(Error::Usage(format!(
                        "unknown answer '{}': use keep-server, keep-local, play-anyway or wait",
                        flags.positionals[1]
                    )));
                }
            };
            Command::Answer {
                game: flags.positionals[0].to_string(),
                answer,
            }
        }
        "where" => {
            let flags = split(&args[1..], &[], &[])?;
            require(&flags.positionals, 0, &[])?;
            Command::Where
        }
        "picture" => {
            let flags = split(&args[1..], &["--out"], &[])?;
            require(&flags.positionals, 2, &["<game>", "<entry>"])?;
            let out = match flags.flag("--out") {
                Some(value) => PathBuf::from(value),
                None => return missing("--out"),
            };
            Command::Picture {
                game: flags.positionals[0].to_string(),
                entry: flags.positionals[1].to_string(),
                out,
            }
        }
        "panel" => {
            let flags = split(&args[1..], &["--port"], &["--no-open"])?;
            require(&flags.positionals, 0, &[])?;
            let port = match flags.flag("--port") {
                None => None,
                Some(value) => {
                    let p = match value.parse::<u16>() {
                        Ok(p) if p >= 1 => p,
                        _ => {
                            return Err(Error::Usage(format!(
                                "--port must be a whole number from 1 to {}",
                                u16::MAX
                            )));
                        }
                    };
                    Some(p)
                }
            };
            let open = !flags.has("--no-open");
            Command::Panel { port, open }
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
    switches: Vec<&'a str>,
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

    fn has(&self, name: &str) -> bool {
        self.switches.contains(&name)
    }
}

fn split<'a>(
    args: &[&'a str],
    allowed_values: &[&str],
    allowed_switches: &[&str],
) -> Result<Flags<'a>> {
    let mut flags = Flags {
        positionals: Vec::new(),
        values: Vec::new(),
        switches: Vec::new(),
    };
    let mut index = 0;
    while index < args.len() {
        let arg = args[index];
        let is_negative_number =
            arg.len() > 1 && arg.starts_with('-') && arg[1..].chars().all(|c| c.is_ascii_digit());
        if arg.starts_with('-') && arg != "-" && !is_negative_number {
            if allowed_switches.contains(&arg) {
                if flags.switches.contains(&arg) {
                    return Err(Error::Usage(format!("flag '{arg}' given twice")));
                }
                flags.switches.push(arg);
                index += 1;
            } else if allowed_values.contains(&arg) {
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
                return Err(Error::Usage(format!("unknown flag '{arg}'")));
            }
        } else {
            flags.positionals.push(arg);
            index += 1;
        }
    }
    Ok(flags)
}

fn parse_app_id(text: &str) -> Result<u32> {
    match text.parse::<u32>() {
        Ok(id) if id >= 1 => Ok(id),
        _ => Err(Error::Usage(format!(
            "<appid> must be a whole number from 1 to {}",
            u32::MAX
        ))),
    }
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
  pull    <game> --anyway [...]
  wrap    [--game <id>] [--manifest <file>] [--saves-dir <dir>] -- <command>...
  which   <appid>
  install-wrap (<appid>... | --all-known) [--remove] [--dry-run]
  answer  <game> keep-server|keep-local|play-anyway|wait
  where
  picture <game> <entry> --out <file>
  panel   [--port <n>] [--no-open]
  setup   --server <url> --user <name> [--device <name>] [--root <folder>]
          (reads the app password from the first line of standard input)
  help | --help | -h
  version | --version | -V

--json prints exactly one envelope line for launchers.
"
}

// ---------------------------------------------------------------------------------------
// Running a command
// ---------------------------------------------------------------------------------------

/// The service every command here talks to until the other two are wired in (F44).
const NEXTCLOUD: &str = "nextcloud";

/// What a command produced: the one envelope line, the text for people, the exit code.
struct Outcome {
    json: String,
    text: String,
    code: i32,
}

fn outcome<T: Serialize>(command: &str, data: &T, text: String, code: i32) -> Outcome {
    Outcome {
        json: api::success(command, data),
        text,
        code,
    }
}

/// Everything a command needs from outside, so tests can stand in for each part.
struct Context<'a> {
    remote: &'a dyn Remote,
    creds: &'a dyn CredentialStore,
    store: &'a LocalStore,
    vars: &'a PathVars,
    platform: Platform,
    now: u64,
}

/// Run one command end to end: parse, load the config, the credential store and the game,
/// connect, call `sync`, print text or the `--json` envelope to `stdout`, and return the exit
/// code (`api::exit_code_for`, `api::exit_code`). A parse error with `--json` among the
/// arguments is still printed as an envelope (command "unknown" when none was recognised).
///
/// This is the real environment: the platform folders from the process environment, the system
/// credential store and the real network. `run_with` does the work.
pub fn run(args: &[String], stdin: &mut dyn Read, stdout: &mut dyn Write) -> i32 {
    let json = args.iter().any(|arg| arg == "--json");
    let vars = match PathVars::from_env(Platform::current(), &|name| std::env::var(name).ok()) {
        Ok(vars) => vars,
        Err(error) => return print_failure(stdout, json, &guess_command(args), &error),
    };
    let store = LocalStore::new(AppDirs::from_vars(&vars));
    let creds = real_credentials();
    let now = stamp::now();

    // Only a command that talks to the server needs a connection; the others get a remote
    // that is never asked.
    let parsed = parse(args);
    let connected: Result<(Box<dyn Remote>, Option<String>)> = match &parsed {
        Ok(Args {
            command: Command::Setup { server, user, .. },
            ..
        }) => read_first_line(stdin).and_then(|line| {
            let remote = connect(server, user, Secret::new(line.clone()))?;
            Ok((remote, Some(line)))
        }),
        Ok(Args {
            command:
                Command::Pull { .. }
                | Command::Push { .. }
                | Command::Status { .. }
                | Command::History { .. }
                | Command::Restore { .. }
                | Command::Where
                | Command::Picture { .. },
            ..
        }) => connection(&store, &*creds).and_then(|(config, secret)| {
            Ok((connect(&config.server, &config.user, secret)?, None))
        }),
        _ => Ok((Box::new(NoRemote), None)),
    };
    let (remote, password_line) = match connected {
        Ok(pair) => pair,
        Err(error) => {
            let (name, json) = match &parsed {
                Ok(parsed) => (command_name(&parsed.command).to_string(), parsed.json),
                Err(_) => (guess_command(args), json),
            };
            return print_failure(stdout, json, &name, &error);
        }
    };
    match password_line {
        // `setup` read its password above; `run_with` reads it again from this one line.
        Some(line) => {
            let mut again = std::io::Cursor::new(format!("{line}\n"));
            run_with(
                args, &mut again, stdout, &*remote, &*creds, &store, &vars, now,
            )
        }
        None => run_with(args, stdin, stdout, &*remote, &*creds, &store, &vars, now),
    }
}

/// `run` with its parts passed in, so `tests/cli_run.rs` drives every command against
/// `MemoryRemote` and `MemoryStore` with no network and no keyring. `remote` is the server the
/// command talks to; `creds` holds the app password (`setup` writes it); `store` and `vars`
/// are this machine's folders; `now` is the clock in seconds since the Unix epoch.
#[allow(clippy::too_many_arguments)]
pub fn run_with(
    args: &[String],
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    remote: &dyn Remote,
    creds: &dyn CredentialStore,
    store: &LocalStore,
    vars: &PathVars,
    now: u64,
) -> i32 {
    let parsed = match parse(args) {
        Ok(parsed) => parsed,
        Err(error) => {
            let json = args.iter().any(|arg| arg == "--json");
            return print_failure(stdout, json, &guess_command(args), &error);
        }
    };
    let name = command_name(&parsed.command);
    let context = Context {
        remote,
        creds,
        store,
        vars,
        platform: Platform::current(),
        now,
    };
    match execute(parsed.command, stdin, &context) {
        Ok(done) => {
            let shown = if parsed.json { &done.json } else { &done.text };
            let _ = writeln!(stdout, "{}", shown.trim_end());
            done.code
        }
        Err(error) => print_failure(stdout, parsed.json, name, &error),
    }
}

fn print_failure(stdout: &mut dyn Write, json: bool, command: &str, error: &Error) -> i32 {
    if json {
        let _ = writeln!(stdout, "{}", api::failure(command, error));
    } else {
        let _ = writeln!(stdout, "save-relay: {error}");
        if matches!(error, Error::Usage(_)) {
            let _ = writeln!(stdout, "Run `save-relay help` for the commands.");
        }
    }
    api::exit_code(error)
}

/// The command word for an envelope when the arguments did not parse: the first argument that
/// is a command, else "unknown".
fn guess_command(args: &[String]) -> String {
    const NAMES: [&str; 15] = [
        "pull",
        "push",
        "status",
        "history",
        "restore",
        "setup",
        "wrap",
        "which",
        "install-wrap",
        "answer",
        "where",
        "picture",
        "panel",
        "help",
        "version",
    ];
    args.iter()
        .find(|arg| arg.as_str() != "--json")
        .filter(|arg| NAMES.contains(&arg.as_str()))
        .cloned()
        .unwrap_or_else(|| "unknown".to_string())
}

fn execute(command: Command, stdin: &mut dyn Read, cx: &Context) -> Result<Outcome> {
    match command {
        Command::Help => Ok(outcome(
            "help",
            &serde_json::json!({ "usage": usage() }),
            usage().to_string(),
            0,
        )),
        Command::Version => {
            let version = env!("CARGO_PKG_VERSION");
            Ok(outcome(
                "version",
                &serde_json::json!({ "version": version }),
                format!("save-relay {version}"),
                0,
            ))
        }
        Command::Setup {
            server,
            user,
            device,
            root,
        } => setup(&server, &user, device.as_deref(), &root, stdin, cx),
        Command::Pull { game, resolve } => sync_command("pull", &game, resolve, cx),
        Command::Push { game, resolve } => sync_command("push", &game, resolve, cx),
        Command::Status { game } => status(&game, cx),
        Command::History { game } => history(&game, cx),
        Command::Restore { game, entry } => restore(&game, &entry, cx),
        // Each of these is filled in by the issue that builds it (docs/code-map.md).
        Command::PullAnyway { .. } => not_built("pull --anyway (F50)"),
        Command::SetupService { .. } => not_built("setup for Google Drive and Dropbox (F44)"),
        Command::Wrap { .. } => not_built("wrap (F58)"),
        Command::Which { .. } => not_built("which (F56)"),
        Command::InstallWrap { .. } => not_built("install-wrap (F59)"),
        Command::Answer { .. } => not_built("answer (F57)"),
        Command::Where => where_command(cx),
        Command::Picture { game, entry, out } => picture_command(&game, &entry, &out, cx),
        Command::Panel { .. } => not_built("panel (F63)"),
    }
}

/// The config of a machine that has been set up, or `Error::NotSetUp`.
fn load_config(cx: &Context) -> Result<Config> {
    cx.store.load_config()?.ok_or(Error::NotSetUp)
}

/// The config and the app password of a machine that has been set up; `Error::NotSetUp` when
/// either is missing. `run` calls this once to build the connection.
pub fn connection(store: &LocalStore, creds: &dyn CredentialStore) -> Result<(Config, Secret)> {
    let config = store.load_config()?.ok_or(Error::NotSetUp)?;
    let account = creds::account(&config.server, &config.user)?;
    let secret = creds.get(&account)?.ok_or(Error::NotSetUp)?;
    Ok((config, secret))
}

fn session<'a>(config: &Config, cx: &'a Context) -> Result<Session<'a>> {
    Ok(Session {
        remote: cx.remote,
        layout: Layout::new(&config.root)?,
        device: config.device.clone(),
        now: cx.now,
    })
}

/// Load the game (before any server call or file write), then remember it when `--manifest`
/// was given.
fn load_game(arg: &GameArg, cx: &Context) -> Result<Game> {
    let game = arg.load(cx.store, cx.platform, cx.vars)?;
    if arg.manifest.is_some() {
        let remembered = cx
            .store
            .load_game(&arg.id)?
            .and_then(|record| record.saves_dir);
        cx.store.save_game(&GameRecord {
            manifest: game.manifest.clone(),
            saves_dir: arg.saves_dir.clone().or(remembered),
        })?;
    }
    Ok(game)
}

fn sync_command(
    command: &str,
    arg: &GameArg,
    resolve: Option<Resolve>,
    cx: &Context,
) -> Result<Outcome> {
    let config = load_config(cx)?;
    let game = load_game(arg, cx)?;
    let s = session(&config, cx)?;
    let report = if command == "pull" {
        sync::pull(&s, &game, cx.store, resolve, false)?
    } else {
        sync::push(&s, &game, cx.store, resolve)?
    };
    let text = sync_text(command, &arg.id, &report);
    let code = api::exit_code_for(report.sync.action);
    Ok(outcome(command, &report, text, code))
}

fn status(arg: &GameArg, cx: &Context) -> Result<Outcome> {
    let config = load_config(cx)?;
    let game = load_game(arg, cx)?;
    let s = session(&config, cx)?;
    let mut data = sync::status(&s, &game, cx.store)?;
    data.service = Service::from_server(&config.server)?.key().to_string();
    let mut text = data.message.clone();
    if let Some(server) = &data.server {
        text.push_str(&format!("\nServer: {}", side_text(server)));
    }
    if let Some(local) = &data.local {
        text.push_str(&format!("\nThis machine: {}", side_text(local)));
    }
    Ok(outcome("status", &data, text, 0))
}

fn history(game: &str, cx: &Context) -> Result<Outcome> {
    let config = load_config(cx)?;
    let s = session(&config, cx)?;
    let data = sync::history(&s, game)?;
    let text = history_text(&data);
    Ok(outcome("history", &data, text, 0))
}

fn restore(arg: &GameArg, entry: &str, cx: &Context) -> Result<Outcome> {
    let config = load_config(cx)?;
    let game = load_game(arg, cx)?;
    let s = session(&config, cx)?;
    let data = sync::restore(&s, &game, cx.store, entry)?;
    let mut text = data.message.clone();
    if let Some(backup) = &data.backup {
        text.push_str(&format!(
            "\nYour local changes were kept first as {backup}."
        ));
    }
    Ok(outcome("restore", &data, text, 0))
}

fn picture_command(
    game: &str,
    entry: &str,
    out: &std::path::Path,
    cx: &Context,
) -> Result<Outcome> {
    let config = load_config(cx)?;
    let s = session(&config, cx)?;
    let data = sync::picture(&s, game, entry, out)?;
    let text = format!("Saved the picture of {entry} to {}.", data.path);
    Ok(outcome("picture", &data, text, 0))
}

fn where_command(cx: &Context) -> Result<Outcome> {
    let config = load_config(cx)?;
    let s = session(&config, cx)?;
    let games = cx.store.list_games()?;
    let data = sync::where_open(&s, &games)?;
    let text = where_text(&data);
    Ok(outcome("where", &data, text, 0))
}

/// `setup` for Nextcloud: the app password is the first line of `stdin`. The login is checked
/// before anything is stored, so a wrong password leaves the machine as it was.
fn setup(
    server: &str,
    user: &str,
    device: Option<&str>,
    root: &str,
    stdin: &mut dyn Read,
    cx: &Context,
) -> Result<Outcome> {
    let password = read_first_line(stdin)?;
    let account = creds::account(server, user)?;
    let layout = Layout::new(root)?;
    let device = layout::device_slug(&match device {
        Some(name) => name.to_string(),
        None => default_device_name(),
    });

    cx.remote.list("")?;
    remote::ensure_dir(cx.remote, layout.root())?;
    cx.creds.set(&account, &Secret::new(password))?;
    let config = Config {
        format: local::FORMAT,
        server: server.to_string(),
        user: user.to_string(),
        root: layout.root().to_string(),
        device,
    };
    cx.store.save_config(&config)?;

    let data = SetupData {
        service: NEXTCLOUD.to_string(),
        server: config.server.clone(),
        user: config.user.clone(),
        device: config.device.clone(),
        root: config.root.clone(),
        credential_store: cx.creds.kind().to_string(),
    };
    let text = format!(
        "Set up: this machine ({}) syncs through {} as {}, in the folder '{}'.\nThe app password is stored in the {} credential store.",
        data.device, data.server, data.user, data.root, data.credential_store
    );
    Ok(outcome("setup", &data, text, 0))
}

fn not_built<T>(what: &'static str) -> Result<T> {
    Err(Error::NotBuilt(NotBuilt(what)))
}

fn default_device_name() -> String {
    ["HOSTNAME", "COMPUTERNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "device".to_string())
}

/// The first line of `stdin` without its line ending. Empty is `Error::Usage`.
fn read_first_line(stdin: &mut dyn Read) -> Result<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while line.len() < 4096 {
        match stdin.read(&mut byte) {
            Ok(0) => break,
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) => line.push(byte[0]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(Error::Io(format!("cannot read standard input: {error}"))),
        }
    }
    let line = String::from_utf8(line)
        .map_err(|_| Error::Usage("the app password on standard input is not text".into()))?;
    let line = line.trim_end_matches('\r').to_string();
    if line.is_empty() {
        return Err(Error::Usage(
            "the app password must be the first line of standard input".into(),
        ));
    }
    Ok(line)
}

// ---- the real parts -------------------------------------------------------------------------

/// A remote for a command that never talks to the server.
struct NoRemote;

impl Remote for NoRemote {
    fn list(&self, _: &str) -> Result<Vec<RemoteEntry>> {
        Err(no_server())
    }
    fn get(&self, _: &str) -> Result<(Vec<u8>, String)> {
        Err(no_server())
    }
    fn put(&self, _: &str, _: &[u8], _: &Precondition) -> Result<String> {
        Err(no_server())
    }
    fn mkcol(&self, _: &str) -> Result<()> {
        Err(no_server())
    }
    fn copy(&self, _: &str, _: &str) -> Result<()> {
        Err(no_server())
    }
    fn move_to(&self, _: &str, _: &str) -> Result<()> {
        Err(no_server())
    }
    fn delete(&self, _: &str) -> Result<()> {
        Err(no_server())
    }
}

fn no_server() -> Error {
    Error::Io("this command has no server connection".into())
}

#[cfg(feature = "keyring")]
fn real_credentials() -> Box<dyn CredentialStore> {
    Box::new(creds::KeyringStore::new())
}

/// A build without the `keyring` feature has no credential store of its own (a mobile build
/// supplies one); every use says so.
#[cfg(not(feature = "keyring"))]
fn real_credentials() -> Box<dyn CredentialStore> {
    struct Missing;
    impl CredentialStore for Missing {
        fn get(&self, _: &str) -> Result<Option<Secret>> {
            Err(unavailable())
        }
        fn set(&self, _: &str, _: &Secret) -> Result<()> {
            Err(unavailable())
        }
        fn delete(&self, _: &str) -> Result<()> {
            Err(unavailable())
        }
        fn kind(&self) -> &'static str {
            "none"
        }
    }
    fn unavailable() -> Error {
        Error::Credentials("this build has no credential store".into())
    }
    Box::new(Missing)
}

#[cfg(feature = "http")]
fn connect(server: &str, user: &str, password: Secret) -> Result<Box<dyn Remote>> {
    let url = webdav::files_url(server, user)?;
    Ok(Box::new(WebDav::new(
        &url,
        user,
        password,
        std::sync::Arc::new(crate::http::UreqTransport::new()),
    )))
}

#[cfg(not(feature = "http"))]
fn connect(server: &str, user: &str, password: Secret) -> Result<Box<dyn Remote>> {
    webdav::files_url(server, user)?;
    Err(Error::Io("this build has no network support".into()))
}

// ---- text mode ------------------------------------------------------------------------------

/// `2026-10-31T19:51:00Z` -> `2026-10-31 19:51 UTC`.
fn when(time: &str) -> String {
    match time.get(..16) {
        Some(head) => format!("{} UTC", head.replacen('T', " ", 1)),
        None => time.to_string(),
    }
}

fn side_text(side: &Side) -> String {
    match side.revision {
        Some(revision) => format!(
            "{}, {} (revision {revision})",
            side.device,
            when(&side.time)
        ),
        None => format!("{}, {}", side.device, when(&side.time)),
    }
}

/// The lines `pull` and `push` print. A conflict is the prompt of docs/code-map.md § API shape
/// as plain text, with the two commands that answer it; every other action is its `message`.
fn sync_text(command: &str, game: &str, report: &SyncReport) -> String {
    let data = &report.sync;
    if data.action != Action::Conflict {
        return data.message.clone();
    }
    let server = data.server.as_ref().map(side_text);
    let local = data
        .local
        .as_ref()
        .map(|side| format!("{}, {}", side.device, when(&side.time)));
    format!(
        "Your saves changed on this machine and on the server since they last synced.\n\
         Server: saved on {}.\n\
         This machine: {}.\n\
         Keep which? The other one stays in history and can be restored.\n\
         \x20 save-relay {command} {game} --resolve keep-server    keep the server's\n\
         \x20 save-relay {command} {game} --resolve keep-local     keep this machine's",
        server.as_deref().unwrap_or("(nothing)"),
        local.as_deref().unwrap_or("(no saves)"),
    )
}

/// `where`: one line per game, in the order `where_open` gives them.
fn where_text(data: &WhereData) -> String {
    if data.games.is_empty() {
        return "No games on this machine yet.".to_string();
    }
    data.games
        .iter()
        .map(|row| {
            let open = match &row.held {
                Some(held) => format!("open on {} since {}", held.device, held.since),
                None => "free".to_string(),
            };
            let server = match &row.server {
                Some(side) => match side.revision {
                    Some(revision) => format!("server revision {revision} from {}", side.device),
                    None => format!("server saves from {}", side.device),
                },
                None => "nothing pushed".to_string(),
            };
            format!("{}: {open}; {server}", row.game)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `history` as a table, newest first; `*` marks the current entry.
fn history_text(data: &HistoryData) -> String {
    if data.entries.is_empty() {
        return format!("No history for {} yet.", data.game);
    }
    let entry_width = data
        .entries
        .iter()
        .map(|entry| entry.entry.len())
        .max()
        .unwrap_or(0)
        .max("ENTRY".len());
    let device_width = data
        .entries
        .iter()
        .map(|entry| entry.device.len())
        .max()
        .unwrap_or(0)
        .max("DEVICE".len());
    let mut lines = vec![format!(
        "  {:<entry_width$}  {:<device_width$}  TIME",
        "ENTRY", "DEVICE"
    )];
    for entry in &data.entries {
        lines.push(format!(
            "{} {:<entry_width$}  {:<device_width$}  {}",
            if entry.current { "*" } else { " " },
            entry.entry,
            entry.device,
            when(&entry.time)
        ));
    }
    lines.push(format!(
        "* is the current entry. Bring one back with: save-relay restore {} <entry>",
        data.game
    ));
    lines.join("\n")
}
