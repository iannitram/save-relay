use save_relay::steam::{
    cloud::{appdetails_url, parse_appdetails, StoreAnswer},
    parse_app_name,
};

#[test]
fn appdetails_url_uses_requested_filters() {
    assert_eq!(
        appdetails_url(504230),
        "https://store.steampowered.com/api/appdetails?appids=504230&filters=basic,categories"
    );
}

#[test]
fn parses_store_answers() {
    let fixture = |name: &str| {
        std::fs::read_to_string(format!("tests/fixtures/steam-store/{name}.json")).unwrap()
    };
    assert_eq!(
        parse_appdetails(&fixture("game-cloud"), 504230).unwrap(),
        StoreAnswer::Game { cloud: true }
    );
    assert_eq!(
        parse_appdetails(&fixture("game-no-cloud"), 504231).unwrap(),
        StoreAnswer::Game { cloud: false }
    );
    assert_eq!(
        parse_appdetails(&fixture("not-game"), 504232).unwrap(),
        StoreAnswer::NotAGame
    );
    assert_eq!(
        parse_appdetails(&fixture("unknown"), 504233).unwrap(),
        StoreAnswer::Unknown
    );
}

#[test]
fn malformed_or_missing_store_answers_are_state_errors() {
    for (text, id) in [("not json", 504230), ("{}", 504230)] {
        assert!(
            matches!(parse_appdetails(text, id), Err(save_relay::Error::StateInvalid(message)) if message.starts_with("steam store: "))
        );
    }
    for text in [
        r#"{"504230":{"success":true,"data":{}}}"#,
        r#"{"504230":{"success":true,"data":{"type":"game","categories":"bad"}}}"#,
    ] {
        assert!(
            matches!(parse_appdetails(text, 504230), Err(save_relay::Error::StateInvalid(message)) if message.starts_with("steam store: "))
        );
    }
}

#[test]
fn parses_appmanifest_name() {
    assert_eq!(
        parse_app_name("\"AppState\" { \"name\" \"Made-up Game\" }"),
        Some("Made-up Game".into())
    );
    assert_eq!(parse_app_name("\"AppState\" { \"appid\" \"42\" }"), None);
}
