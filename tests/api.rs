use save_relay::api::{exit_code, exit_code_for, failure, success, Action, Side, SyncData};
use save_relay::transfer::Counts;
use save_relay::{Error, NotBuilt};
use serde_json::{json, Value};

fn parsed(line: &str) -> Value {
    assert!(!line.contains('\n'), "not one line: {line:?}");
    serde_json::from_str(line).unwrap()
}

#[test]
fn success_is_the_ok_envelope_on_one_line() {
    let line = success("status", &json!({"game": "g", "reachable": true}));
    assert_eq!(
        line,
        r#"{"ok":true,"command":"status","data":{"game":"g","reachable":true}}"#
    );
}

#[test]
fn success_carries_sync_data_in_snake_case() {
    let data = SyncData {
        game: "uncharted-1".into(),
        action: Action::Pulled,
        revision: Some(7),
        server: Some(Side {
            device: "strix-halo-tablet".into(),
            time: "2026-10-31T19:51:00Z".into(),
            revision: Some(7),
        }),
        local: None,
        backup: None,
        counts: Counts {
            uploaded: 0,
            downloaded: 3,
            deleted: 1,
        },
        message: "Pulled revision 7.".into(),
    };
    let v = parsed(&success("pull", &data));
    assert_eq!(
        v,
        json!({"ok": true, "command": "pull", "data": {
            "game": "uncharted-1", "action": "pulled", "revision": 7,
            "server": {"device": "strix-halo-tablet", "time": "2026-10-31T19:51:00Z", "revision": 7},
            "local": null, "backup": null,
            "counts": {"uploaded": 0, "downloaded": 3, "deleted": 1},
            "message": "Pulled revision 7."}})
    );
}

#[test]
fn actions_serialize_as_their_snake_case_names() {
    let all = [
        (Action::Noop, "noop"),
        (Action::Adopted, "adopted"),
        (Action::Pulled, "pulled"),
        (Action::Pushed, "pushed"),
        (Action::Offline, "offline"),
        (Action::Deferred, "deferred"),
        (Action::Conflict, "conflict"),
    ];
    for (action, name) in all {
        assert_eq!(parsed(&success("push", &action))["data"], json!(name));
    }
}

#[test]
fn failure_is_the_error_envelope_with_code_and_message() {
    let line = failure("pull", &Error::AuthFailed);
    assert_eq!(
        line,
        r#"{"ok":false,"command":"pull","error":{"code":"auth_failed","message":"the server refused the user name or app password"}}"#
    );
    let v = parsed(&failure("history", &Error::NoSuchEntry("x \"y\"".into())));
    assert_eq!(
        v["error"],
        json!({"code": "no_such_entry", "message": "no history entry 'x \"y\"'"})
    );
    assert_eq!(v["ok"], json!(false));
    assert!(v.get("data").is_none());
}

#[test]
fn conflict_exits_3_and_every_other_action_0() {
    assert_eq!(exit_code_for(Action::Conflict), 3);
    for a in [
        Action::Noop,
        Action::Adopted,
        Action::Pulled,
        Action::Pushed,
        Action::Offline,
        Action::Deferred,
    ] {
        assert_eq!(exit_code_for(a), 0, "{a:?}");
    }
}

#[test]
fn error_exit_codes() {
    let s = || "m".to_string();
    let table = [
        (Error::Usage(s()), 2),
        (Error::ManifestInvalid(s()), 2),
        (Error::GameUnknown(s()), 2),
        (Error::NoSuchEntry(s()), 2),
        (Error::NotSetUp, 4),
        (Error::AuthFailed, 4),
        (Error::Credentials(s()), 4),
        (Error::Unreachable(s()), 5),
        (Error::Server(s()), 5),
        (Error::StateInvalid(s()), 5),
        (Error::Changed, 5),
        (Error::NotFound(s()), 5),
        (Error::Io(s()), 1),
        (Error::NotBuilt(NotBuilt("x")), 1),
    ];
    for (e, code) in table {
        assert_eq!(exit_code(&e), code, "{e:?}");
    }
}
