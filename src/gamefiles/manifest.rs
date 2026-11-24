//! The game-files manifest, v1 (docs/assets.md "Manifest v1"): strict JSON, at most 4 MiB,
//! no duplicate keys, no unknown fields, and paths that are safe to create on every platform.

use crate::error::{Error, Result};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;

const MAX_FILES: usize = 100_000;
const MAX_PATH_BYTES: usize = 4096;
const MAX_SEGMENT_BYTES: usize = 255;

/// One file of a game: where it goes, how long it is and its SHA-256 (64 lowercase hex).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// A validated manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub app_id: String,
    pub files: Vec<Entry>,
    pub total_bytes: u64,
    /// SHA-256 of the exact manifest bytes (hex): the session identity.
    pub digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    version: u64,
    app_id: String,
    files: Vec<Entry>,
}

/// Rejects a JSON document that repeats a key in any object (serde_json keeps the last one
/// silently, which would let two readers disagree about what the manifest says).
struct NoDuplicateKeys;

impl<'de> DeserializeSeed<'de> for NoDuplicateKeys {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for NoDuplicateKeys {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_bool<E>(self, _: bool) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_i64<E>(self, _: i64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_u64<E>(self, _: u64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_f64<E>(self, _: f64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_str<E>(self, _: &str) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_unit<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
        while seq.next_element_seed(NoDuplicateKeys)?.is_some() {}
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        let mut seen = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate key \"{key}\"")));
            }
            map.next_value_seed(NoDuplicateKeys)?;
        }
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::ManifestInvalid(message.into())
}

/// An app id: `[a-z0-9][a-z0-9._-]*`, at most 128 bytes.
pub fn check_app_id(app_id: &str) -> Result<()> {
    let mut chars = app_id.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let rest_ok =
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if app_id.len() > 128 || !first_ok || !rest_ok {
        return Err(Error::Usage(
            "an app id is 1 to 128 characters of a-z, 0-9, '.', '_' and '-', starting with a letter or digit".into(),
        ));
    }
    Ok(())
}

fn reserved_device_name(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

/// Why `path` cannot be a payload path, or `None` when it is fine.
fn path_problem(path: &str) -> Option<&'static str> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return Some("a path is empty or longer than 4096 bytes");
    }
    if path.contains('\\') {
        return Some("a path contains a backslash");
    }
    for segment in path.split('/') {
        if matches!(segment, "" | "." | "..") {
            return Some("a path is absolute or has an empty, '.' or '..' segment");
        }
        if segment.len() > MAX_SEGMENT_BYTES {
            return Some("a path segment is longer than 255 bytes");
        }
        if segment
            .chars()
            .any(|c| c.is_control() || "<>:\"|?*".contains(c))
        {
            return Some("a path has a control character or one of < > : \" | ? *");
        }
        if segment.ends_with('.') || segment.ends_with(' ') {
            return Some("a path segment ends with a dot or a space");
        }
        if reserved_device_name(segment) {
            return Some("a path segment is a reserved Windows device name");
        }
        if segment.to_ascii_lowercase().starts_with(".save-relay") {
            return Some("a path segment uses a name reserved for installer metadata");
        }
    }
    None
}

/// Parse and validate `bytes` as the manifest of `app_id`.
pub fn parse(bytes: &[u8], app_id: &str) -> Result<Manifest> {
    check_app_id(app_id)?;
    if bytes.len() > super::MANIFEST_MAX_BYTES {
        return Err(invalid("longer than 4 MiB"));
    }
    let mut checker = serde_json::Deserializer::from_slice(bytes);
    NoDuplicateKeys
        .deserialize(&mut checker)
        .and_then(|()| checker.end())
        .map_err(|e| invalid(e.to_string()))?;
    let raw: Raw = serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
    if raw.version != 1 {
        return Err(invalid(format!("unsupported version {}", raw.version)));
    }
    if raw.app_id != app_id {
        return Err(invalid("app_id is not the app that was asked for"));
    }
    if raw.files.is_empty() || raw.files.len() > MAX_FILES {
        return Err(invalid("files must hold 1 to 100000 entries"));
    }
    let mut total: u64 = 0;
    let mut folded = HashSet::with_capacity(raw.files.len());
    for entry in &raw.files {
        if let Some(problem) = path_problem(&entry.path) {
            return Err(invalid(format!("{problem}: {}", shown(&entry.path))));
        }
        let digest_ok = entry.sha256.len() == 64
            && entry
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !digest_ok {
            return Err(invalid(format!(
                "sha256 must be 64 lowercase hex characters: {}",
                shown(&entry.path)
            )));
        }
        total = total
            .checked_add(entry.size)
            .ok_or_else(|| invalid("the total size overflows"))?;
        if !folded.insert(entry.path.to_lowercase()) {
            return Err(invalid(format!(
                "a path repeats, or differs only by letter case: {}",
                shown(&entry.path)
            )));
        }
    }
    // A file cannot also be a folder: no path may be a parent of another.
    for entry in &raw.files {
        let lower = entry.path.to_lowercase();
        let mut end = 0;
        while let Some(slash) = lower[end..].find('/') {
            end += slash;
            if folded.contains(&lower[..end]) {
                return Err(invalid(format!(
                    "a file is also a folder: {}",
                    shown(&entry.path)
                )));
            }
            end += 1;
        }
    }
    Ok(Manifest {
        app_id: raw.app_id,
        files: raw.files,
        total_bytes: total,
        digest: hex(&Sha256::digest(bytes)),
    })
}

/// A path for a message: short and without control characters.
fn shown(path: &str) -> String {
    path.chars().filter(|c| !c.is_control()).take(80).collect()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
