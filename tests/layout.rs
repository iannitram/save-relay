//! Locked tests for `layout` (F03). Do not edit.
use save_relay::layout::{device_slug, entry_name, parse_entry_name, Layout};
use save_relay::Error;

#[test]
fn new_trims_slashes() {
    let l = Layout::new("/Games/save-relay/").unwrap();
    assert_eq!(l.root(), "Games/save-relay");
    assert_eq!(Layout::new("save-relay").unwrap().root(), "save-relay");
}

#[test]
fn new_rejects_bad_roots() {
    for bad in ["", "/", "//", "a\\b", "a/../b", "..", "./a", "a//b"] {
        assert!(
            matches!(Layout::new(bad), Err(Error::Usage(_))),
            "root {bad:?} should be a usage error"
        );
    }
}

#[test]
fn paths_under_the_game() {
    let l = Layout::new("/Games/save-relay/").unwrap();
    assert_eq!(l.game_dir("uncharted-1"), "Games/save-relay/uncharted-1");
    assert_eq!(
        l.state_path("uncharted-1"),
        "Games/save-relay/uncharted-1/state.json"
    );
    assert_eq!(
        l.history_dir("uncharted-1"),
        "Games/save-relay/uncharted-1/history"
    );
    assert_eq!(
        l.entry_dir("uncharted-1", "20261031T195100Z-amd-handheld"),
        "Games/save-relay/uncharted-1/history/20261031T195100Z-amd-handheld"
    );
    assert_eq!(
        l.incoming_dir("uncharted-1", "20261031T195100Z-amd-handheld"),
        "Games/save-relay/uncharted-1/incoming/20261031T195100Z-amd-handheld"
    );
}

#[test]
fn entry_name_joins_with_a_dash() {
    assert_eq!(
        entry_name("20261031T195100Z", "amd-handheld"),
        "20261031T195100Z-amd-handheld"
    );
}

#[test]
fn parse_entry_name_splits_stamp_and_device() {
    assert_eq!(
        parse_entry_name("20261031T195100Z-amd-handheld"),
        Some(("20261031T195100Z".to_string(), "amd-handheld".to_string()))
    );
    assert_eq!(
        parse_entry_name("20261031T195100Z-x"),
        Some(("20261031T195100Z".to_string(), "x".to_string()))
    );
}

#[test]
fn parse_entry_name_rejects_others() {
    for bad in [
        "",
        "20261031T195100Z",
        "20261031T195100Z-",
        "20261031T195100Zamd",
        "20261031X195100Z-amd",
        "2026103T195100Z-amd",
        "20261331T195100Z-amd",
        "notes",
        "incoming",
    ] {
        assert_eq!(parse_entry_name(bad), None, "{bad:?}");
    }
}

#[test]
fn device_slug_examples() {
    assert_eq!(device_slug("Steam Deck (OLED)"), "steam-deck-oled");
    assert_eq!(device_slug("amd-handheld"), "amd-handheld");
    assert_eq!(device_slug("  RTX__3090 Ti!! "), "rtx-3090-ti");
    assert_eq!(device_slug("a--b"), "a-b");
    assert_eq!(device_slug("Ünïcode Box"), "n-code-box");
}

#[test]
fn device_slug_empty_becomes_device() {
    assert_eq!(device_slug(""), "device");
    assert_eq!(device_slug("!!! ???"), "device");
}

#[test]
fn device_slug_cuts_at_32_then_trims() {
    let long = "abcdefghij".repeat(5);
    assert_eq!(device_slug(&long), &long[..32]);
    // the 32nd character is a dash: cut, then trimmed again
    let dash = format!("{}-{}", "a".repeat(31), "b".repeat(10));
    assert_eq!(device_slug(&dash), "a".repeat(31));
}
