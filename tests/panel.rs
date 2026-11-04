//! `save-relay panel` (F63, `src/panel.rs`, `cli::panel_command`): the token, the request
//! parser, the routes, the API arguments, the responses, the `panel` arm of the command line,
//! and a real loopback socket served with a fake command runner (no Steam, no server, no
//! keyring). The socket tests bind port 0 and read the port from the announcement.

use save_relay::api::PanelData;
use save_relay::cli::{self, Command};
use save_relay::panel::{
    self, api_args, parse_request, response, route, token_from_bytes, Request, Route, API_COMMANDS,
    TOKEN_HEADER,
};
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const TOKEN: &str = "T0ken-for_the-tests-0123456789abcdefghiJ";
const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 16];

/// The header line that carries the test token.
fn token_line() -> String {
    format!("{TOKEN_HEADER}: {TOKEN}")
}

/// Raw request bytes with a `Content-Length` that matches `body`.
fn raw(method: &str, target: &str, headers: &[String], body: &str) -> Vec<u8> {
    let mut text = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    for header in headers {
        text.push_str(header);
        text.push_str("\r\n");
    }
    text.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    text.into_bytes()
}

fn parsed(method: &str, target: &str, headers: &[String], body: &str) -> Request {
    parse_request(&raw(method, target, headers, body))
        .expect("a well-formed request")
        .expect("a complete request")
}

// ---- the token ------------------------------------------------------------------------------

#[test]
fn token_is_43_base64url_characters_from_32_bytes() {
    assert_eq!(token_from_bytes(&[0; 32]), "A".repeat(43));
    let token = token_from_bytes(&[0xFF; 32]);
    assert_eq!(token.len(), 43);
    assert!(token
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'));
    let mut other = [0u8; 32];
    other[31] = 1;
    assert_ne!(token_from_bytes(&other), token_from_bytes(&[0; 32]));
}

// ---- the request parser ---------------------------------------------------------------------

#[test]
fn a_complete_request_is_parsed_with_its_path_query_headers_and_body() {
    let request = parsed(
        "POST",
        "/api/status?token=a%2Bb&space=x+y&bare",
        &["X-Note: hello  ".to_string()],
        "{}",
    );
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/api/status");
    assert_eq!(
        request.query,
        vec![
            ("token".to_string(), "a+b".to_string()),
            ("space".to_string(), "x y".to_string()),
            ("bare".to_string(), String::new()),
        ]
    );
    assert!(request
        .headers
        .contains(&("X-Note".to_string(), "hello".to_string())));
    assert_eq!(request.body, b"{}".to_vec());
}

#[test]
fn the_path_is_percent_decoded() {
    let request = parsed("GET", "/picture/my%20game/entry%C3%A9", &[], "");
    assert_eq!(request.path, "/picture/my game/entry\u{e9}");
}

#[test]
fn an_incomplete_request_is_none_until_the_body_has_arrived() {
    assert_eq!(
        parse_request(b"GET / HTTP/1.1\r\nHost: x\r\n").unwrap(),
        None
    );
    let partial = b"POST /api/status HTTP/1.1\r\nContent-Length: 10\r\n\r\n{}";
    assert_eq!(parse_request(partial).unwrap(), None);
}

#[test]
fn bytes_after_the_body_are_not_part_of_the_request() {
    let bytes = b"POST /api/status HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}GET / HTTP/1.1\r\n\r\n";
    let request = parse_request(bytes).unwrap().unwrap();
    assert_eq!(request.body, b"{}".to_vec());
}

#[test]
fn a_malformed_request_is_a_usage_error() {
    let bad: [&[u8]; 14] = [
        b"GET / HTTP/1.0\r\n\r\n",
        b"get / HTTP/1.1\r\n\r\n",
        b"GET http://example.test/ HTTP/1.1\r\n\r\n",
        b"GET /a%2Fb HTTP/1.1\r\n\r\n",
        b"GET /a%2fb HTTP/1.1\r\n\r\n",
        b"GET /a%zz HTTP/1.1\r\n\r\n",
        b"GET /a%FF HTTP/1.1\r\n\r\n",
        b"GET / HTTP/1.1\r\nBadHeader\r\n\r\n",
        b"GET / HTTP/1.1\r\nBad Name: x\r\n\r\n",
        b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
        b"POST / HTTP/1.1\r\nContent-Length: abc\r\n\r\n",
        b"POST / HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nx",
        b"POST / HTTP/1.1\r\nContent-Length: 2000000\r\n\r\n",
        b"GET  / HTTP/1.1\r\n\r\n",
    ];
    for bytes in bad {
        let text = String::from_utf8_lossy(bytes).into_owned();
        let error = parse_request(bytes).expect_err(&text);
        assert_eq!(error.code(), "usage", "{text}");
    }
}

#[test]
fn headers_that_never_end_are_refused_past_16_kib() {
    let mut bytes = b"GET / HTTP/1.1\r\nX-Pad: ".to_vec();
    bytes.extend(vec![b'a'; 17 * 1024]);
    assert_eq!(parse_request(&bytes).unwrap_err().code(), "usage");
}

// ---- routes ---------------------------------------------------------------------------------

#[test]
fn the_token_line_carries_the_test_token() {
    assert!(token_line().ends_with(TOKEN));
}

#[test]
fn the_page_needs_the_token_in_the_header_or_the_query() {
    let page = |headers: &[String]| parsed("GET", "/", headers, "");
    assert_eq!(route(&page(&[]), TOKEN), Route::Forbidden);
    assert_eq!(
        route(&page(&["X-Save-Relay-Token: wrong".to_string()]), TOKEN),
        Route::Forbidden
    );
    let almost = format!("{TOKEN_HEADER}: {TOKEN}x");
    assert_eq!(route(&page(&[almost]), TOKEN), Route::Forbidden);
    assert_eq!(
        route(&page(&[token_line()]), TOKEN),
        Route::Asset("index.html")
    );
    assert_eq!(
        route(&parsed("GET", &format!("/?token={TOKEN}"), &[], ""), TOKEN),
        Route::Asset("index.html")
    );
    assert_eq!(
        route(&parsed("GET", "/index.html", &[token_line()], ""), TOKEN),
        Route::Asset("index.html")
    );
}

#[test]
fn the_header_name_is_case_insensitive_and_the_header_wins_over_the_query() {
    let lower = format!("x-save-relay-token: {TOKEN}");
    assert_eq!(
        route(&parsed("GET", "/", &[lower], ""), TOKEN),
        Route::Asset("index.html")
    );
    let wrong_header = format!("{TOKEN_HEADER}: nope");
    assert_eq!(
        route(
            &parsed("GET", &format!("/?token={TOKEN}"), &[wrong_header], ""),
            TOKEN
        ),
        Route::Forbidden
    );
}

#[test]
fn the_token_is_checked_before_the_path() {
    assert_eq!(
        route(&parsed("GET", "/nope", &[], ""), TOKEN),
        Route::Forbidden
    );
    assert_eq!(
        route(&parsed("GET", "/nope", &[token_line()], ""), TOKEN),
        Route::NotFound
    );
    assert_eq!(
        route(&parsed("PUT", "/", &[token_line()], ""), TOKEN),
        Route::NotFound
    );
}

#[test]
fn the_front_end_files_need_the_token_like_every_other_request() {
    let by_header = parsed("GET", "/app.js", &[token_line()], "");
    let by_query = parsed("GET", &format!("/app.css?token={TOKEN}"), &[], "");
    #[cfg(feature = "panel-dist")]
    {
        assert_eq!(route(&by_header, TOKEN), Route::Asset("app.js"));
        assert_eq!(route(&by_query, TOKEN), Route::Asset("app.css"));
    }
    #[cfg(not(feature = "panel-dist"))]
    {
        // The placeholder build has no other files.
        assert_eq!(route(&by_header, TOKEN), Route::NotFound);
        assert_eq!(route(&by_query, TOKEN), Route::NotFound);
    }
    assert_eq!(
        route(&parsed("GET", "/app.js", &[], ""), TOKEN),
        Route::Forbidden
    );
    let wrong = parsed("GET", "/app.css?token=nope", &[], "");
    assert_eq!(route(&wrong, TOKEN), Route::Forbidden);
    // A POST to a file name is not an API call, with or without the token.
    assert_eq!(
        route(&parsed("POST", "/app.js", &[token_line()], ""), TOKEN),
        Route::NotFound
    );
    assert_eq!(
        route(&parsed("POST", "/app.js", &[], ""), TOKEN),
        Route::Forbidden
    );
}

#[test]
fn the_served_page_carries_the_token_on_its_front_end_files() {
    let html = br#"<script type="module" crossorigin src="./app.js"></script><link rel="stylesheet" crossorigin href="./app.css"><a href="./app.json">x</a>"#;
    let served = String::from_utf8(panel::served_page(TOKEN, html)).expect("text");
    assert_eq!(
        served,
        format!(
            r#"<script type="module" crossorigin src="./app.js?token={TOKEN}"></script><link rel="stylesheet" crossorigin href="./app.css?token={TOKEN}"><a href="./app.json">x</a>"#
        )
    );
    let placeholder = b"<p>this build has no panel</p>";
    assert_eq!(panel::served_page(TOKEN, placeholder), placeholder.to_vec());
}

#[test]
fn api_posts_run_the_known_commands_with_the_token() {
    for command in API_COMMANDS {
        let request = parsed("POST", &format!("/api/{command}"), &[token_line()], "{}");
        assert_eq!(
            route(&request, TOKEN),
            Route::Api(command.to_string()),
            "{command}"
        );
    }
    let by_query = parsed("POST", &format!("/api/status?token={TOKEN}"), &[], "{}");
    assert_eq!(route(&by_query, TOKEN), Route::Api("status".to_string()));
}

#[test]
fn commands_outside_the_panel_list_and_malformed_api_paths_are_not_found() {
    for path in [
        "/api/wrap",
        "/api/install-wrap",
        "/api/panel",
        "/api/",
        "/api/status/extra",
        "/api/Status",
    ] {
        let request = parsed("POST", path, &[token_line()], "{}");
        assert_eq!(route(&request, TOKEN), Route::NotFound, "{path}");
    }
    assert_eq!(
        route(&parsed("GET", "/api/status", &[token_line()], ""), TOKEN),
        Route::NotFound
    );
    assert_eq!(
        route(&parsed("POST", "/picture/g/e", &[token_line()], ""), TOKEN),
        Route::NotFound
    );
}

#[test]
fn pictures_take_a_game_and_an_entry_with_an_optional_file_segment() {
    let picture = |path: &str| route(&parsed("GET", path, &[token_line()], ""), TOKEN);
    let expected = Route::Picture {
        game: "steam-1".to_string(),
        entry: "2026-11-01".to_string(),
    };
    assert_eq!(picture("/picture/steam-1/2026-11-01"), expected);
    assert_eq!(
        picture("/picture/steam-1/2026-11-01/.save-relay-picture.png"),
        expected
    );
    assert_eq!(picture("/picture/steam-1"), Route::NotFound);
    assert_eq!(picture("/picture/steam-1/a/b/c"), Route::NotFound);
    assert_eq!(picture("/picture//entry"), Route::NotFound);
    assert_eq!(picture("/picture/--json/entry"), Route::NotFound);
    assert_eq!(
        route(&parsed("GET", "/picture/steam-1/entry", &[], ""), TOKEN),
        Route::Forbidden
    );
}

// ---- API arguments --------------------------------------------------------------------------

fn args(command: &str, body: &str) -> Vec<String> {
    api_args(command, body.as_bytes()).expect("accepted")
}

fn refused(command: &str, body: &str) {
    let error = api_args(command, body.as_bytes()).expect_err(body);
    assert_eq!(error.code(), "usage", "{command} {body}");
}

#[test]
fn each_command_becomes_its_command_line_with_json_last() {
    assert_eq!(
        args(
            "status",
            r#"{"game":"g","manifest":"m.json","saves-dir":"d"}"#
        ),
        [
            "status",
            "g",
            "--manifest",
            "m.json",
            "--saves-dir",
            "d",
            "--json"
        ]
    );
    assert_eq!(
        args("history", r#"{"game":"g"}"#),
        ["history", "g", "--json"]
    );
    assert_eq!(
        args("restore", r#"{"game":"g","entry":"e"}"#),
        ["restore", "g", "e", "--json"]
    );
    assert_eq!(
        args(
            "pull",
            r#"{"game":"g","resolve":"keep-local","anyway":true}"#
        ),
        ["pull", "g", "--resolve", "keep-local", "--anyway", "--json"]
    );
    assert_eq!(
        args("pull", r#"{"game":"g","anyway":false}"#),
        ["pull", "g", "--json"]
    );
    assert_eq!(
        args("push", r#"{"game":"g","resolve":"keep-server"}"#),
        ["push", "g", "--resolve", "keep-server", "--json"]
    );
    assert_eq!(
        args("answer", r#"{"game":"g","answer":"wait"}"#),
        ["answer", "g", "wait", "--json"]
    );
    assert_eq!(args("where", "{}"), ["where", "--json"]);
    assert_eq!(args("where", ""), ["where", "--json"]);
    assert_eq!(
        args("which", r#"{"appid":105600}"#),
        ["which", "105600", "--json"]
    );
    assert_eq!(
        args("which", r#"{"appid":"105600"}"#),
        ["which", "105600", "--json"]
    );
    assert_eq!(
        args("picture", r#"{"game":"g","entry":"e"}"#),
        ["picture", "g", "e", "--json"]
    );
    assert_eq!(
        args("setup", r#"{"service":"gdrive","no-browser":true}"#),
        ["setup", "--service", "gdrive", "--no-browser", "--json"]
    );
    assert_eq!(
        args(
            "setup",
            r#"{"service":"dropbox","finish":true,"device":"deck"}"#
        ),
        [
            "setup",
            "--service",
            "dropbox",
            "--device",
            "deck",
            "--finish",
            "--json"
        ]
    );
}

#[test]
fn every_panel_command_is_accepted_by_its_own_shape() {
    for command in API_COMMANDS {
        let line = match command {
            "where" => args(command, "{}"),
            "which" => args(command, r#"{"appid":1}"#),
            "restore" | "picture" => args(command, r#"{"game":"g","entry":"e"}"#),
            "answer" => args(command, r#"{"game":"g","answer":"wait"}"#),
            "setup" => args(command, r#"{"service":"gdrive","no-browser":true}"#),
            _ => args(command, r#"{"game":"g"}"#),
        };
        assert_eq!(line[0], command);
        assert_eq!(line.last().map(String::as_str), Some("--json"));
    }
}

#[test]
fn appids_must_be_whole_numbers_in_range() {
    refused("which", r#"{"appid":-1}"#);
    refused("which", r#"{"appid":4294967296}"#);
    refused("which", r#"{"appid":1.5}"#);
    refused("which", r#"{"appid":true}"#);
    assert_eq!(
        args("which", r#"{"appid":4294967295}"#),
        ["which", "4294967295", "--json"]
    );
}

#[test]
fn commands_outside_the_panel_are_refused() {
    for command in ["wrap", "install-wrap", "panel", "nope", ""] {
        refused(command, "{}");
    }
}

#[test]
fn fields_the_command_does_not_take_or_a_missing_one_are_refused() {
    refused("status", "{}");
    refused("status", r#"{"game":"g","anyway":true}"#);
    refused("status", r#"{"game":"g","extra":"x"}"#);
    refused("history", r#"{"game":"g","resolve":"keep-local"}"#);
    refused("restore", r#"{"game":"g"}"#);
    refused("picture", r#"{"game":"g","entry":"e","out":"/tmp/x"}"#);
    refused("answer", r#"{"game":"g"}"#);
}

#[test]
fn values_must_be_strings_that_do_not_look_like_flags() {
    refused("status", r#"{"game":42}"#);
    refused("status", r#"{"game":""}"#);
    refused("status", r#"{"game":"-json"}"#);
    refused("status", "{\"game\":\"g\\u0000x\"}");
    refused("pull", r#"{"game":"g","anyway":"yes"}"#);
    refused("pull", r#"{"game":"g","resolve":true}"#);
}

#[test]
fn setup_signs_in_to_drive_or_dropbox_without_a_browser_of_its_own() {
    refused(
        "setup",
        r#"{"service":"nextcloud","server":"https://x.test","user":"u"}"#,
    );
    refused("setup", r#"{"service":"nextcloud","no-browser":true}"#);
    refused("setup", r#"{"service":"gdrive"}"#);
    refused("setup", r#"{"service":"gdrive","no-browser":false}"#);
    refused("setup", r#"{"no-browser":true}"#);
    refused(
        "setup",
        r#"{"service":"gdrive","no-browser":true,"server":"https://x.test"}"#,
    );
    refused("setup", r#"{"service":"other","no-browser":true}"#);
}

#[test]
fn a_body_that_is_not_a_json_object_is_refused() {
    refused("status", "[]");
    refused("status", "\"game\"");
    refused("status", "not json");
    refused("status", "{");
}

// ---- responses ------------------------------------------------------------------------------

#[test]
fn a_response_is_an_http_1_1_answer_with_length_and_no_store() {
    assert_eq!(
        response(200, "text/plain", b"hi"),
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nhi".to_vec()
    );
    let forbidden = String::from_utf8(response(403, "text/plain", b"")).unwrap();
    assert!(forbidden.starts_with("HTTP/1.1 403 Forbidden\r\n"));
    assert!(forbidden.contains("Content-Length: 0\r\n"));
}

// ---- the panel arm of the command line ------------------------------------------------------

fn command_line(line: &[&str]) -> Result<cli::Args, save_relay::Error> {
    let owned: Vec<String> = line.iter().map(|word| word.to_string()).collect();
    cli::parse(&owned)
}

#[test]
fn panel_parses_its_port_and_no_open_flags() {
    let plain = command_line(&["panel"]).unwrap();
    assert_eq!(
        plain.command,
        Command::Panel {
            port: None,
            open: true
        }
    );
    assert!(!plain.json);

    let both = command_line(&["panel", "--port", "8080", "--no-open", "--json"]).unwrap();
    assert_eq!(
        both.command,
        Command::Panel {
            port: Some(8080),
            open: false
        }
    );
    assert!(both.json);
    assert_eq!(cli::command_name(&both.command), "panel");
}

#[test]
fn panel_refuses_a_bad_port_and_stray_words() {
    for line in [
        &["panel", "--port", "0"][..],
        &["panel", "--port", "65536"],
        &["panel", "--port", "x"],
        &["panel", "extra"],
    ] {
        let error = command_line(line).expect_err(&line.join(" "));
        assert_eq!(error.code(), "usage", "{}", line.join(" "));
    }
}

// ---- the server on a real loopback socket ---------------------------------------------------

/// The fake command runner: a `--json` envelope that echoes its arguments. `picture` writes
/// `picture_bytes` to its `--out` file (an empty file for "no picture"); `status` with `game`
/// `broken` prints no envelope.
fn fake_run(picture_bytes: Vec<u8>) -> impl Fn(&[String]) -> (i32, Vec<u8>) + Send + 'static {
    move |args: &[String]| {
        if args[0] == "picture" {
            let out = args.iter().position(|word| word == "--out").expect("--out");
            let _ = fs::write(&args[out + 1], &picture_bytes);
            let code = if picture_bytes.is_empty() { 1 } else { 0 };
            return (code, b"{}".to_vec());
        }
        if args.get(1).map(String::as_str) == Some("broken") {
            return (1, b"not an envelope".to_vec());
        }
        let envelope = serde_json::json!({
            "ok": true,
            "command": args[0],
            "data": { "argv": args },
        });
        (0, envelope.to_string().into_bytes())
    }
}

/// Serve on port 0 in a thread; the announcement gives the port and the URL with the token.
/// The thread serves until the test process ends.
fn start(run: impl Fn(&[String]) -> (i32, Vec<u8>) + Send + 'static) -> PanelData {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let announce = |data: &PanelData| {
            let _ = sender.send(data.clone());
        };
        let _ = panel::serve(Some(0), false, &announce, &run);
    });
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("the panel announces its URL")
}

fn token_in(url: &str) -> String {
    url.split("token=")
        .nth(1)
        .expect("a token in the URL")
        .to_string()
}

/// Send `request` and read the answer to its end (the server closes the connection).
fn exchange(port: u16, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    stream.write_all(request).expect("send");
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).expect("read the answer");
    answer
}

/// The status line and the body of an answer.
fn split(answer: &[u8]) -> (String, Vec<u8>) {
    let end = answer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a header end");
    let status = String::from_utf8_lossy(&answer[..end])
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    (status, answer[end + 4..].to_vec())
}

fn get(port: u16, target: &str, header: Option<String>) -> (String, Vec<u8>) {
    let mut text = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    if let Some(header) = header {
        text.push_str(&header);
        text.push_str("\r\n");
    }
    text.push_str("\r\n");
    split(&exchange(port, text.as_bytes()))
}

fn post(port: u16, target: &str, header: Option<String>, body: &str) -> (String, Vec<u8>) {
    let mut text = format!(
        "POST {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(header) = header {
        text.push_str(&header);
        text.push_str("\r\n");
    }
    text.push_str("\r\n");
    text.push_str(body);
    split(&exchange(port, text.as_bytes()))
}

#[test]
fn the_server_announces_a_loopback_url_with_a_43_character_token() {
    let data = start(fake_run(Vec::new()));
    assert!(data
        .url
        .starts_with(&format!("http://127.0.0.1:{}/?token=", data.port)));
    assert_eq!(token_in(&data.url).len(), 43);
    assert_ne!(data.port, 0);
}

#[test]
fn the_page_is_served_with_the_token_and_refused_without_it() {
    let data = start(fake_run(Vec::new()));
    let token = token_in(&data.url);
    let (status, body) = get(data.port, &format!("/?token={token}"), None);
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(String::from_utf8_lossy(&body).contains('<'));
    let (status, _) = get(data.port, "/", None);
    assert_eq!(status, "HTTP/1.1 403 Forbidden");
    let wrong = format!("{TOKEN_HEADER}: {}", "x".repeat(43));
    let (status, _) = get(data.port, "/", Some(wrong));
    assert_eq!(status, "HTTP/1.1 403 Forbidden");
    let right = format!("{TOKEN_HEADER}: {token}");
    let (status, _) = get(data.port, "/", Some(right));
    assert_eq!(status, "HTTP/1.1 200 OK");
}

#[test]
fn an_api_post_runs_the_command_line_and_answers_its_envelope() {
    let data = start(fake_run(Vec::new()));
    let header = format!("{TOKEN_HEADER}: {}", token_in(&data.url));
    let (status, body) = post(data.port, "/api/status", Some(header), r#"{"game":"g"}"#);
    assert_eq!(status, "HTTP/1.1 200 OK");
    let envelope: Value = serde_json::from_slice(&body).expect("JSON");
    assert_eq!(envelope["ok"], true);
    assert_eq!(
        envelope["data"]["argv"],
        serde_json::json!(["status", "g", "--json"])
    );

    let (status, body) = post(data.port, "/api/status", None, r#"{"game":"g"}"#);
    assert_eq!(status, "HTTP/1.1 403 Forbidden");
    assert_eq!(body, b"forbidden".to_vec());
}

#[test]
fn an_api_refusal_is_a_400_failure_envelope_and_an_unknown_command_is_404() {
    let data = start(fake_run(Vec::new()));
    let header = format!("{TOKEN_HEADER}: {}", token_in(&data.url));
    let (status, body) = post(data.port, "/api/status", Some(header.clone()), "{}");
    assert_eq!(status, "HTTP/1.1 400 Bad Request");
    let envelope: Value = serde_json::from_slice(&body).expect("JSON");
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "usage");

    let (status, body) = post(data.port, "/api/picture", Some(header.clone()), "{}");
    assert_eq!(status, "HTTP/1.1 400 Bad Request");
    let envelope: Value = serde_json::from_slice(&body).expect("JSON");
    assert_eq!(envelope["error"]["code"], "usage");

    let (status, _) = post(data.port, "/api/wrap", Some(header), "{}");
    assert_eq!(status, "HTTP/1.1 404 Not Found");
}

#[test]
fn an_output_that_is_not_an_envelope_is_a_500() {
    let data = start(fake_run(Vec::new()));
    let header = format!("{TOKEN_HEADER}: {}", token_in(&data.url));
    let (status, _) = post(
        data.port,
        "/api/status",
        Some(header),
        r#"{"game":"broken"}"#,
    );
    assert_eq!(status, "HTTP/1.1 500 Internal Server Error");
}

#[test]
fn a_picture_is_served_as_its_image_type_or_not_found() {
    let data = start(fake_run(PNG.to_vec()));
    let token = token_in(&data.url);
    let (status, body) = get(
        data.port,
        &format!("/picture/steam-1/entry/card.png?token={token}"),
        None,
    );
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(body, PNG.to_vec());

    let jpeg = start(fake_run(JPEG.to_vec()));
    let (status, body) = get(
        jpeg.port,
        &format!("/picture/g/e?token={}", token_in(&jpeg.url)),
        None,
    );
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(body, JPEG.to_vec());

    let none = start(fake_run(Vec::new()));
    let (status, _) = get(
        none.port,
        &format!("/picture/g/e?token={}", token_in(&none.url)),
        None,
    );
    assert_eq!(status, "HTTP/1.1 404 Not Found");

    let (status, _) = get(data.port, "/picture/g/e", None);
    assert_eq!(status, "HTTP/1.1 403 Forbidden");
}

#[test]
fn the_picture_url_the_front_end_builds_loads_with_the_token_in_its_query() {
    // The front end's `panelPicture(token)` (panel/src/App.tsx): `/picture/<game>/<entry>/<file>`
    // with each segment percent-encoded and `?token=` for the `<img>`, which cannot send the header.
    let data = start(fake_run(PNG.to_vec()));
    let token = token_in(&data.url);
    let built = format!("/picture/steam-1/2026-11-01/.save-relay-picture.png?token={token}");
    let (status, body) = get(data.port, &built, None);
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(body, PNG.to_vec());

    let without = "/picture/steam-1/2026-11-01/.save-relay-picture.png";
    let (status, _) = get(data.port, without, None);
    assert_eq!(status, "HTTP/1.1 403 Forbidden");
}

#[test]
fn the_front_end_files_are_refused_without_the_token_over_the_socket() {
    let data = start(fake_run(Vec::new()));
    let token = token_in(&data.url);
    let (status, _) = get(data.port, "/app.js", None);
    assert_eq!(status, "HTTP/1.1 403 Forbidden");
    let wrong = format!("{TOKEN_HEADER}: {}", "x".repeat(43));
    let (status, _) = get(data.port, "/app.css", Some(wrong));
    assert_eq!(status, "HTTP/1.1 403 Forbidden");
    let right = format!("{TOKEN_HEADER}: {token}");
    let (status, _) = get(data.port, "/app.js", Some(right));
    #[cfg(feature = "panel-dist")]
    assert_eq!(status, "HTTP/1.1 200 OK");
    #[cfg(not(feature = "panel-dist"))]
    assert_eq!(status, "HTTP/1.1 404 Not Found");
}

#[cfg(feature = "panel-dist")]
#[test]
fn the_page_loads_its_front_end_files_with_the_token_it_carries() {
    let data = start(fake_run(Vec::new()));
    let token = token_in(&data.url);
    let (status, page) = get(data.port, &format!("/?token={token}"), None);
    assert_eq!(status, "HTTP/1.1 200 OK");
    let page = String::from_utf8_lossy(&page).into_owned();
    for file in ["app.js", "app.css"] {
        assert!(page.contains(&format!("./{file}?token={token}")), "{file}");
        let (status, body) = get(data.port, &format!("/{file}?token={token}"), None);
        assert_eq!(status, "HTTP/1.1 200 OK", "{file}");
        assert!(!body.is_empty(), "{file}");
    }
}

#[test]
fn a_malformed_request_is_a_400_and_the_next_connection_is_still_served() {
    let data = start(fake_run(Vec::new()));
    let (status, _) = split(&exchange(data.port, b"GET / HTTP/1.0\r\n\r\n"));
    assert_eq!(status, "HTTP/1.1 400 Bad Request");
    let (status, _) = get(data.port, &format!("/?token={}", token_in(&data.url)), None);
    assert_eq!(status, "HTTP/1.1 200 OK");
}

#[test]
fn a_port_that_is_taken_is_an_error_and_nothing_is_announced() {
    let held = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let port = held.local_addr().expect("address").port();
    let announce = |_: &PanelData| panic!("nothing is announced when the port is taken");
    let run = fake_run(Vec::new());
    let error = panel::serve(Some(port), false, &announce, &run).expect_err("port in use");
    assert_eq!(error.code(), "io");
}
