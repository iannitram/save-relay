use save_relay::creds::Secret;
use save_relay::oauth::{needs_refresh, parse_token_response, Tokens, REFRESH_MARGIN};
use save_relay::service::Service;
use save_relay::Error;

const GOOGLE_OK: &str = r#"{"access_token":"ya29.a","expires_in":3599,"refresh_token":"1//r",
"scope":"https://www.googleapis.com/auth/drive.file","token_type":"Bearer"}"#;
const DROPBOX_OK: &str = r#"{"access_token":"sl.x","token_type":"bearer","expires_in":14400,
"refresh_token":"rt","scope":"files.content.read","uid":"1","account_id":"dbid:1"}"#;

#[test]
fn a_sign_in_answer_gives_both_tokens() {
    let want = Tokens {
        access: Secret::new("ya29.a"),
        expires_at: 1000 + 3599,
        refresh: Some(Secret::new("1//r")),
    };
    assert_eq!(
        parse_token_response(Service::Gdrive, 200, GOOGLE_OK, 1000),
        Ok(want)
    );
    let got = parse_token_response(Service::Dropbox, 200, DROPBOX_OK, 5).unwrap();
    assert_eq!(got.access.expose(), "sl.x");
    assert_eq!(got.expires_at, 14405);
    assert_eq!(got.refresh.unwrap().expose(), "rt");
}

#[test]
fn a_refresh_answer_may_leave_out_expiry_and_refresh_token() {
    let got = parse_token_response(Service::Gdrive, 200, r#"{"access_token":"a2"}"#, 7).unwrap();
    assert_eq!(got.access.expose(), "a2");
    assert_eq!(got.expires_at, 3607);
    assert_eq!(got.refresh, None);
}

#[test]
fn tokens_never_show_in_debug() {
    let got = parse_token_response(Service::Gdrive, 200, GOOGLE_OK, 0).unwrap();
    let text = format!("{got:?}");
    assert!(!text.contains("ya29.a") && !text.contains("1//r"), "{text}");
}

#[test]
fn invalid_grant_means_sign_in_again() {
    let body =
        r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#;
    let g = parse_token_response(Service::Gdrive, 400, body, 0);
    assert_eq!(g, Err(Error::ReauthNeeded("Google Drive".into())));
    let d = parse_token_response(Service::Dropbox, 401, r#"{"error":"invalid_grant"}"#, 0);
    assert_eq!(d, Err(Error::ReauthNeeded("Dropbox".into())));
    assert_eq!(d.unwrap_err().code(), "reauth_needed");
}

#[test]
fn other_errors_say_what_the_service_said() {
    let body =
        r#"{"error":"invalid_client","error_description":"The OAuth client was not found."}"#;
    let got = parse_token_response(Service::Gdrive, 401, body, 0);
    let want = "invalid_client: The OAuth client was not found.";
    assert_eq!(got, Err(Error::SignInFailed(want.into())));
    let got = parse_token_response(Service::Dropbox, 400, r#"{"error":"access_denied"}"#, 0);
    assert_eq!(got, Err(Error::SignInFailed("access_denied".into())));
    assert_eq!(got.unwrap_err().code(), "sign_in_failed");
}

#[test]
fn broken_answers_are_server_errors() {
    let s = |status: u16, body: &str| parse_token_response(Service::Gdrive, status, body, 0);
    let err = |code: u16| Err(Error::Server(format!("token endpoint: {code}")));
    assert_eq!(s(500, "oops"), err(500));
    assert_eq!(s(503, r#"{"error":"invalid_grant"}"#), err(503));
    assert_eq!(s(200, "{}"), err(200));
    assert_eq!(s(200, "not json"), err(200));
    assert_eq!(s(200, r#"{"access_token":7}"#), err(200));
    assert_eq!(s(302, r#"{"access_token":"a"}"#), err(302));
}

#[test]
fn refresh_a_minute_before_expiry() {
    assert_eq!(REFRESH_MARGIN, 60);
    assert!(!needs_refresh(1000, 939));
    assert!(needs_refresh(1000, 940));
    assert!(needs_refresh(1000, 2000));
    assert!(needs_refresh(0, 0));
}
