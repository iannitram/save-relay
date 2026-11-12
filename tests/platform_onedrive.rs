//! #250 item 11: a OneDrive-redirected Documents folder is `<winDocuments>` on Windows.
use save_relay::platform::{PathVars, Platform};
use std::fs;

fn vars(onedrive: Option<String>) -> PathVars {
    let env = move |name: &str| match name {
        "USERPROFILE" => Some("C:/Users/p".to_string()),
        "LOCALAPPDATA" => Some("C:/Users/p/AppData/Local".to_string()),
        "APPDATA" => Some("C:/Users/p/AppData/Roaming".to_string()),
        "OneDrive" => onedrive.clone(),
        _ => None,
    };
    PathVars::from_env(Platform::Windows, &env).unwrap()
}

#[test]
fn documents_inside_onedrive_win_when_that_folder_exists() {
    let root = std::env::temp_dir().join(format!("save-relay-od-{}", std::process::id()));
    fs::create_dir_all(root.join("Documents")).unwrap();
    let v = vars(Some(root.display().to_string()));
    assert_eq!(v.documents, Some(root.join("Documents")));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn without_a_onedrive_documents_folder_documents_stay_under_the_profile() {
    let v = vars(Some("/nonexistent-onedrive".into()));
    assert_eq!(
        v.documents,
        Some("C:/Users/p".into()).map(|p: std::path::PathBuf| p.join("Documents"))
    );
    assert_eq!(vars(None).documents, v.documents);
}
