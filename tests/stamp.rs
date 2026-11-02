//! Locked tests for `stamp` (F02). Do not edit: the code must make these pass.

use save_relay::stamp::{entry_stamp, parse_entry_stamp, parse_rfc3339, rfc3339};

/// (unix seconds, RFC 3339, entry stamp)
const CASES: &[(u64, &str, &str)] = &[
    (0, "1970-01-01T00:00:00Z", "19700101T000000Z"),
    (1793476260, "2026-10-31T19:51:00Z", "20261031T195100Z"),
    (946684799, "1999-12-31T23:59:59Z", "19991231T235959Z"),
    (951782400, "2000-02-29T00:00:00Z", "20000229T000000Z"),
    (1709164800, "2024-02-29T00:00:00Z", "20240229T000000Z"),
    (1709251199, "2024-02-29T23:59:59Z", "20240229T235959Z"),
    (1798761599, "2026-12-31T23:59:59Z", "20261231T235959Z"),
    (4107499200, "2100-02-28T12:00:00Z", "21000228T120000Z"),
    (4107542400, "2100-03-01T00:00:00Z", "21000301T000000Z"),
];

#[test]
fn rfc3339_formats_known_times() {
    for &(unix, text, _) in CASES {
        assert_eq!(rfc3339(unix), text, "rfc3339({unix})");
    }
}

#[test]
fn entry_stamp_formats_known_times() {
    for &(unix, _, stamp) in CASES {
        assert_eq!(entry_stamp(unix), stamp, "entry_stamp({unix})");
    }
}

#[test]
fn parse_rfc3339_inverts_known_times() {
    for &(unix, text, _) in CASES {
        assert_eq!(parse_rfc3339(text), Some(unix), "parse_rfc3339({text})");
    }
}

#[test]
fn parse_entry_stamp_inverts_known_times() {
    for &(unix, _, stamp) in CASES {
        assert_eq!(
            parse_entry_stamp(stamp),
            Some(unix),
            "parse_entry_stamp({stamp})"
        );
    }
}

#[test]
fn parse_rfc3339_rejects_other_forms() {
    for bad in [
        "",
        "2026-10-31T19:51:00",
        "2026-10-31T19:51:00+00:00",
        "2026-10-31 19:51:00Z",
        "2026-10-31T19:51:00.5Z",
        "2026-10-31t19:51:00z",
        "2026-13-01T00:00:00Z",
        "2026-02-29T00:00:00Z",
        "2100-02-29T00:00:00Z",
        "2026-10-32T00:00:00Z",
        "2026-10-31T24:00:00Z",
        "2026-10-31T19:60:00Z",
        "2026-10-31T19:51:60Z",
        "1969-12-31T23:59:59Z",
        "+026-10-31T19:51:00Z",
        " 2026-10-31T19:51:00Z",
        "20261031T195100Z",
    ] {
        assert_eq!(parse_rfc3339(bad), None, "parse_rfc3339({bad:?})");
    }
}

#[test]
fn parse_entry_stamp_rejects_other_forms() {
    for bad in [
        "",
        "20261031T195100",
        "20261031T195100z",
        "20261031t195100Z",
        "20261031T1951000Z",
        "2026103T195100Z",
        "20261331T195100Z",
        "20260229T000000Z",
        "20261031T245100Z",
        "2026-10-31T19:51:00Z",
        "20261031T195100Z-x",
        "2026103aT195100Z",
    ] {
        assert_eq!(parse_entry_stamp(bad), None, "parse_entry_stamp({bad:?})");
    }
}

#[test]
fn entry_stamps_sort_as_text_in_time_order() {
    let times = [
        0u64, 946684799, 951782400, 1709164800, 1793476260, 4107542400,
    ];
    let stamps: Vec<String> = times.iter().map(|&t| entry_stamp(t)).collect();
    let mut sorted = stamps.clone();
    sorted.sort();
    assert_eq!(stamps, sorted);
}

#[test]
fn round_trips_every_hour_across_a_leap_year() {
    // 2024-01-01T00:00:00Z .. 2025-01-01T00:00:00Z, hourly plus a 7-minute 13-second offset.
    let mut t = 1704067200u64 + 433;
    while t < 1735689600 {
        assert_eq!(parse_rfc3339(&rfc3339(t)), Some(t));
        assert_eq!(parse_entry_stamp(&entry_stamp(t)), Some(t));
        t += 3600;
    }
}
