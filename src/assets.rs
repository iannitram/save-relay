//! Installer asset manifest validation. Transfer and mobile APIs are not yet implemented.
use crate::error::{Error, Result};
use serde::Deserialize;
use std::collections::BTreeSet;

const MAX_MANIFEST: usize = 4 * 1024 * 1024;
const MAX_FILES: usize = 100_000;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssetFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssetManifest {
    pub version: u32,
    #[serde(default)]
    pub app_id: Option<String>,
    pub files: Vec<AssetFile>,
}

fn invalid() -> Error {
    Error::ManifestInvalid("invalid installer manifest".into())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && id
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

fn valid_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 4096 {
        return false;
    }
    path.split('/').all(|p| {
        if p.is_empty()
            || p.len() > 255
            || p == "."
            || p == ".."
            || p.ends_with(['.', ' '])
            || p.chars()
                .any(|c| c.is_control() || "\\<>:\"|?*".contains(c))
        {
            return false;
        }
        let stem = p.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
            return false;
        }
        if (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && matches!(stem.as_bytes()[3], b'1'..=b'9')
        {
            return false;
        }
        !p.to_ascii_lowercase().starts_with(".save-relay-")
    })
}

/// Parse a bounded manifest, binding a legacy no-app-id manifest only when explicitly allowed.
pub fn parse_manifest(raw: &[u8], app_id: &str, allow_legacy: bool) -> Result<AssetManifest> {
    if raw.len() > MAX_MANIFEST || !valid_id(app_id) {
        return Err(invalid());
    }
    let manifest: AssetManifest = serde_json::from_slice(raw).map_err(|_| invalid())?;
    if manifest.version != 1
        || manifest.files.is_empty()
        || manifest.files.len() > MAX_FILES
        || match &manifest.app_id {
            Some(id) => id != app_id,
            None => !allow_legacy,
        }
    {
        return Err(invalid());
    }
    let mut paths = BTreeSet::new();
    let mut total = 0u64;
    for f in &manifest.files {
        if !valid_path(&f.path)
            || f.sha256.len() != 64
            || !f
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid());
        }
        let folded = f.path.to_lowercase();
        if folded == "user"
            || folded.starts_with("user/")
            || folded == "config.ini"
            || folded == "logs"
            || folded.starts_with("logs/")
            || folded == "manifest.json"
            || !paths.insert(folded)
        {
            return Err(invalid());
        }
        total = total.checked_add(f.size).ok_or_else(invalid)?;
    }
    for path in &paths {
        for (i, _) in path.match_indices('/') {
            if paths.contains(&path[..i]) {
                return Err(invalid());
            }
        }
    }
    Ok(manifest)
}

/// Validate a single identity-encoded HTTP range before any bytes are persisted.
pub fn validate_range_response(
    response: &crate::http::Response,
    offset: u64,
    requested: usize,
    expected_total: u64,
) -> Result<()> {
    let invalid_range = || Error::Server("invalid or unsupported range response".into());
    if requested == 0
        || requested > crate::remote::MAX_RANGE_BYTES
        || response.status != 206
        || response
            .header_value("Content-Encoding")
            .is_some_and(|v| !v.eq_ignore_ascii_case("identity"))
        || response
            .header_value("Content-Type")
            .is_some_and(|v| v.to_ascii_lowercase().starts_with("multipart/"))
    {
        return Err(invalid_range());
    }
    let value = response
        .header_value("Content-Range")
        .ok_or_else(invalid_range)?;
    let (span, total) = value
        .strip_prefix("bytes ")
        .and_then(|v| v.split_once('/'))
        .ok_or_else(invalid_range)?;
    let (start, end) = span.split_once('-').ok_or_else(invalid_range)?;
    let start: u64 = start.parse().map_err(|_| invalid_range())?;
    let end: u64 = end.parse().map_err(|_| invalid_range())?;
    let total: u64 = total.parse().map_err(|_| invalid_range())?;
    let len = end
        .checked_sub(start)
        .and_then(|v| v.checked_add(1))
        .ok_or_else(invalid_range)?;
    if start != offset
        || total != expected_total
        || end >= total
        || len > requested as u64
        || len != response.body.len() as u64
    {
        return Err(invalid_range());
    }
    Ok(())
}
