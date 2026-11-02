use save_relay::Error;
use save_relay::fake::{FakeDrive, REFRESH_TOKEN};
use save_relay::gdrive::{API, FOLDER_MIME, UPLOAD};
use save_relay::http::{Request, Response, Transport};
use save_relay::oauth::GOOGLE;
use serde_json::{Value, json};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

fn ok(fake: &FakeDrive, request: Request) -> Response {
    fake.send(&request).expect("the request reaches the fake")
}

fn json_body(response: &Response) -> Value {
    serde_json::from_slice(&response.body).expect("a JSON answer")
}

fn token_request() -> Request {
    Request::new("POST", GOOGLE.token_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!("grant_type=refresh_token&refresh_token={REFRESH_TOKEN}").into_bytes())
}

fn token(fake: &FakeDrive) -> String {
    let response = ok(fake, token_request());
    assert_eq!(response.status, 200, "{}", response.text());
    let body = json_body(&response);
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 3600);
    body["access_token"]
        .as_str()
        .expect("an access token")
        .to_string()
}

fn with_auth(request: Request, access: &str) -> Request {
    request.header("Authorization", &format!("Bearer {access}"))
}

fn multipart(metadata: &Value, bytes: &[u8]) -> (String, Vec<u8>) {
    let boundary = "fake-drive-test-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(metadata.to_string().as_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--").as_bytes());
    (format!("multipart/related; boundary={boundary}"), body)
}

fn create_json(fake: &FakeDrive, access: &str, metadata: Value) -> Response {
    ok(
        fake,
        with_auth(
            Request::new("POST", &format!("{API}/files"))
                .header("Content-Type", "application/json")
                .body(metadata.to_string().into_bytes()),
            access,
        ),
    )
}

fn create_multipart(fake: &FakeDrive, access: &str, metadata: Value, bytes: &[u8]) -> Response {
    let (content_type, body) = multipart(&metadata, bytes);
    ok(
        fake,
        with_auth(
            Request::new("POST", &format!("{UPLOAD}/files?uploadType=multipart"))
                .header("Content-Type", &content_type)
                .body(body),
            access,
        ),
    )
}

fn update_json_at(fake: &FakeDrive, access: &str, url: &str, metadata: Value) -> Response {
    ok(
        fake,
        with_auth(
            Request::new("PATCH", url)
                .header("Content-Type", "application/json")
                .body(metadata.to_string().into_bytes()),
            access,
        ),
    )
}

fn update_json(fake: &FakeDrive, access: &str, id: &str, metadata: Value) -> Response {
    update_json_at(fake, access, &format!("{API}/files/{id}"), metadata)
}

fn update_multipart(
    fake: &FakeDrive,
    access: &str,
    id: &str,
    metadata: Value,
    bytes: &[u8],
) -> Response {
    let (content_type, body) = multipart(&metadata, bytes);
    ok(
        fake,
        with_auth(
            Request::new(
                "PATCH",
                &format!("{UPLOAD}/files/{id}?uploadType=multipart"),
            )
            .header("Content-Type", &content_type)
            .body(body),
            access,
        ),
    )
}

fn get_meta(fake: &FakeDrive, access: &str, id: &str) -> Response {
    ok(
        fake,
        with_auth(Request::new("GET", &format!("{API}/files/{id}")), access),
    )
}

fn get_media(fake: &FakeDrive, access: &str, id: &str) -> Response {
    ok(
        fake,
        with_auth(
            Request::new("GET", &format!("{API}/files/{id}?alt=media")),
            access,
        ),
    )
}

fn list_at(fake: &FakeDrive, access: &str, url: &str) -> Response {
    ok(fake, with_auth(Request::new("GET", url), access))
}

fn list(fake: &FakeDrive, access: &str, q: &str) -> Response {
    list_at(fake, access, &format!("{API}/files?q={q}"))
}

fn list_page(fake: &FakeDrive, access: &str, q: &str, page: &str) -> Response {
    list_at(fake, access, &format!("{API}/files?q={q}&pageToken={page}"))
}

fn generate_ids(fake: &FakeDrive, access: &str, count: usize) -> Response {
    list_at(
        fake,
        access,
        &format!("{API}/files/generateIds?count={count}"),
    )
}

fn copy_file(fake: &FakeDrive, access: &str, id: &str, metadata: Value) -> Response {
    ok(
        fake,
        with_auth(
            Request::new("POST", &format!("{API}/files/{id}/copy"))
                .header("Content-Type", "application/json")
                .body(metadata.to_string().into_bytes()),
            access,
        ),
    )
}

fn delete_file(fake: &FakeDrive, access: &str, id: &str) -> Response {
    ok(
        fake,
        with_auth(Request::new("DELETE", &format!("{API}/files/{id}")), access),
    )
}

fn about(fake: &FakeDrive, access: &str) -> Response {
    list_at(fake, access, &format!("{API}/about?fields=user"))
}

fn names(response: &Response) -> Vec<String> {
    let body = json_body(response);
    body["files"]
        .as_array()
        .expect("a page of files")
        .iter()
        .map(|file| file["name"].as_str().expect("a name").to_string())
        .collect()
}

fn id_of(response: &Response) -> String {
    json_body(response)["id"]
        .as_str()
        .expect("an id")
        .to_string()
}

fn version_of(response: &Response) -> u64 {
    let body = json_body(response);
    body["version"]
        .as_str()
        .expect("a version string")
        .parse()
        .expect("a version number")
}

#[test]
fn the_token_endpoint_numbers_access_tokens_and_checks_the_refresh_token() {
    let fake = FakeDrive::new();
    assert_eq!(token(&fake), "fake-access-1");
    assert_eq!(token(&fake), "fake-access-2");

    let wrong = ok(
        &fake,
        Request::new("POST", GOOGLE.token_url)
            .body(b"grant_type=refresh_token&refresh_token=nope".to_vec()),
    );
    assert_eq!(wrong.status, 400);
    assert_eq!(json_body(&wrong)["error"], "invalid_grant");
    assert!(wrong.header_value("Date").is_some());

    let unsupported = ok(
        &fake,
        Request::new("POST", GOOGLE.token_url)
            .body(b"grant_type=authorization_code&refresh_token=x".to_vec()),
    );
    assert_eq!(unsupported.status, 400);
    assert_eq!(json_body(&unsupported)["error"], "unsupported_grant_type");
}

#[test]
fn about_reports_the_account_and_a_date() {
    let fake = FakeDrive::new();
    let access = token(&fake);
    let response = about(&fake, &access);
    assert_eq!(response.status, 200);
    assert_eq!(
        json_body(&response)["user"]["emailAddress"],
        "fake@example.com"
    );
    assert_eq!(
        response.header_value("Date"),
        Some("Sat, 31 Oct 2026 19:51:00 GMT")
    );
}

#[test]
fn requests_show_method_and_url_but_never_a_token() {
    let fake = FakeDrive::new();
    let access = token(&fake);
    let response = list(&fake, &access, "'root' in parents and trashed = false");
    assert_eq!(response.status, 200);

    let requests = fake.requests();
    assert_eq!(requests[0], format!("POST {}", GOOGLE.token_url));
    assert!(
        requests[1].starts_with(&format!("GET {API}/files?q=")),
        "{requests:?}"
    );
    for line in requests {
        assert!(!line.contains(&access), "{line}");
        assert!(!line.contains(REFRESH_TOKEN), "{line}");
        assert!(!line.contains("Bearer"), "{line}");
    }
}

#[test]
fn a_generated_id_works_once_and_then_conflicts() {
    let fake = FakeDrive::new();
    let access = token(&fake);
    let generated = generate_ids(&fake, &access, 1);
    assert_eq!(generated.status, 200);
    let ids = json_body(&generated)["ids"]
        .as_array()
        .expect("generated ids")
        .clone();
    assert_eq!(ids.len(), 1);
    let id = ids[0].as_str().expect("an id").to_string();

    let created = create_json(
        &fake,
        &access,
        json!({"id": id.clone(), "name": "claimed.json", "parents": ["root"]}),
    );
    assert_eq!(created.status, 201, "{}", created.text());
    assert_eq!(id_of(&created), id);
    assert_eq!(json_body(&created)["version"], "1");
    assert_eq!(json_body(&created)["parents"], json!(["root"]));

    let again = create_json(
        &fake,
        &access,
        json!({"id": id.clone(), "name": "second.json", "parents": ["root"]}),
    );
    assert_eq!(again.status, 409);
    assert!(again.text().contains(&id), "{}", again.text());
    assert_eq!(fake.file_paths(), vec!["claimed.json"]);
}

#[test]
fn ids_parents_and_repeated_names_are_kept() {
    let fake = FakeDrive::new();
    let access = token(&fake);
    let folder = create_json(
        &fake,
        &access,
        json!({"name": "save-relay", "mimeType": FOLDER_MIME, "parents": ["root"]}),
    );
    assert_eq!(folder.status, 201);
    let folder_id = id_of(&folder);

    let first = create_json(
        &fake,
        &access,
        json!({
            "name": "state.json",
            "parents": [folder_id.clone()],
            "appProperties": {"save_relay_claim": "one"},
        }),
    );
    let second = create_json(
        &fake,
        &access,
        json!({
            "name": "state.json",
            "parents": [folder_id.clone()],
            "appProperties": {"save_relay_claim": "two"},
        }),
    );
    assert_eq!(first.status, 201);
    assert_eq!(second.status, 201);
    assert_ne!(id_of(&first), id_of(&second));
    assert_eq!(json_body(&first)["name"], "state.json");
    assert_eq!(json_body(&first)["parents"], json!([folder_id.clone()]));
    assert_eq!(
        json_body(&first)["appProperties"]["save_relay_claim"],
        "one"
    );

    let duplicate_folder = create_json(
        &fake,
        &access,
        json!({"name": "save-relay", "mimeType": FOLDER_MIME, "parents": ["root"]}),
    );
    assert_eq!(duplicate_folder.status, 201);
    assert_ne!(id_of(&duplicate_folder), folder_id);

    let root = list(&fake, &access, "'root' in parents and trashed = false");
    assert_eq!(names(&root), vec!["save-relay", "save-relay"]);

    let q = format!("'{folder_id}' in parents and trashed = false and name = 'state.json'");
    let same_name = list(&fake, &access, &q);
    assert_eq!(names(&same_name), vec!["state.json", "state.json"]);
    let files = json_body(&same_name)["files"]
        .as_array()
        .expect("the two files")
        .clone();
    assert_ne!(files[0]["id"], files[1]["id"]);
    assert_eq!(files[0]["parents"], json!([folder_id]));

    assert_eq!(
        fake.file_paths(),
        vec!["save-relay/state.json", "save-relay/state.json"]
    );
}

#[test]
fn list_follows_both_children_query_forms_and_pages_by_two() {
    let fake = FakeDrive::new();
    let access = token(&fake);
    let folder = create_json(
        &fake,
        &access,
        json!({"name": "g", "mimeType": FOLDER_MIME, "parents": ["root"]}),
    );
    let folder_id = id_of(&folder);
    for name in ["a", "b", "c", "d", "e"] {
        let response = create_json(
            &fake,
            &access,
            json!({"name": name, "parents": [folder_id.clone()]}),
        );
        assert_eq!(response.status, 201, "{}", response.text());
    }

    let q = format!("'{folder_id}' in parents and trashed = false");
    let first = list(&fake, &access, &q);
    assert_eq!(names(&first), vec!["a", "b"]);
    let first_token = json_body(&first)["nextPageToken"]
        .as_str()
        .expect("a next page token")
        .to_string();

    let second = list_page(&fake, &access, &q, &first_token);
    assert_eq!(names(&second), vec!["c", "d"]);
    let second_token = json_body(&second)["nextPageToken"]
        .as_str()
        .expect("a next page token")
        .to_string();

    let third = list_page(&fake, &access, &q, &second_token);
    assert_eq!(names(&third), vec!["e"]);
    assert!(json_body(&third).get("nextPageToken").is_none());

    let by_name = list(
        &fake,
        &access,
        &format!("'{folder_id}' in parents and trashed = false and name = 'c'"),
    );
    assert_eq!(names(&by_name), vec!["c"]);
    assert!(json_body(&by_name).get("nextPageToken").is_none());

    let quoted = create_json(
        &fake,
        &access,
        json!({"name": "it's", "parents": [folder_id.clone()]}),
    );
    assert_eq!(quoted.status, 201);
    let apostrophe = list(
        &fake,
        &access,
        &format!("'{folder_id}' in parents and trashed = false and name = 'it\\'s'"),
    );
    assert_eq!(names(&apostrophe), vec!["it's"]);

    let unknown = list(&fake, &access, "name contains 'x'");
    assert_eq!(unknown.status, 400);
    assert!(
        unknown.text().contains("name contains 'x'"),
        "{}",
        unknown.text()
    );
}

#[test]
fn get_answers_metadata_and_alt_media() {
    let fake = FakeDrive::new();
    let access = token(&fake);
    let created = create_multipart(
        &fake,
        &access,
        json!({
            "name": "a.sav",
            "parents": ["root"],
            "appProperties": {"save_relay_claim": "c1"},
        }),
        b"save bytes",
    );
    assert_eq!(created.status, 201, "{}", created.text());
    let id = id_of(&created);

    let metadata = get_meta(&fake, &access, &id);
    assert_eq!(metadata.status, 200);
    let body = json_body(&metadata);
    assert_eq!(body["kind"], "drive#file");
    assert_eq!(body["name"], "a.sav");
    assert_eq!(body["size"], "10");
    assert_eq!(body["version"], "1");
    assert_eq!(body["parents"], json!(["root"]));
    assert_eq!(body["createdTime"], "2026-10-31T19:51:00Z");
    assert_eq!(body["appProperties"]["save_relay_claim"], "c1");

    let media = get_media(&fake, &access, &id);
    assert_eq!(media.status, 200);
    assert_eq!(media.body, b"save bytes");

    let missing = get_meta(&fake, &access, "not-here");
    assert_eq!(missing.status, 404);
    assert!(missing.text().contains("not-here"));

    let folder = create_json(
        &fake,
        &access,
        json!({"name": "folder", "mimeType": FOLDER_MIME, "parents": ["root"]}),
    );
    let folder_media = get_media(&fake, &access, &id_of(&folder));
    assert_eq!(folder_media.status, 400);
}

#[test]
fn version_grows_on_content_name_parents_and_app_properties() {
    let fake = FakeDrive::new();
    let access = token(&fake);
    let g = id_of(&create_json(
        &fake,
        &access,
        json!({"name": "g", "mimeType": FOLDER_MIME, "parents": ["root"]}),
    ));
    let h = id_of(&create_json(
        &fake,
        &access,
        json!({"name": "h", "mimeType": FOLDER_MIME, "parents": ["root"]}),
    ));
    let created = create_multipart(
        &fake,
        &access,
        json!({"name": "a.sav", "parents": [g.clone()]}),
        b"one",
    );
    assert_eq!(created.status, 201);
    let id = id_of(&created);
    let v1 = version_of(&created);

    let content = update_multipart(&fake, &access, &id, json!({}), b"two");
    assert_eq!(content.status, 200, "{}", content.text());
    let v2 = version_of(&content);
    assert!(v2 > v1, "{v2} <= {v1}");
    assert_eq!(get_media(&fake, &access, &id).body, b"two");

    let named = update_json(&fake, &access, &id, json!({"name": "b.sav"}));
    assert_eq!(named.status, 200);
    let v3 = version_of(&named);
    assert!(v3 > v2, "{v3} <= {v2}");
    assert_eq!(json_body(&named)["name"], "b.sav");

    let moved = update_json_at(
        &fake,
        &access,
        &format!("{API}/files/{id}?addParents={h}&removeParents={g}"),
        json!({}),
    );
    assert_eq!(moved.status, 200, "{}", moved.text());
    let v4 = version_of(&moved);
    assert!(v4 > v3, "{v4} <= {v3}");
    assert_eq!(json_body(&moved)["parents"], json!([h.clone()]));
    assert_eq!(
        names(&list(
            &fake,
            &access,
            &format!("'{h}' in parents and trashed = false")
        )),
        vec!["b.sav"]
    );
    assert!(
        names(&list(
            &fake,
            &access,
            &format!("'{g}' in parents and trashed = false")
        ))
        .is_empty()
    );

    let properties = update_json(
        &fake,
        &access,
        &id,
        json!({"appProperties": {"save_relay_claim": "c9", "extra": "x"}}),
    );
    assert_eq!(properties.status, 200);
    let v5 = version_of(&properties);
    assert!(v5 > v4, "{v5} <= {v4}");
    assert_eq!(
        json_body(&properties)["appProperties"]["save_relay_claim"],
        "c9"
    );
    assert_eq!(json_body(&properties)["appProperties"]["extra"], "x");

    let reparented = update_json(&fake, &access, &id, json!({"parents": [g.clone()]}));
    assert_eq!(reparented.status, 200);
    let v6 = version_of(&reparented);
    assert!(v6 > v5, "{v6} <= {v5}");
    assert_eq!(json_body(&reparented)["parents"], json!([g]));
}

#[test]
fn created_time_and_the_date_header_follow_advance() {
    let fake = FakeDrive::new();
    let access = token(&fake);
    let first = create_json(
        &fake,
        &access,
        json!({"name": "first.json", "parents": ["root"]}),
    );
    assert_eq!(json_body(&first)["createdTime"], "2026-10-31T19:51:00Z");
    assert_eq!(
        first.header_value("Date"),
        Some("Sat, 31 Oct 2026 19:51:00 GMT")
    );

    fake.advance(121);
    let second = create_json(
        &fake,
        &access,
        json!({"name": "second.json", "parents": ["root"]}),
    );
    assert_eq!(json_body(&second)["createdTime"], "2026-10-31T19:53:01Z");
    assert_eq!(
        second.header_value("Date"),
        Some("Sat, 31 Oct 2026 19:53:01 GMT")
    );
    assert_eq!(
        json_body(&get_meta(&fake, &access, &id_of(&first)))["createdTime"],
        "2026-10-31T19:51:00Z"
    );
}

#[test]
fn the_hook_runs_before_a_request_and_may_send_again() {
    let fake = Rc::new(FakeDrive::new());
    let access = token(&fake);
    let created = create_json(
        &fake,
        &access,
        json!({"name": "state.json", "parents": ["root"]}),
    );
    let id = id_of(&created);

    let slipped = Rc::new(AtomicBool::new(false));
    let fake_for_hook = Rc::clone(&fake);
    let slipped_for_hook = Rc::clone(&slipped);
    let access_for_hook = access.clone();
    let id_for_hook = id.clone();
    fake.set_hook(Some(Box::new(move |request: &Request| {
        if request.method == "GET" && !slipped_for_hook.swap(true, Ordering::SeqCst) {
            let update = Request::new("PATCH", &format!("{API}/files/{id_for_hook}"))
                .header("Authorization", &format!("Bearer {access_for_hook}"))
                .header("Content-Type", "application/json")
                .body(json!({"name": "slipped.json"}).to_string().into_bytes());
            let response = fake_for_hook
                .send(&update)
                .expect("the hook's request reaches the fake");
            assert_eq!(response.status, 200, "{}", response.text());
        }
    })));

    let got = get_meta(&fake, &access, &id);
    assert_eq!(got.status, 200);
    assert_eq!(json_body(&got)["name"], "slipped.json");
    assert!(slipped.load(Ordering::SeqCst));
    assert!(
        fake.requests()
            .iter()
            .any(|line| line == &format!("PATCH {API}/files/{id}"))
    );

    fake.set_hook(None);
    let after = get_meta(&fake, &access, &id);
    assert_eq!(json_body(&after)["name"], "slipped.json");
}

#[test]
fn expired_access_tokens_get_401_and_new_ones_work() {
    let fake = FakeDrive::new();
    let first = token(&fake);

    let none = ok(&fake, Request::new("GET", &format!("{API}/about")));
    assert_eq!(none.status, 401);
    assert_eq!(json_body(&none)["error"]["code"], 401);

    fake.expire_access_tokens();
    let refused = list(&fake, &first, "'root' in parents and trashed = false");
    assert_eq!(refused.status, 401);
    assert!(refused.header_value("Date").is_some());

    let second = token(&fake);
    assert_eq!(second, "fake-access-2");
    let allowed = list(&fake, &second, "'root' in parents and trashed = false");
    assert_eq!(allowed.status, 200);
}

#[test]
fn revoke_makes_the_refresh_invalid_grant_and_refuses_access_tokens() {
    let fake = FakeDrive::new();
    let access = token(&fake);
    fake.revoke();

    let refresh = ok(&fake, token_request());
    assert_eq!(refresh.status, 400);
    assert_eq!(json_body(&refresh)["error"], "invalid_grant");

    let refused = list(&fake, &access, "'root' in parents and trashed = false");
    assert_eq!(refused.status, 401);
}

#[test]
fn offline_is_unreachable_but_file_paths_still_work() {
    let fake = FakeDrive::new();
    let access = token(&fake);
    create_json(
        &fake,
        &access,
        json!({"name": "a.sav", "parents": ["root"]}),
    );
    fake.set_reachable(false);

    let error = fake
        .send(&Request::new("GET", &format!("{API}/files/anything")))
        .expect_err("offline");
    assert_eq!(error, Error::Unreachable("fake drive is offline".into()));
    assert_eq!(fake.file_paths(), vec!["a.sav"]);
    assert!(
        fake.requests()
            .last()
            .expect("the offline request")
            .starts_with("GET ")
    );

    fake.set_reachable(true);
    let response = list(&fake, &access, "'root' in parents and trashed = false");
    assert_eq!(response.status, 200);
}

#[test]
fn copy_and_delete_change_the_tree() {
    let fake = FakeDrive::new();
    let access = token(&fake);
    let g = id_of(&create_json(
        &fake,
        &access,
        json!({"name": "g", "mimeType": FOLDER_MIME, "parents": ["root"]}),
    ));
    let h = id_of(&create_json(
        &fake,
        &access,
        json!({"name": "h", "mimeType": FOLDER_MIME, "parents": ["root"]}),
    ));
    let created = create_multipart(
        &fake,
        &access,
        json!({"name": "a.sav", "parents": [g.clone()]}),
        b"save",
    );
    let source = id_of(&created);

    let copied = copy_file(
        &fake,
        &access,
        &source,
        json!({"name": "b.sav", "parents": [h.clone()]}),
    );
    assert_eq!(copied.status, 200, "{}", copied.text());
    let copy = id_of(&copied);
    assert_ne!(copy, source);
    assert_eq!(json_body(&copied)["version"], "1");
    assert_eq!(json_body(&copied)["parents"], json!([h.clone()]));
    assert_eq!(get_media(&fake, &access, &copy).body, b"save");

    let deleted = delete_file(&fake, &access, &source);
    assert_eq!(deleted.status, 204);
    assert_eq!(get_meta(&fake, &access, &source).status, 404);
    assert_eq!(fake.file_paths(), vec!["h/b.sav"]);

    let deleted_folder = delete_file(&fake, &access, &h);
    assert_eq!(deleted_folder.status, 204);
    assert!(fake.file_paths().is_empty());
    assert_eq!(get_meta(&fake, &access, &copy).status, 404);

    let missing = delete_file(&fake, &access, "not-here");
    assert_eq!(missing.status, 404);
    assert!(missing.text().contains("not-here"));
}

#[test]
fn a_create_with_a_missing_parent_is_not_found() {
    let fake = FakeDrive::new();
    let access = token(&fake);
    let response = create_json(
        &fake,
        &access,
        json!({"name": "a.sav", "parents": ["not-a-folder"]}),
    );
    assert_eq!(response.status, 404);
    assert!(response.text().contains("not-a-folder"));
    assert!(fake.file_paths().is_empty());
}
