use save_relay::dropbox::{header_json, API};
use save_relay::fake::{FakeDropbox, REFRESH_TOKEN};
use save_relay::http::{Request, Response, Transport};
use save_relay::Error;
use serde_json::{json, Value};

const TOKEN: &str = "https://api.dropboxapi.com/oauth2/token";
const UPLOAD: &str = "https://content.dropboxapi.com/2/files/upload";
const DOWNLOAD: &str = "https://content.dropboxapi.com/2/files/download";

fn ok(fake: &FakeDropbox, request: Request) -> Response {
    fake.send(&request).expect("the request reaches the fake")
}

fn json_body(response: &Response) -> Value {
    serde_json::from_slice(&response.body).expect("a JSON answer")
}

fn token_request() -> Request {
    Request::new("POST", TOKEN)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!("grant_type=refresh_token&refresh_token={REFRESH_TOKEN}").into_bytes())
}

fn token(fake: &FakeDropbox) -> String {
    let response = ok(fake, token_request());
    assert_eq!(response.status, 200, "{}", response.text());
    let body = json_body(&response);
    assert_eq!(body["token_type"], "bearer");
    assert_eq!(body["expires_in"], 14400);
    body["access_token"]
        .as_str()
        .expect("an access token")
        .to_string()
}

fn rpc(fake: &FakeDropbox, access: &str, endpoint: &str, body: Value) -> Response {
    ok(
        fake,
        Request::new("POST", &format!("{API}/{endpoint}"))
            .header("Authorization", &format!("Bearer {access}"))
            .body(body.to_string().into_bytes()),
    )
}

fn upload(fake: &FakeDropbox, access: &str, path: &str, bytes: &[u8], mode: Value) -> Response {
    let arg = json!({
        "path": path,
        "mode": mode,
        "autorename": false,
        "mute": true,
        "strict_conflict": true,
    });
    ok(
        fake,
        Request::new("POST", UPLOAD)
            .header("Authorization", &format!("Bearer {access}"))
            .header("Dropbox-API-Arg", &header_json(&arg))
            .body(bytes.to_vec()),
    )
}

fn download(fake: &FakeDropbox, access: &str, path: &str) -> Response {
    let arg = header_json(&json!({"path": path}));
    ok(
        fake,
        Request::new("POST", DOWNLOAD)
            .header("Authorization", &format!("Bearer {access}"))
            .header("Dropbox-API-Arg", &arg),
    )
}

fn rev(response: &Response) -> String {
    json_body(response)["rev"]
        .as_str()
        .expect("a rev")
        .to_string()
}

fn summary(response: &Response) -> String {
    json_body(response)["error_summary"]
        .as_str()
        .expect("an error summary")
        .to_string()
}

fn names(response: &Response) -> Vec<String> {
    let body = json_body(response);
    body["entries"]
        .as_array()
        .expect("a page of entries")
        .iter()
        .map(|entry| entry["name"].as_str().expect("a name").to_string())
        .collect()
}

#[test]
fn the_token_endpoint_numbers_access_tokens() {
    let fake = FakeDropbox::new();
    assert_eq!(token(&fake), "fake-access-1");
    assert_eq!(token(&fake), "fake-access-2");

    let wrong = ok(
        &fake,
        Request::new("POST", TOKEN).body(b"grant_type=refresh_token&refresh_token=nope".to_vec()),
    );
    assert_eq!(wrong.status, 400);
    assert_eq!(json_body(&wrong)["error"], "invalid_grant");

    let unsupported = ok(
        &fake,
        Request::new("POST", TOKEN).body(b"grant_type=code&refresh_token=x".to_vec()),
    );
    assert_eq!(unsupported.status, 400);
    assert_eq!(json_body(&unsupported)["error"], "unsupported_grant_type");
}

#[test]
fn requests_show_method_and_url_but_never_a_token() {
    let fake = FakeDropbox::new();
    let access = token(&fake);
    let response = rpc(&fake, &access, "files/list_folder", json!({"path": ""}));
    assert_eq!(response.status, 200);

    let requests = fake.requests();
    assert_eq!(
        requests,
        vec![
            format!("POST {TOKEN}"),
            format!("POST {API}/files/list_folder"),
        ]
    );
    for line in requests {
        assert!(!line.contains(&access), "{line}");
        assert!(!line.contains(REFRESH_TOKEN), "{line}");
        assert!(!line.contains("Bearer"), "{line}");
    }
}

#[test]
fn uploads_create_missing_parents_and_every_write_has_a_new_rev() {
    let fake = FakeDropbox::new();
    let access = token(&fake);

    let first = upload(
        &fake,
        &access,
        "/games/star/state.json",
        b"one",
        json!("add"),
    );
    assert_eq!(first.status, 200, "{}", first.text());
    let metadata = json_body(&first);
    assert_eq!(metadata[".tag"], "file");
    assert_eq!(metadata["name"], "state.json");
    assert_eq!(metadata["path_display"], "/games/star/state.json");
    assert_eq!(metadata["size"], 3);
    let first_rev = rev(&first);
    assert_eq!(fake.file_paths(), vec!["games/star/state.json"]);

    let root = rpc(&fake, &access, "files/list_folder", json!({"path": ""}));
    let entries = json_body(&root)["entries"].clone();
    assert_eq!(entries[0][".tag"], "folder");
    assert_eq!(entries[0]["name"], "games");
    let games = rpc(
        &fake,
        &access,
        "files/list_folder",
        json!({"path": "/games"}),
    );
    let entries = json_body(&games)["entries"].clone();
    assert_eq!(entries[0]["name"], "star");
    assert_eq!(entries[0][".tag"], "folder");

    let second = upload(
        &fake,
        &access,
        "/games/star/state.json",
        b"two",
        json!("overwrite"),
    );
    assert_eq!(second.status, 200);
    let second_rev = rev(&second);
    assert_ne!(first_rev, second_rev);
    assert_eq!(fake.file_paths(), vec!["games/star/state.json"]);
}

#[test]
fn update_mode_checks_the_rev() {
    let fake = FakeDropbox::new();
    let access = token(&fake);
    let created = upload(&fake, &access, "/g/state.json", b"one", json!("add"));
    let first_rev = rev(&created);

    let updated = upload(
        &fake,
        &access,
        "/g/state.json",
        b"two",
        json!({".tag": "update", "update": first_rev}),
    );
    assert_eq!(updated.status, 200, "{}", updated.text());
    let second_rev = rev(&updated);
    assert_ne!(first_rev, second_rev);

    let stale = upload(
        &fake,
        &access,
        "/g/state.json",
        b"three",
        json!({".tag": "update", "update": first_rev}),
    );
    assert_eq!(stale.status, 409);
    assert_eq!(summary(&stale), "path/conflict/file/..");
    assert_eq!(json_body(&stale)["error"][".tag"], "path");
    assert_eq!(download(&fake, &access, "/g/state.json").body, b"two");

    let missing = upload(
        &fake,
        &access,
        "/g/missing.json",
        b"x",
        json!({".tag": "update", "update": second_rev}),
    );
    assert_eq!(missing.status, 409);
    assert_eq!(summary(&missing), "path/not_found/..");
}

#[test]
fn add_mode_refuses_an_existing_file() {
    let fake = FakeDropbox::new();
    let access = token(&fake);
    let first = upload(&fake, &access, "/g/state.json", b"one", json!("add"));
    assert_eq!(first.status, 200);

    let again = upload(&fake, &access, "/g/state.json", b"two", json!("add"));
    assert_eq!(again.status, 409);
    assert!(
        summary(&again).starts_with("path/conflict"),
        "{}",
        summary(&again)
    );
    assert_eq!(json_body(&again)["error"][".tag"], "path");
    assert_eq!(download(&fake, &access, "/g/state.json").body, b"one");
}

#[test]
fn list_folder_pages_two_at_a_time_and_continues() {
    let fake = FakeDropbox::new();
    let access = token(&fake);
    let made = rpc(
        &fake,
        &access,
        "files/create_folder_v2",
        json!({"path": "/g"}),
    );
    assert_eq!(made.status, 200);
    for name in ["a", "b", "c", "d", "e"] {
        let path = format!("/g/{name}");
        let response = upload(&fake, &access, &path, name.as_bytes(), json!("add"));
        assert_eq!(response.status, 200);
    }

    let first = rpc(&fake, &access, "files/list_folder", json!({"path": "/g"}));
    assert_eq!(names(&first), vec!["a", "b"]);
    assert_eq!(json_body(&first)["has_more"], true);
    let cursor = json_body(&first)["cursor"]
        .as_str()
        .expect("a cursor")
        .to_string();

    let second = rpc(
        &fake,
        &access,
        "files/list_folder/continue",
        json!({"cursor": cursor}),
    );
    assert_eq!(names(&second), vec!["c", "d"]);
    assert_eq!(json_body(&second)["has_more"], true);
    let cursor = json_body(&second)["cursor"]
        .as_str()
        .expect("a cursor")
        .to_string();

    let third = rpc(
        &fake,
        &access,
        "files/list_folder/continue",
        json!({"cursor": cursor}),
    );
    assert_eq!(names(&third), vec!["e"]);
    assert_eq!(json_body(&third)["has_more"], false);

    let file = rpc(&fake, &access, "files/list_folder", json!({"path": "/g/a"}));
    assert_eq!(file.status, 409);
    assert_eq!(summary(&file), "path/not_folder/..");
    let missing = rpc(
        &fake,
        &access,
        "files/list_folder",
        json!({"path": "/nope"}),
    );
    assert_eq!(missing.status, 409);
    assert_eq!(summary(&missing), "path/not_found/..");
}

#[test]
fn get_metadata_answers_files_folders_and_missing_paths() {
    let fake = FakeDropbox::new();
    let access = token(&fake);
    let created = upload(&fake, &access, "/g/a.sav", b"save", json!("add"));
    let file_rev = rev(&created);
    let made = rpc(
        &fake,
        &access,
        "files/create_folder_v2",
        json!({"path": "/g/sub"}),
    );
    assert_eq!(made.status, 200);

    let file = rpc(
        &fake,
        &access,
        "files/get_metadata",
        json!({"path": "/g/a.sav"}),
    );
    assert_eq!(file.status, 200);
    assert_eq!(json_body(&file)[".tag"], "file");
    assert_eq!(json_body(&file)["rev"].as_str(), Some(file_rev.as_str()));

    let folder = rpc(
        &fake,
        &access,
        "files/get_metadata",
        json!({"path": "/g/sub"}),
    );
    assert_eq!(folder.status, 200);
    assert_eq!(json_body(&folder)[".tag"], "folder");
    assert_eq!(json_body(&folder)["name"], "sub");

    let missing = rpc(
        &fake,
        &access,
        "files/get_metadata",
        json!({"path": "/g/nope"}),
    );
    assert_eq!(missing.status, 409);
    assert_eq!(summary(&missing), "path/not_found/..");
}

#[test]
fn download_sends_bytes_and_the_result_header() {
    let fake = FakeDropbox::new();
    let access = token(&fake);
    let created = upload(&fake, &access, "/g/a.sav", b"save bytes", json!("add"));
    let file_rev = rev(&created);
    let made = rpc(
        &fake,
        &access,
        "files/create_folder_v2",
        json!({"path": "/g/dir"}),
    );
    assert_eq!(made.status, 200);

    let got = download(&fake, &access, "/g/a.sav");
    assert_eq!(got.status, 200);
    assert_eq!(got.body, b"save bytes");
    let header = got
        .header_value("Dropbox-API-Result")
        .expect("the result header");
    assert!(header.is_ascii(), "{header}");
    let metadata: Value = serde_json::from_str(header).expect("JSON in the header");
    assert_eq!(metadata[".tag"], "file");
    assert_eq!(metadata["rev"].as_str(), Some(file_rev.as_str()));
    assert_eq!(metadata["size"], 10);

    let missing = download(&fake, &access, "/g/nope");
    assert_eq!(missing.status, 409);
    assert_eq!(summary(&missing), "path/not_found/..");
    let folder = download(&fake, &access, "/g/dir");
    assert_eq!(folder.status, 409);
    assert_eq!(summary(&folder), "path/not_file/..");
}

#[test]
fn folders_are_created_copied_and_moved_whole() {
    let fake = FakeDropbox::new();
    let access = token(&fake);

    let made = rpc(
        &fake,
        &access,
        "files/create_folder_v2",
        json!({"path": "/g"}),
    );
    assert_eq!(made.status, 200);
    assert_eq!(json_body(&made)["metadata"][".tag"], "folder");
    assert_eq!(json_body(&made)["metadata"]["name"], "g");

    let again = rpc(
        &fake,
        &access,
        "files/create_folder_v2",
        json!({"path": "/g"}),
    );
    assert_eq!(again.status, 409);
    assert_eq!(summary(&again), "path/conflict/folder/..");

    let created = upload(&fake, &access, "/g/state.json", b"state", json!("add"));
    let original_rev = rev(&created);

    let copied = rpc(
        &fake,
        &access,
        "files/copy_v2",
        json!({"from_path": "/g/state.json", "to_path": "/g/state.copy.json"}),
    );
    assert_eq!(copied.status, 200);
    assert_eq!(json_body(&copied)["metadata"][".tag"], "file");
    assert_ne!(
        json_body(&copied)["metadata"]["rev"].as_str(),
        Some(original_rev.as_str())
    );

    let folder = rpc(
        &fake,
        &access,
        "files/copy_v2",
        json!({"from_path": "/g", "to_path": "/h"}),
    );
    assert_eq!(folder.status, 200);
    assert_eq!(json_body(&folder)["metadata"][".tag"], "folder");
    assert_eq!(
        fake.file_paths(),
        vec![
            "g/state.copy.json",
            "g/state.json",
            "h/state.copy.json",
            "h/state.json",
        ]
    );

    let exists = rpc(
        &fake,
        &access,
        "files/copy_v2",
        json!({"from_path": "/g", "to_path": "/h"}),
    );
    assert_eq!(exists.status, 409);
    assert!(
        summary(&exists).starts_with("to/conflict"),
        "{}",
        summary(&exists)
    );

    let moved = rpc(
        &fake,
        &access,
        "files/move_v2",
        json!({"from_path": "/h", "to_path": "/m"}),
    );
    assert_eq!(moved.status, 200);
    assert_eq!(json_body(&moved)["metadata"][".tag"], "folder");
    assert_eq!(
        fake.file_paths(),
        vec![
            "g/state.copy.json",
            "g/state.json",
            "m/state.copy.json",
            "m/state.json",
        ]
    );

    let missing_copy = rpc(
        &fake,
        &access,
        "files/copy_v2",
        json!({"from_path": "/nope", "to_path": "/x"}),
    );
    assert_eq!(missing_copy.status, 409);
    assert_eq!(summary(&missing_copy), "from_lookup/not_found/..");

    let missing_move = rpc(
        &fake,
        &access,
        "files/move_v2",
        json!({"from_path": "/nope", "to_path": "/x"}),
    );
    assert_eq!(missing_move.status, 409);
    assert_eq!(summary(&missing_move), "from_lookup/not_found/..");
}

#[test]
fn delete_removes_a_file_or_a_whole_folder() {
    let fake = FakeDropbox::new();
    let access = token(&fake);
    upload(&fake, &access, "/g/a.sav", b"a", json!("add"));
    upload(&fake, &access, "/g/sub/b.sav", b"b", json!("add"));
    assert_eq!(fake.file_paths(), vec!["g/a.sav", "g/sub/b.sav"]);

    let file = rpc(
        &fake,
        &access,
        "files/delete_v2",
        json!({"path": "/g/a.sav"}),
    );
    assert_eq!(file.status, 200);
    assert_eq!(json_body(&file)["metadata"][".tag"], "file");
    assert_eq!(fake.file_paths(), vec!["g/sub/b.sav"]);

    let folder = rpc(&fake, &access, "files/delete_v2", json!({"path": "/g"}));
    assert_eq!(folder.status, 200);
    assert_eq!(json_body(&folder)["metadata"][".tag"], "folder");
    assert!(fake.file_paths().is_empty());

    let missing = rpc(&fake, &access, "files/delete_v2", json!({"path": "/g"}));
    assert_eq!(missing.status, 409);
    assert_eq!(summary(&missing), "path_lookup/not_found/..");
}

#[test]
fn current_account_has_an_email() {
    let fake = FakeDropbox::new();
    let access = token(&fake);
    let account = rpc(&fake, &access, "users/get_current_account", json!(null));
    assert_eq!(account.status, 200);
    assert_eq!(json_body(&account)["email"], "fake@example.com");
    assert_eq!(json_body(&account)["name"]["display_name"], "Fake User");
}

#[test]
fn expired_and_revoked_tokens_are_refused() {
    let fake = FakeDropbox::new();
    let first = token(&fake);

    let none = ok(
        &fake,
        Request::new("POST", &format!("{API}/files/list_folder"))
            .body(json!({"path": ""}).to_string().into_bytes()),
    );
    assert_eq!(none.status, 401);
    assert_eq!(summary(&none), "invalid_access_token/");

    let wrong = rpc(
        &fake,
        "fake-access-999",
        "files/list_folder",
        json!({"path": ""}),
    );
    assert_eq!(wrong.status, 401);
    assert_eq!(summary(&wrong), "invalid_access_token/");

    fake.expire_access_tokens();
    let expired = rpc(&fake, &first, "files/list_folder", json!({"path": ""}));
    assert_eq!(expired.status, 401);
    assert_eq!(summary(&expired), "expired_access_token/");

    let second = token(&fake);
    assert_eq!(second, "fake-access-2");
    let fresh = rpc(&fake, &second, "files/list_folder", json!({"path": ""}));
    assert_eq!(fresh.status, 200);

    fake.revoke();
    let refresh = ok(&fake, token_request());
    assert_eq!(refresh.status, 400);
    assert_eq!(json_body(&refresh)["error"], "invalid_grant");
    let refused = rpc(&fake, &second, "files/list_folder", json!({"path": ""}));
    assert_eq!(refused.status, 401);
}

#[test]
fn offline_is_unreachable_but_file_paths_still_work() {
    let fake = FakeDropbox::new();
    let access = token(&fake);
    upload(&fake, &access, "/g/a.sav", b"a", json!("add"));
    fake.set_reachable(false);

    let error = fake
        .send(
            &Request::new("POST", &format!("{API}/files/list_folder"))
                .header("Authorization", &format!("Bearer {access}"))
                .body(json!({"path": ""}).to_string().into_bytes()),
        )
        .expect_err("offline");
    assert_eq!(error, Error::Unreachable("fake dropbox is offline".into()));
    assert_eq!(fake.file_paths(), vec!["g/a.sav"]);

    fake.set_reachable(true);
    let response = rpc(&fake, &access, "files/list_folder", json!({"path": ""}));
    assert_eq!(response.status, 200);
}

#[test]
fn non_ascii_paths_work_through_the_header_argument() {
    let fake = FakeDropbox::new();
    let access = token(&fake);
    let path = "/é/save 😀.sav";
    let created = upload(&fake, &access, path, b"x", json!("add"));
    assert_eq!(created.status, 200, "{}", created.text());
    assert_eq!(fake.file_paths(), vec!["é/save 😀.sav"]);

    let got = download(&fake, &access, path);
    assert_eq!(got.status, 200);
    assert_eq!(got.body, b"x");
    let header = got
        .header_value("Dropbox-API-Result")
        .expect("the result header");
    assert!(header.is_ascii(), "{header}");
    let metadata: Value = serde_json::from_str(header).expect("JSON in the header");
    assert_eq!(metadata["path_display"], path);
}
