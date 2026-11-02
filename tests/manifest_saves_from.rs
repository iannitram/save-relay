use save_relay::cli::{parse, Command, GameArg};
use save_relay::local::{GameRecord, LocalStore};
use save_relay::manifest::Manifest;
use save_relay::platform::{AppDirs, PathVars, Platform};
use save_relay::Error;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const TOTK: &str = r#"{"format":1,"id":"totk","name":"The Legend of Zelda: Tears of the Kingdom (1.4.3)","saves":{},"saves_from":"launcher","include":["slot_00/**","slot_01/**","slot_02/**","slot_03/**","slot_04/**","slot_05/**","option.sav"],"exclude":[]}"#;

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("saves-from-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn vars(&self) -> PathVars {
        PathVars {
            home: self.0.clone(),
            data: self.0.join("data"),
            config: self.0.join("config"),
            documents: None,
        }
    }

    fn store(&self) -> LocalStore {
        LocalStore::new(AppDirs::from_vars(&self.vars()))
    }

    fn manifest(&self, text: &str) -> PathBuf {
        let path = self.0.join("manifest.json");
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(snapshot(&path));
        } else {
            files.insert(path.clone(), std::fs::read(path).unwrap());
        }
    }
    files
}

#[test]
fn launcher_manifest_parses_and_round_trips() {
    let manifest = Manifest::from_json(TOTK).unwrap();
    assert!(manifest.saves.is_empty());
    assert_eq!(manifest.saves_from.as_deref(), Some("launcher"));
    assert_eq!(manifest.include.len(), 7);
    assert_eq!(manifest.validate(), Ok(()));
    let json = serde_json::to_string(&manifest).unwrap();
    assert_eq!(Manifest::from_json(&json).unwrap(), manifest);
    let minimal = r#"{"format":1,"id":"totk","name":"...","saves":{},"saves_from":"launcher","include":["slot_00/**","option.sav"],"exclude":[]}"#;
    assert!(Manifest::from_json(minimal).is_ok());
}

#[test]
fn empty_saves_without_source_keeps_the_locked_error() {
    let text = TOTK.replace(",\"saves_from\":\"launcher\"", "");
    assert_eq!(
        Manifest::from_json(&text),
        Err(Error::ManifestInvalid("saves: must not be empty".into()))
    );
}

#[test]
fn every_other_source_value_is_invalid() {
    for value in [
        r#""auto""#,
        r#""Launcher""#,
        r#""""#,
        "null",
        "1",
        "true",
        "{}",
        "[]",
    ] {
        let text = TOTK.replace(
            r#""saves_from":"launcher""#,
            &format!("\"saves_from\":{value}"),
        );
        match Manifest::from_json(&text) {
            Err(Error::ManifestInvalid(message)) => {
                assert!(message.starts_with("saves_from:"), "{value}: {message}");
            }
            other => panic!("{value}: {other:?}"),
        }
    }
    let mut manifest = Manifest::from_json(TOTK).unwrap();
    manifest.saves_from = Some("auto".into());
    assert!(
        matches!(manifest.validate(), Err(Error::ManifestInvalid(m)) if m.starts_with("saves_from:"))
    );
}

fn command_args(command: &str, manifest: &Path) -> Vec<String> {
    let mut args = vec![command.into(), "totk".into()];
    if command == "restore" {
        args.push("20261031T195100Z-device".into());
    }
    args.extend(["--manifest".into(), manifest.to_str().unwrap().into()]);
    args
}

#[test]
fn all_four_commands_refuse_without_saves_dir_before_writing() {
    let fixture = Fixture::new("refusal");
    let path = fixture.manifest(TOTK);
    let before = snapshot(&fixture.0);
    for command in ["pull", "push", "status", "restore"] {
        for json in [false, true] {
            let mut args = command_args(command, &path);
            if json {
                args.push("--json".into());
            }
            let output = std::process::Command::new(env!("CARGO_BIN_EXE_save-relay"))
                .args(args)
                .current_dir(&fixture.0)
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2), "{command}: {output:?}");
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(text.contains("--saves-dir"), "{command}: {text}");
            if json {
                assert_eq!(text.lines().count(), 1);
                let envelope: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(envelope["ok"], false);
                assert_eq!(envelope["command"], command);
                assert_eq!(envelope["error"]["code"], "usage");
            }
            assert_eq!(snapshot(&fixture.0), before);
        }
    }
}

#[test]
fn all_four_commands_use_the_explicit_folder_verbatim() {
    let fixture = Fixture::new("explicit");
    let path = fixture.manifest(TOTK);
    let folder = fixture.0.join("profile/save/device account/{data}");
    for command in ["pull", "push", "status", "restore"] {
        let mut args = command_args(command, &path);
        args.extend(["--saves-dir".into(), folder.to_str().unwrap().into()]);
        let parsed = parse(&args).unwrap();
        let game = match parsed.command {
            Command::Pull { game, .. }
            | Command::Push { game, .. }
            | Command::Status { game }
            | Command::Restore { game, .. } => game,
            other => panic!("{other:?}"),
        };
        let loaded = game
            .load(&fixture.store(), Platform::Mac, &fixture.vars())
            .unwrap();
        assert_eq!(loaded.saves_dir, folder);
        assert!(!folder.exists());
    }
}

#[test]
fn launcher_never_falls_back_to_templates_or_remembered_folders() {
    let fixture = Fixture::new("fallback");
    let mut manifest = Manifest::from_json(TOTK).unwrap();
    manifest
        .saves
        .insert(Platform::Mac, "guessed-folder".into());
    assert!(matches!(
        manifest.saves_dir(Platform::Mac, &fixture.vars()),
        Err(Error::Usage(message)) if message.contains("--saves-dir")
    ));
    let store = fixture.store();
    store
        .save_game(&GameRecord {
            manifest,
            saves_dir: Some(fixture.0.join("old-account")),
        })
        .unwrap();
    let game = GameArg {
        id: "totk".into(),
        manifest: None,
        saves_dir: None,
    };
    let before = snapshot(&fixture.0);
    assert!(matches!(
        game.load(&store, Platform::Mac, &fixture.vars()),
        Err(Error::Usage(message)) if message.contains("--saves-dir")
    ));
    assert_eq!(snapshot(&fixture.0), before);
}
