//! Locked tests for F48: the open-elsewhere mark (`presence`). Do not edit.

use save_relay::presence::{check, Hold, Mark, FILE, FORMAT};
use save_relay::Error;

const GOOD: &str =
    r#"{"format":1,"device":"amd-handheld","since":"2026-11-01T19:12:00Z","revision_read":7}"#;

fn mark(device: &str) -> Mark {
    let since = "2026-11-01T19:12:00Z".to_string();
    let device = device.to_string();
    Mark {
        format: 1,
        device,
        since,
        revision_read: Some(7),
    }
}

fn invalid(text: &str) -> bool {
    matches!(Mark::from_json(text), Err(Error::StateInvalid(_)))
}

#[test]
fn constants() {
    assert_eq!(FORMAT, 1);
    assert_eq!(FILE, "playing.json");
}

#[test]
fn to_json_is_compact_in_field_order() {
    assert_eq!(mark("amd-handheld").to_json(), GOOD);
    let none = Mark {
        revision_read: None,
        ..mark("strix-halo-tablet")
    };
    let want = r#"{"format":1,"device":"strix-halo-tablet","since":"2026-11-01T19:12:00Z","revision_read":null}"#;
    assert_eq!(none.to_json(), want);
}

#[test]
fn from_json_reads_a_good_mark() {
    assert_eq!(Mark::from_json(GOOD).unwrap(), mark("amd-handheld"));
    let pretty = "{\n  \"format\": 1,\n  \"device\": \"amd-handheld\",\n  \"since\": \"2026-11-01T19:12:00Z\",\n  \"revision_read\": null\n}\n";
    let m = Mark::from_json(pretty).unwrap();
    assert_eq!(
        m,
        Mark {
            revision_read: None,
            ..mark("amd-handheld")
        }
    );
    let m = mark("rtx-3090-ti-desktop");
    assert_eq!(Mark::from_json(&m.to_json()).unwrap(), m);
}

#[test]
fn from_json_refuses_bad_marks() {
    assert!(invalid(""));
    assert!(invalid("not json"));
    assert!(invalid(r#"{"format":1,"device":"a"}"#));
    let with = |format: u32, device: &str, since: &str| {
        let m = Mark {
            format,
            device: device.to_string(),
            since: since.to_string(),
            revision_read: None,
        };
        m.to_json()
    };
    assert!(invalid(&with(1, "", "2026-11-01T19:12:00Z")));
    assert!(invalid(&with(1, "a", "yesterday")));
    assert!(invalid(&with(1, "a", "2026-11-01 19:12:00")));
    assert!(invalid(&with(0, "a", "2026-11-01T19:12:00Z")));
    assert!(!invalid(&with(1, "a", "2026-11-01T19:12:00Z")));
}

#[test]
fn a_newer_format_says_so() {
    let text = r#"{"format":2,"device":"a","since":"2026-11-01T19:12:00Z","revision_read":null}"#;
    match Mark::from_json(text) {
        Err(Error::StateInvalid(m)) => assert!(m.contains("made by a newer save-relay"), "{m}"),
        other => panic!("expected StateInvalid, got {other:?}"),
    }
}

#[test]
fn check_says_who_holds_the_game() {
    assert_eq!(check(None, "amd-handheld"), Hold::Free);
    let m = mark("amd-handheld");
    assert_eq!(check(Some(&m), "amd-handheld"), Hold::Mine);
    let device = "amd-handheld".to_string();
    let elsewhere = Hold::Elsewhere {
        device,
        since: "2026-11-01T19:12:00Z".to_string(),
    };
    assert_eq!(check(Some(&m), "strix-halo-tablet"), elsewhere);
    assert_eq!(
        check(Some(&m), "AMD-handheld"),
        elsewhere,
        "names compare exactly"
    );
}
