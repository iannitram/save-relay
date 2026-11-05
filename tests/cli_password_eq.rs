use save_relay::cli::parse;
use save_relay::Error;

const REFUSAL: &str = "the password is read from standard input, not from --password";

fn p(line: &str) -> Result<save_relay::cli::Args, Error> {
    let args: Vec<String> = line.split_whitespace().map(String::from).collect();
    parse(&args)
}

/// The line must be refused with the standard-input message, and the secret must not appear in it.
fn refused(line: &str) {
    match p(line) {
        Err(Error::Usage(m)) => {
            assert!(m == REFUSAL, "{line}: {m}");
            assert!(!m.contains("hunter2"), "{line}: {m}");
        }
        other => panic!("{line}: expected a usage error, got {other:?}"),
    }
}

#[test]
fn password_equals_is_refused_on_every_command() {
    for line in [
        "pull demo-game --password=hunter2",
        "push demo-game --password=hunter2",
        "status demo-game --password=hunter2",
        "history demo-game --password=hunter2",
        "restore demo-game e1 --password=hunter2",
        "setup --server https://x --user me --password=hunter2",
        "which --password=hunter2",
        "answer demo-game --password=hunter2",
        "where --password=hunter2",
        "panel --password=hunter2",
        "--json pull demo-game --password=hunter2",
        "pull --password=hunter2 demo-game",
    ] {
        refused(line);
    }
}

#[test]
fn password_equals_with_empty_value_is_refused() {
    refused("pull demo-game --password=");
}

#[test]
fn bare_password_is_still_refused() {
    refused("pull demo-game --password hunter2");
}

#[test]
fn wrap_refuses_password_equals_before_the_dash() {
    refused("wrap --password=hunter2 -- game");
}

#[test]
fn wrap_passes_password_equals_after_the_dash_to_the_child() {
    match p("wrap -- game --password=hunter2") {
        Ok(parsed) => assert_eq!(
            parsed.command,
            save_relay::cli::Command::Wrap {
                game: None,
                manifest: None,
                saves_dir: None,
                command: vec!["game".to_string(), "--password=hunter2".to_string()],
            }
        ),
        other => panic!("expected Ok, got {other:?}"),
    }
}

#[test]
fn unknown_flag_error_omits_the_value() {
    match p("pull demo-game --token=hunter2") {
        Err(Error::Usage(m)) => {
            assert!(m == "unknown flag '--token'", "{m}");
            assert!(!m.contains("hunter2"), "{m}");
        }
        other => panic!("expected a usage error, got {other:?}"),
    }
}

#[test]
fn unknown_flag_without_equals_is_unchanged() {
    match p("pull demo-game --token") {
        Err(Error::Usage(m)) => assert!(m == "unknown flag '--token'", "{m}"),
        other => panic!("expected a usage error, got {other:?}"),
    }
}

#[test]
fn similar_flag_name_is_not_the_password() {
    match p("pull demo-game --passwords=x") {
        Err(Error::Usage(m)) => assert!(m == "unknown flag '--passwords'", "{m}"),
        other => panic!("expected a usage error, got {other:?}"),
    }
}
