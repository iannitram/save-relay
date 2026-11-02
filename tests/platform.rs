//! Locked tests for `platform` (F04). Do not edit.
use save_relay::platform::{AppDirs, PathVars, Platform};
use save_relay::Error;
use std::path::PathBuf;

fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |k| {
        pairs
            .iter()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| v.to_string())
    }
}

#[test]
fn linux_defaults() {
    let v = PathVars::from_env(Platform::Linux, &env(&[("HOME", "/home/me")])).unwrap();
    let home = PathBuf::from("/home/me");
    assert_eq!(v.home, home);
    assert_eq!(v.data, home.join(".local/share"));
    assert_eq!(v.config, home.join(".config"));
    assert_eq!(v.documents, Some(home.join("Documents")));
}

#[test]
fn linux_xdg_wins() {
    let e = env(&[
        ("HOME", "/home/me"),
        ("XDG_DATA_HOME", "/data"),
        ("XDG_CONFIG_HOME", "/conf"),
    ]);
    let v = PathVars::from_env(Platform::Linux, &e).unwrap();
    assert_eq!(v.data, PathBuf::from("/data"));
    assert_eq!(v.config, PathBuf::from("/conf"));
}

#[test]
fn empty_value_counts_as_unset() {
    let e = env(&[
        ("HOME", "/home/me"),
        ("XDG_DATA_HOME", ""),
        ("XDG_CONFIG_HOME", ""),
    ]);
    let v = PathVars::from_env(Platform::Linux, &e).unwrap();
    assert_eq!(v.data, PathBuf::from("/home/me").join(".local/share"));
    assert_eq!(v.config, PathBuf::from("/home/me").join(".config"));
    let r = PathVars::from_env(Platform::Linux, &env(&[("HOME", "")]));
    assert!(matches!(r, Err(Error::Io(m)) if m.contains("HOME")));
}

#[test]
fn missing_home_is_io_naming_it() {
    let r = PathVars::from_env(Platform::Linux, &env(&[]));
    assert!(matches!(r, Err(Error::Io(m)) if m == "HOME is not set"));
    let r = PathVars::from_env(Platform::Mac, &env(&[]));
    assert!(matches!(r, Err(Error::Io(m)) if m == "HOME is not set"));
}

#[test]
fn mac_application_support() {
    let v = PathVars::from_env(Platform::Mac, &env(&[("HOME", "/Users/me")])).unwrap();
    let home = PathBuf::from("/Users/me");
    let support = home.join("Library/Application Support");
    assert_eq!(v.home, home);
    assert_eq!(v.data, support);
    assert_eq!(v.config, support);
    assert_eq!(v.documents, Some(home.join("Documents")));
}

#[test]
fn windows_folders() {
    let e = env(&[
        ("USERPROFILE", "C:\\Users\\me"),
        ("LOCALAPPDATA", "C:\\Users\\me\\AppData\\Local"),
        ("APPDATA", "C:\\Users\\me\\AppData\\Roaming"),
        ("HOME", "/ignored"),
    ]);
    let v = PathVars::from_env(Platform::Windows, &e).unwrap();
    assert_eq!(v.home, PathBuf::from("C:\\Users\\me"));
    assert_eq!(v.data, PathBuf::from("C:\\Users\\me\\AppData\\Local"));
    assert_eq!(v.config, PathBuf::from("C:\\Users\\me\\AppData\\Roaming"));
    assert_eq!(
        v.documents,
        Some(PathBuf::from("C:\\Users\\me").join("Documents"))
    );
}

#[test]
fn windows_missing_appdata_is_io() {
    let e = env(&[("USERPROFILE", "C:\\Users\\me"), ("LOCALAPPDATA", "C:\\L")]);
    let r = PathVars::from_env(Platform::Windows, &e);
    assert!(matches!(r, Err(Error::Io(m)) if m == "APPDATA is not set"));
}

#[test]
fn mobile_is_usage() {
    let e = env(&[("HOME", "/home/me")]);
    assert!(matches!(
        PathVars::from_env(Platform::Android, &e),
        Err(Error::Usage(_))
    ));
    assert!(matches!(
        PathVars::from_env(Platform::Ios, &e),
        Err(Error::Usage(_))
    ));
}

#[test]
fn app_dirs_add_save_relay() {
    let vars = PathVars {
        home: PathBuf::from("/h"),
        data: PathBuf::from("/d"),
        config: PathBuf::from("/c"),
        documents: None,
    };
    let dirs = AppDirs::from_vars(&vars);
    assert_eq!(dirs.config, PathBuf::from("/c").join("save-relay"));
    assert_eq!(dirs.data, PathBuf::from("/d").join("save-relay"));
}
