//! Locked tests for issue #71: `docs/wrap.md`, `docs/panel.md`, the sidecar
//! section of `docs/manifest.md`, and the code map rows. Do not edit.
use save_relay::cli::parse;

fn read(relative: &str) -> String {
    let path = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), relative);
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{relative} exists"))
}

/// The lines (trimmed) of each fenced block whose opening fence is exactly `fence`.
fn blocks(text: &str, fence: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for line in text.lines() {
        let t = line.trim();
        match current.as_mut() {
            None if t == fence => current = Some(Vec::new()),
            Some(_) if t == "```" => out.push(current.take().unwrap()),
            Some(b) => b.push(t.to_string()),
            None => {}
        }
    }
    out
}

/// The `save-relay ...` command lines inside ```text blocks.
fn commands(text: &str) -> Vec<String> {
    blocks(text, "```text")
        .into_iter()
        .flatten()
        .filter(|l| l.starts_with("save-relay "))
        .collect()
}

/// The text of the `## ` section whose heading contains `needle` (case-insensitive),
/// up to the next `## ` heading.
fn section(text: &str, needle: &str) -> Option<String> {
    let mut found = None;
    let mut body = String::new();
    for line in text.lines() {
        if line.starts_with("## ") {
            if found.is_some() {
                break;
            }
            if line.to_lowercase().contains(&needle.to_lowercase()) {
                found = Some(());
            }
            continue;
        }
        if found.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    found.map(|_| body)
}

fn headings(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.starts_with("## ") || l.starts_with("### "))
        .map(|l| l.to_lowercase())
        .collect()
}

fn assert_has_all(name: &str, text: &str, words: &[&str]) {
    for w in words {
        assert!(text.contains(w), "{name} does not mention {w}");
    }
}

fn assert_heading_has(name: &str, text: &str, words: &[&str]) {
    let hs = headings(text);
    for w in words {
        let w = w.to_lowercase();
        assert!(
            hs.iter().any(|h| h.contains(&w)),
            "{name} has no heading about {w}"
        );
    }
}

fn assert_no_private_details(name: &str, text: &str) {
    for bad in ["/home/", "C:\\Users", "@gmail"] {
        assert!(!text.contains(bad), "{name} contains {bad}");
    }
}

fn assert_each_command_parses(name: &str, text: &str) {
    for line in commands(text) {
        let args: Vec<String> = line.split_whitespace().skip(1).map(String::from).collect();
        assert!(parse(&args).is_ok(), "{name}: {line}: {:?}", parse(&args));
    }
}

fn assert_sh_lines_are_one_command(name: &str, text: &str) {
    for block in blocks(text, "```sh") {
        for line in block {
            let code = line.split(" #").next().unwrap_or("").trim();
            if code.is_empty() || code.starts_with('#') {
                continue;
            }
            assert!(
                !code.contains('|') && !code.contains("&&"),
                "{name}: one command per line, got: {code}"
            );
            assert!(
                !code.contains('"') && !code.contains('\''),
                "{name}: no quotes in sh examples: {code}"
            );
        }
    }
}

// ---- docs/wrap.md ----

fn wrap_doc() -> String {
    read("docs/wrap.md")
}

#[test]
fn wrap_has_the_sections() {
    let d = wrap_doc();
    assert_heading_has(
        "docs/wrap.md",
        &d,
        &[
            "wrap",
            "install-wrap",
            "which",
            "Proton",
            "pending",
            "answer",
        ],
    );
}

#[test]
fn wrap_names_the_flags_and_rules() {
    assert_has_all(
        "docs/wrap.md",
        &wrap_doc(),
        &[
            "`--all-known`",
            "`--dry-run`",
            "`--remove`",
            "`--manifest`",
            "`--saves-dir`",
            "backup",
            "Steam",
            "closed",
            "pull",
            "push",
            "exit code",
            "prefix",
            "steamuser",
            "registry",
            "pending",
            "keep-server",
            "keep-local",
            "play-anyway",
            "wait",
            "save_location_unclear",
            "own manifest",
        ],
    );
}

#[test]
fn wrap_command_lines_parse() {
    let d = wrap_doc();
    let lines = commands(&d);
    for want in [
        "install-wrap",
        "--dry-run",
        "--all-known",
        "--remove",
        "which",
        "answer",
        "wrap -- %command%",
    ] {
        assert!(
            lines.iter().any(|l| l.contains(want)),
            "docs/wrap.md has no example with {want}"
        );
    }
    assert_each_command_parses("docs/wrap.md", &d);
}

#[test]
fn wrap_sh_lines_are_one_command() {
    assert_sh_lines_are_one_command("docs/wrap.md", &wrap_doc());
}

// ---- docs/panel.md ----

fn panel_doc() -> String {
    read("docs/panel.md")
}

#[test]
fn panel_has_the_sections() {
    assert_heading_has(
        "docs/panel.md",
        &panel_doc(),
        &["browser", "Game Mode", "sign-in", "controller"],
    );
}

#[test]
fn panel_names_the_flags_access_and_views() {
    assert_has_all(
        "docs/panel.md",
        &panel_doc(),
        &[
            "`--port`",
            "`--no-open`",
            "127.0.0.1",
            "token",
            "Decky",
            "zip",
            "Game Mode",
            "controller",
            "Google Drive",
            "Dropbox",
            "QR",
            "`--finish`",
            "terminal",
            "Games",
            "Pending choices",
            "history",
            "Restore",
        ],
    );
}

#[test]
fn panel_command_lines_parse() {
    let d = panel_doc();
    let lines = commands(&d);
    for want in [
        "panel",
        "--no-open",
        "--port",
        "setup --service gdrive --finish",
    ] {
        assert!(
            lines.iter().any(|l| l.contains(want)),
            "docs/panel.md has no example with {want}"
        );
    }
    assert_each_command_parses("docs/panel.md", &d);
}

#[test]
fn panel_sh_lines_are_one_command() {
    assert_sh_lines_are_one_command("docs/panel.md", &panel_doc());
}

// ---- docs/manifest.md sidecar section ----

fn manifest_doc() -> String {
    read("docs/manifest.md")
}

#[test]
fn manifest_has_a_sidecar_section_before_the_examples() {
    let d = manifest_doc();
    let sidecar = d
        .lines()
        .position(|l| l.starts_with("## ") && l.to_lowercase().contains("sidecar"))
        .expect("docs/manifest.md has a ## Sidecar section");
    let examples = d
        .lines()
        .position(|l| l.trim() == "## Examples")
        .expect("docs/manifest.md has ## Examples");
    assert!(
        sidecar < examples,
        "the sidecar section comes before ## Examples"
    );
}

#[test]
fn sidecar_section_says_what_the_limits_are() {
    let d = manifest_doc();
    let s = section(&d, "sidecar").expect("sidecar section");
    assert_has_all(
        "docs/manifest.md sidecar section",
        &s,
        &[
            "`.save-relay-meta.json`",
            "`place`",
            "`picture`",
            "80",
            "JPEG",
            "PNG",
            "512 KiB",
            "`.save-relay-`",
            "never",
            "push",
            "note",
        ],
    );
}

#[test]
fn sidecar_example_is_jsonc_with_place_and_picture() {
    let d = manifest_doc();
    let s = section(&d, "sidecar").expect("sidecar section");
    let examples = blocks(&s, "```jsonc");
    assert!(
        !examples.is_empty(),
        "the sidecar section has a ```jsonc example"
    );
    let text = examples[0].join("\n");
    assert!(text.contains("\"place\""), "sidecar example has place");
    assert!(text.contains("\"picture\""), "sidecar example has picture");
    assert!(
        text.contains("\".save-relay-"),
        "the sidecar example's picture is a reserved .save-relay- file"
    );
}

#[test]
fn manifest_json_examples_still_use_the_json_fence_for_manifests_only() {
    let d = manifest_doc();
    let s = section(&d, "sidecar").expect("sidecar section");
    assert!(
        blocks(&s, "```json").is_empty(),
        "sidecar examples use ```jsonc, not ```json (tests/manifest_doc.rs parses ```json as a manifest)"
    );
}

// ---- shared rules ----

#[test]
fn new_docs_have_no_private_details() {
    assert_no_private_details("docs/wrap.md", &wrap_doc());
    assert_no_private_details("docs/panel.md", &panel_doc());
    assert_no_private_details("docs/manifest.md", &manifest_doc());
}

#[test]
fn code_map_rows_move_to_present() {
    let map = read("docs/code-map.md");
    for doc in ["docs/wrap.md", "docs/panel.md"] {
        let row = map
            .lines()
            .find(|l| l.contains(&format!("`{doc}`")))
            .unwrap_or_else(|| panic!("docs/code-map.md has a row for {doc}"));
        assert!(row.contains("present"), "row for {doc} says present: {row}");
        assert!(
            !row.contains("planned"),
            "row for {doc} is not planned: {row}"
        );
    }
}

#[test]
fn f78_wrap_docs_explain_cloud_states_and_commands() {
    let doc = wrap_doc();
    let section = section(&doc, "Games without Steam Cloud").expect("the F78 section");
    assert_has_all(
        "docs/wrap.md Games without Steam Cloud",
        &section,
        &[
            "steam-games",
            "--no-steam-cloud",
            "unknown",
            "app ids",
            "skipped",
        ],
    );
    let lines = commands(&doc);
    for want in [
        "steam-games",
        "steam-games --no-steam-cloud",
        "install-wrap --no-steam-cloud --dry-run",
        "install-wrap --no-steam-cloud",
    ] {
        assert!(
            lines
                .iter()
                .any(|line| line == &format!("save-relay {want}")),
            "docs/wrap.md has the F78 command: {want}"
        );
    }
    assert_each_command_parses("docs/wrap.md", &doc);
}
