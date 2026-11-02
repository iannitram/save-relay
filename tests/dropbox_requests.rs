use save_relay::dropbox::{api_path, classify, header_json, upload_arg, Op, Outcome};
use save_relay::remote::Precondition;
use save_relay::Error;
use serde_json::{json, Value};

#[test]
fn paths_start_with_a_slash_except_the_root() {
    assert_eq!(api_path(""), "");
    assert_eq!(api_path("a"), "/a");
    assert_eq!(
        api_path("save-relay/g/state.json"),
        "/save-relay/g/state.json"
    );
    assert_eq!(api_path("Uncharted 1/x"), "/Uncharted 1/x");
}

#[test]
fn header_json_is_ascii_and_round_trips() {
    assert_eq!(header_json(&json!({"a": 1})), r#"{"a":1}"#);
    for v in [
        json!({"path": "/é"}),
        json!({"path": "/😀 x"}),
        json!(["ü", "ok"]),
    ] {
        let text = header_json(&v);
        assert!(text.is_ascii(), "{text}");
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), v);
    }
    assert_eq!(header_json(&json!("é")), r#""\u00e9""#);
    assert_eq!(header_json(&json!("😀")), r#""\ud83d\ude00""#);
}

fn arg(path: &str, pre: &Precondition) -> Value {
    let text = upload_arg(path, pre);
    assert!(text.is_ascii(), "{text}");
    serde_json::from_str(&text).unwrap()
}

#[test]
fn upload_modes_follow_the_precondition() {
    let base = |mode: Value| {
        json!({"path": "/g/state.json", "mode": mode, "autorename": false, "mute": true,
               "strict_conflict": true})
    };
    let p = "g/state.json";
    assert_eq!(arg(p, &Precondition::Any), base(json!("overwrite")));
    assert_eq!(arg(p, &Precondition::Absent), base(json!("add")));
    let m = Precondition::Match("015f9a".into());
    assert_eq!(
        arg(p, &m),
        base(json!({".tag": "update", "update": "015f9a"}))
    );
    assert_eq!(
        arg("é/s.bin", &Precondition::Any)["path"],
        json!("/é/s.bin")
    );
}

fn summary(s: &str) -> String {
    json!({"error_summary": s, "error": {".tag": "x"}}).to_string()
}

#[test]
fn classify_status_codes() {
    assert_eq!(classify(Op::Get, 200, "{}", "a"), Outcome::Ok);
    assert_eq!(classify(Op::Put, 401, "", "a"), Outcome::Refresh);
    for s in [429, 500, 503, 599] {
        assert_eq!(classify(Op::List, s, "", "a"), Outcome::Retry, "{s}");
    }
    let got = classify(Op::Delete, 400, "bad request", "a");
    assert_eq!(
        got,
        Outcome::Err(Error::Server("dropbox 400: bad request".into()))
    );
    let got = classify(Op::Get, 403, &summary("no_permission/.."), "a");
    assert_eq!(
        got,
        Outcome::Err(Error::Server("dropbox 403: no_permission/..".into()))
    );
}

#[test]
fn classify_conflicts() {
    let c = |op, s: &str| classify(op, 409, &summary(s), "g/x");
    assert_eq!(c(Op::Mkcol, "path/conflict/folder/."), Outcome::Ok);
    assert_eq!(
        c(Op::Put, "path/conflict/file/.."),
        Outcome::Err(Error::Changed)
    );
    let file = Outcome::Err(Error::Server("a file is there: g/x".into()));
    assert_eq!(c(Op::Mkcol, "path/conflict/file/"), file);
    for op in [Op::Copy, Op::Move] {
        let want = Outcome::Err(Error::Server("exists: to/conflict/folder/".into()));
        assert_eq!(c(op, "to/conflict/folder/"), want);
    }
}

#[test]
fn classify_missing_paths() {
    let nf = Outcome::Err(Error::NotFound("g/x".into()));
    let c = |op, s: &str| classify(op, 409, &summary(s), "g/x");
    assert_eq!(c(Op::Get, "path/not_found/.."), nf);
    assert_eq!(c(Op::List, "path/not_found/"), nf);
    assert_eq!(c(Op::Delete, "path_lookup/not_found/."), nf);
    assert_eq!(c(Op::Copy, "from_lookup/not_found/"), nf);
    assert_eq!(c(Op::Move, "from_lookup/not_found/"), nf);
    assert_eq!(c(Op::Get, "path/not_file/"), nf);
    assert_eq!(c(Op::List, "path/not_folder/."), nf);
    let other = Outcome::Err(Error::Server("dropbox 409: path/not_file/".into()));
    assert_eq!(c(Op::Delete, "path/not_file/"), other);
    let other = Outcome::Err(Error::Server("dropbox 409: nope".into()));
    assert_eq!(classify(Op::Put, 409, "nope", "g/x"), other);
}
