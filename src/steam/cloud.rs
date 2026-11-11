//! Steam's public appdetails answer and its local one-week cache (ADR 0004).

use crate::error::{Error, Result};
use crate::http::{Request, Transport};
use crate::platform::AppDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

/// Keep store answers for a week.
pub const REFRESH_SECS: u64 = 7 * 24 * 60 * 60;
const FORMAT: u32 = 1;
const MAX_PER_RUN: usize = 100;

/// Steam's public appdetails endpoint for one app, requesting the fields this parser needs.
pub fn appdetails_url(app_id: u32) -> String {
    format!(
        "https://store.steampowered.com/api/appdetails?appids={app_id}&filters=basic,categories"
    )
}

/// What the public store says about one app id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreAnswer {
    /// A store game, and whether its categories include Steam Cloud (23).
    Game { cloud: bool },
    /// A successful answer for a product whose type is not `game`.
    NotAGame,
    /// Steam returned `success: false`; the app type is unknown.
    Unknown,
}

/// The answer exposed to installed-game consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudState {
    Yes,
    No,
    NotAGame,
    Unknown,
}

/// Counts the requests and answers from one refresh. Store throttling or no network ends a
/// run early without discarding answers already written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RefreshReport {
    pub asked: usize,
    pub stored: usize,
    pub stopped_early: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    answer: StoredAnswer,
    checked: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StoredAnswer {
    Yes,
    No,
    NotAGame,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FileData {
    format: u32,
    apps: BTreeMap<u32, Entry>,
}

/// The Steam Cloud answers cached on this machine in `<data>/save-relay/steam/cloud.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    path: PathBuf,
    apps: BTreeMap<u32, Entry>,
}

impl Cache {
    /// Open this machine's cache. Missing means empty; malformed or unsupported data is invalid.
    pub fn open(dirs: &AppDirs) -> Result<Cache> {
        let path = dirs.data.join("steam/cloud.json");
        let apps = read_cache(&path)?;
        Ok(Cache { path, apps })
    }

    /// The cached answer for `app_id`, or unknown when there is no answer.
    pub fn state(&self, app_id: u32) -> CloudState {
        match self.apps.get(&app_id).map(|entry| entry.answer) {
            Some(StoredAnswer::Yes) => CloudState::Yes,
            Some(StoredAnswer::No) => CloudState::No,
            Some(StoredAnswer::NotAGame) => CloudState::NotAGame,
            Some(StoredAnswer::Unknown) | None => CloudState::Unknown,
        }
    }

    /// Fetch uncached or expired answers, at most 100 per run. Each answer is saved before the
    /// next request, so a stopped or interrupted run keeps completed work. `Unknown` is cached
    /// too: Steam `success: false` is retried after a week, not on every command.
    pub fn refresh(
        &mut self,
        transport: &dyn Transport,
        app_ids: &[u32],
        now: u64,
        force: bool,
    ) -> Result<RefreshReport> {
        let mut report = RefreshReport::default();
        let mut seen = BTreeSet::new();
        for app_id in app_ids.iter().copied().filter(|id| seen.insert(*id)) {
            if report.asked == MAX_PER_RUN {
                break;
            }
            let fresh = self
                .apps
                .get(&app_id)
                .is_some_and(|entry| now.saturating_sub(entry.checked) < REFRESH_SECS);
            if !force && fresh {
                continue;
            }

            report.asked += 1;
            let url = appdetails_url(app_id);
            let response = match transport.send(&Request::new("GET", &url)) {
                Ok(response) => response,
                Err(Error::Unreachable(_)) => {
                    report.stopped_early = true;
                    break;
                }
                Err(error) => return Err(error),
            };
            if !(200..300).contains(&response.status) {
                report.stopped_early = true;
                break;
            }
            let text = response.text();
            let answer = parse_appdetails(&text, app_id)?;
            let entry = Entry {
                answer: match answer {
                    StoreAnswer::Game { cloud: true } => StoredAnswer::Yes,
                    StoreAnswer::Game { cloud: false } => StoredAnswer::No,
                    StoreAnswer::NotAGame => StoredAnswer::NotAGame,
                    StoreAnswer::Unknown => StoredAnswer::Unknown,
                },
                checked: now,
            };
            if self.merge_write(app_id, entry)? {
                report.stored += 1;
            }
        }
        Ok(report)
    }

    // Lock only the merge/write step, never the network request. On collisions the most recent
    // checked timestamp wins, preserving a newer answer from another process.
    fn merge_write(&mut self, app_id: u32, entry: Entry) -> Result<bool> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::Io("cache has no parent".into()))?;
        fs::create_dir_all(parent)?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(parent.join("cloud.lock"))?;
        lock.lock()?;
        let mut apps = read_cache(&self.path)?;
        let replace = apps
            .get(&app_id)
            .is_none_or(|existing| entry.checked >= existing.checked);
        if replace {
            apps.insert(app_id, entry);
            write_cache(&self.path, &apps)?;
        }
        self.apps = apps;
        Ok(replace)
    }
}

fn read_cache(path: &Path) -> Result<BTreeMap<u32, Entry>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(Error::StateInvalid(error.to_string())),
    };
    let data: FileData = serde_json::from_str(&text)
        .map_err(|error| Error::StateInvalid(format!("steam cloud cache: {error}")))?;
    if data.format != FORMAT {
        return Err(Error::StateInvalid(format!(
            "steam cloud cache: unsupported format {}",
            data.format
        )));
    }
    Ok(data.apps)
}

fn write_cache(path: &Path, apps: &BTreeMap<u32, Entry>) -> Result<()> {
    #[derive(Serialize)]
    struct FileView<'a> {
        format: u32,
        apps: &'a BTreeMap<u32, Entry>,
    }
    crate::local::write_json(
        path,
        &FileView {
            format: FORMAT,
            apps,
        },
    )
}

/// Parse one appdetails response. Invalid JSON or a missing app id is a state error.
pub fn parse_appdetails(text: &str, app_id: u32) -> Result<StoreAnswer> {
    let invalid = |reason: &str| Error::StateInvalid(format!("steam store: {reason}"));
    let root: Value = serde_json::from_str(text).map_err(|_| invalid("invalid JSON"))?;
    let answer = root
        .get(app_id.to_string())
        .ok_or_else(|| invalid("missing app id"))?;
    if answer.get("success").and_then(Value::as_bool) == Some(false) {
        return Ok(StoreAnswer::Unknown);
    }
    if answer.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(invalid("missing success value"));
    }
    let data = answer.get("data").ok_or_else(|| invalid("missing data"))?;
    let kind = data
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing or invalid type"))?;
    if kind != "game" {
        return Ok(StoreAnswer::NotAGame);
    }
    let categories = match data.get("categories") {
        None => None,
        Some(value) => Some(
            value
                .as_array()
                .ok_or_else(|| invalid("invalid categories"))?,
        ),
    };
    let mut cloud = false;
    for category in categories.into_iter().flatten() {
        let id = category
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid("invalid category id"))?;
        cloud |= id == 23;
    }
    Ok(StoreAnswer::Game { cloud })
}
