//! The include/exclude patterns of a manifest, matched against a save file's relative path.
//! Pure; no file system.

/// Does `pattern` match the whole of `path`? Both use `/` between segments, are relative
/// and case-sensitive.
///
/// - `*` matches any run of characters inside one segment (never a `/`), including none.
/// - `?` matches exactly one character that is not `/`.
/// - `**` as a whole segment matches zero or more whole segments; `a/**` matches everything
///   below `a` (not `a` itself); `**/x` matches `x` at any depth.
/// - Every other character matches itself.
pub fn matches(pattern: &str, path: &str) -> bool {
    crate::not_built!("glob::matches")
}

/// A path is selected when it matches at least one `include` pattern and no `exclude` pattern.
pub fn selected(include: &[String], exclude: &[String], path: &str) -> bool {
    crate::not_built!("glob::selected")
}
