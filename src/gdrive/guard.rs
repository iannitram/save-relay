//! The M9 guard for Google Drive: a write with `Precondition::Match` that cannot replace a
//! version another machine wrote meanwhile, on an API with no conditional upload
//! (docs/adr/0002-storage-backends.md has the why and the remaining risk).
//!
//! Every file `gdrive::Drive` writes carries, in its `appProperties`, a **claim id** made by
//! `files.generateIds`. Drive refuses a second `files.create` with an id already in use (409),
//! so creating a file with that id is a one-winner step. A conditional write of a file read
//! at version `V` with claim `C` (its etag is `make_etag(V, C)`):
//!
//! 1. Read the file's metadata; a `version` other than `V` is `Error::Changed`.
//! 2. Claim: `files.create` a small file `.claim-<C>` with id `C` beside it, whose own
//!    `appProperties` hold a fresh `NEXT_PROPERTY` id. 201 is `Claim::Won`; 409 means another
//!    machine claimed `C`: read that claim's age (`createdTime` against the answer's `Date`)
//!    and its `NEXT_PROPERTY`: `Claim::Taken`.
//! 3. Read the version again and ask `decide`. `Break(next)` repeats step 2 with id `next`
//!    (the claimer died: its lease ran out and the file never changed).
//! 4. `Write`: upload the new content with a fresh claim id in `CLAIM_PROPERTY`.
//! 5. Read the metadata once more; `check_written` must pass (else `Error::Server`).
//!
//! Claim files are kept for a day (a later push deletes older ones), so a claim id is never
//! free again while a slow machine may still hold an etag naming it.

use crate::error::{not_built, Result};

/// The `appProperties` key holding a file's claim id for its next write.
pub const CLAIM_PROPERTY: &str = "save_relay_claim";
/// The `appProperties` key of a claim file holding the id a lease-breaker claims.
pub const NEXT_PROPERTY: &str = "save_relay_next";
/// A claim older than this (seconds, by Drive's clock) whose file has not changed is stale.
pub const LEASE_SECS: u64 = 120;
/// A writer that has held its claim this long (by its own clock) gives up before writing.
pub const SLOW_SECS: u64 = 60;
/// How many stale claims in a row a writer may break before it gives up.
pub const MAX_HOPS: u32 = 3;

/// `"v<version>.<claim>"`: `make_etag(12, "1AbC")` -> `"v12.1AbC"`; an empty claim gives `"v12."`.
pub fn make_etag(version: u64, claim: &str) -> String {
    crate::not_built!("gdrive::guard::make_etag")
}

/// The inverse of `make_etag`: `"v12.1AbC"` -> `Some((12, "1AbC"))`, `"v12."` ->
/// `Some((12, ""))`. Anything else (no `v`, no `.`, a version that is not a whole number) is
/// `None`. The claim is everything after the first `.`.
pub fn parse_etag(etag: &str) -> Option<(u64, String)> {
    crate::not_built!("gdrive::guard::parse_etag")
}

/// The answer to a claim's `files.create`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// 201: this machine holds the claim.
    Won,
    /// 409: another machine created the claim `age_secs` ago (Drive's clock); `next` is its
    /// `NEXT_PROPERTY`, `None` when the claim file has none.
    Taken { age_secs: u64, next: Option<String> },
}

/// What the writer does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Upload now.
    Write,
    /// Another machine wrote, or is writing: `Error::Changed` (sync re-plans; M3 then asks).
    Changed,
    /// The claimer died: claim this id instead (one hop further).
    Break(String),
    /// This machine held its claim too long to write safely:
    /// `Error::Unreachable("drive: the write took too long")`, so the push is retried later.
    TooSlow,
}

/// The guard's decision after a claim, from the version read when the etag was made
/// (`expected`), the version now (`now_version`), the claim's answer, how long this machine
/// has held a `Won` claim (`held_secs`, its own clock), and how many stale claims it already
/// broke (`hops`):
///
/// | version | claim | condition | step |
/// |---|---|---|---|
/// | `now_version != expected` | any | | Changed |
/// | same | Won | `held_secs < SLOW_SECS` | Write |
/// | same | Won | `held_secs >= SLOW_SECS` | TooSlow |
/// | same | Taken | `age_secs < LEASE_SECS` | Changed |
/// | same | Taken, `next` is `None` | | Changed |
/// | same | Taken, `next` is `Some(n)` | `hops >= MAX_HOPS` | Changed |
/// | same | Taken, `next` is `Some(n)` | otherwise | Break(n) |
///
/// Rows are checked top to bottom; the first that fits wins. Pure.
pub fn decide(expected: u64, now_version: u64, claim: &Claim, held_secs: u64, hops: u32) -> Step {
    crate::not_built!("gdrive::guard::decide")
}

/// Step 5: the metadata read after the upload must show the version the upload answered
/// with and the claim id it set. Otherwise another machine wrote in between:
/// `Error::Server("drive: state changed during the write (version <seen>): run again")`.
pub fn check_written(
    written_version: u64,
    written_claim: &str,
    seen_version: u64,
    seen_claim: Option<&str>,
) -> Result<()> {
    not_built("gdrive::guard::check_written")
}
