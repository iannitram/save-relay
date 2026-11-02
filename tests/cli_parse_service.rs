use save_relay::cli::{command_name, parse, Args, Command, SignInStep, DEFAULT_ROOT};
use save_relay::service::Service;
use save_relay::Error;

fn p(line: &str) -> Result<Args, Error> {
    let args: Vec<String> = line.split_whitespace().map(String::from).collect();
    parse(&args)
}

fn signin(service: Option<Service>, device: Option<&str>, root: &str, step: SignInStep) -> Command {
    Command::SetupService {
        service,
        device: device.map(String::from),
        root: root.to_string(),
        step,
    }
}

fn usage_err(line: &str, words: &[&str]) {
    match p(line) {
        Err(Error::Usage(m)) => {
            for w in words {
                assert!(m.contains(w), "{line}: {m} lacks {w}");
            }
        }
        other => panic!("{line}: expected a usage error, got {other:?}"),
    }
}

#[test]
fn drive_and_dropbox_sign_in_steps() {
    let got = p("setup --service gdrive").unwrap();
    let want = signin(
        Some(Service::Gdrive),
        None,
        DEFAULT_ROOT,
        SignInStep::Browser,
    );
    assert_eq!((got.command, got.json), (want, false));
    let got = p("setup --no-browser --device desk --service dropbox --root G/sr --json").unwrap();
    let want = signin(
        Some(Service::Dropbox),
        Some("desk"),
        "G/sr",
        SignInStep::Code,
    );
    assert_eq!((got.command, got.json), (want, true));
    let got = p("setup --finish --service gdrive --json").unwrap().command;
    let want = signin(
        Some(Service::Gdrive),
        None,
        DEFAULT_ROOT,
        SignInStep::Finish,
    );
    assert_eq!(got, want);
}

#[test]
fn nextcloud_with_or_without_service() {
    let want = Command::Setup {
        server: "https://cloud.example.org".into(),
        user: "me".into(),
        device: None,
        root: DEFAULT_ROOT.into(),
    };
    let line = "setup --service nextcloud --server https://cloud.example.org --user me";
    assert_eq!(p(line).unwrap().command, want);
    let line = "setup --user me --server https://cloud.example.org";
    assert_eq!(p(line).unwrap().command, want);
}

#[test]
fn no_service_asks_on_standard_input() {
    let want = signin(None, None, DEFAULT_ROOT, SignInStep::Browser);
    assert_eq!(p("setup").unwrap().command, want);
    let want = signin(None, Some("deck"), "x", SignInStep::Browser);
    assert_eq!(p("setup --root x --device deck").unwrap().command, want);
    usage_err("setup --json", &["--service"]);
}

#[test]
fn service_usage_errors() {
    let cases: [(&str, &[&str]); 11] = [
        ("setup --service onedrive", &["onedrive"]),
        ("setup --service", &["--service"]),
        ("setup --service gdrive --service dropbox", &["twice"]),
        ("setup --service gdrive --server https://x", &["--server"]),
        ("setup --service dropbox --user me", &["--user"]),
        (
            "setup --service gdrive --no-browser --finish",
            &["--no-browser", "--finish"],
        ),
        (
            "setup --server https://x --user me --no-browser",
            &["--no-browser"],
        ),
        (
            "setup --service nextcloud --finish --server h --user u",
            &["--finish"],
        ),
        ("setup --no-browser", &["--no-browser"]),
        ("setup --service nextcloud", &["--server"]),
        ("pull g --service gdrive", &["--service"]),
    ];
    for (line, words) in cases {
        usage_err(line, words);
    }
    usage_err("status g --no-browser", &["--no-browser"]);
}

#[test]
fn both_setup_forms_are_named_setup() {
    for line in [
        "setup --service gdrive",
        "setup",
        "setup --server h --user u",
    ] {
        assert_eq!(command_name(&p(line).unwrap().command), "setup", "{line}");
    }
}
