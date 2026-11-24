use save_relay::cli::refuse_terminal_password;
use serde_json::Value;

#[test]
fn terminal_nextcloud_is_refused_before_reading_and_pipes_are_accepted() {
    for args in [
        vec![
            "setup",
            "--server",
            "https://example.org",
            "--user",
            "alice",
        ],
        vec![
            "setup",
            "nextcloud",
            "--url",
            "https://example.org",
            "--user",
            "alice",
            "--password-stdin",
            "--json",
        ],
    ] {
        let args: Vec<String> = args.into_iter().map(String::from).collect();
        let mut out = Vec::new();
        assert_eq!(refuse_terminal_password(&args, true, &mut out), Some(2));
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Pipe it on standard input"));
        if args.iter().any(|arg| arg == "--json") {
            let envelope: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(envelope["command"], "setup");
            assert_eq!(envelope["ok"], false);
        }
        assert_eq!(
            refuse_terminal_password(&args, false, &mut Vec::new()),
            None
        );
    }
    let oauth = vec!["setup".into(), "--service".into(), "dropbox".into()];
    assert_eq!(
        refuse_terminal_password(&oauth, true, &mut Vec::new()),
        None
    );
}
