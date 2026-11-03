//! In-memory fakes of the Google Drive and Dropbox HTTP APIs, as `http::Transport`s, so the
//! real backends (`dropbox::Dropbox`, `gdrive::Drive`) and the sign-in refresh
//! (`oauth::TokenSource`) run in tests with no network and no account (SPEC.md M9: the same
//! tests against every service). Each fake answers only the requests its backend sends, with
//! the status codes and JSON the real API documents, and checks the bearer token.
//! `remote::MemoryRemote` is the fake one level up, for sync tests.

mod dropbox;
mod gdrive;

pub use dropbox::FakeDropbox;
pub use gdrive::FakeDrive;

/// The refresh token both fakes accept until `revoke`.
pub const REFRESH_TOKEN: &str = "fake-refresh-token";

/// The code the Dropbox fake accepts for a sign-in's code exchange (what the person types back).
pub const SIGN_IN_CODE: &str = "fake-sign-in-code";

/// The device code the Drive fake issues for its device flow and accepts when polled.
pub const DEVICE_CODE: &str = "fake-device-code";
