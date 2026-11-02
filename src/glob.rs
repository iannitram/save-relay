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
    let pattern_segments: Vec<&str> = pattern.split('/').collect();
    let path_segments: Vec<&str> = path.split('/').collect();

    match_segments(&pattern_segments, &path_segments, 0, 0)
}

/// A path is selected when it matches at least one `include` pattern and no `exclude` pattern.
pub fn selected(include: &[String], exclude: &[String], path: &str) -> bool {
    // Check if the path matches any include pattern
    let included = include.iter().any(|pattern| matches(pattern, path));

    // If not included, return false immediately
    if !included {
        return false;
    }

    // Check if the path matches any exclude pattern
    let excluded = exclude.iter().any(|pattern| matches(pattern, path));

    // Return true only if not excluded
    !excluded
}

/// Helper function to recursively match segments
fn match_segments(pattern: &[&str], path: &[&str], p_idx: usize, path_idx: usize) -> bool {
    // If we've consumed all pattern segments, check if we've also consumed all path segments
    if p_idx == pattern.len() {
        return path_idx == path.len();
    }

    let pattern_segment = pattern[p_idx];

    // Handle double star pattern
    if pattern_segment == "**" {
        // If this is the last pattern segment (trailing **), we must ensure that:
        // "a/**" vs "a" -> false (because trailing "**" needs at least one segment below it)
        if p_idx == pattern.len() - 1 {
            // For a trailing "**", we can only match if there are path segments to consume
            // This ensures that patterns like "a/**" won't match just "a"
            return path_idx < path.len();
        } else {
            // For non-trailing "**", we have two cases:
            // 1. Skip this pattern (match 0 segments)
            // 2. Consume at least one path segment and continue with the same pattern

            // Case 1: skip this "**" pattern - try matching with the same path index
            if match_segments(pattern, path, p_idx + 1, path_idx) {
                return true;
            }

            // Case 2: consume at least one segment from path
            if path_idx < path.len() {
                return match_segments(pattern, path, p_idx, path_idx + 1);
            }

            return false;
        }
    }

    // If we've consumed all path segments but still have pattern segments, fail
    if path_idx == path.len() {
        return false;
    }

    let path_segment = path[path_idx];

    // Check if current pattern segment matches current path segment
    if match_segment(pattern_segment, path_segment) {
        // Continue matching with next segments
        return match_segments(pattern, path, p_idx + 1, path_idx + 1);
    }

    false
}

/// Helper function to match a single pattern segment against a path segment
fn match_segment(pattern: &str, path: &str) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let path_chars: Vec<char> = path.chars().collect();

    match_chars(&pattern_chars, &path_chars, 0, 0)
}

/// Helper function to recursively match characters in a segment
fn match_chars(pattern: &[char], path: &[char], p_idx: usize, path_idx: usize) -> bool {
    // If we've consumed all pattern characters, check if we've also consumed all path characters
    if p_idx == pattern.len() {
        return path_idx == path.len();
    }

    let pattern_char = pattern[p_idx];

    match pattern_char {
        '*' => {
            // '*' can match zero or more characters in the path
            // Try matching 0 characters first
            if match_chars(pattern, path, p_idx + 1, path_idx) {
                return true;
            }

            // Try matching 1 or more characters
            let mut i = path_idx;
            while i < path.len() {
                if match_chars(pattern, path, p_idx + 1, i + 1) {
                    return true;
                }
                i += 1;
            }

            false
        }
        '?' => {
            // '?' matches exactly one character (that is not '/')
            if path_idx < path.len() && path[path_idx] != '/' {
                match_chars(pattern, path, p_idx + 1, path_idx + 1)
            } else {
                false
            }
        }
        _ => {
            // Regular character matching
            if path_idx < path.len() && pattern_char == path[path_idx] {
                match_chars(pattern, path, p_idx + 1, path_idx + 1)
            } else {
                false
            }
        }
    }
}
