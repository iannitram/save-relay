//! The three backends the M9 suites run against. Each call builds a FRESH backend (and fake
//! server), so no scenario sees another's files, and hands it to the check with a switch that
//! makes that backend unreachable. `tests/remote_contract.rs` and `tests/sync_services.rs` both
//! use these, so the two suites cannot drift apart on how a backend is built.

use save_relay::clients::{Client, GOOGLE_DESKTOP};
use save_relay::creds::Secret;
use save_relay::dropbox::Dropbox;
use save_relay::fake::{FakeDrive, FakeDropbox, REFRESH_TOKEN};
use save_relay::gdrive::Drive;
use save_relay::oauth::{TokenSource, DROPBOX, GOOGLE};
use save_relay::remote::{MemoryRemote, Remote};
use std::sync::Arc;

/// The fixed clock the Drive fake and its token source run on.
const START: u64 = 1_793_476_260;

fn clock() -> u64 {
    START
}

/// The in-memory server every sync test used before M9.
pub fn memory(check: impl Fn(&dyn Remote, &dyn Fn(bool))) {
    let remote = MemoryRemote::new();
    check(&remote, &|reachable| remote.set_reachable(reachable));
}

/// Dropbox over `FakeDropbox`; any retry is a bug in the test (the fake never asks for one).
#[allow(clippy::arc_with_non_send_sync)]
pub fn dropbox(check: impl Fn(&dyn Remote, &dyn Fn(bool))) {
    let fake = Arc::new(FakeDropbox::new());
    let tokens = TokenSource::new(
        &DROPBOX,
        Client {
            id: "test",
            secret: None,
        },
        Secret::new(REFRESH_TOKEN),
        fake.clone(),
        || 1000,
    );
    let backend = Dropbox::with_sleep(tokens, fake.clone(), |_| panic!("unexpected retry"));
    check(&backend, &|reachable| fake.set_reachable(reachable));
}

/// Google Drive over `FakeDrive`; any retry is a bug in the test (the fake never asks for one).
#[allow(clippy::arc_with_non_send_sync)]
pub fn drive(check: impl Fn(&dyn Remote, &dyn Fn(bool))) {
    let fake = Arc::new(FakeDrive::new());
    let tokens = TokenSource::new(
        &GOOGLE,
        GOOGLE_DESKTOP,
        Secret::new(REFRESH_TOKEN),
        fake.clone(),
        clock,
    );
    let backend =
        Drive::new(tokens, fake.clone(), clock).with_wait(Arc::new(|_| panic!("unexpected retry")));
    check(&backend, &|reachable| fake.set_reachable(reachable));
}
