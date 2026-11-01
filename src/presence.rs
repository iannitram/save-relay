//! The open-elsewhere mark (SPEC.md M11; docs/adr/0003-baton-wrap-cards-panel.md): a small
//! `<root>/<game>/playing.json` beside `state.json` that says which machine has the game open.
//!
//! ```json
//! {"format":1,"device":"amd-handheld","since":"2026-11-01T19:12:00Z","revision_read":7}
//! ```
//!
//! `pull` writes it (`Precondition::Any`, after its own work, even on `noop`; not when
//! offline); `push` removes it only when it names this device, never another machine's. A
//! `deferred` push leaves it. It never expires: the launcher asks (Wait or Play anyway), it
//! never blocks on its own. It is advice, not a lock: the M3 and M9 checks still guard
//! `state.json`. Pure.

use crate::error::{not_built, Result};
use serde::{Deserialize, Serialize};

/// The mark format this build reads and writes.
pub const FORMAT: u32 = 1;

/// The mark's file name inside a game's folder (`layout::Layout::playing_path`).
pub const FILE: &str = "playing.json";

/// `playing.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub format: u32,
    /// The machine that has the game open (`layout::device_slug` form).
    pub device: String,
    /// When its `pull` ran, `stamp::rfc3339` form.
    pub since: String,
    /// The server revision that `pull` left the machine at; `null` when the server had none.
    pub revision_read: Option<u64>,
}

impl Mark {
    /// Parse and check: `format == FORMAT` (a newer one says "made by a newer save-relay"),
    /// `device` not empty, `since` parses as RFC 3339. Any failure is `Error::StateInvalid`.
    pub fn from_json(text: &str) -> Result<Mark> {
        not_built("presence::Mark::from_json")
    }

    /// Compact JSON on one line, fields in declaration order, no trailing newline.
    pub fn to_json(&self) -> String {
        crate::not_built!("presence::Mark::to_json")
    }
}

/// What the mark means for this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hold {
    /// No mark: nobody has the game open.
    Free,
    /// This machine's own mark (it never pushed after its last pull: a crash, or a deferred push).
    Mine,
    /// Another machine has the game open: `plan::decide_with_mark` answers `Held` at launch.
    Elsewhere { device: String, since: String },
}

/// `None` is `Free`; a mark whose `device` equals `this_device` is `Mine`; any other is
/// `Elsewhere` with that mark's device and `since`.
pub fn check(mark: Option<&Mark>, this_device: &str) -> Hold {
    crate::not_built!("presence::check")
}
