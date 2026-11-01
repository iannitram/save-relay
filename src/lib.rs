//! save-relay: share one game's saves between machines through the service chosen at setup:
//! a Nextcloud server over WebDAV, Google Drive or Dropbox. A launcher calls `save-relay pull`
//! before Play and `save-relay push` after the game quits (SPEC.md M1-M10). Around that: the
//! open-elsewhere mark, the Steam wrap, the ludusavi data, save cards and the panel (M11-M15,
//! docs/adr/0003-baton-wrap-cards-panel.md). The map of this crate is `docs/code-map.md`.
//!
//! Skeleton (2026-10-31; M11-M15 2026-11-01): every module has its final public signatures. A function that is not
//! built yet returns `Err(Error::NotBuilt(..))`, or panics with "not built yet" when its final
//! signature cannot fail; each one is filled in by its own issue. MIT.

// Skeleton only: stubs ignore their inputs. Remove this line when the last stub is built.
#![allow(unused_variables)]

pub mod api;
pub mod card;
pub mod cli;
pub mod clients;
pub mod creds;
pub mod dropbox;
pub mod error;
pub mod fake;
pub mod gdrive;
pub mod glob;
pub mod http;
pub mod layout;
pub mod local;
pub mod ludusavi;
pub mod manifest;
pub mod oauth;
pub mod panel;
pub mod pending;
pub mod plan;
pub mod platform;
pub mod presence;
pub mod remote;
pub mod retention;
pub mod scan;
pub mod service;
pub mod stamp;
pub mod state;
pub mod steam;
pub mod sync;
pub mod transfer;
pub mod webdav;
pub mod wrap;

pub use error::{Error, NotBuilt, Result};

/// Panics with "not built yet: <what>"; the body of a stub whose final signature cannot fail.
/// Kept (allowed unused) until the last stub is built, then delete it with `error::not_built`.
#[allow(unused_macros)]
macro_rules! not_built {
    ($what:expr) => {
        panic!("not built yet: {}", $what)
    };
}
#[allow(unused_imports)]
pub(crate) use not_built;
