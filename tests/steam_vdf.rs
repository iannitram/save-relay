use save_relay::steam::vdf::{parse, to_text, Vdf};
use save_relay::Error;

const LIBRARYFOLDERS: &str = r#""libraryfolders"
{
	"0"
	{
		"path"		"/home/steam/.local/share/Steam"
		"label"		""
		"apps"
		{
			"1659420"		"51539607552"
			"1145360"		"13750127820"
		}
	}
	"1"
	{
		"path"		"/home/steam/Games/SteamLibrary"
		"apps"
		{
			"480"		"0"
		}
	}
}
"#;

const APPMANIFEST: &str = r#""AppState"
{
	"appid"		"1145360"
	"Universe"		"1"
	"name"		"Hades"
	"StateFlags"		"4"
	"installdir"		"Hades"
	"LastUpdated"		"1698765432"
	"appbytes"		"13750127820"
}
"#;

const LOCALCONFIG: &str = r#""UserLocalConfigStore"
{
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
				"apps"
				{
					"1145360"
					{
						"LaunchOptions"		""
						"LastPlayed"		"1700000000"
					}
				}
			}
		}
	}
}
"#;

const MINIMAL_LOCALCONFIG: &str = r#""UserLocalConfigStore"
{
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
			}
		}
	}
}
"#;

const CREATED_LOCALCONFIG: &str = r#""UserLocalConfigStore"
{
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
				"apps"
				{
					"1145360"
					{
						"LaunchOptions"		"--skip-intro"
					}
				}
			}
		}
	}
}
"#;

const COMMENTED: &str = r#"// a top-level comment

"root"
{
	// a comment between pairs
	"url"		"https://example.com/a//b" // a trailing comment
	"key"		"value"

	"other"		"two"
}
"#;

const ESCAPED: &str = r#""esc"
{
	"quote"		"say \"hi\""
	"backslash"		"C:\\Games\\Hades"
	"newline"		"line1\nline2"
	"tab"		"col1\tcol2"
}
"#;

fn error_line(text: &str) -> (usize, String) {
    let error = parse(text).expect_err("expected malformed input to fail");
    match error {
        Error::Io(message) => {
            let rest = message
                .strip_prefix("vdf: line ")
                .unwrap_or_else(|| panic!("no line in {message}"));
            let (line, _) = rest
                .split_once(':')
                .unwrap_or_else(|| panic!("no line in {message}"));
            (line.parse().expect("line number"), message)
        }
        other => panic!("expected Error::Io, got {other:?}"),
    }
}

#[test]
fn steam_formatted_text_round_trips_byte_for_byte() {
    for text in [LIBRARYFOLDERS, APPMANIFEST, LOCALCONFIG] {
        let parsed = parse(text).expect("fixture parses");
        assert_eq!(to_text(&parsed), text);
    }
}

#[test]
fn get_and_get_path_and_as_str_read_the_fixtures() {
    let vdf = parse(LIBRARYFOLDERS).expect("libraryfolders parses");
    let zero = vdf
        .get("libraryfolders")
        .and_then(|libraries| libraries.get("0"))
        .expect("library 0");
    assert_eq!(
        zero.get("path").and_then(Vdf::as_str),
        Some("/home/steam/.local/share/Steam")
    );
    assert_eq!(zero.as_str(), None);

    let apps = vdf
        .get_path(&["libraryfolders", "0", "apps"])
        .expect("apps block");
    assert_eq!(apps.as_str(), None);
    assert_eq!(
        apps.get("1659420").and_then(Vdf::as_str),
        Some("51539607552")
    );
    assert_eq!(
        vdf.get_path(&["libraryfolders", "1", "apps", "480"])
            .and_then(Vdf::as_str),
        Some("0")
    );

    let app = parse(APPMANIFEST).expect("appmanifest parses");
    assert_eq!(
        app.get_path(&["AppState", "installdir"])
            .and_then(Vdf::as_str),
        Some("Hades")
    );
}

#[test]
fn missing_keys_and_wrong_depths_are_none() {
    let vdf = parse(LIBRARYFOLDERS).expect("libraryfolders parses");
    assert_eq!(vdf.get("nope"), None);
    assert_eq!(vdf.get_path(&["libraryfolders", "0", "nope"]), None);
    assert_eq!(
        vdf.get_path(&["libraryfolders", "0", "path", "deeper"]),
        None
    );
    assert_eq!(vdf.get_path(&["nope", "0", "apps"]), None);
}

#[test]
fn keys_match_without_ascii_case() {
    let local = parse(LOCALCONFIG).expect("localconfig parses");
    assert_eq!(
        local
            .get_path(&[
                "userlocalconfigstore",
                "SOFTWARE",
                "valve",
                "steam",
                "APPS",
                "1145360",
                "LAUNCHOPTIONS",
            ])
            .and_then(Vdf::as_str),
        Some("")
    );
}

#[test]
fn repeated_keys_keep_order_and_first_match() {
    let text = "\"root\"\n{\n\t\"dup\"\t\t\"first\"\n\t\"dup\"\t\t\"second\"\n}\n";
    let vdf = parse(text).expect("repeated keys parse");
    assert_eq!(
        vdf.get_path(&["root", "dup"]).and_then(Vdf::as_str),
        Some("first")
    );
    assert_eq!(to_text(&vdf), text);

    let mut changed = vdf.clone();
    changed.set_path(&["root", "dup"], Some(Vdf::Str("changed".into())));
    let Vdf::Map(root) = changed.get("root").expect("root") else {
        panic!("root is a Map");
    };
    assert_eq!(root.len(), 2);
    assert_eq!(root[0].1.as_str(), Some("changed"));
    assert_eq!(root[1].1.as_str(), Some("second"));

    let mut removed = vdf.clone();
    removed.set_path(&["root", "dup"], None);
    let Vdf::Map(root) = removed.get("root").expect("root") else {
        panic!("root is a Map");
    };
    assert_eq!(root.len(), 1);
    assert_eq!(root[0].1.as_str(), Some("second"));
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    let vdf = parse(COMMENTED).expect("comments parse");
    assert_eq!(
        vdf.get_path(&["root", "url"]).and_then(Vdf::as_str),
        Some("https://example.com/a//b")
    );
    assert_eq!(
        vdf.get_path(&["root", "key"]).and_then(Vdf::as_str),
        Some("value")
    );
    assert_eq!(
        vdf.get_path(&["root", "other"]).and_then(Vdf::as_str),
        Some("two")
    );
}

#[test]
fn escapes_are_read_and_written_both_ways() {
    let vdf = parse(ESCAPED).expect("escapes parse");
    let esc = vdf.get("esc").expect("esc block");
    assert_eq!(esc.get("quote").and_then(Vdf::as_str), Some("say \"hi\""));
    assert_eq!(
        esc.get("backslash").and_then(Vdf::as_str),
        Some("C:\\Games\\Hades")
    );
    assert_eq!(
        esc.get("newline").and_then(Vdf::as_str),
        Some("line1\nline2")
    );
    assert_eq!(esc.get("tab").and_then(Vdf::as_str), Some("col1\tcol2"));
    assert_eq!(to_text(&vdf), ESCAPED);

    let raw = Vdf::Map(vec![("k".into(), Vdf::Str("q\" b\\ n\n t\t".into()))]);
    let text = to_text(&raw);
    assert_eq!(text, "\"k\"\t\t\"q\\\" b\\\\ n\\n t\\t\"\n");
    assert_eq!(parse(&text).expect("escaped output parses"), raw);
}

#[test]
fn an_unknown_escape_is_kept_as_it_is() {
    let vdf = parse("\"root\"\n{\n\t\"k\"\t\t\"a\\qb\"\n}\n").expect("unknown escape parses");
    assert_eq!(
        vdf.get_path(&["root", "k"]).and_then(Vdf::as_str),
        Some("a\\qb")
    );
}

#[test]
fn an_unclosed_quote_reports_its_starting_line() {
    let (line, message) = error_line("\"root\"\n{\n\t\"key\"\t\t\"open\n}\n");
    assert_eq!(line, 3);
    assert!(message.contains("unclosed quote"), "{message}");
}

#[test]
fn an_unclosed_block_reports_its_opening_line() {
    let (line, message) = error_line("\"root\"\n{\n\t\"key\"\t\t\"value\"\n");
    assert_eq!(line, 2);
    assert!(message.contains("unclosed block"), "{message}");
}

#[test]
fn a_close_without_a_block_reports_its_line() {
    let (line, message) = error_line("\"root\"\n{\n\t\"key\"\t\t\"value\"\n}\n}\n");
    assert_eq!(line, 5);
    assert!(message.contains("no block"), "{message}");
}

#[test]
fn a_key_without_a_value_reports_the_key_line() {
    let (line, message) = error_line("\"root\"\n{\n\t\"key\"\n}\n");
    assert_eq!(line, 3);
    assert!(message.contains("key with no value"), "{message}");
}

#[test]
fn set_path_creates_missing_maps_and_a_value() {
    let mut vdf = parse(MINIMAL_LOCALCONFIG).expect("minimal localconfig parses");
    vdf.set_path(
        &[
            "UserLocalConfigStore",
            "Software",
            "Valve",
            "Steam",
            "apps",
            "1145360",
            "LaunchOptions",
        ],
        Some(Vdf::Str("--skip-intro".into())),
    );
    assert_eq!(
        vdf.get_path(&[
            "UserLocalConfigStore",
            "Software",
            "Valve",
            "Steam",
            "apps",
            "1145360",
            "LaunchOptions",
        ])
        .and_then(Vdf::as_str),
        Some("--skip-intro")
    );
    assert_eq!(to_text(&vdf), CREATED_LOCALCONFIG);
}

#[test]
fn set_path_replaces_only_the_edited_value() {
    let mut vdf = parse(LOCALCONFIG).expect("localconfig parses");
    vdf.set_path(
        &[
            "UserLocalConfigStore",
            "Software",
            "Valve",
            "Steam",
            "apps",
            "1145360",
            "LaunchOptions",
        ],
        Some(Vdf::Str("--skip-intro".into())),
    );
    let expected = LOCALCONFIG.replace(
        "\"LaunchOptions\"\t\t\"\"",
        "\"LaunchOptions\"\t\t\"--skip-intro\"",
    );
    assert_eq!(to_text(&vdf), expected);
}

#[test]
fn set_path_removes_the_first_matching_pair() {
    let mut vdf = parse(LOCALCONFIG).expect("localconfig parses");
    vdf.set_path(
        &[
            "UserLocalConfigStore",
            "Software",
            "Valve",
            "Steam",
            "apps",
            "1145360",
            "LaunchOptions",
        ],
        None,
    );
    let launch_line = "\t\t\t\t\t\t\"LaunchOptions\"\t\t\"\"\n";
    assert!(LOCALCONFIG.contains(launch_line));
    assert_eq!(to_text(&vdf), LOCALCONFIG.replacen(launch_line, "", 1));
}

#[test]
fn set_path_removes_nothing_when_the_path_is_absent() {
    let mut vdf = parse(LOCALCONFIG).expect("localconfig parses");
    let before = to_text(&vdf);
    vdf.set_path(
        &[
            "UserLocalConfigStore",
            "Software",
            "Valve",
            "Steam",
            "apps",
            "999999",
            "LaunchOptions",
        ],
        None,
    );
    assert_eq!(to_text(&vdf), before);
}

#[test]
fn set_path_replaces_a_str_met_on_the_way_with_a_map() {
    let mut vdf = parse("\"root\"\n{\n\t\"a\"\t\t\"str\"\n}\n").expect("small file parses");
    vdf.set_path(&["root", "a", "b", "c"], Some(Vdf::Str("deep".into())));
    assert_eq!(
        vdf.get_path(&["root", "a", "b", "c"]).and_then(Vdf::as_str),
        Some("deep")
    );
    assert!(matches!(vdf.get_path(&["root", "a"]), Some(Vdf::Map(_))));
}

#[test]
fn set_path_keeps_the_first_matchs_spelling() {
    let text = "\"root\"\n{\n\t\"Apps\"\n\t{\n\t\t\"1\"\t\t\"one\"\n\t}\n}\n";
    let mut vdf = parse(text).expect("mixed-case file parses");
    vdf.set_path(&["root", "apps", "2"], Some(Vdf::Str("two".into())));
    let written = to_text(&vdf);
    assert_eq!(written.matches("\"Apps\"").count(), 1);
    assert!(!written.contains("\"apps\""));
    assert_eq!(
        vdf.get_path(&["root", "APPS", "2"]).and_then(Vdf::as_str),
        Some("two")
    );
}
