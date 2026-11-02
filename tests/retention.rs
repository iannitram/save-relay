//! Locked tests for `retention` (F08). Do not edit.
use save_relay::retention::{to_drop, KEEP};

fn entry(day: u32) -> String {
    format!("202610{day:02}T120000Z-amd-handheld")
}

fn entries(days: std::ops::RangeInclusive<u32>) -> Vec<String> {
    days.map(entry).collect()
}

#[test]
fn nothing_to_drop_at_or_under_keep() {
    assert!(to_drop(&entries(1..=10), 10, &entry(10)).is_empty());
    assert!(to_drop(&entries(1..=3), 10, &entry(3)).is_empty());
    assert!(to_drop(&[], 10, "").is_empty());
}

#[test]
fn drops_oldest_first() {
    assert_eq!(KEEP, 10);
    let got = to_drop(&entries(1..=13), KEEP, &entry(13));
    assert_eq!(got, vec![entry(1), entry(2), entry(3)]);
}

#[test]
fn input_order_does_not_matter() {
    let mut list = entries(1..=12);
    list.reverse();
    list.swap(0, 5);
    assert_eq!(to_drop(&list, 10, &entry(12)), vec![entry(1), entry(2)]);
}

#[test]
fn current_is_never_dropped_and_counts_in_keep() {
    // current is old (after a restore): keep it plus the 9 newest others
    let got = to_drop(&entries(1..=12), 10, &entry(1));
    assert_eq!(got, vec![entry(2), entry(3)]);
}

#[test]
fn keep_zero_counts_as_one() {
    let got = to_drop(&entries(1..=3), 0, &entry(3));
    assert_eq!(got, vec![entry(1), entry(2)]);
    let got = to_drop(&entries(1..=3), 0, &entry(1));
    assert_eq!(got, vec![entry(2), entry(3)]);
}

#[test]
fn foreign_names_are_kept_and_not_counted() {
    let mut list = entries(1..=11);
    list.push("notes".to_string());
    list.push("20261031T120000Z".to_string());
    list.push("old-backup".to_string());
    assert_eq!(to_drop(&list, 10, &entry(11)), vec![entry(1)]);
}

#[test]
fn duplicates_count_once() {
    let mut list = entries(1..=10);
    list.extend(entries(1..=10));
    assert!(to_drop(&list, 10, &entry(10)).is_empty());
    list.push(entry(11));
    assert_eq!(to_drop(&list, 10, &entry(11)), vec![entry(1)]);
}

#[test]
fn time_order_is_text_order_across_devices() {
    let list = vec![
        "20261031T120000Z-strix-halo-tablet".to_string(),
        "20261030T120000Z-rtx-3090-ti".to_string(),
        "20261101T080000Z-amd-handheld".to_string(),
    ];
    assert_eq!(
        to_drop(&list, 2, "20261101T080000Z-amd-handheld"),
        vec!["20261030T120000Z-rtx-3090-ti".to_string()]
    );
}
