//! Which history entries to delete after a push: the server keeps the last 10 per game
//! (SPEC.md M4). Pure.

/// How many history entries a game keeps.
pub const KEEP: usize = 10;

/// The entries to delete, oldest first. `entries` are entry names (`layout::entry_name` form);
/// their text order is their time order. Keep `current` always, plus the newest others up to
/// `keep` entries in all (`keep` 0 counts as 1). A name that `layout::parse_entry_name` rejects
/// is not ours: never dropped and not counted. Duplicates count once.
pub fn to_drop(entries: &[String], keep: usize, current: &str) -> Vec<String> {
    crate::not_built!("retention::to_drop")
}
