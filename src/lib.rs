//! save-relay: share one game's saves between machines through the service chosen at setup:
//! a Nextcloud server over WebDAV, Google Drive or Dropbox. A launcher calls `save-relay pull`
//! before Play and `save-relay push` after the game quits (SPEC.md M1-M10). Around that: the
//! open-elsewhere mark, the Steam wrap, the ludusavi data, save cards and the panel (M11-M15,
//! docs/adr/0003-baton-wrap-cards-panel.md). The map of this crate is `docs/code-map.md`.

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

pub use error::{Error, Result};
