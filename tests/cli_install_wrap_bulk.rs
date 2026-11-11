#[path = "common/steam_games.rs"]
#[allow(dead_code)]
mod steam_games_fixture;

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Machine {
    steam: PathBuf,
    data: PathBuf,
    config: PathBuf,
    home: PathBuf,
}

impl Machine {
    fn new(index_ids: &[u32]) -> Machine {
        let root = std::env::temp_dir().join(format!(
            "save-relay-bulk-wrap-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let steam = steam_games_fixture::fixture(&root);
        let home = root.join("home");
        let (data, config) = if cfg!(target_os = "macos") {
            let support = home.join("Library/Application Support");
            (support.clone(), support)
        } else {
            (root.join("xdg-data"), root.join("xdg-config"))
        };
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&home).unwrap();
        Machine {
            steam,
            data,
            config,
            home,
        }
        .write_caches(index_ids)
    }

    fn write_caches(self, index_ids: &[u32]) -> Machine {
        let checked = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let apps = json!({
            "504230": {"answer":"yes", "checked":checked},
            "504231": {"answer":"no", "checked":checked},
            "504232": {"answer":"unknown", "checked":checked},
            "504233": {"answer":"not-a-game", "checked":checked}
        });
        let cloud_dir = self.data.join("save-relay/steam");
        fs::create_dir_all(&cloud_dir).unwrap();
        fs::write(
            cloud_dir.join("cloud.json"),
            serde_json::to_vec(&json!({"format":1, "apps":apps})).unwrap(),
        )
        .unwrap();

        let ludusavi_dir = self.data.join("save-relay/ludusavi");
        fs::create_dir_all(&ludusavi_dir).unwrap();
        fs::write(ludusavi_dir.join("manifest.yaml"), "{}").unwrap();
        let names = [
            (504230, "Cloud Game"),
            (504231, "No Cloud Game"),
            (504232, "Unknown Game"),
            (504233, "Runtime Tool"),
        ];
        let index = names
            .into_iter()
            .filter(|(id, _)| index_ids.contains(id))
            .map(|(id, name)| (id.to_string(), Value::String(name.into())))
            .collect::<serde_json::Map<String, Value>>();
        fs::write(
            ludusavi_dir.join("steam-index.json"),
            serde_json::to_vec(&json!({"format":1, "etag":null, "checked":checked, "apps":index}))
                .unwrap(),
        )
        .unwrap();
        fs::write(
            ludusavi_dir.join("meta.json"),
            serde_json::to_vec(&json!({"format":1, "etag":null, "checked":checked})).unwrap(),
        )
        .unwrap();
        self
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_save-relay"))
            .args(args)
            .env("SAVE_RELAY_STEAM_DIR", &self.steam)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_DATA_HOME", &self.data)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("LOCALAPPDATA", &self.data)
            .env("APPDATA", &self.config)
            .output()
            .unwrap()
    }

    fn localconfig(&self) -> PathBuf {
        self.steam.join("userdata/123/config/localconfig.vdf")
    }

    fn launch_option(&self, app_id: u32) -> Option<String> {
        let text = fs::read_to_string(self.localconfig()).unwrap();
        let parsed = save_relay::steam::vdf::parse(&text).unwrap();
        save_relay::steam::launch::launch_options(&parsed, app_id)
    }
}

fn response(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn has_skip(data: &Value, app_id: u32, reason: &str) -> bool {
    data["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .any(|skip| skip["app_id"] == app_id && skip["reason"] == reason)
}

#[test]
fn dry_run_lists_only_no_cloud_with_saves_and_writes_nothing() {
    let machine = Machine::new(&[504231, 504232]);
    let before = fs::read(machine.localconfig()).unwrap();
    let data = response(machine.run(&["install-wrap", "--no-steam-cloud", "--dry-run", "--json"]))
        ["data"]
        .clone();
    assert_eq!(data["changed"].as_array().unwrap().len(), 1);
    assert_eq!(data["changed"][0]["app_id"], 504231);
    assert!(has_skip(&data, 504232, "steam_cloud_unknown"));
    assert_eq!(data["skipped"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read(machine.localconfig()).unwrap(), before);
    assert_eq!(
        machine.launch_option(504230).as_deref(),
        Some("\"save-relay\" wrap -- %command%")
    );
    assert!(!machine
        .launch_option(504231)
        .as_deref()
        .is_some_and(save_relay::steam::launch::is_wrapped));
    assert!(!machine
        .steam
        .join("userdata/.save-relay-install-wrap.lock")
        .exists());
    assert!(fs::read_dir(machine.steam.join("userdata/123/config"))
        .unwrap()
        .all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".bak")
        }));
}

#[test]
fn real_run_wraps_only_the_no_cloud_game_and_reports_unknown() {
    let machine = Machine::new(&[504231, 504232]);
    let before = fs::read(machine.localconfig()).unwrap();
    let data =
        response(machine.run(&["install-wrap", "--no-steam-cloud", "--json"]))["data"].clone();
    assert_eq!(data["changed"].as_array().unwrap().len(), 1);
    assert_eq!(data["changed"][0]["app_id"], 504231);
    assert!(has_skip(&data, 504232, "steam_cloud_unknown"));
    assert!(save_relay::steam::launch::is_wrapped(
        &machine.launch_option(504231).unwrap()
    ));
    assert_eq!(
        machine.launch_option(504230).as_deref(),
        Some("\"save-relay\" wrap -- %command%")
    );
    assert_eq!(machine.launch_option(504232), None);
    assert_ne!(fs::read(machine.localconfig()).unwrap(), before);
    assert_eq!(data["backups"].as_array().unwrap().len(), 1);
    assert!(Path::new(data["backups"][0].as_str().unwrap()).is_file());
    assert!(!machine
        .steam
        .join("userdata/.save-relay-install-wrap.lock")
        .exists());
}

#[test]
fn no_eligible_games_succeeds_with_skips_even_without_a_steam_user() {
    let machine = Machine::new(&[504232]);
    fs::remove_dir_all(machine.steam.join("userdata")).unwrap();
    let data =
        response(machine.run(&["install-wrap", "--no-steam-cloud", "--json"]))["data"].clone();
    assert!(data["changed"].as_array().unwrap().is_empty());
    assert!(data["backups"].as_array().unwrap().is_empty());
    assert!(has_skip(&data, 504231, "no_saves_entry"));
    assert!(has_skip(&data, 504232, "steam_cloud_unknown"));
}

#[test]
fn cloud_yes_game_is_never_reported_as_skipped() {
    let machine = Machine::new(&[504231, 504232]);
    let data = response(machine.run(&["install-wrap", "--no-steam-cloud", "--dry-run", "--json"]))
        ["data"]
        .clone();
    assert!(!data["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .any(|skip| skip["app_id"] == 504230));
}

#[cfg(target_os = "linux")]
#[test]
fn steam_running_guard_precedes_cache_reads() {
    let machine = Machine::new(&[504231, 504232]);
    let pid_dir = machine.home.join(".steam");
    fs::create_dir_all(&pid_dir).unwrap();
    fs::write(pid_dir.join("steam.pid"), std::process::id().to_string()).unwrap();
    fs::write(
        machine.data.join("save-relay/steam/cloud.json"),
        "malformed cache",
    )
    .unwrap();
    let output = machine.run(&["install-wrap", "--no-steam-cloud", "--json"]);
    assert!(!output.status.success());
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], "steam_running");
}
