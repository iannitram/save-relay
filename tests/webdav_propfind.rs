use save_relay::Error;
use save_relay::remote::RemoteEntry;
use save_relay::webdav::parse_propfind;

fn entry(name: &str, is_dir: bool, size: u64, etag: Option<&str>) -> RemoteEntry {
    RemoteEntry {
        name: name.to_string(),
        is_dir,
        size,
        etag: etag.map(str::to_string),
    }
}

const NEXTCLOUD: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns">
  <d:response>
    <d:href>/remote.php/dav/files/alex/save-relay/star-runner/</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype><d:collection/></d:resourcetype>
        <d:getlastmodified>Sun, 01 Nov 2026 10:00:00 GMT</d:getlastmodified>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/files/alex/save-relay/star-runner/profile.dat</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype/>
        <d:getcontentlength>12</d:getcontentlength>
        <d:getetag>"d4e5f6"</d:getetag>
        <d:getlastmodified>Sun, 01 Nov 2026 10:05:00 GMT</d:getlastmodified>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/files/alex/save-relay/star-runner/Episode%201%20-%20Launch.sav</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype/>
        <d:getcontentlength>48213</d:getcontentlength>
        <d:getetag>"a1b2c3"</d:getetag>
        <d:getlastmodified>Sun, 01 Nov 2026 10:01:00 GMT</d:getlastmodified>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/files/alex/save-relay/star-runner/slot%20100%25%20done/</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype><d:collection/></d:resourcetype>
        <d:getlastmodified>Sun, 01 Nov 2026 10:02:00 GMT</d:getlastmodified>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
</d:multistatus>"#;

#[test]
fn nextcloud_response_becomes_sorted_children() {
    let want = vec![
        entry("Episode 1 - Launch.sav", false, 48213, Some("\"a1b2c3\"")),
        entry("profile.dat", false, 12, Some("\"d4e5f6\"")),
        entry("slot 100% done", true, 0, None),
    ];
    let got = parse_propfind(
        NEXTCLOUD,
        "/remote.php/dav/files/alex/save-relay/star-runner/",
    )
    .unwrap();
    assert_eq!(got, want);
}

#[test]
fn the_folder_itself_is_left_out_with_or_without_its_slash() {
    let path = "/remote.php/dav/files/alex/save-relay/star-runner";
    let with = parse_propfind(NEXTCLOUD, &format!("{path}/")).unwrap();
    let without = parse_propfind(NEXTCLOUD, path).unwrap();
    assert_eq!(with, without);
    assert!(with.iter().all(|e| e.name != "star-runner"), "{with:?}");
}

#[test]
fn names_with_spaces_and_percent_are_decoded() {
    let got = parse_propfind(
        NEXTCLOUD,
        "/remote.php/dav/files/alex/save-relay/star-runner/",
    )
    .unwrap();
    let names: Vec<&str> = got.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["Episode 1 - Launch.sav", "profile.dat", "slot 100% done"]
    );
}

const RCLONE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/save-relay/night-drive/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/></D:resourcetype>
        <D:getlastmodified>Sun, 01 Nov 2026 11:00:00 GMT</D:getlastmodified>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/save-relay/night-drive/Continue%20Game.sav</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype/>
        <D:getcontentlength>2048</D:getcontentlength>
        <D:getetag>"rclone-etag-7"</D:getetag>
        <D:getlastmodified>Sun, 01 Nov 2026 11:22:33 GMT</D:getlastmodified>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/save-relay/night-drive/Backups/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/></D:resourcetype>
        <D:getetag>"backups-etag"</D:getetag>
        <D:getlastmodified>Sun, 01 Nov 2026 11:30:00 GMT</D:getlastmodified>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

#[test]
fn rclone_response_with_its_own_prefix_parses_the_same_way() {
    let want = vec![
        entry("Backups", true, 0, Some("\"backups-etag\"")),
        entry("Continue Game.sav", false, 2048, Some("\"rclone-etag-7\"")),
    ];
    let got = parse_propfind(RCLONE, "/save-relay/night-drive/").unwrap();
    assert_eq!(got, want);
}

const DEFAULT_NAMESPACE: &str = r#"<multistatus xmlns="DAV:">
  <response>
    <href>/dav/files/sam/tiny-game/</href>
    <propstat>
      <prop><resourcetype><collection/></resourcetype></prop>
      <status>HTTP/1.1 200 OK</status>
    </propstat>
  </response>
  <response>
    <href>/dav/files/sam/tiny-game/one%20save.sav</href>
    <propstat>
      <prop>
        <resourcetype/>
        <getcontentlength>3</getcontentlength>
        <getetag>W/"weak-1"</getetag>
      </prop>
      <status>HTTP/1.1 200 OK</status>
    </propstat>
  </response>
</multistatus>"#;

#[test]
fn a_default_dav_namespace_with_no_prefix_parses_too() {
    let want = vec![entry("one save.sav", false, 3, Some("W/\"weak-1\""))];
    let got = parse_propfind(DEFAULT_NAMESPACE, "/dav/files/sam/tiny-game/").unwrap();
    assert_eq!(got, want);
}

const EMPTY_FOLDER: &str = r#"<d:multistatus xmlns:d="DAV:">
  <d:response>
    <d:href>/remote.php/dav/files/alex/save-relay/empty/</d:href>
    <d:propstat>
      <d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
</d:multistatus>"#;

#[test]
fn an_empty_folder_has_no_children() {
    let got = parse_propfind(EMPTY_FOLDER, "/remote.php/dav/files/alex/save-relay/empty/").unwrap();
    assert!(got.is_empty(), "{got:?}");
}

const FOREIGN_NAMESPACE: &str = r#"<d:multistatus xmlns:d="DAV:" xmlns:x="urn:not-dav">
  <d:response>
    <d:href>/remote.php/dav/files/alex/save-relay/game/real.sav</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype/>
        <d:getcontentlength>7</d:getcontentlength>
        <d:getetag>"real"</d:getetag>
        <x:href>/remote.php/dav/files/alex/save-relay/game/fake.sav</x:href>
        <x:getcontentlength>999</x:getcontentlength>
        <x:getetag>"fake"</x:getetag>
        <x:collection/>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
</d:multistatus>"#;

#[test]
fn elements_outside_the_dav_namespace_are_ignored() {
    let want = vec![entry("real.sav", false, 7, Some("\"real\""))];
    let got = parse_propfind(
        FOREIGN_NAMESPACE,
        "/remote.php/dav/files/alex/save-relay/game/",
    )
    .unwrap();
    assert_eq!(got, want);
}

#[test]
fn a_malformed_body_is_a_server_error_not_a_panic() {
    let mismatched = r#"<d:multistatus xmlns:d="DAV:">
      <d:response><d:href>/x/a.sav</d:href></d:wrong>
    </d:multistatus>"#;
    let got = parse_propfind(mismatched, "/x/");
    assert!(matches!(got, Err(Error::Server(_))), "{got:?}");
    assert_eq!(got.unwrap_err().code(), "server_error");

    let truncated = r#"<d:multistatus xmlns:d="DAV:">
      <d:response><d:href>/x/a.sav</d:href>"#;
    let got = parse_propfind(truncated, "/x/");
    assert!(matches!(got, Err(Error::Server(_))), "{got:?}");
}

#[test]
fn not_even_xml_is_a_server_error() {
    let got = parse_propfind("this is not xml <<<", "/x/");
    assert!(matches!(got, Err(Error::Server(_))), "{got:?}");
}
