//! `FakeDropbox`: the Dropbox endpoints `dropbox::Dropbox` uses, over one App folder in memory.

use crate::error::{not_built, Result};
use crate::http::{Request, Response, Transport};

/// Answers `files/list_folder` (+ `/continue`, pages of 2 so paging is exercised),
/// `files/get_metadata`, `files/upload` (modes `add`, `overwrite`, `update` with
/// `strict_conflict`), `files/download` (`Dropbox-API-Result` header), `files/create_folder_v2`,
/// `files/copy_v2`, `files/move_v2`, `files/delete_v2`, `users/get_current_account`, and
/// `oauth2/token` (refresh). Like real Dropbox it creates missing parent folders on upload;
/// every write gives a new `rev`. Interior mutability (a `Mutex`).
#[derive(Debug, Default)]
pub struct FakeDropbox {
    _private: (),
}

impl FakeDropbox {
    pub fn new() -> FakeDropbox {
        FakeDropbox::default()
    }

    /// While `false`, `send` returns `Error::Unreachable("fake dropbox is offline")`.
    pub fn set_reachable(&self, reachable: bool) {
        crate::not_built!("fake::FakeDropbox::set_reachable")
    }

    /// From now on the token endpoint answers `invalid_grant` and every access token is refused.
    pub fn revoke(&self) {
        crate::not_built!("fake::FakeDropbox::revoke")
    }

    /// Every access token handed out so far gets 401 `expired_access_token`.
    pub fn expire_access_tokens(&self) {
        crate::not_built!("fake::FakeDropbox::expire_access_tokens")
    }

    /// Every file path in the App folder, `Remote` form, sorted. Works while unreachable.
    pub fn file_paths(&self) -> Vec<String> {
        crate::not_built!("fake::FakeDropbox::file_paths")
    }

    /// `"<METHOD> <url>"` of every request so far, in order (no headers, no tokens).
    pub fn requests(&self) -> Vec<String> {
        crate::not_built!("fake::FakeDropbox::requests")
    }
}

impl Transport for FakeDropbox {
    fn send(&self, request: &Request) -> Result<Response> {
        not_built("fake::FakeDropbox::send")
    }
}
