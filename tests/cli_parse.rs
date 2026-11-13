use save_relay::cli::{command_name, parse, usage, Args, Command, GameArg, DEFAULT_ROOT};
use save_relay::sync::Resolve;
use save_relay::Error;
use std::path::PathBuf;

fn p(line: &str) -> Result<Args, Error> {
    let args: Vec<String> = line.split_whitespace().map(String::from).collect();
    parse(&args)
}

fn game(id: &str, manifest: Option<&str>, saves_dir: Option<&str>) -> GameArg {
    let id = id.to_string();
    let (manifest, saves_dir) = (manifest.map(PathBuf::from), saves_dir.map(PathBuf::from));
    GameArg {
        id,
        manifest,
        saves_dir,
    }
}

fn pull(game: GameArg, resolve: Option<Resolve>) -> Command {
    Command::Pull { game, resolve }
}

fn push(game: GameArg, resolve: Option<Resolve>) -> Command {
    Command::Push { game, resolve }
}

fn ok(command: Command, json: bool) -> Result<Args, Error> {
    Ok(Args { command, json })
}

/// A usage error whose message contains `word`.
fn usage_err(line: &str, word: &str) {
    match p(line) {
        Err(Error::Usage(m)) => assert!(m.contains(word), "{line}: {m}"),
        other => panic!("{line}: expected a usage error, got {other:?}"),
    }
}

#[test]
fn pull_and_push_with_every_flag_in_any_order() {
    let g = game("uncharted-1", Some("m.json"), Some("saves"));
    let line = "pull uncharted-1 --manifest m.json --saves-dir saves --json";
    assert_eq!(p(line), ok(pull(g.clone(), None), true));
    let line = "push --json --resolve keep-local --saves-dir saves uncharted-1 --manifest m.json";
    assert_eq!(p(line), ok(push(g, Some(Resolve::KeepLocal)), true));
    let want = pull(game("g", None, None), Some(Resolve::KeepServer));
    assert_eq!(p("pull g --resolve keep-server"), ok(want, false));
}

#[test]
fn slot_ids_are_independent_and_default_id_is_unchanged() {
    assert_eq!(
        p("pull uncharted-2 --slot 3 --saves-dir slot3"),
        ok(
            pull(game("uncharted-2-slot-3", None, Some("slot3")), None),
            false
        )
    );
    assert_eq!(
        p("pull uncharted-2 --saves-dir saves"),
        ok(pull(game("uncharted-2", None, Some("saves")), None), false)
    );
    usage_err("pull uncharted-2 --slot 11", "1-10");
    assert_eq!(
        p("wrap --game uncharted-2 --slot 2 -- /bin/true"),
        ok(
            Command::Wrap {
                game: Some("uncharted-2-slot-2".into()),
                manifest: None,
                saves_dir: None,
                command: vec!["/bin/true".into()],
            },
            false,
        )
    );
    usage_err("wrap --slot 2 -- /bin/true", "requires --game");
}

#[test]
fn launch_option_accepts_numeric_ids_and_preserves_empty_or_dash_prefixed_current() {
    let parse_args = |items: &[&str]| {
        let owned = items
            .iter()
            .map(|item| (*item).to_string())
            .collect::<Vec<_>>();
        parse(&owned)
    };
    assert_eq!(
        parse_args(&["launch-option", "42", "--current=-novid %command%"]),
        Ok(Args {
            command: Command::LaunchOption {
                app_id: 42,
                remove: false,
                current: Some("-novid %command%".to_string()),
            },
            json: false,
        })
    );
    assert_eq!(
        parse_args(&["launch-option", "42", "--current="]),
        Ok(Args {
            command: Command::LaunchOption {
                app_id: 42,
                remove: false,
                current: Some(String::new()),
            },
            json: false,
        })
    );
    for value in ["--json", "--password", "--current", "--current=value"] {
        assert_eq!(
            parse_args(&["launch-option", "42", "--current", value, "--json"]),
            Ok(Args {
                command: Command::LaunchOption {
                    app_id: 42,
                    remove: false,
                    current: Some(value.to_string()),
                },
                json: true,
            })
        );
    }
    usage_err("launch-option 0", "appid");
}

#[test]
fn status_history_restore() {
    let want = Command::Status {
        game: game("g", None, Some("d")),
    };
    assert_eq!(p("status g --saves-dir d"), ok(want, false));
    let want = Command::History { game: "g".into() };
    assert_eq!(p("history g --json"), ok(want, true));
    let (g, entry) = (
        game("g", Some("m"), None),
        "20261031T195100Z-amd".to_string(),
    );
    let line = "restore g 20261031T195100Z-amd --manifest m";
    assert_eq!(p(line), ok(Command::Restore { game: g, entry }, false));
}

#[test]
fn setup_with_and_without_defaults() {
    let setup = |device: Option<&str>, root: &str| {
        let (server, user) = ("https://cloud.example.org".to_string(), "me".to_string());
        let (device, root) = (device.map(String::from), root.to_string());
        Command::Setup {
            server,
            user,
            device,
            root,
        }
    };
    let line = "setup --server https://cloud.example.org --user me";
    assert_eq!(p(line), ok(setup(None, DEFAULT_ROOT), false));
    let line =
        "setup --root G/sr --user me --device desk --server https://cloud.example.org --json";
    assert_eq!(p(line), ok(setup(Some("desk"), "G/sr"), true));
    assert_eq!(DEFAULT_ROOT, "save-relay");
}

#[test]
fn help_and_version() {
    for line in ["", "help", "--help", "-h", "--json"] {
        assert_eq!(p(line).unwrap().command, Command::Help, "{line:?}");
    }
    for line in ["version", "--version", "-V"] {
        assert_eq!(p(line).unwrap().command, Command::Version, "{line}");
    }
}

#[test]
fn usage_errors_say_what_is_wrong() {
    let cases = [
        ("fetch g", "fetch"),
        ("pull g --force", "--force"),
        ("pull", "<game>"),
        ("restore g", "<entry>"),
        ("pull g h", "h"),
        ("pull g --manifest a --manifest b", "twice"),
        ("pull g --manifest", "--manifest"),
        ("pull g --resolve newest", "keep-server"),
        ("history g --manifest m", "--manifest"),
        ("status g --resolve keep-local", "--resolve"),
        ("setup --user me", "--server"),
        ("setup --server https://cloud.example.org", "--user"),
        (
            "setup --server https://x --user me --manifest m",
            "--manifest",
        ),
        ("pull g --manifest=m.json", "--manifest=m.json"),
        (
            "setup --server https://x --user me --password hunter2",
            "standard input",
        ),
    ];
    for (line, word) in cases {
        usage_err(line, word);
    }
}

#[test]
fn command_names() {
    let lines =
        "pull g|push g|status g|history g|restore g e|setup --server h --user u|help|version";
    let names: Vec<&str> = lines
        .split('|')
        .map(|l| command_name(&p(l).unwrap().command))
        .collect();
    let want = [
        "pull", "push", "status", "history", "restore", "setup", "help", "version",
    ];
    assert_eq!(names, want);
}

#[test]
fn usage_names_every_command_and_the_password_rule() {
    let text = usage();
    for word in [
        "pull", "push", "status", "history", "restore", "setup", "--json",
    ] {
        assert!(text.contains(word), "usage lacks {word}");
    }
    assert!(text.contains("standard input"));
}

#[test]
fn history_and_picture_reject_an_invalid_game_id() {
    for line in [
        "history ../x",
        "history -x",
        "picture ../x e --out o.png",
        "picture a/b e --out o.png",
    ] {
        assert!(matches!(p(line), Err(Error::Usage(_))), "{line}");
    }
    assert!(p("history ok-game").is_ok());
}
