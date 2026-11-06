//! Locked tests for F60: the save card (`card`). Do not edit.

use save_relay::card::*;
use save_relay::Error;

const GOOD: &str = r#"{"format":1,"device":"amd-handheld","time":"2026-10-31T19:51:00Z","playtime_secs":5400,"place":"Chapter 7: The Fortress","picture":".save-relay-picture.jpg"}"#;

/// `GOOD`, read with serde's derive, not with `from_json`.
fn card() -> Card {
    serde_json::from_str(GOOD).unwrap()
}

fn some(text: &str) -> Option<String> {
    Some(text.to_string())
}

const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];

#[test]
fn json() {
    assert_eq!(card().to_json(), GOOD);
    assert_eq!(Card::from_json(GOOD).unwrap(), card());
    let bare = r#"{"format":1,"device":"a","time":"2026-10-31T19:51:00Z"}"#;
    let c = Card::from_json(bare).unwrap();
    assert_eq!((c.playtime_secs, c.place, c.picture), (None, None, None));
    assert!(matches!(Card::from_json("[]"), Err(Error::StateInvalid(_))));
}

#[test]
fn from_json_refuses_bad_cards() {
    let bad = |from: &str, to: &str| {
        let text = GOOD.replace(from, to);
        assert_ne!(text, GOOD);
        matches!(Card::from_json(&text), Err(Error::StateInvalid(_)))
    };
    let place = "Chapter 7: The Fortress";
    assert!(bad("\"format\":1", "\"format\":2"));
    assert!(bad("amd-handheld", ""));
    assert!(bad("2026-10-31T19:51:00Z", "2026-10-31"));
    assert!(bad(place, &"x".repeat(81)));
    assert!(bad(".save-relay-picture.jpg", "shot.jpg"));
    assert!(bad(".save-relay-picture.jpg", CARD_FILE));
    assert!(!bad(place, &"é".repeat(80)));
    assert!(!bad(".jpg", ".png"));
    assert!(!bad("\"Chapter 7: The Fortress\"", "null"));
}

#[test]
fn sidecar() {
    assert_eq!(SIDECAR, ".save-relay-meta.json");
    let text = r#"{"place":"Chapter 7","picture":"shots/last.png","port":"x"}"#;
    let want = Sidecar {
        place: some("Chapter 7"),
        picture: some("shots/last.png"),
    };
    assert_eq!(parse_sidecar(text), Ok(want));
    assert_eq!(parse_sidecar("{}"), Ok(Sidecar::default()));
    let bad = [
        "/etc/passwd",
        "../x.png",
        "a/../../x.png",
        r"C:\\x.png",
        r"\\\\server\\x.png",
    ];
    let bad = bad.map(|p| format!(r#"{{"picture":"{p}"}}"#));
    for text in ["", "{"].into_iter().chain(bad.iter().map(String::as_str)) {
        let note = parse_sidecar(text).unwrap_err();
        assert!(!note.is_empty() && !note.contains('\n'), "{text}: {note:?}");
    }
}

#[test]
fn pictures() {
    assert_eq!(
        check_picture(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 16]),
        Ok(PictureKind::Jpeg)
    );
    assert_eq!(check_picture(PNG), Ok(PictureKind::Png));
    let mut big = vec![0u8; MAX_PICTURE_BYTES];
    big[..3].copy_from_slice(&[0xFF, 0xD8, 0xFF]);
    assert_eq!(check_picture(&big), Ok(PictureKind::Jpeg));
    big.push(0);
    assert_eq!(
        check_picture(&big),
        Err("picture left out: over 512 KiB".to_string())
    );
    for not in [&b""[..], &[0xFF, 0xD8][..], &b"GIF89a"[..], &PNG[..7]] {
        let note = "picture left out: not JPEG or PNG".to_string();
        assert_eq!(check_picture(not), Err(note), "{not:?}");
    }
}

#[test]
fn places_and_playtimes() {
    let messy = "  Chapter 7:\n  The\tFortress  ";
    assert_eq!(clean_place(messy), some("Chapter 7: The Fortress"));
    assert_eq!(clean_place(" \n\t "), None);
    assert_eq!(clean_place(""), None);
    assert_eq!(clean_place(&"ab".repeat(50)), Some("ab".repeat(40)));
    assert_eq!(clean_place(&"é".repeat(100)), Some("é".repeat(80)));
    assert_eq!(playtime(None, 1_000), None);
    assert_eq!(playtime(Some(1_000), 6_400), Some(5_400));
    assert_eq!(playtime(Some(1_000), 1_000), Some(0));
    assert_eq!(playtime(Some(2_000), 1_000), None);
    assert_eq!(playtime(Some(0), 86_400), Some(86_400));
    assert_eq!(playtime(Some(0), 86_401), None);
}

#[test]
fn reserved_names() {
    assert!(is_reserved(".save-relay-meta.json"));
    assert!(is_reserved("slot1/.save-relay-part"));
    assert!(is_reserved(".save-relay-x/save1.dat"));
    assert!(!is_reserved("save1.dat"));
    assert!(!is_reserved("slot1/save-relay-meta.json"));
    assert!(!is_reserved("slot1/x.save-relay-y"));
}

#[test]
fn the_card_a_push_writes() {
    let (now, start) = (1_793_476_260, Some(1_793_470_860));
    let place = Some("  Chapter 7:\nThe Fortress ");
    let c = for_push("amd-handheld", now, start, place, Some(PictureKind::Png));
    assert_eq!(c.to_json(), GOOD.replace(".jpg", ".png"));
    let bare = for_push("rtx-3090-ti-desktop", now, None, Some("  "), None);
    assert_eq!(bare.device, "rtx-3090-ti-desktop");
    assert_eq!(
        (bare.playtime_secs, bare.place, bare.picture),
        (None, None, None)
    );
}
