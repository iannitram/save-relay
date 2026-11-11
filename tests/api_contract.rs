//! The fixture is the contract: every `panel/src/fixtures/*.json` envelope deserializes into
//! the matching `api` type and serializes back to the same JSON value. The TypeScript side
//! reads the same files (`panel/src/api.test.ts`).

use save_relay::api::{
    AnswerData, HistoryData, InstallWrapData, LaunchOptionData, PictureData, RestoreData,
    SignInPrompt, StatusData, SteamGamesData, SyncReport, WhereData, WhichData,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

/// `data` through the Rust type and back.
fn round_trip<T: DeserializeOwned + Serialize>(name: &str, data: &Value) {
    let typed: T = serde_json::from_value(data.clone())
        .unwrap_or_else(|e| panic!("{name}: does not deserialize: {e}"));
    let back = serde_json::to_value(&typed).unwrap();
    assert_eq!(&back, data, "{name}: serializes back to different JSON");
}

/// The exit-code group of an error code (`api::exit_code`'s table), by the fixture's name.
fn group(code: &str) -> i32 {
    match code {
        "usage"
        | "manifest_invalid"
        | "game_unknown"
        | "no_such_entry"
        | "save_location_unclear"
        | "steam_not_found"
        | "steam_running"
        | "no_pending_choice" => 2,
        "not_set_up" | "auth_failed" | "credentials" | "reauth_needed" | "sign_in_failed" => 4,
        "server_unreachable" | "server_error" | "state_invalid" | "changed_on_server"
        | "not_found" => 5,
        _ => 1,
    }
}

#[test]
fn every_fixture_round_trips() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("panel/src/fixtures");
    let mut seen = 0;
    for entry in fs::read_dir(&dir).expect("panel/src/fixtures exists") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap().to_string();
        let envelope: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let command = envelope["command"].as_str().expect("command").to_string();
        if envelope["ok"] == Value::Bool(false) {
            let error = &envelope["error"];
            assert_eq!(
                error.as_object().unwrap().len(),
                2,
                "{name}: error has code and message"
            );
            let code = error["code"].as_str().expect("code");
            assert!(error["message"].is_string(), "{name}");
            let want: i32 = match name.as_str() {
                "error-usage" => 2,
                "error-auth" => 4,
                "error-server" => 5,
                "error-other" => 1,
                _ => panic!("{name}: no exit-code group named for this error fixture"),
            };
            assert_eq!(group(code), want, "{name}: {code}");
            seen += 1;
            continue;
        }
        assert_eq!(envelope["ok"], Value::Bool(true), "{name}");
        assert_eq!(
            envelope.as_object().unwrap().len(),
            3,
            "{name}: ok, command, data"
        );
        let data = &envelope["data"];
        match command.as_str() {
            "pull" | "push" => round_trip::<SyncReport>(&name, data),
            "status" => round_trip::<StatusData>(&name, data),
            "history" => round_trip::<HistoryData>(&name, data),
            "restore" => round_trip::<RestoreData>(&name, data),
            "where" => round_trip::<WhereData>(&name, data),
            "which" => round_trip::<WhichData>(&name, data),
            "steam-games" => round_trip::<SteamGamesData>(&name, data),
            "install-wrap" => round_trip::<InstallWrapData>(&name, data),
            "launch-option" => round_trip::<LaunchOptionData>(&name, data),
            "answer" => round_trip::<AnswerData>(&name, data),
            "picture" => round_trip::<PictureData>(&name, data),
            "setup" => round_trip::<SignInPrompt>(&name, data),
            other => panic!("{name}: no Rust type for command {other}"),
        }
        seen += 1;
    }
    assert!(seen >= 19, "expected every fixture, saw {seen}");
}

#[test]
fn held_and_cards_are_written_as_null() {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("panel/src/fixtures/pull-pulled.json"),
    )
    .unwrap();
    let envelope: Value = serde_json::from_str(&text).unwrap();
    let report: SyncReport = serde_json::from_value(envelope["data"].clone()).unwrap();
    let back = serde_json::to_value(&report).unwrap();
    assert_eq!(back["held"], Value::Null);
    assert_eq!(back["local_card"], Value::Null);
}
