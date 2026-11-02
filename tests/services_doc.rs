use save_relay::cli::parse;

fn doc() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/services.md");
    std::fs::read_to_string(path).expect("docs/services.md exists")
}

/// The command lines inside ```text blocks that start with `save-relay `.
fn commands(doc: &str) -> Vec<String> {
    let (mut inside, mut out) = (false, Vec::new());
    for line in doc.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            inside = !inside && t == "```text";
        } else if inside && t.starts_with("save-relay ") {
            out.push(t.to_string());
        }
    }
    out
}

#[test]
fn has_the_sections() {
    let d = doc();
    for h in [
        "## Choosing a service",
        "## Nextcloud",
        "## Google Drive",
        "## Dropbox",
        "## Steam's Game Mode",
        "## Switching services",
    ] {
        assert!(d.lines().any(|l| l.trim() == h), "missing heading {h}");
    }
}

#[test]
fn names_the_flags_access_and_errors() {
    let d = doc();
    for w in [
        "`--service`",
        "`--no-browser`",
        "`--finish`",
        "`drive.file`",
        "App folder",
        "PKCE",
        "app password",
        "credential store",
        "`reauth_needed`",
        "`sign_in_failed`",
        "`save-relay status`",
        "starts fresh",
    ] {
        assert!(d.contains(w), "docs/services.md does not mention {w}");
    }
}

#[test]
fn every_command_line_parses() {
    let d = doc();
    let lines = commands(&d);
    for want in [
        "--service gdrive",
        "--service dropbox",
        "--server ",
        "--finish",
    ] {
        assert!(
            lines.iter().any(|l| l.contains(want)),
            "no example with {want}"
        );
    }
    for line in &lines {
        let args: Vec<String> = line.split_whitespace().skip(1).map(String::from).collect();
        assert!(parse(&args).is_ok(), "{line}: {:?}", parse(&args));
    }
}

#[test]
fn no_real_addresses() {
    for word in doc().split_whitespace().filter(|w| w.contains('@')) {
        let w = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        assert!(
            w.ends_with("example.org"),
            "{word}: use an example.org address"
        );
    }
}
