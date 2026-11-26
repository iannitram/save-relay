//! Shared by the game-files tests: a manifest builder, an in-memory `GameSource` whose answers a
//! test can bend, and a real HTTP server (std sockets) that serves Dropbox-shaped ranged
//! downloads, reached through the real `UreqTransport`.
#![allow(dead_code)]

use save_relay::gamefiles::{GameSource, RangeAnswer};
use save_relay::Error;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[cfg(feature = "http")]
mod net;
#[cfg(feature = "http")]
#[allow(unused_imports)]
pub use net::*;

pub fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A deterministic body of `len` bytes that differs from place to place.
pub fn body(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| {
            (i as u8)
                .wrapping_mul(31)
                .wrapping_add(seed)
                .wrapping_add((i >> 8) as u8)
        })
        .collect()
}

/// The manifest JSON for `files` (path, bytes).
pub fn manifest(app: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let entries: Vec<String> = files
        .iter()
        .map(|(path, bytes)| {
            format!(
                r#"{{"path":"{path}","size":{},"sha256":"{}"}}"#,
                bytes.len(),
                sha(bytes)
            )
        })
        .collect();
    format!(
        r#"{{"version":1,"app_id":"{app}","files":[{}]}}"#,
        entries.join(",")
    )
    .into_bytes()
}

/// A fresh empty folder under the system temp dir.
pub fn folder(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("sr-gf-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

type Hook = Box<dyn FnMut(&str, u64, u64, RangeAnswer) -> save_relay::Result<RangeAnswer> + Send>;

/// A `GameSource` over a map of service paths to bytes. Answers are honest 206s with a
/// `rev` per file unless `hook` bends them.
pub struct MemorySource {
    pub files: Mutex<HashMap<String, Vec<u8>>>,
    pub revs: Mutex<HashMap<String, String>>,
    /// Every fetch: (path, start, len).
    pub log: Mutex<Vec<(String, u64, u64)>>,
    pub hook: Mutex<Option<Hook>>,
    /// Fail the next n fetches with `Unreachable`.
    pub fail_next: Mutex<u32>,
}

impl MemorySource {
    pub fn new(app: &str, files: &[(&str, &[u8])]) -> Arc<MemorySource> {
        let source = MemorySource {
            files: Mutex::new(HashMap::new()),
            revs: Mutex::new(HashMap::new()),
            log: Mutex::new(Vec::new()),
            hook: Mutex::new(None),
            fail_next: Mutex::new(0),
        };
        source.files.lock().unwrap().insert(
            format!("game-files/{app}/manifest.json"),
            manifest(app, files),
        );
        for (path, bytes) in files {
            source
                .files
                .lock()
                .unwrap()
                .insert(format!("game-files/{app}/files/{path}"), bytes.to_vec());
        }
        Arc::new(source)
    }
    pub fn set_hook(
        &self,
        hook: impl FnMut(&str, u64, u64, RangeAnswer) -> save_relay::Result<RangeAnswer>
            + Send
            + 'static,
    ) {
        *self.hook.lock().unwrap() = Some(Box::new(hook));
    }
    pub fn clear_hook(&self) {
        *self.hook.lock().unwrap() = None;
    }
    pub fn fetches(&self) -> Vec<(String, u64, u64)> {
        self.log.lock().unwrap().clone()
    }
    pub fn clear_log(&self) {
        self.log.lock().unwrap().clear();
    }
    pub fn bytes_fetched(&self) -> u64 {
        self.log.lock().unwrap().iter().map(|(_, _, len)| len).sum()
    }
}

impl GameSource for MemorySource {
    fn service(&self) -> &'static str {
        "memory"
    }
    fn read_bounded(&self, path: &str, max: usize) -> save_relay::Result<Vec<u8>> {
        let bytes = self
            .files
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| Error::NotFound(path.into()))?;
        if bytes.len() > max {
            return Err(Error::Server("exceeds the limit".into()));
        }
        Ok(bytes)
    }
    fn fetch(&self, path: &str, start: u64, len: u64) -> save_relay::Result<RangeAnswer> {
        self.log.lock().unwrap().push((path.into(), start, len));
        {
            let mut fail = self.fail_next.lock().unwrap();
            if *fail > 0 {
                *fail -= 1;
                return Err(Error::Unreachable("down".into()));
            }
        }
        let bytes = self
            .files
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| Error::NotFound(path.into()))?;
        let end = (start + len).min(bytes.len() as u64);
        let rev = self
            .revs
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .unwrap_or_else(|| "rev-1".into());
        let answer = RangeAnswer {
            status: 206,
            content_range: Some(format!("bytes {start}-{}/{}", end - 1, bytes.len())),
            content_encoding: None,
            content_type: Some("application/octet-stream".into()),
            identity: Some(rev),
            body: bytes[start as usize..end as usize].to_vec(),
        };
        match self.hook.lock().unwrap().as_mut() {
            Some(hook) => hook(path, start, len, answer),
            None => Ok(answer),
        }
    }
}
