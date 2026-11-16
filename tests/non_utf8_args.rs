use serde_json::Value;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::process::Command;

#[cfg(unix)]
#[test]
fn a_non_utf8_argument_returns_a_json_error_envelope() {
    let output = Command::new(env!("CARGO_BIN_EXE_save-relay"))
        .arg("help")
        .arg(std::ffi::OsStr::from_bytes(b"bad\xffid"))
        .arg("--json")
        .output()
        .expect("run CLI");
    assert_eq!(output.status.code(), Some(2));
    let envelope: Value = serde_json::from_slice(&output.stdout).expect("JSON envelope");
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "usage");
    assert!(String::from_utf8_lossy(&output.stdout).contains("valid UTF-8"));
}

#[cfg(windows)]
#[test]
fn an_unpaired_surrogate_argument_returns_a_json_error_envelope() {
    use std::os::windows::ffi::OsStringExt;

    let output = Command::new(env!("CARGO_BIN_EXE_save-relay"))
        .arg(std::ffi::OsString::from_wide(&[0xD800]))
        .arg("--json")
        .output()
        .expect("run CLI");
    assert_eq!(output.status.code(), Some(2));
    let envelope: Value = serde_json::from_slice(&output.stdout).expect("JSON envelope");
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "usage");
    assert!(String::from_utf8_lossy(&output.stdout).contains("valid UTF-8"));
}
