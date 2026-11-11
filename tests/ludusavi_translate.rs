use save_relay::ludusavi::{candidates, translate, Entry, Facts, Located};
use save_relay::platform::{PathVars, Platform};
use save_relay::Error;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn facts(platform: Platform) -> Facts {
    Facts {
        platform,
        vars: PathVars {
            home: "fixture/home".into(),
            data: "fixture/data".into(),
            config: "fixture/config".into(),
            documents: Some("fixture/documents".into()),
        },
        app_id: 1145360,
        steam_root: "fixture/steam".into(),
        library: "fixture/library".into(),
        install_dir: Some("fixture/library/steamapps/common/Hades".into()),
        user_id: Some("123".into()),
        os_user: "player".into(),
        proton: false,
    }
}

fn entry(files: Value) -> Entry {
    serde_json::from_value(json!({ "files": files })).unwrap()
}

fn locate(
    entry: &Entry,
    facts: &Facts,
    existing: &[&str],
    dirs: &[&str],
) -> Result<Located, Error> {
    let existing: BTreeSet<PathBuf> = existing.iter().map(PathBuf::from).collect();
    let dirs: BTreeSet<PathBuf> = dirs.iter().map(PathBuf::from).collect();
    translate(
        "Hades",
        entry,
        facts,
        &|path| existing.contains(path),
        &|path| dirs.contains(path),
    )
}

fn unclear(result: Result<Located, Error>) -> String {
    match result {
        Err(Error::SaveLocationUnclear(message)) => message,
        other => panic!("expected unclear save location, got {other:?}"),
    }
}

fn rejects_broad_folder(template: &str, facts: &Facts, folder: &str) {
    let entry = entry(json!({template: {"tags": ["save"]}}));
    let message = unclear(locate(&entry, facts, &[folder], &[folder]));
    assert!(message.contains("overly broad"), "{message}");
}

#[test]
fn bare_platform_roots_are_unclear() {
    let windows = facts(Platform::Windows);
    rejects_broad_folder("<home>", &windows, "fixture/home");
    rejects_broad_folder("<winAppData>", &windows, "fixture/config");
    rejects_broad_folder("<winLocalAppData>", &windows, "fixture/data");
    rejects_broad_folder("<winDocuments>", &windows, "fixture/documents");
    rejects_broad_folder("<root>", &windows, "fixture/library");
    rejects_broad_folder("<base>", &windows, "fixture/library/steamapps/common/Hades");
    rejects_broad_folder("fixture/steam", &windows, "fixture/steam");

    let linux = facts(Platform::Linux);
    rejects_broad_folder("<xdgData>", &linux, "fixture/data");
    rejects_broad_folder("<xdgConfig>", &linux, "fixture/config");

    let mut proton = linux;
    proton.proton = true;
    rejects_broad_folder(
        "<home>",
        &proton,
        "fixture/library/steamapps/compatdata/1145360/pfx/drive_c/users/steamuser",
    );
}

#[test]
fn windows_path_comparison_handles_case_and_separator_variants() {
    let mut windows = facts(Platform::Windows);
    windows.vars.home = PathBuf::from("C:/Users/Player");
    rejects_broad_folder(r"c:\Users", &windows, "c:/Users");

    windows.vars.home = PathBuf::from("C:/Users/Émile");
    rejects_broad_folder(r"c:\users\ÉMILE", &windows, "c:/users/ÉMILE");
}

#[cfg(windows)]
#[test]
fn refuses_windows_drive_root_and_users_parent_of_home() {
    let windows = facts(Platform::Windows);
    rejects_broad_folder("C:/", &windows, "C:/");

    let mut windows = windows;
    windows.vars.home = PathBuf::from("C:/Users/Player");
    rejects_broad_folder("C:/Users", &windows, "C:/Users");
    rejects_broad_folder("c:/users", &windows, "c:/users");
}

#[cfg(unix)]
#[test]
fn refuses_unix_filesystem_root() {
    rejects_broad_folder("/", &facts(Platform::Linux), "/");
}

fn hades() -> Entry {
    entry(json!({
        "<winDocuments>/Saved Games/Hades": {"tags": ["save"], "when": [{"os": "windows"}]}
    }))
}

#[test]
fn hades_windows_documents_produces_valid_manifest() {
    let facts = facts(Platform::Windows);
    let folder = "fixture/documents/Saved Games/Hades";
    assert_eq!(candidates(&hades(), &facts), [folder]);
    let located = locate(&hades(), &facts, &[folder], &[folder]).unwrap();
    assert_eq!(located.saves_dir, Path::new(folder));
    assert_eq!(located.manifest.format, 1);
    assert_eq!(located.manifest.id, "steam-1145360");
    assert_eq!(located.manifest.name, "Hades");
    assert_eq!(located.manifest.include, ["**"]);
    assert!(located.manifest.exclude.is_empty());
    assert_eq!(located.manifest.saves.len(), 1);
    located.manifest.validate().unwrap();
    assert_eq!(
        located
            .manifest
            .saves_dir(facts.platform, &facts.vars)
            .unwrap(),
        located.saves_dir
    );
}

#[test]
fn hades_linux_proton_uses_prefix_documents() {
    let mut facts = facts(Platform::Linux);
    assert!(candidates(&hades(), &facts).is_empty());
    facts.proton = true;
    let folder = "fixture/library/steamapps/compatdata/1145360/pfx/drive_c/users/steamuser/Documents/Saved Games/Hades";
    assert_eq!(candidates(&hades(), &facts), [folder]);
    let located = locate(&hades(), &facts, &[folder], &[folder]).unwrap();
    assert_eq!(located.saves_dir, Path::new(folder));
    assert_eq!(located.manifest.saves[&Platform::Linux], folder);
    assert_eq!(located.manifest.include, ["**"]);
}

#[test]
fn native_linux_xdg_data_file_uses_parent() {
    let entry = entry(json!({
        "<xdgData>/Hades/profile.sav": {"tags": ["save"], "when": [{"os": "linux", "store": "steam"}]}
    }));
    let located = locate(
        &entry,
        &facts(Platform::Linux),
        &["fixture/data/Hades"],
        &[],
    )
    .unwrap();
    assert_eq!(located.saves_dir, Path::new("fixture/data/Hades"));
    assert_eq!(located.manifest.include, ["profile.sav"]);
}

#[test]
fn config_only_is_unclear() {
    let entry = entry(json!({"<home>/Hades": {"tags": ["config"]}}));
    assert!(candidates(&entry, &facts(Platform::Mac)).is_empty());
    assert!(unclear(locate(&entry, &facts(Platform::Mac), &[], &[])).contains("no existing"));
}

#[test]
fn store_user_id_requires_known_user() {
    let entry = entry(json!({"<root>/userdata/<storeUserId>/saves": {"tags": ["save"]}}));
    let mut facts = facts(Platform::Windows);
    assert_eq!(
        candidates(&entry, &facts),
        ["fixture/library/userdata/123/saves"]
    );
    facts.user_id = None;
    assert!(candidates(&entry, &facts).is_empty());
    unclear(locate(&entry, &facts, &[], &[]));
}

#[test]
fn two_existing_folders_are_unclear_and_both_named() {
    let entry = entry(json!({
        "<home>/first/*.sav": {"tags": ["save"]},
        "<home>/second/*.sav": {"tags": ["save"]}
    }));
    let message = unclear(locate(
        &entry,
        &facts(Platform::Mac),
        &["fixture/home/first", "fixture/home/second"],
        &[],
    ));
    assert!(message.contains("multiple existing"));
    assert!(message.contains("fixture/home/first"));
    assert!(message.contains("fixture/home/second"));
}

#[test]
fn no_existing_folder_is_unclear_and_names_attempts() {
    let message = unclear(locate(&hades(), &facts(Platform::Windows), &[], &[]));
    assert!(message.contains("no existing"));
    assert!(message.contains("fixture/documents/Saved Games"));
    unclear(locate(&Entry::default(), &facts(Platform::Linux), &[], &[]));
}

#[test]
fn base_glob_splits_at_first_wildcard_segment() {
    let entry = entry(json!({
        "<base>/saves/*.sav": {"tags": ["save"]},
        "<base>/saves/slot?/**/profile.sav": {"tags": ["save"]},
        "<base>/saves/**/backup?.sav": {"tags": ["save"]}
    }));
    let folder = "fixture/library/steamapps/common/Hades/saves";
    let located = locate(&entry, &facts(Platform::Mac), &[folder], &[]).unwrap();
    assert_eq!(located.saves_dir, Path::new(folder));
    assert_eq!(
        located.manifest.include,
        ["**/backup?.sav", "*.sav", "slot?/**/profile.sav"]
    );
    located.manifest.validate().unwrap();
}

#[test]
fn unknown_or_unclosed_placeholder_is_dropped() {
    let entry = entry(json!({
        "<unknown>/saves": {"tags": ["save"]},
        "<home>/<unknown>/saves": {"tags": ["save"]},
        "<home>/saves/<unfinished": {"tags": ["save"]}
    }));
    assert!(candidates(&entry, &facts(Platform::Linux)).is_empty());
    unclear(locate(&entry, &facts(Platform::Linux), &[], &[]));
}

#[test]
fn epic_store_condition_is_dropped() {
    let entry = entry(json!({"<home>/saves": {"tags": ["save"], "when": [{"store": "epic"}]}}));
    assert!(candidates(&entry, &facts(Platform::Windows)).is_empty());
    unclear(locate(&entry, &facts(Platform::Windows), &[], &[]));
}

#[test]
fn placeholders_respect_each_platform() {
    let entry = entry(json!({
        "<home>/saves": {"tags": ["save"]},
        "<winAppData>/saves": {"tags": ["save"]},
        "<winLocalAppData>/saves": {"tags": ["save"]},
        "<winDocuments>/saves": {"tags": ["save"]},
        "<xdgData>/saves": {"tags": ["save"]},
        "<xdgConfig>/saves": {"tags": ["save"]},
        "<root>/<osUserName>/<storeUserId>": {"tags": ["save"]}
    }));
    for platform in [
        Platform::Windows,
        Platform::Linux,
        Platform::Mac,
        Platform::Android,
        Platform::Ios,
    ] {
        let mut expected = vec!["fixture/home/saves", "fixture/library/player/123"];
        if matches!(platform, Platform::Windows | Platform::Linux) {
            expected.extend(["fixture/config/saves", "fixture/data/saves"]);
        }
        if platform == Platform::Windows {
            expected.push("fixture/documents/saves");
        }
        expected.sort();
        assert_eq!(
            candidates(&entry, &facts(platform)),
            expected,
            "{platform:?}"
        );
    }
}

#[test]
fn proton_maps_all_windows_placeholders_home_and_username() {
    let entry = entry(json!({
        "<home>/<osUserName>": {"tags": ["save"]},
        "<winAppData>/saves": {"tags": ["save"]},
        "<winLocalAppData>/saves": {"tags": ["save"]},
        "<winDocuments>/saves": {"tags": ["save"]}
    }));
    let mut facts = facts(Platform::Linux);
    facts.proton = true;
    let prefix = "fixture/library/steamapps/compatdata/1145360/pfx/drive_c/users/steamuser";
    assert_eq!(
        candidates(&entry, &facts),
        [
            format!("{prefix}/AppData/Local/saves"),
            format!("{prefix}/AppData/Roaming/saves"),
            format!("{prefix}/Documents/saves"),
            format!("{prefix}/steamuser"),
        ]
    );
}

#[test]
fn missing_optional_folders_are_dropped_without_install_dir_guess() {
    let entry: Entry = serde_json::from_value(json!({
        "installDir": {"Hades": {}},
        "files": {
            "<base>/saves": {"tags": ["save"]},
            "<winDocuments>/saves": {"tags": ["save"]}
        }
    }))
    .unwrap();
    let mut facts = facts(Platform::Windows);
    facts.install_dir = None;
    facts.vars.documents = None;
    assert!(candidates(&entry, &facts).is_empty());
}

#[test]
fn game_placeholder_uses_translate_name_only() {
    let entry = entry(json!({"<home>/<game>/*.sav": {"tags": ["save"]}}));
    let facts = facts(Platform::Mac);
    assert!(candidates(&entry, &facts).is_empty());
    let located = locate(&entry, &facts, &["fixture/home/Hades"], &[]).unwrap();
    assert_eq!(located.saves_dir, Path::new("fixture/home/Hades"));
    assert_eq!(located.manifest.include, ["*.sav"]);
}

#[test]
fn when_conditions_are_alternatives_and_must_match_os_and_store_together() {
    let entry = entry(json!({
        "<home>/accepted": {"tags": ["config", "save"], "when": [{"os": "windows"}, {"os": "mac", "store": "steam"}]},
        "<home>/rejected": {"tags": ["save"], "when": [{"os": "mac", "store": "epic"}, {"os": "linux", "store": "steam"}]}
    }));
    assert_eq!(
        candidates(&entry, &facts(Platform::Mac)),
        ["fixture/home/accepted"]
    );
}

#[test]
fn candidates_normalize_separators_sort_and_deduplicate() {
    let entry = entry(json!({
        "<home>/z/*.sav": {"tags": ["save"]},
        "<home>/a/slot?.sav": {"tags": ["save"]},
        "fixture/home/a/slot?.sav": {"tags": ["save"]}
    }));
    let mut facts = facts(Platform::Windows);
    facts.vars.home = r"fixture\home".into();
    assert_eq!(
        candidates(&entry, &facts),
        ["fixture/home/a/slot?.sav", "fixture/home/z/*.sav"]
    );
}

#[test]
fn shared_folder_collects_patterns_and_drops_missing_folders() {
    let entry = entry(json!({
        "<home>/saves/*.sav": {"tags": ["save"]},
        "<home>/saves/profile.dat": {"tags": ["save"]},
        "<home>/absent/*.sav": {"tags": ["save"]}
    }));
    let located = locate(&entry, &facts(Platform::Mac), &["fixture/home/saves"], &[]).unwrap();
    assert_eq!(located.manifest.include, ["*.sav", "profile.dat"]);
    assert_eq!(located.saves_dir, Path::new("fixture/home/saves"));
}

#[test]
fn parent_segments_and_windows_reserved_names_never_reach_the_manifest() {
    let parent = entry(json!({
        "<home>/../escape/*.sav": {"tags": ["save"]}
    }));
    let message = unclear(locate(
        &parent,
        &facts(Platform::Mac),
        &["fixture/escape"],
        &[],
    ));
    assert!(message.contains("no existing save folder"));
    let reserved = entry(json!({
        "<home>/saves/CON.sav": {"tags": ["save"]}
    }));
    match locate(
        &reserved,
        &facts(Platform::Mac),
        &["fixture/home/saves"],
        &[],
    ) {
        Err(Error::ManifestInvalid(message)) => assert!(message.contains("CON.sav")),
        other => panic!("expected an invalid manifest, got {other:?}"),
    }
}
