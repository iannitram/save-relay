//! The platform browser session's PKCE parts (#254): the app brings the redirect URI, the
//! sign-in URL is the same shape as the desktop one, and a redirect that does not match its
//! `state` or arrives after the timeout is refused before any network call.

use save_relay::clients::Client;
use save_relay::error::{Error, Result};
use save_relay::fake::REFRESH_TOKEN;
use save_relay::http::{Request, Response, Transport};
use save_relay::oauth::{self, DROPBOX, GOOGLE};

/// A transport that panics if the code exchange is reached: these refusals happen before it.
struct Refuse;

impl Transport for Refuse {
    fn send(&self, _: &Request) -> Result<Response> {
        panic!("no request expected")
    }
}

fn state_of(url: &str) -> String {
    url.split("state=")
        .nth(1)
        .and_then(|rest| rest.split('&').next())
        .expect("a state in the sign-in URL")
        .to_string()
}

#[test]
fn a_platform_sign_in_uses_the_apps_redirect_and_the_desktop_pkce_parts() {
    let client = Client {
        id: "dropbox-key",
        secret: None,
    };
    let dropbox = oauth::start_platform_sign_in(&DROPBOX, client, "saverelay://oauth", 1_000)
        .expect("a sign-in starts");
    assert_eq!(dropbox.service, DROPBOX.service);
    assert_eq!(dropbox.redirect_uri, "saverelay://oauth");
    assert!(dropbox
        .url
        .starts_with("https://www.dropbox.com/oauth2/authorize?"));
    assert!(dropbox.url.contains("response_type=code"));
    assert!(dropbox.url.contains("code_challenge_method=S256"));
    assert!(dropbox.url.contains("redirect_uri=saverelay%3A%2F%2Foauth"));
    assert!(dropbox.url.contains("token_access_type=offline"));
    assert!(dropbox.url.contains(&format!("state={}", dropbox.state)));
    assert!(dropbox.url.contains("client_id=dropbox-key"));
    assert!(dropbox.url.contains("files.content.write"));

    let google_client = Client {
        id: "google-id",
        secret: Some("google-secret"),
    };
    let google =
        oauth::start_platform_sign_in(&GOOGLE, google_client, "com.example.game:/oauth", 1_000)
            .expect("a sign-in starts");
    assert!(google
        .url
        .starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
    assert!(google
        .url
        .contains("scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.file"));
    assert!(google.url.contains("access_type=offline"));
    assert!(google.url.contains("prompt=consent"));
    assert!(google.url.contains("code_challenge_method=S256"));
    // The client secret never appears in a sign-in URL.
    assert!(!google.url.contains("google-secret"));
}

#[test]
fn a_platform_sign_in_refuses_the_wrong_state_or_an_expired_redirect() {
    let client = Client {
        id: "dropbox-key",
        secret: None,
    };
    let sign_in = oauth::start_platform_sign_in(&DROPBOX, client, "saverelay://oauth", 1_000)
        .expect("a sign-in starts");

    let wrong = oauth::finish_platform_sign_in(
        &DROPBOX,
        client,
        &sign_in,
        "saverelay://oauth?code=the-code&state=another-sign-in",
        &Refuse,
        || 1_000,
    );
    assert!(matches!(wrong, Err(Error::SignInFailed(_))), "{wrong:?}");

    let expired = oauth::finish_platform_sign_in(
        &DROPBOX,
        client,
        &sign_in,
        &format!("saverelay://oauth?code=the-code&state={}", sign_in.state),
        &Refuse,
        || 1_000 + oauth::SIGN_IN_TIMEOUT,
    );
    assert!(
        matches!(expired, Err(Error::SignInFailed(_))),
        "{expired:?}"
    );

    // A sign-in started for Dropbox cannot be finished as Google's.
    let other_service = oauth::finish_platform_sign_in(
        &GOOGLE,
        client,
        &sign_in,
        &format!(
            "saverelay://oauth?code=the-code&state={}",
            state_of(&sign_in.url)
        ),
        &Refuse,
        || 1_000,
    );
    assert!(
        matches!(other_service, Err(Error::SignInFailed(_))),
        "{other_service:?}"
    );
}

#[test]
fn a_platform_sign_in_needs_a_redirect_uri() {
    let client = Client {
        id: "dropbox-key",
        secret: None,
    };
    let error = oauth::start_platform_sign_in(&DROPBOX, client, "  ", 1_000).unwrap_err();
    assert!(matches!(error, Error::Usage(_)), "{error:?}");
}

#[test]
fn the_code_exchange_returns_the_refresh_token_from_the_service() {
    let client = Client {
        id: "dropbox-key",
        secret: None,
    };
    let fake = save_relay::fake::FakeDropbox::new();
    let sign_in = oauth::start_platform_sign_in(&DROPBOX, client, "saverelay://oauth", 1_000)
        .expect("a sign-in starts");
    let tokens = oauth::finish_platform_sign_in(
        &DROPBOX,
        client,
        &sign_in,
        &format!(
            "saverelay://oauth?code={}&state={}",
            save_relay::fake::SIGN_IN_CODE,
            sign_in.state
        ),
        &fake,
        || 1_000,
    )
    .expect("the code exchanges");
    assert_eq!(
        tokens.refresh.as_ref().map(|s| s.expose()),
        Some(REFRESH_TOKEN)
    );
}
