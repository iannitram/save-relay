//! Live check of `WebDav` against a Nextcloud test container (F17). Ignored by default, so
//! normal CI never needs a server. Run it with:
//!
//! ```text
//! SR_NC_SERVER=http://127.0.0.1:8099 SR_NC_USER=<user> SR_NC_PASSWORD=<password> \
//!   cargo test --test webdav_live -- --ignored --nocapture
//! ```
//!
//! It works in one folder it creates (`sr-live-<pid>`) and deletes that folder at the end.
#![cfg(feature = "http")]
use save_relay::creds::Secret;
use save_relay::http::UreqTransport;
use save_relay::remote::{Precondition, Remote};
use save_relay::webdav::{files_url, WebDav};
use save_relay::Error;
use std::sync::Arc;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} to run the live check"))
}

fn dav_with(password: &str) -> WebDav {
    let url = files_url(&env("SR_NC_SERVER"), &env("SR_NC_USER")).unwrap();
    WebDav::new(
        &url,
        &env("SR_NC_USER"),
        Secret::new(password),
        Arc::new(UreqTransport::new()),
    )
}

#[test]
#[ignore = "needs a Nextcloud test container: see the file comment"]
fn all_seven_methods_and_status_mappings_against_a_live_server() {
    let dav = dav_with(&env("SR_NC_PASSWORD"));
    let root = format!("sr-live-{}", std::process::id());
    let run = || -> () {
        // mkcol: new folder, then again (405 -> Ok), then under a missing parent (409 -> NotFound).
        dav.mkcol(&root).unwrap();
        println!("mkcol new folder: Ok");
        dav.mkcol(&root).unwrap();
        println!("mkcol existing folder (405): Ok");
        let err = dav.mkcol(&format!("{root}/missing/child")).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{err:?}");
        println!("mkcol missing parent (409): {err:?}");

        // put + get: Absent creates, Absent again is 412 -> Changed.
        let file = format!("{root}/a file.txt");
        let etag1 = dav.put(&file, b"one", &Precondition::Absent).unwrap();
        println!("put Absent: etag {etag1}");
        let err = dav.put(&file, b"x", &Precondition::Absent).unwrap_err();
        assert!(matches!(err, Error::Changed), "{err:?}");
        println!("put Absent on existing (412): {err:?}");
        let (bytes, got) = dav.get(&file).unwrap();
        assert_eq!(bytes, b"one");
        assert_eq!(got, etag1);
        println!("get: {} bytes, etag matches put", bytes.len());

        // put Match: right etag wins, stale etag is Changed.
        let etag2 = dav
            .put(&file, b"two", &Precondition::Match(etag1.clone()))
            .unwrap();
        assert_ne!(etag1, etag2);
        println!("put Match(current): new etag");
        let err = dav
            .put(&file, b"three", &Precondition::Match(etag1))
            .unwrap_err();
        assert!(matches!(err, Error::Changed), "{err:?}");
        println!("put Match(stale) (412): {err:?}");
        dav.put(&file, b"four", &Precondition::Any).unwrap();
        println!("put Any: Ok");
        let err = dav
            .put(&format!("{root}/nope/x.txt"), b"x", &Precondition::Any)
            .unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{err:?}");
        println!("put missing parent (409): {err:?}");

        // list: the folder's child, with name, size and etag.
        let entries = dav.list(&root).unwrap();
        println!("list {root}: {entries:?}");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a file.txt");
        assert!(entries[0].etag.is_some());

        // copy, then copy onto the existing destination (Overwrite: F -> Server("exists")).
        let copy = format!("{root}/copy.txt");
        dav.copy(&file, &copy).unwrap();
        println!("copy: Ok");
        let err = dav.copy(&file, &copy).unwrap_err();
        assert!(matches!(&err, Error::Server(m) if m == "exists"), "{err:?}");
        println!("copy onto existing (412, Overwrite: F): {err:?}");
        let err = dav.copy(&file, &format!("{root}/nope/c.txt")).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{err:?}");
        println!("copy to missing parent (409): {err:?}");

        // move_to, and move onto an existing destination.
        let moved = format!("{root}/moved.txt");
        dav.move_to(&copy, &moved).unwrap();
        println!("move_to: Ok");
        let err = dav.move_to(&file, &moved).unwrap_err();
        assert!(matches!(&err, Error::Server(m) if m == "exists"), "{err:?}");
        println!("move_to onto existing (412, Overwrite: F): {err:?}");
        assert!(matches!(dav.get(&copy), Err(Error::NotFound(_))));
        assert_eq!(dav.get(&moved).unwrap().0, b"four");
        println!("get old path after move (404): NotFound");

        // delete a file, then the same again (404).
        dav.delete(&moved).unwrap();
        println!("delete file: Ok");
        let err = dav.delete(&moved).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{err:?}");
        println!("delete missing (404): {err:?}");
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
    // Clean up whatever is left, whether or not the checks passed.
    let _ = dav.delete(&root);
    assert!(matches!(dav.list(&root), Err(Error::NotFound(_))));
    println!("cleanup: {root} removed");
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

#[test]
#[ignore = "needs a Nextcloud test container: see the file comment"]
fn a_wrong_password_is_auth_failed_and_never_appears_in_an_error() {
    let wrong = "wrong-password-canary-4711";
    let dav = dav_with(wrong);
    let errors = vec![
        dav.list("").unwrap_err(),
        dav.get("x.txt").unwrap_err(),
        dav.put("x.txt", b"x", &Precondition::Any).unwrap_err(),
        dav.mkcol("sr-live-auth").unwrap_err(),
        dav.copy("x.txt", "y.txt").unwrap_err(),
        dav.move_to("x.txt", "y.txt").unwrap_err(),
        dav.delete("x.txt").unwrap_err(),
    ];
    for error in &errors {
        assert!(matches!(error, Error::AuthFailed), "{error:?}");
        let text = format!("{error} {error:?}");
        assert!(!text.contains(wrong), "password leaked: {text}");
    }
    println!("401 on all seven methods: AuthFailed, password absent from Display and Debug");
}
