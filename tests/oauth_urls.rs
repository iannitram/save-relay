use save_relay::oauth::{authorize_url, form_encode, parse_redirect, DROPBOX, GOOGLE};
use save_relay::Error;

#[test]
fn form_encode_keeps_unreserved_and_encodes_the_rest() {
    assert_eq!(
        form_encode(&[("a", "b c"), ("x", "1&2")]),
        "a=b%20c&x=1%262"
    );
    assert_eq!(form_encode(&[]), "");
    assert_eq!(form_encode(&[("k", "AZaz09-._~")]), "k=AZaz09-._~");
    assert_eq!(form_encode(&[("k", "+=/é")]), "k=%2B%3D%2F%C3%A9");
    let got = form_encode(&[("redirect_uri", "http://127.0.0.1:53710/")]);
    assert_eq!(got, "redirect_uri=http%3A%2F%2F127.0.0.1%3A53710%2F");
}

#[test]
fn google_sign_in_url_asks_for_drive_file_only() {
    let url = authorize_url(&GOOGLE, "id.apps", "http://127.0.0.1:40123/", "CH", "ST");
    let want = "https://accounts.google.com/o/oauth2/v2/auth?client_id=id.apps\
        &redirect_uri=http%3A%2F%2F127.0.0.1%3A40123%2F&response_type=code\
        &scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.file\
        &code_challenge=CH&code_challenge_method=S256&state=ST\
        &access_type=offline&prompt=consent";
    assert_eq!(url, want);
}

#[test]
fn dropbox_code_flow_url_has_no_redirect() {
    let url = authorize_url(&DROPBOX, "key1", "", "CH", "ST");
    let want = "https://www.dropbox.com/oauth2/authorize?client_id=key1&response_type=code\
        &scope=account_info.read%20files.metadata.read%20files.metadata.write\
        %20files.content.read%20files.content.write\
        &code_challenge=CH&code_challenge_method=S256&state=ST&token_access_type=offline";
    assert_eq!(url, want);
    let with = authorize_url(&DROPBOX, "key1", "http://127.0.0.1:53710/", "CH", "ST");
    assert!(with.contains("&redirect_uri=http%3A%2F%2F127.0.0.1%3A53710%2F&response_type"));
}

#[test]
fn redirect_gives_the_decoded_code() {
    let target = "/?code=4%2F0Ab-x&state=ST&scope=https%3A%2F%2Fwww.googleapis.com";
    assert_eq!(parse_redirect(target, "ST").unwrap(), "4/0Ab-x");
    assert_eq!(parse_redirect("/?state=ST&code=abc", "ST").unwrap(), "abc");
    assert_eq!(
        parse_redirect("/cb?code=a+b&state=ST", "ST").unwrap(),
        "a b"
    );
}

fn failed(target: &str, state: &str) -> String {
    match parse_redirect(target, state) {
        Err(Error::SignInFailed(m)) => m,
        other => panic!("{target}: expected sign_in_failed, got {other:?}"),
    }
}

#[test]
fn redirect_failures() {
    let mismatch = "the answer did not match this sign-in";
    assert_eq!(failed("/?code=abc&state=OTHER", "ST"), mismatch);
    assert_eq!(failed("/?code=abc", "ST"), mismatch);
    assert_eq!(failed("/", "ST"), mismatch);
    assert_eq!(failed("/favicon.ico", "ST"), mismatch);
    assert_eq!(
        failed("/?error=access_denied&state=ST", "ST"),
        "access_denied"
    );
    assert_eq!(
        failed("/?state=WRONG&error=access_denied", "ST"),
        "access_denied"
    );
    assert_eq!(failed("/?state=ST", "ST"), "no code in the answer");
}
