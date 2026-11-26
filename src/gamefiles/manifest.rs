//! The game-files manifest, v1 (docs/assets.md "Manifest v1"): strict JSON, at most 4 MiB,
//! no duplicate keys, no unknown fields, and paths that are safe to create on every platform.

use crate::error::{Error, Result};
use caseless::Caseless;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use unicode_normalization::UnicodeNormalization;

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
    /// The file must be executable once installed (mode 0755 on Unix; ignored on Windows,
    /// which has no execute bit). Default false.
    #[serde(default)]
    pub exec: bool,
}

/// A validated manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub app_id: String,
    pub files: Vec<Entry>,
    /// Optional publisher build label (docs/assets.md "Manifest v1"); informational.
    pub build_id: Option<String>,
    /// Optional ABI label of the Switch-game runtime the files target; informational.
    pub nx_game_abi: Option<String>,
    pub total_bytes: u64,
    /// SHA-256 of the exact manifest bytes (hex): the session identity.
    pub digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    version: u64,
    app_id: Option<String>,
    build_id: Option<String>,
    nx_game_abi: Option<String>,
    files: Vec<Entry>,
}

/// How strictly a manifest binds to its app (docs/assets.md "Manifest v1" and "ACNH
/// compatibility"). The mode comes from the host's code, never from a remote document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode<'a> {
    /// `app_id` is required and equals the requested id.
    Standard,
    /// An explicitly configured legacy mapping (ACNH): the manifest may omit `app_id`, in
    /// which case the caller's id and fixed remote `root` are bound into the session
    /// identity. Paths naming the player's `user/`, `config.ini` or logs are refused, so
    /// that data is never touched.
    Legacy { root: &'a str },
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

/// A label field (`build_id`, `nx_game_abi`): 1 to 128 bytes of printable ASCII.
fn check_label(name: &str, value: &Option<String>) -> Result<()> {
    match value {
        Some(v)
            if v.is_empty() || v.len() > 128 || !v.bytes().all(|b| (0x20..0x7f).contains(&b)) =>
        {
            Err(invalid(format!(
                "{name} must be 1 to 128 printable ASCII characters"
            )))
        }
        _ => Ok(()),
    }
}

/// Whether a (folded) payload path is a place the device owns, in every layout: the player's
/// `user/` folder and `config.ini`, bare or one level down under `root/` (the layout of the
/// ported games). Compared on the folded key, so case and Unicode forms cannot dodge it.
fn device_owned(key: &str) -> bool {
    let mut parts = key.split('/');
    matches!(
        (parts.next(), parts.next()),
        (Some("user" | "config.ini"), _) | (Some("root"), Some("user" | "config.ini"))
    )
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

/// A case- and Unicode-insensitive form of one path segment, so names that a case-insensitive
/// or normalizing file system would treat as one compare equal. It models NTFS (simple case
/// folding by an upcase table, no normalization), APFS and HFS+ (normalization-insensitive,
/// case-insensitive when so formatted) and exFAT (upcase table) by being stricter than each:
/// full Unicode default case folding (`ß` = `ss`, `ς` = `σ`, `ſ` = `s`, so `Straße` and
/// `STRASSE` collide) combined with canonical and compatibility decomposition (NFD, then NFKD
/// with the fold re-applied, as the Unicode FAQ prescribes for a stable key). A manifest that
/// passes cannot hold two names any of those systems would open as one file. It does not model
/// locale tailoring (Turkish dotless i) or 8.3 short names.
pub(crate) fn fold_segment(segment: &str) -> String {
    segment
        .chars()
        .nfd()
        .default_case_fold()
        .nfkd()
        .default_case_fold()
        .nfkd()
        .collect()
}

fn fold_path(path: &str) -> String {
    path.split('/')
        .map(fold_segment)
        .collect::<Vec<_>>()
        .join("/")
}

fn reserved_device_name(folded: &str) -> bool {
    let stem = folded
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.as_bytes()[3].is_ascii_digit())
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
        let folded = fold_segment(segment);
        if reserved_device_name(&folded) {
            return Some("a path segment is a reserved Windows device name");
        }
        if folded.starts_with(".save-relay") {
            return Some("a path segment uses a name reserved for installer metadata");
        }
    }
    None
}

/// Check a host-chosen relative remote folder (a legacy root, or a caller's `remote_root`):
/// the same rules as a payload path.
pub fn check_remote_root(root: &str) -> Result<()> {
    match path_problem(root) {
        None => Ok(()),
        Some(problem) => Err(Error::Usage(format!("invalid remote root: {problem}"))),
    }
}

/// Parse and validate `bytes` as the standard manifest of `app_id`.
pub fn parse(bytes: &[u8], app_id: &str) -> Result<Manifest> {
    parse_with(bytes, app_id, Mode::Standard)
}

/// Parse and validate `bytes` as the manifest of `app_id` in `mode`.
pub fn parse_with(bytes: &[u8], app_id: &str, mode: Mode<'_>) -> Result<Manifest> {
    check_app_id(app_id)?;
    if let Mode::Legacy { root } = mode {
        check_remote_root(root)?;
    }
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
    match (&raw.app_id, mode) {
        (Some(id), _) if id != app_id => {
            return Err(invalid("app_id is not the app that was asked for"))
        }
        (None, Mode::Standard) => return Err(invalid("app_id is missing")),
        _ => {}
    }
    check_label("build_id", &raw.build_id)?;
    check_label("nx_game_abi", &raw.nx_game_abi)?;
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
        let key = fold_path(&entry.path);
        let first = key.split('/').next().unwrap_or("");
        let legacy_only =
            matches!(mode, Mode::Legacy { .. }) && (first == "logs" || key == "manifest.json");
        if device_owned(&key) || legacy_only {
            return Err(invalid(format!(
                "a path names device-owned data or the manifest itself: {}",
                shown(&entry.path)
            )));
        }
        if !folded.insert(key) {
            return Err(invalid(format!(
                "a path repeats, or differs only by letter case or Unicode form: {}",
                shown(&entry.path)
            )));
        }
    }
    // A file cannot also be a folder: no path may be a parent of another.
    for entry in &raw.files {
        let lower = fold_path(&entry.path);
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
    // A legacy manifest carries no app id of its own, so the session identity binds the id and
    // the fixed remote root along with the exact bytes.
    let digest = match mode {
        Mode::Standard => hex(&Sha256::digest(bytes)),
        Mode::Legacy { root } => {
            let mut hasher = Sha256::new();
            for part in ["save-relay-legacy-v1", app_id, root] {
                hasher.update(part.as_bytes());
                hasher.update([0u8]);
            }
            hasher.update(bytes);
            hex(&hasher.finalize())
        }
    };
    Ok(Manifest {
        app_id: app_id.to_string(),
        files: raw.files,
        build_id: raw.build_id,
        nx_game_abi: raw.nx_game_abi,
        total_bytes: total,
        digest,
    })
}

/// Make a verified part executable before it is published (`Entry::exec`): mode 0755 on Unix,
/// nothing on Windows. The mode travels with the inode, so the published name has it from the
/// first moment.
pub(crate) fn make_executable(file: &std::fs::File) -> std::io::Result<()> {
    // fchmod on a handle the no-follow layer opened: never a path walk.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o755))
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Ok(())
    }
}

/// A path for a message: short and without control characters.
fn shown(path: &str) -> String {
    path.chars().filter(|c| !c.is_control()).take(80).collect()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
