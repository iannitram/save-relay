//! Which history entries to delete after a push: the server keeps the last 10 per game
//! (SPEC.md M4). Pure.

/// How many history entries a game keeps.
pub const KEEP: usize = 10;

/// The entries to delete, oldest first. `entries` are entry names (`layout::entry_name` form);
/// their text order is their time order. Keep `current` always, plus the newest others up to
/// `keep` entries in all (`keep` 0 counts as 1). A name that `layout::parse_entry_name` rejects
/// is not ours: never dropped and not counted. Duplicates count once.
pub fn to_drop(entries: &[String], keep: usize, current: &str) -> Vec<String> {
    use crate::layout;

    // 1) Filter valid entries, sort oldest first, remove duplicates
    let mut ours: Vec<String> = entries
        .iter()
        .filter(|e| layout::parse_entry_name(e).is_some())
        .cloned()
        .collect();
    ours.sort(); // oldest first
    ours.dedup();

    // 2) Determine how many to keep (at least 1)
    let keep = std::cmp::max(keep, 1);

    // 3) Check if current is among our entries
    let has_current = ours.iter().any(|e| e == current);

    // 4) Calculate how many OTHER entries to keep
    let others_keep = if has_current { keep - 1 } else { keep };

    // 5) Get the other entries (excluding current) still in oldest first order
    let others: Vec<&String> = ours.iter().filter(|e| e.as_str() != current).collect();

    // 6) If we have at most others_keep other entries, nothing to drop
    if others.len() <= others_keep {
        return vec![];
    }

    // Drop the oldest ones first (the ones that come before the last others_keep entries)
    others[..others.len() - others_keep]
        .iter()
        .cloned()
        .cloned()
        .collect()
}
