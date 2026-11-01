//! Time as UTC text. Times are whole seconds since the Unix epoch (`u64`) inside the crate and
//! one of two text forms outside it. Pure, except `now`.

use std::time::{SystemTime, UNIX_EPOCH};

/// A clock: seconds since the Unix epoch. `now` is the real one; tests pass a fixed one.
pub type Clock = fn() -> u64;

/// Seconds since the Unix epoch, now. Built in the skeleton.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// RFC 3339 UTC with a `Z`: `1793476260` -> `"2026-10-31T19:51:00Z"`.
pub fn rfc3339(unix: u64) -> String {
    crate::not_built!("stamp::rfc3339")
}

/// The inverse of `rfc3339`; only that exact form (`YYYY-MM-DDTHH:MM:SSZ`), else `None`.
pub fn parse_rfc3339(text: &str) -> Option<u64> {
    crate::not_built!("stamp::parse_rfc3339")
}

/// The compact form used in history entry names: `1793476260` -> `"20261031T195100Z"`.
/// Sorting these as text sorts them by time.
pub fn entry_stamp(unix: u64) -> String {
    crate::not_built!("stamp::entry_stamp")
}

/// The inverse of `entry_stamp`; only that exact form (16 characters), else `None`.
pub fn parse_entry_stamp(text: &str) -> Option<u64> {
    crate::not_built!("stamp::parse_entry_stamp")
}
