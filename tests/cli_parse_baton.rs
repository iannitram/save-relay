use save_relay::cli::{command_name, parse, usage, Args, Command, GameArg};
use save_relay::pending::Answer;
use save_relay::service::Service;
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

fn ok(command: Command, json: bool) -> Result<Args, Error> {
    Ok(Args { command, json })
}

fn usage_err(line: &str, word: &str) {
    match p(line) {
        Err(Error::Usage(m)) => assert!(m.contains(word), "{line}: {m}"),
        other => panic!("{line}: expected a usage error containing {word:?}, got {other:?}"),
    }
}

#[test]
fn pull_anyway_with_and_without_json_in_two_flag_orders() {
    let g = game("witcher-3", Some("m.json"), None);
    // without --json, flag order 1
    let line1 = "pull witcher-3 --anyway --manifest m.json";
    let want1 = Command::PullAnyway {
        game: g.clone(),
        resolve: None,
    };
    assert_eq!(p(line1), ok(want1, false));

    // without --json, flag order 2
    let line2 = "pull witcher-3 --manifest m.json --anyway";
    let want2 = Command::PullAnyway {
        game: g,
        resolve: None,
    };
    assert_eq!(p(line2), ok(want2, false));

    // with --json, flag order 1
    let g_dir = game("witcher-3", None, Some("saves"));
    let line3 = "pull witcher-3 --anyway --saves-dir saves --json";
    let want3 = Command::PullAnyway {
        game: g_dir.clone(),
        resolve: None,
    };
    assert_eq!(p(line3), ok(want3, true));

    // with --json, flag order 2
    let line4 = "pull --json witcher-3 --saves-dir saves --anyway";
    let want4 = Command::PullAnyway {
        game: g_dir,
        resolve: None,
    };
    assert_eq!(p(line4), ok(want4, true));

    // with --resolve
    let want_res = Command::PullAnyway {
        game: game("g", None, None),
        resolve: Some(Resolve::KeepServer),
    };
    assert_eq!(
        p("pull g --anyway --resolve keep-server"),
        ok(want_res.clone(), false)
    );
    assert_eq!(
        p("pull g --resolve keep-server --anyway"),
        ok(want_res, false)
    );

    // plain pull remains Pull
    let plain = Command::Pull {
        game: game("g", None, None),
        resolve: None,
    };
    assert_eq!(p("pull g"), ok(plain, false));
}

#[test]
fn pull_anyway_errors_and_push_rejects_anyway() {
    usage_err("push g --anyway", "--anyway");
    usage_err("status g --anyway", "--anyway");
    usage_err("pull g --anyway --anyway", "twice");
}

#[test]
fn wrap_with_and_without_json_in_two_flag_orders() {
    // without --json, flag order 1
    let line1 = "wrap --game g1 --manifest m.json --saves-dir s -- ./start -d";
    let want1 = Command::Wrap {
        game: Some("g1".into()),
        manifest: Some(PathBuf::from("m.json")),
        saves_dir: Some(PathBuf::from("s")),
        command: vec!["./start".into(), "-d".into()],
    };
    assert_eq!(p(line1), ok(want1, false));

    // without --json, flag order 2
    let line2 = "wrap --saves-dir s --game g1 --manifest m.json -- ./start -d";
    let want2 = Command::Wrap {
        game: Some("g1".into()),
        manifest: Some(PathBuf::from("m.json")),
        saves_dir: Some(PathBuf::from("s")),
        command: vec!["./start".into(), "-d".into()],
    };
    assert_eq!(p(line2), ok(want2, false));

    // with --json, flag order 1
    let line3 = "wrap --json --game g1 -- ./start";
    let want3 = Command::Wrap {
        game: Some("g1".into()),
        manifest: None,
        saves_dir: None,
        command: vec!["./start".into()],
    };
    assert_eq!(p(line3), ok(want3, true));

    // with --json, flag order 2
    let line4 = "wrap --game g1 --json -- ./start";
    let want4 = Command::Wrap {
        game: Some("g1".into()),
        manifest: None,
        saves_dir: None,
        command: vec!["./start".into()],
    };
    assert_eq!(p(line4), ok(want4, true));

    // without optional flags
    let line5 = "wrap -- run_game";
    let want5 = Command::Wrap {
        game: None,
        manifest: None,
        saves_dir: None,
        command: vec!["run_game".into()],
    };
    assert_eq!(p(line5), ok(want5, false));
}

#[test]
fn wrap_keeps_command_flags_verbatim() {
    // wrap -- game --json -x keeps --json and -x in command and save-relay's json is false
    let parsed = p("wrap -- game --json -x").unwrap();
    assert!(!parsed.json, "save-relay json should be false");
    let want = Command::Wrap {
        game: None,
        manifest: None,
        saves_dir: None,
        command: vec!["game".into(), "--json".into(), "-x".into()],
    };
    assert_eq!(parsed.command, want);

    // --game after -- also belongs to child
    let parsed2 = p("wrap -- game --game other").unwrap();
    assert_eq!(
        parsed2.command,
        Command::Wrap {
            game: None,
            manifest: None,
            saves_dir: None,
            command: vec!["game".into(), "--game".into(), "other".into()],
        }
    );

    // child command with --password passes through without triggering save-relay password error
    let parsed3 = p("wrap -- game --password secret").unwrap();
    assert_eq!(
        parsed3.command,
        Command::Wrap {
            game: None,
            manifest: None,
            saves_dir: None,
            command: vec!["game".into(), "--password".into(), "secret".into()],
        }
    );
}

#[test]
fn wrap_usage_errors() {
    usage_err("wrap", "--");
    usage_err("wrap --game g", "--");
    usage_err("wrap --", "<command>");
    usage_err("wrap --game g --", "<command>");
    usage_err("wrap g -- cmd", "unexpected argument");
    usage_err("wrap --password secret -- cmd", "standard input");
}

#[test]
fn which_with_and_without_json_in_two_flag_orders() {
    // without --json
    let want = Command::Which { app_id: 105600 };
    assert_eq!(p("which 105600"), ok(want.clone(), false));

    // with --json, flag order 1
    assert_eq!(p("which 105600 --json"), ok(want.clone(), true));

    // with --json, flag order 2
    assert_eq!(p("which --json 105600"), ok(want, true));

    // boundary whole numbers
    assert_eq!(p("which 1"), ok(Command::Which { app_id: 1 }, false));
    assert_eq!(
        p(&format!("which {}", u32::MAX)),
        ok(Command::Which { app_id: u32::MAX }, false)
    );
}

#[test]
fn which_errors() {
    usage_err("which 0", "<appid>");
    usage_err("which -1", "<appid>");
    usage_err("which 4294967296", "<appid>");
    usage_err("which x", "<appid>");
    usage_err("which", "<appid>");
    usage_err("which 123 456", "unexpected argument");
}

#[test]
fn install_wrap_with_and_without_json_in_two_flag_orders() {
    // with appids, without --json, flag order 1
    let line1 = "install-wrap 456 123 --remove --dry-run";
    let want1 = Command::InstallWrap {
        app_ids: vec![123, 456],
        all_known: false,
        remove: true,
        dry_run: true,
    };
    assert_eq!(p(line1), ok(want1, false));

    // with appids, without --json, flag order 2
    let line2 = "install-wrap --dry-run 123 456 --remove";
    let want2 = Command::InstallWrap {
        app_ids: vec![123, 456],
        all_known: false,
        remove: true,
        dry_run: true,
    };
    assert_eq!(p(line2), ok(want2, false));

    // with appids, with --json, flag order 1
    let line3 = "install-wrap 123 --remove --json";
    let want3 = Command::InstallWrap {
        app_ids: vec![123],
        all_known: false,
        remove: true,
        dry_run: false,
    };
    assert_eq!(p(line3), ok(want3, true));

    // with appids, with --json, flag order 2
    let line4 = "install-wrap --json 123 --remove";
    let want4 = Command::InstallWrap {
        app_ids: vec![123],
        all_known: false,
        remove: true,
        dry_run: false,
    };
    assert_eq!(p(line4), ok(want4, true));

    // with --all-known, without --json, flag order 1
    let line5 = "install-wrap --all-known --dry-run";
    let want5 = Command::InstallWrap {
        app_ids: vec![],
        all_known: true,
        remove: false,
        dry_run: true,
    };
    assert_eq!(p(line5), ok(want5, false));

    // with --all-known, without --json, flag order 2
    let line6 = "install-wrap --dry-run --all-known";
    let want6 = Command::InstallWrap {
        app_ids: vec![],
        all_known: true,
        remove: false,
        dry_run: true,
    };
    assert_eq!(p(line6), ok(want6, false));

    // with --all-known, with --json, flag order 1
    let line7 = "install-wrap --all-known --json";
    let want7 = Command::InstallWrap {
        app_ids: vec![],
        all_known: true,
        remove: false,
        dry_run: false,
    };
    assert_eq!(p(line7), ok(want7, true));

    // with --all-known, with --json, flag order 2
    let line8 = "install-wrap --json --all-known";
    let want8 = Command::InstallWrap {
        app_ids: vec![],
        all_known: true,
        remove: false,
        dry_run: false,
    };
    assert_eq!(p(line8), ok(want8, true));
}

#[test]
fn install_wrap_sorts_and_deduplicates_app_ids() {
    let line = "install-wrap 700 300 700 100 300";
    let res = p(line).unwrap();
    assert_eq!(
        res.command,
        Command::InstallWrap {
            app_ids: vec![100, 300, 700],
            all_known: false,
            remove: false,
            dry_run: false,
        }
    );
}

#[test]
fn install_wrap_errors() {
    // both appids and --all-known
    usage_err("install-wrap 123 --all-known", "--all-known");
    usage_err("install-wrap --all-known 123 456", "--all-known");

    // neither appids nor --all-known
    usage_err("install-wrap", "--all-known");
    usage_err("install-wrap --remove", "--all-known");
    usage_err("install-wrap --dry-run", "--all-known");

    // bad app ids
    usage_err("install-wrap 0", "<appid>");
    usage_err("install-wrap -1", "<appid>");
    usage_err("install-wrap 4294967296", "<appid>");
    usage_err("install-wrap x", "<appid>");
}

#[test]
fn answer_with_and_without_json_in_two_flag_orders() {
    let cases = [
        ("answer g keep-server", Answer::KeepServer),
        ("answer g keep-local", Answer::KeepLocal),
        ("answer g play-anyway", Answer::PlayAnyway),
        ("answer g wait", Answer::Wait),
    ];
    for (line, ans) in cases {
        let want = Command::Answer {
            game: "g".into(),
            answer: ans,
        };
        assert_eq!(p(line), ok(want, false), "{line}");
    }

    // with --json, flag order 1
    let want = Command::Answer {
        game: "g".into(),
        answer: Answer::Wait,
    };
    assert_eq!(p("answer g wait --json"), ok(want.clone(), true));

    // with --json, flag order 2
    assert_eq!(p("answer --json g wait"), ok(want, true));
}

#[test]
fn answer_errors() {
    // bad answer names the four allowed answers
    let line = "answer g bad-ans";
    match p(line) {
        Err(Error::Usage(m)) => {
            assert!(m.contains("keep-server"), "{m}");
            assert!(m.contains("keep-local"), "{m}");
            assert!(m.contains("play-anyway"), "{m}");
            assert!(m.contains("wait"), "{m}");
        }
        other => panic!("{line}: expected usage error, got {other:?}"),
    }

    usage_err("answer g", "<answer>");
    usage_err("answer", "<game>");
    usage_err("answer g wait extra", "unexpected argument");
}

#[test]
fn where_with_and_without_json_in_two_flag_orders() {
    // without --json
    assert_eq!(p("where"), ok(Command::Where, false));

    // with --json, flag order 1
    assert_eq!(p("where --json"), ok(Command::Where, true));

    // with --json, flag order 2
    assert_eq!(p("--json where"), ok(Command::Where, true));

    // no positional allowed
    usage_err("where g", "unexpected argument");
}

#[test]
fn picture_with_and_without_json_in_two_flag_orders() {
    let entry = "20261031T195100Z-desk";
    let want = Command::Picture {
        game: "g".into(),
        entry: entry.into(),
        out: PathBuf::from("pic.jpg"),
    };

    // without --json, flag order 1
    let line1 = format!("picture g {entry} --out pic.jpg");
    assert_eq!(p(&line1), ok(want.clone(), false));

    // without --json, flag order 2
    let line2 = format!("picture --out pic.jpg g {entry}");
    assert_eq!(p(&line2), ok(want.clone(), false));

    // with --json, flag order 1
    let line3 = format!("picture g {entry} --out pic.jpg --json");
    assert_eq!(p(&line3), ok(want.clone(), true));

    // with --json, flag order 2
    let line4 = format!("picture --json --out pic.jpg g {entry}");
    assert_eq!(p(&line4), ok(want, true));
}

#[test]
fn picture_errors() {
    usage_err("picture g 20261031T195100Z-desk", "--out");
    usage_err("picture g --out pic.jpg", "<entry>");
    usage_err("picture --out pic.jpg", "<game>");
    usage_err("picture g e --out", "--out");
    usage_err("picture g e --out pic.jpg extra", "unexpected argument");
}

#[test]
fn panel_with_and_without_json_in_two_flag_orders() {
    // defaults
    assert_eq!(
        p("panel"),
        ok(
            Command::Panel {
                port: None,
                open: true
            },
            false
        )
    );
    assert_eq!(
        p("panel --no-open"),
        ok(
            Command::Panel {
                port: None,
                open: false
            },
            false
        )
    );

    // without --json, flag order 1
    let line1 = "panel --port 8080 --no-open";
    let want1 = Command::Panel {
        port: Some(8080),
        open: false,
    };
    assert_eq!(p(line1), ok(want1, false));

    // without --json, flag order 2
    let line2 = "panel --no-open --port 8080";
    let want2 = Command::Panel {
        port: Some(8080),
        open: false,
    };
    assert_eq!(p(line2), ok(want2, false));

    // with --json, flag order 1
    let line3 = "panel --port 8080 --no-open --json";
    let want3 = Command::Panel {
        port: Some(8080),
        open: false,
    };
    assert_eq!(p(line3), ok(want3, true));

    // with --json, flag order 2
    let line4 = "panel --json --no-open --port 8080";
    let want4 = Command::Panel {
        port: Some(8080),
        open: false,
    };
    assert_eq!(p(line4), ok(want4, true));

    // port boundaries
    assert_eq!(
        p("panel --port 1"),
        ok(
            Command::Panel {
                port: Some(1),
                open: true
            },
            false
        )
    );
    assert_eq!(
        p("panel --port 65535"),
        ok(
            Command::Panel {
                port: Some(65535),
                open: true
            },
            false
        )
    );
}

#[test]
fn panel_errors() {
    usage_err("panel --port 0", "--port");
    usage_err("panel --port 70000", "--port");
    usage_err("panel --port x", "--port");
    usage_err("panel --port", "--port");
    usage_err("panel extra", "unexpected argument");
}

#[test]
fn command_name_covers_every_variant() {
    let cases = [
        (
            Command::Pull {
                game: game("g", None, None),
                resolve: None,
            },
            "pull",
        ),
        (
            Command::PullAnyway {
                game: game("g", None, None),
                resolve: None,
            },
            "pull",
        ),
        (
            Command::Push {
                game: game("g", None, None),
                resolve: None,
            },
            "push",
        ),
        (
            Command::Status {
                game: game("g", None, None),
            },
            "status",
        ),
        (Command::History { game: "g".into() }, "history"),
        (
            Command::Restore {
                game: game("g", None, None),
                entry: "e".into(),
            },
            "restore",
        ),
        (
            Command::Setup {
                server: "s".into(),
                user: "u".into(),
                device: None,
                root: "r".into(),
            },
            "setup",
        ),
        (
            Command::SetupService {
                service: Some(Service::Dropbox),
                device: None,
                root: "r".into(),
                step: save_relay::cli::SignInStep::Browser,
            },
            "setup",
        ),
        (
            Command::Wrap {
                game: None,
                manifest: None,
                saves_dir: None,
                command: vec!["c".into()],
            },
            "wrap",
        ),
        (Command::Which { app_id: 123 }, "which"),
        (
            Command::InstallWrap {
                app_ids: vec![123],
                all_known: false,
                remove: false,
                dry_run: false,
            },
            "install-wrap",
        ),
        (
            Command::Answer {
                game: "g".into(),
                answer: Answer::Wait,
            },
            "answer",
        ),
        (Command::Where, "where"),
        (
            Command::Picture {
                game: "g".into(),
                entry: "e".into(),
                out: PathBuf::from("p"),
            },
            "picture",
        ),
        (
            Command::Panel {
                port: None,
                open: true,
            },
            "panel",
        ),
        (Command::Help, "help"),
        (Command::Version, "version"),
    ];

    for (cmd, want) in cases {
        assert_eq!(command_name(&cmd), want, "{cmd:?}");
    }
}

#[test]
fn usage_covers_all_commands() {
    let text = usage();
    for word in [
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
        "--json",
    ] {
        assert!(text.contains(word), "usage text missing {word}");
    }
}
