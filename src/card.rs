//! Save cards (SPEC.md M14; docs/adr/0003-baton-wrap-cards-panel.md): every push carries a
//! small card saying who saved, when, how long the session was, and when known a place name
//! and a picture, so a conflict or a restore reads as two cards, not two device names.
//!
//! A card lives in its history entry as `CARD_FILE`, beside `transfer::RECORD`, with its
//! picture (`.save-relay-picture.jpg` or `.png`); neither is ever written into the saves
//! folder. A port supplies the place and picture through a sidecar file, `SIDECAR`, in the
//! saves folder; it is never part of the save set (every `.save-relay-*` name is reserved).
//! For a wrapped game the picture is Steam's own screenshot (`steam::pick_screenshot`).
//! Pure.

use crate::error::Result;
use serde::{Deserialize, Serialize};

/// The card format this build reads and writes.
pub const FORMAT: u32 = 1;

/// Every file name save-relay keeps beside saves starts with this; such files are never saves.
pub const RESERVED_PREFIX: &str = ".save-relay-";

/// The card inside a history entry.
pub const CARD_FILE: &str = ".save-relay-card.json";

/// A port's sidecar in the saves folder: `{"place": "...", "picture": "<file in the saves folder>"}`.
pub const SIDECAR: &str = ".save-relay-meta.json";

/// A place name is cut to this many characters (not bytes).
pub const MAX_PLACE_CHARS: usize = 80;

/// A bigger picture is dropped, with a note.
pub const MAX_PICTURE_BYTES: usize = 512 * 1024;

/// A session longer than this has no playtime (the start record was stale).
pub const MAX_PLAYTIME_SECS: u64 = 24 * 60 * 60;

/// `.save-relay-card.json` in a history entry. Old entries have none (`card: null` in the API).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub format: u32,
    /// The machine that pushed (`layout::device_slug` form).
    pub device: String,
    /// When, `stamp::rfc3339` form.
    pub time: String,
    /// Seconds from the session's `pull` to this push; `null` when unknown (`playtime`).
    #[serde(default)]
    pub playtime_secs: Option<u64>,
    /// "Chapter 7: The Fortress"; at most `MAX_PLACE_CHARS`.
    #[serde(default)]
    pub place: Option<String>,
    /// The picture's file name in the entry (`PictureKind::file_name`); `null` for none.
    #[serde(default)]
    pub picture: Option<String>,
}

impl Card {
    /// Parse and check: `format == FORMAT`, `device` not empty, `time` RFC 3339, `place` at
    /// most `MAX_PLACE_CHARS`, `picture` one of the two `PictureKind::file_name`s. Any failure
    /// is `Error::StateInvalid` (the caller shows such a card as none).
    pub fn from_json(text: &str) -> Result<Card> {
        let card: Card = serde_json::from_str(text)
            .map_err(|e| crate::Error::StateInvalid(format!("card: {e}")))?;
        if card.format != FORMAT
            || card.device.is_empty()
            || crate::stamp::parse_rfc3339(&card.time).is_none()
            || card.place.as_deref().is_some_and(|place| place.chars().count() > MAX_PLACE_CHARS)
            || card.picture.as_deref().is_some_and(|picture| {
                picture != PictureKind::Jpeg.file_name() && picture != PictureKind::Png.file_name()
            })
        {
            return Err(crate::Error::StateInvalid("card: invalid card fields".to_string()));
        }
        Ok(card)
    }

    /// Compact JSON on one line, fields in declaration order, no trailing newline.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("serializing a card cannot fail")
    }
}

/// A port's `SIDECAR`, as read. Unknown fields are ignored, so ports can add their own.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    #[serde(default)]
    pub place: Option<String>,
    /// A file name relative to the saves folder, `/`-separated, no `..`.
    #[serde(default)]
    pub picture: Option<String>,
}

/// Read a sidecar. A sidecar never fails a push: bad JSON, or a `picture` path that is
/// absolute or has a `..` segment, gives `Err(note)` with one short line for `message`.
pub fn parse_sidecar(text: &str) -> std::result::Result<Sidecar, String> {
    let sidecar: Sidecar = serde_json::from_str(text)
        .map_err(|e| format!("sidecar left out: {e}"))?;
    if let Some(picture) = sidecar.picture.as_deref() {
        if picture.starts_with('/') || picture.contains('\\') || picture.contains(':')
            || picture.split('/').any(|segment| segment == "..")
        {
            return Err(format!("sidecar left out: picture path {picture} is not inside the saves folder"));
        }
    }
    Ok(sidecar)
}

/// A picture format, known by its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureKind {
    /// `FF D8 FF`
    Jpeg,
    /// `89 50 4E 47 0D 0A 1A 0A`
    Png,
}

impl PictureKind {
    /// `.save-relay-picture.jpg` or `.save-relay-picture.png`.
    pub fn file_name(self) -> &'static str {
        match self {
            PictureKind::Jpeg => ".save-relay-picture.jpg",
            PictureKind::Png => ".save-relay-picture.png",
        }
    }

    /// `image/jpeg` or `image/png`.
    pub fn mime(self) -> &'static str {
        match self {
            PictureKind::Jpeg => "image/jpeg",
            PictureKind::Png => "image/png",
        }
    }
}

/// Check a picture's bytes: JPEG or PNG by magic bytes and at most `MAX_PICTURE_BYTES`. A
/// picture is never an error: anything else is `Err(note)` ("picture left out: not JPEG or
/// PNG", "picture left out: over 512 KiB").
pub fn check_picture(bytes: &[u8]) -> std::result::Result<PictureKind, String> {
    let kind = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        PictureKind::Jpeg
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        PictureKind::Png
    } else {
        return Err("picture left out: not JPEG or PNG".to_string());
    };
    if bytes.len() > MAX_PICTURE_BYTES {
        return Err("picture left out: over 512 KiB".to_string());
    }
    Ok(kind)
}

/// A place name as stored: trimmed, inner runs of whitespace (newlines too) as one space,
/// cut to `MAX_PLACE_CHARS` characters; empty becomes `None`.
pub fn clean_place(text: &str) -> Option<String> {
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let place = cleaned.chars().take(MAX_PLACE_CHARS).collect::<String>();
    (!place.is_empty()).then_some(place)
}

/// The session's playtime: `now - start`, or `None` when there is no start, the start is
/// after `now`, or the difference is over `MAX_PLAYTIME_SECS`.
pub fn playtime(start: Option<u64>, now: u64) -> Option<u64> {
    let elapsed = now.checked_sub(start?)?;
    (elapsed <= MAX_PLAYTIME_SECS).then_some(elapsed)
}

/// True for a save-set path any of whose `/` segments starts with `RESERVED_PREFIX`
/// (`scan` leaves these out, so a sidecar or a part file is never a save).
pub fn is_reserved(path: &str) -> bool {
    path.split('/').any(|segment| segment.starts_with(RESERVED_PREFIX))
}

/// The card a push writes: `FORMAT`, `device`, `time = stamp::rfc3339(now)`,
/// `playtime(session_start, now)`, `place` through `clean_place`, and `picture` as
/// `kind.file_name()` when a checked picture goes with it.
pub fn for_push(
    device: &str,
    now: u64,
    session_start: Option<u64>,
    place: Option<&str>,
    picture: Option<PictureKind>,
) -> Card {
    Card {
        format: FORMAT,
        device: device.to_string(),
        time: crate::stamp::rfc3339(now),
        playtime_secs: playtime(session_start, now),
        place: place.and_then(clean_place),
        picture: picture.map(|kind| kind.file_name().to_string()),
    }
}
