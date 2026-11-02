//! Locked tests for `manifest` (F05). Do not edit.
use save_relay::manifest::{expand, Manifest};
use save_relay::platform::{PathVars, Platform};
use save_relay::Error;
use std::path::PathBuf;

const GOOD: &str = r#"{
  "format": 1,
  "id": "uncharted-1",
  "name": "Uncharted: Drake's Fortune",
  "saves": { "linux": "{data}/uncharted-pc/user", "windows": "{config}\\uncharted" },
  "include": ["Uncharted 1/SAVEDATA*/**"],
  "exclude": ["**/*.tmp"]
}"#;

fn vars(documents: Option<&str>) -> PathVars {
    PathVars {
        home: PathBuf::from("/home/me"),
        data: PathBuf::from("/home/me/.local/share"),
        config: PathBuf::from("/home/me/.config"),
        documents: documents.map(PathBuf::from),
    }
}

fn invalid(text: &str, field: &str) {
    match Manifest::from_json(text) {
        Err(Error::ManifestInvalid(m)) => assert!(m.starts_with(field), "{m:?} for {field}"),
        other => panic!("expected ManifestInvalid for {field}, got {other:?}"),
    }
}

#[test]
fn parses_a_good_manifest() {
    let m = Manifest::from_json(GOOD).unwrap();
    assert_eq!(m.id, "uncharted-1");
    assert_eq!(m.name, "Uncharted: Drake's Fortune");
    assert_eq!(m.saves.len(), 2);
    assert_eq!(m.saves[&Platform::Linux], "{data}/uncharted-pc/user");
    assert_eq!(m.include, vec!["Uncharted 1/SAVEDATA*/**".to_string()]);
    assert_eq!(m.exclude, vec!["**/*.tmp".to_string()]);
    assert_eq!(m.validate(), Ok(()));
}

#[test]
fn exclude_defaults_to_empty() {
    let text = GOOD.replace(
        r#",
  "exclude": ["**/*.tmp"]"#,
        "",
    );
    assert!(Manifest::from_json(&text).unwrap().exclude.is_empty());
}

#[test]
fn bad_json_is_invalid() {
    assert!(matches!(
        Manifest::from_json("{"),
        Err(Error::ManifestInvalid(_))
    ));
    assert!(matches!(
        Manifest::from_json("{}"),
        Err(Error::ManifestInvalid(_))
    ));
}

#[test]
fn checks_format_and_id() {
    invalid(&GOOD.replace(r#""format": 1"#, r#""format": 2"#), "format");
    for id in ["", "Uncharted", "-x", "a_b", "a b", &"a".repeat(65)] {
        invalid(&GOOD.replace("\"uncharted-1\"", &format!("{id:?}")), "id");
    }
    for id in ["a", "1", "x-", &"a".repeat(64)] {
        assert!(Manifest::from_json(&GOOD.replace("\"uncharted-1\"", &format!("{id:?}"))).is_ok());
    }
}

#[test]
fn checks_name_saves_include() {
    invalid(
        &GOOD.replace("\"Uncharted: Drake's Fortune\"", "\"\""),
        "name",
    );
    let no_saves =
        r#""saves": { "linux": "{data}/uncharted-pc/user", "windows": "{config}\\uncharted" }"#;
    invalid(&GOOD.replace(no_saves, r#""saves": {}"#), "saves");
    invalid(
        &GOOD.replace(r#"["Uncharted 1/SAVEDATA*/**"]"#, "[]"),
        "include",
    );
}

#[test]
fn checks_patterns() {
    for p in ["/abs/**", "a/../b", "..", "a\\\\b"] {
        invalid(&GOOD.replace("Uncharted 1/SAVEDATA*/**", p), "include");
        invalid(&GOOD.replace("**/*.tmp", p), "exclude");
    }
}

#[test]
fn saves_dir_expands_the_platform_template() {
    let m = Manifest::from_json(GOOD).unwrap();
    let v = vars(None);
    let want = PathBuf::from(format!("{}/uncharted-pc/user", v.data.display()));
    assert_eq!(m.saves_dir(Platform::Linux, &v).unwrap(), want);
    match m.saves_dir(Platform::Mac, &v) {
        Err(Error::ManifestInvalid(msg)) => assert_eq!(msg, "saves: no folder for mac"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn expand_replaces_each_placeholder() {
    let v = vars(Some("/home/me/Documents"));
    let cases = [
        ("{home}/x", format!("{}/x", v.home.display())),
        ("{data}/x", format!("{}/x", v.data.display())),
        ("{config}/x", format!("{}/x", v.config.display())),
        (
            "{documents}/My Games/x",
            "/home/me/Documents/My Games/x".to_string(),
        ),
        (
            "{home}/a/{data}",
            format!("{}/a/{}", v.home.display(), v.data.display()),
        ),
        ("/plain/path", "/plain/path".to_string()),
    ];
    for (template, want) in cases {
        assert_eq!(
            expand(template, &v).unwrap(),
            PathBuf::from(want),
            "{template}"
        );
    }
}

#[test]
fn expand_rejects_unknown_unclosed_and_missing_documents() {
    let v = vars(None);
    for t in ["{saves}/x", "{HOME}/x", "{home", "{documents}/x"] {
        assert!(
            matches!(expand(t, &v), Err(Error::ManifestInvalid(_))),
            "{t}"
        );
    }
}
