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
    let seconds = (unix % 60u64) as u32;
    let minutes = ((unix / 60u64) % 60u64) as u32;
    let hours = ((unix / 3600u64) % 24u64) as u32;
    let days = unix / 86400u64;

    // Convert days since Unix epoch to year/month/day using Howard Hinnant's algorithm
    let (year, month, day) = civil_from_days(days as i64);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

/// The inverse of `rfc3339`; only that exact form (`YYYY-MM-DDTHH:MM:SSZ`), else `None`.
pub fn parse_rfc3339(text: &str) -> Option<u64> {
    if text.len() != 20 || !text.ends_with('Z') {
        return None;
    }

    // Check the format more carefully
    if text.chars().nth(4) != Some('-')
        || text.chars().nth(7) != Some('-')
        || text.chars().nth(10) != Some('T')
        || text.chars().nth(13) != Some(':')
        || text.chars().nth(16) != Some(':')
    {
        return None;
    }

    // Check that the year starts with a digit (not + or -)
    if !text.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }

    // Make sure we have exactly 4 digits for year
    if !text[0..4].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let year = text[0..4].parse::<u32>().ok()?;
    let month = text[5..7].parse::<u32>().ok()?;
    let day = text[8..10].parse::<u32>().ok()?;
    let hour = text[11..13].parse::<u32>().ok()?;
    let minute = text[14..16].parse::<u32>().ok()?;
    let second = text[17..19].parse::<u32>().ok()?;

    // Validate ranges
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }

    // Check for invalid dates (leap year, etc.)
    if !is_valid_date(year, month, day) {
        return None;
    }

    let days = civil_from_days_to_days(year, month, day);

    // Check for potential overflow before multiplication
    if days > u64::MAX / 86400u64 {
        return None;
    }

    let seconds_since_epoch =
        days * 86400u64 + hour as u64 * 3600u64 + minute as u64 * 60u64 + second as u64;

    Some(seconds_since_epoch)
}

/// The compact form used in history entry names: `1793476260` -> `"20261031T195100Z"`.
/// Sorting these as text sorts them by time.
pub fn entry_stamp(unix: u64) -> String {
    let seconds = (unix % 60u64) as u32;
    let minutes = ((unix / 60u64) % 60u64) as u32;
    let hours = ((unix / 3600u64) % 24u64) as u32;
    let days = unix / 86400u64;

    // Convert days since Unix epoch to year/month/day using Howard Hinnant's algorithm
    let (year, month, day) = civil_from_days(days as i64);

    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

/// The inverse of `entry_stamp`; only that exact form (16 characters), else `None`.
pub fn parse_entry_stamp(text: &str) -> Option<u64> {
    if text.len() != 16 {
        return None;
    }

    // Check that all characters are digits except the T and Z
    if !text
        .chars()
        .enumerate()
        .all(|(i, c)| i == 8 || i == 15 || c.is_ascii_digit() || c == 'T' || c == 'Z')
    {
        return None;
    }

    // Make sure T is uppercase
    if text.chars().nth(8) != Some('T') {
        return None;
    }

    let year = text[0..4].parse::<u32>().ok()?;
    let month = text[4..6].parse::<u32>().ok()?;
    let day = text[6..8].parse::<u32>().ok()?;
    let hour = text[9..11].parse::<u32>().ok()?;
    let minute = text[11..13].parse::<u32>().ok()?;
    let second = text[13..15].parse::<u32>().ok()?;

    // Make sure it's uppercase Z (not lowercase z)
    if text.chars().nth(15) != Some('Z') {
        return None;
    }

    // Validate ranges
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }

    // Check for invalid dates (leap year, etc.)
    if !is_valid_date(year, month, day) {
        return None;
    }

    let days = civil_from_days_to_days(year, month, day);
    let seconds_since_epoch =
        days * 86400u64 + hour as u64 * 3600u64 + minute as u64 * 60u64 + second as u64;

    Some(seconds_since_epoch)
}

// Helper function to convert days since Unix epoch (1970-01-01) to year/month/day
// Using Howard Hinnant's civil date algorithm with i64 to avoid overflow issues
fn civil_from_days(days: i64) -> (u32, u32, u32) {
    let z = days + 719468; // Days from year 0 to 1970-01-01

    // Use the algorithm for converting days since epoch to civil date
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + (m <= 2) as i64;

    (y as u32, m as u32, d as u32)
}

// Helper function to convert year/month/day to days since Unix epoch (1970-01-01)
fn civil_from_days_to_days(year: u32, month: u32, day: u32) -> u64 {
    // Using the inverse of Howard Hinnant's algorithm
    let y = year as i64 - (month <= 2) as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let month_i64 = month as i64;
    let doy =
        (153 * (if month_i64 > 2 {
            month_i64 - 3
        } else {
            month_i64 + 9
        }) + 2)
            / 5
            + day as i64
            - 1i64;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;

    days as u64
}

// Helper function to check if a date is valid
fn is_valid_date(year: u32, month: u32, day: u32) -> bool {
    if !(1..=12).contains(&month) {
        return false;
    }

    // Days in each month (non-leap year)
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) {
                29
            } else {
                28
            }
        }
        _ => return false,
    };

    if day < 1 || day > max_day {
        return false;
    }

    true
}
