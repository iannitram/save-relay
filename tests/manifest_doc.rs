//! Locked tests for `docs/manifest.md` (F25). Do not edit.
use save_relay::manifest::Manifest;

fn read(relative: &str) -> String {
    let path = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), relative);
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{relative} exists"))
}

fn doc() -> String {
    read("docs/manifest.md")
}

fn json_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let t = line.trim();
        match current.as_mut() {
            None if t == "```json" => current = Some(String::new()),
            Some(_) if t == "```" => blocks.push(current.take().unwrap()),
            Some(b) => {
                b.push_str(line);
                b.push('\n');
            }
            None => {}
        }
    }
    blocks
}

fn manifests() -> Vec<Manifest> {
    json_blocks(&doc())
        .iter()
        .map(|b| {
            Manifest::from_json(b).unwrap_or_else(|e| panic!("example does not parse: {e}\n{b}"))
        })
        .collect()
}

#[test]
fn every_json_example_is_a_valid_manifest() {
    assert!(!manifests().is_empty(), "the guide has a ```json example");
}

#[test]
fn has_the_uncharted_example_and_a_second_game() {
    let all = manifests();
    let uncharted = all
        .iter()
        .find(|m| m.id == "uncharted-1")
        .expect("uncharted-1 example");
    assert!(uncharted
        .include
        .contains(&"Uncharted 1/SAVEDATA*/**".to_string()));
    let ids: std::collections::BTreeSet<&str> = all.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.len() >= 2, "a second game with its own id, got {ids:?}");
}

#[test]
fn has_a_saves_from_launcher_example() {
    let launcher = manifests()
        .into_iter()
        .find(|m| m.saves_from.as_deref() == Some("launcher"));
    let launcher = launcher.expect("a ```json example with \"saves_from\": \"launcher\"");
    assert!(launcher.saves.is_empty());
}

#[test]
fn names_every_field() {
    let d = doc();
    for field in [
        "format",
        "id",
        "name",
        "saves",
        "saves_from",
        "include",
        "exclude",
    ] {
        assert!(
            d.contains(&format!("`{field}`")),
            "field `{field}` explained"
        );
    }
}

#[test]
fn names_every_placeholder_per_os_and_the_override() {
    let d = doc();
    for p in ["{home}", "{data}", "{config}", "{documents}", "--saves-dir"] {
        assert!(d.contains(p), "{p} explained");
    }
    for os in ["Linux", "Windows", "macOS"] {
        assert!(d.contains(os), "{os} named");
    }
    for folder in [
        "XDG_DATA_HOME",
        "LOCALAPPDATA",
        "APPDATA",
        "Application Support",
    ] {
        assert!(d.contains(folder), "{folder} named");
    }
}

#[test]
fn explains_the_pattern_rules() {
    let d = doc();
    for rule in ["`*`", "`?`", "`**`", "`..`", "relative"] {
        assert!(d.contains(rule), "pattern rule {rule} explained");
    }
}

#[test]
fn says_one_manifest_per_game() {
    assert!(doc().to_lowercase().contains("one manifest per game"));
}

#[test]
fn has_the_sections() {
    let d = doc().to_lowercase();
    let headings: Vec<&str> = d.lines().filter(|l| l.starts_with("## ")).collect();
    for word in ["fields", "pattern", "placeholder", "override", "example"] {
        assert!(
            headings.iter().any(|h| h.contains(word)),
            "a ## heading names {word}, got {headings:?}"
        );
    }
    assert!(
        d.lines().next().unwrap_or("").starts_with("# "),
        "starts with a # title"
    );
}

#[test]
fn is_short_and_has_no_private_details() {
    let d = doc();
    let lines = d.lines().count();
    assert!((40..=200).contains(&lines), "{lines} lines");
    for private in ["/home/", "C:\\Users", "@gmail", "@example"] {
        assert!(!d.contains(private), "{private} must not appear");
    }
}

#[test]
fn the_code_map_row_moves_to_present() {
    let map = read("docs/code-map.md");
    let row = map
        .lines()
        .find(|l| l.starts_with("| `docs/manifest.md` |"))
        .expect("the code map has a docs/manifest.md row");
    assert!(row.contains("| present |"), "row is present: {row}");
    assert!(!row.contains("| planned |"), "row no longer planned: {row}");
}
