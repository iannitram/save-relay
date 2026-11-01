//! `FakeDrive`: the Drive v3 endpoints `gdrive::Drive` uses, over a file tree in memory.

use crate::error::{not_built, Result};
use crate::http::{Request, Response, Transport};

/// A hook run before the fake handles a request, with the request; it may call `send` on the
/// same fake (the lock is not held), so a test can slip a second machine's push between two
/// steps of the guard.
pub type Hook = Box<dyn FnMut(&Request)>;

/// Answers `files.list` (the `q` forms `gdrive::children_query` makes; pages of 2),
/// `files.get` (metadata and `alt=media`), `files.create` (with a client id from
/// `files.generateIds`: 409 when that id is in use, as Drive does), `files.update` (content,
/// name, `addParents`/`removeParents`, `appProperties`), `files.copy`, `files.delete`,
/// `files.generateIds`, `about`, and the Google token endpoint (refresh). Files have ids,
/// parents, `version` (grows on every change), `createdTime` from its own clock, and may share
/// a name, as on Drive. Every answer carries a `Date` header from that clock.
/// Interior mutability (a `Mutex`).
#[derive(Default)]
pub struct FakeDrive {
    _private: (),
}

impl FakeDrive {
    /// Its clock starts at `stamp` 1793476260 (2026-10-31T19:51:00Z).
    pub fn new() -> FakeDrive {
        FakeDrive::default()
    }

    /// While `false`, `send` returns `Error::Unreachable("fake drive is offline")`.
    pub fn set_reachable(&self, reachable: bool) {
        crate::not_built!("fake::FakeDrive::set_reachable")
    }

    /// Move its clock forward (lease expiry tests).
    pub fn advance(&self, secs: u64) {
        crate::not_built!("fake::FakeDrive::advance")
    }

    /// From now on the token endpoint answers `invalid_grant` and every access token is refused.
    pub fn revoke(&self) {
        crate::not_built!("fake::FakeDrive::revoke")
    }

    /// Every access token handed out so far gets 401.
    pub fn expire_access_tokens(&self) {
        crate::not_built!("fake::FakeDrive::expire_access_tokens")
    }

    /// Run `hook` before each request until it is replaced or cleared with `None`.
    pub fn set_hook(&self, hook: Option<Hook>) {
        crate::not_built!("fake::FakeDrive::set_hook")
    }

    /// Every file (not folder) as a `/`-joined path of names from My Drive, sorted; files
    /// with the same path appear once per copy. Works while unreachable.
    pub fn file_paths(&self) -> Vec<String> {
        crate::not_built!("fake::FakeDrive::file_paths")
    }

    /// `"<METHOD> <url>"` of every request so far, in order (no headers, no tokens).
    pub fn requests(&self) -> Vec<String> {
        crate::not_built!("fake::FakeDrive::requests")
    }
}

impl Transport for FakeDrive {
    fn send(&self, request: &Request) -> Result<Response> {
        not_built("fake::FakeDrive::send")
    }
}
