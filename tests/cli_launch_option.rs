use save_relay::steam::launch::{is_wrapped, unwrapped};
use serde_json::Value;
use std::fs;
use std::process::Command;

fn run(root: &std::path::Path, current: &str) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_save-relay"))
        .arg("launch-option")
        .arg("42")
        .arg(format!("--current={current}"))
        .arg("--json")
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .output()
        .expect("launch-option CLI runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).expect("JSON envelope")
}

#[test]
fn launch_option_uses_the_cli_binary_and_preserves_current_option_text() {
    let root =
        std::env::temp_dir().join(format!("save-relay-launch-option-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    let existing = run(&root, "-novid %command%");
    assert_eq!(existing["ok"], true);
    assert_eq!(existing["data"]["app_id"], 42);
    assert_eq!(existing["data"]["before"], "-novid %command%");
    let after = existing["data"]["after"].as_str().unwrap();
    assert!(after.starts_with("-novid "), "{after}");
    assert!(is_wrapped(after), "{after}");
    assert_eq!(unwrapped(after).as_deref(), Some("-novid %command%"));

    let empty = run(&root, "");
    assert_eq!(empty["data"]["before"], "");
    assert!(empty["data"]["after"]
        .as_str()
        .unwrap()
        .ends_with("%command%"));
    fs::remove_dir_all(root).unwrap();
}
