use save_relay::webdav::{encode_path, encode_segment, files_url};
use save_relay::Error;

#[test]
fn files_url_for_a_plain_server() {
    let want = "https://cloud.example.org/remote.php/dav/files/me/";
    assert_eq!(files_url("https://cloud.example.org", "me").unwrap(), want);
    assert_eq!(files_url("https://cloud.example.org/", "me").unwrap(), want);
}

#[test]
fn files_url_keeps_a_path_and_a_port() {
    assert_eq!(
        files_url("https://example.org/nextcloud/", "me").unwrap(),
        "https://example.org/nextcloud/remote.php/dav/files/me/"
    );
    assert_eq!(
        files_url("http://192.168.1.20:8080", "me").unwrap(),
        "http://192.168.1.20:8080/remote.php/dav/files/me/"
    );
}

#[test]
fn files_url_encodes_the_user() {
    assert_eq!(
        files_url("https://cloud.example.org", "Player One@home").unwrap(),
        "https://cloud.example.org/remote.php/dav/files/Player%20One%40home/"
    );
}

#[test]
fn files_url_needs_http_or_https() {
    for bad in [
        "cloud.example.org",
        "ftp://cloud.example.org",
        "",
        "HTTPS//x",
    ] {
        assert!(
            matches!(files_url(bad, "me"), Err(Error::Usage(_))),
            "{bad}"
        );
    }
}

#[test]
fn encode_segment_keeps_unreserved_and_encodes_the_rest() {
    assert_eq!(encode_segment("Uncharted 1"), "Uncharted%201");
    assert_eq!(encode_segment("AZaz09-._~"), "AZaz09-._~");
    assert_eq!(encode_segment("a/b"), "a%2Fb");
    assert_eq!(encode_segment("50%#?&+"), "50%25%23%3F%26%2B");
    assert_eq!(encode_segment("é"), "%C3%A9");
    assert_eq!(encode_segment(""), "");
}

#[test]
fn encode_path_encodes_each_segment_and_keeps_slashes() {
    assert_eq!(
        encode_path("save-relay/uncharted-1/history/20261031T195100Z-amd-handheld"),
        "save-relay/uncharted-1/history/20261031T195100Z-amd-handheld"
    );
    assert_eq!(
        encode_path("Uncharted 1/SAVEDATA00/SYS DATA.bin"),
        "Uncharted%201/SAVEDATA00/SYS%20DATA.bin"
    );
    assert_eq!(encode_path("a b"), "a%20b");
}
