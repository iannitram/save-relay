//! The engine's other layouts (#309): the explicit legacy mapping, and the inline-manifest
//! shape behind the two compatibility C functions (`game_files`), all over the one engine.

mod support_gamefiles;

use save_relay::error::Class;
use save_relay::game_files::{
    download_game_files, status_game_files, GameFileEntry, GameFilesManifest,
};
use save_relay::gamefiles::engine::{self, Options};
use save_relay::gamefiles::{Layout, Progress, State};
use save_relay::Error;
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use support_gamefiles::*;

const ROOT: &str = "acnh-universal-game/android";

/// A source whose files sit at `<prefix>/<path>`, with an optional `<prefix>/manifest.json`.
fn source_at(
    prefix: &str,
    manifest_json: Option<Vec<u8>>,
    files: &[(&str, &[u8])],
) -> MemorySource {
    let source = MemorySource::new("unused", &[]);
    let source = Arc::try_unwrap(source).ok().unwrap();
    {
        let mut map = source.files.lock().unwrap();
        map.clear();
        if let Some(bytes) = manifest_json {
            map.insert(format!("{prefix}/manifest.json"), bytes);
        }
        for (path, bytes) in files {
            map.insert(format!("{prefix}/{path}"), bytes.to_vec());
        }
    }
    source
}

fn run(
    source: &MemorySource,
    app: &str,
    dest: &Path,
    state: &Path,
    layout: Layout,
) -> save_relay::Result<Progress> {
    let options = Options {
        chunk: 1000,
        retries: 0,
        sleep: Arc::new(|_| {}),
        layout,
        ..Options::default()
    };
    engine::run(
        source,
        app,
        dest,
        state,
        &options,
        &AtomicBool::new(false),
        &mut |_| {},
    )
}

#[test]
fn legacy_layout_reads_root_manifest_without_app_id_and_files_directly_under_root() {
    let a = body(1, 2500);
    let b = body(2, 10);
    let manifest = format!(
        r#"{{"version":1,"files":[{{"path":"data/a.bin","size":{},"sha256":"{}"}},{{"path":"b.bin","size":{},"sha256":"{}"}}]}}"#,
        a.len(),
        sha(&a),
        b.len(),
        sha(&b)
    );
    let source = source_at(
        ROOT,
        Some(manifest.into_bytes()),
        &[("data/a.bin", &a), ("b.bin", &b)],
    );
    let base = folder("legacy-ok");
    let layout = Layout::Legacy { root: ROOT.into() };
    let done = run(
        &source,
        "acnh-universal",
        &base.join("g"),
        &base.join("s"),
        layout,
    )
    .unwrap();
    assert_eq!(done.state, State::Complete);
    assert_eq!(fs::read(base.join("g/data/a.bin")).unwrap(), a);
    assert!(source
        .fetches()
        .iter()
        .all(|(p, _, _)| p.starts_with("acnh-universal-game/android/")));
}

#[test]
fn legacy_layout_refuses_a_manifest_that_names_player_data_before_writing_anything() {
    let a = body(1, 10);
    let manifest = format!(
        r#"{{"version":1,"files":[{{"path":"user/save.dat","size":10,"sha256":"{}"}}]}}"#,
        sha(&a)
    );
    let source = source_at(ROOT, Some(manifest.into_bytes()), &[("user/save.dat", &a)]);
    let base = folder("legacy-user");
    let layout = Layout::Legacy { root: ROOT.into() };
    let err = run(
        &source,
        "acnh-universal",
        &base.join("g"),
        &base.join("s"),
        layout,
    )
    .unwrap_err();
    assert!(matches!(err, Error::ManifestInvalid(_)), "{err:?}");
    assert!(source.fetches().is_empty());
    assert!(!base.join("g/user").exists());
}

#[test]
fn a_layout_root_that_is_not_a_plain_relative_path_is_refused() {
    let source = source_at("x", None, &[]);
    let base = folder("legacy-root");
    for root in ["", "../x", "/abs", "a//b"] {
        let layout = Layout::Legacy { root: root.into() };
        let err = run(
            &source,
            "acnh-universal",
            &base.join("g"),
            &base.join("s"),
            layout,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Usage(_)), "{root:?}: {err:?}");
    }
}

fn compat(game: &str, remote_root: Option<&str>, files: &[(&str, &[u8])]) -> GameFilesManifest {
    GameFilesManifest {
        version: 1,
        game_id: game.into(),
        remote_root: remote_root.map(str::to_string),
        files: files
            .iter()
            .map(|(p, b)| GameFileEntry {
                path: (*p).into(),
                size: b.len() as u64,
                sha256: sha(b),
            })
            .collect(),
    }
}

#[test]
fn compat_download_uses_the_main_remote_layout_state_inside_dest_and_resumes() {
    let a = body(1, 3000);
    let b = body(2, 2500);
    let manifest = compat(
        "uncharted-1",
        None,
        &[("u1data/a.bin", &a), ("u1data/b.bin", &b)],
    );
    let source = source_at(
        "game-files/uncharted-1",
        None,
        &[("u1data/a.bin", &a), ("u1data/b.bin", &b)],
    );
    let dest = folder("compat-dl").join("game");

    // The first pass dies after one chunk; checkpoints sit in <dest>/.save-relay-game-files.
    let mut sent = 0;
    source.set_hook(move |_, _, _, answer| {
        sent += 1;
        if sent > 1 {
            Err(Error::Unreachable("power".into()))
        } else {
            Ok(answer)
        }
    });
    assert!(download_game_files(&source, &manifest, &dest, 1024, &[]).is_err());
    assert!(dest.join(".save-relay-game-files").is_dir());
    let status = status_game_files(&manifest, &dest).unwrap();
    assert_eq!(
        (
            status.completed_files,
            status.partial_files,
            status.bytes_on_disk
        ),
        (0, 1, 1024)
    );
    assert!(!status.complete);

    source.clear_hook();
    source.clear_log();
    let report = download_game_files(&source, &manifest, &dest, 1024, &[]).unwrap();
    assert_eq!((report.downloaded_files, report.skipped_files), (2, 0));
    assert_eq!(report.bytes_downloaded, (3000 - 1024) + 2500);
    assert!(report.complete);
    assert!(source
        .fetches()
        .iter()
        .all(|(p, _, _)| p.starts_with("game-files/uncharted-1/u1data/")));
    assert_eq!(fs::read(dest.join("u1data/a.bin")).unwrap(), a);

    let again = download_game_files(&source, &manifest, &dest, 1024, &[]).unwrap();
    assert_eq!(
        (
            again.downloaded_files,
            again.skipped_files,
            again.bytes_downloaded
        ),
        (0, 2, 0)
    );
    let status = status_game_files(&manifest, &dest).unwrap();
    assert!(status.complete);
    assert_eq!(status.completed_files, 2);
    assert_eq!(status.bytes_on_disk, 5500);
}

#[test]
fn compat_never_overwrites_a_mismatching_existing_file() {
    let a = body(1, 100);
    let manifest = compat("g1", None, &[("a.bin", &a)]);
    let source = source_at("game-files/g1", None, &[("a.bin", &a)]);
    let dest = folder("compat-conflict").join("game");
    fs::create_dir_all(&dest).unwrap();
    fs::write(dest.join("a.bin"), b"the player's own file").unwrap();
    let err = download_game_files(&source, &manifest, &dest, 1024, &[]).unwrap_err();
    assert!(matches!(err, Error::DestinationConflict(_)), "{err:?}");
    assert_eq!(err.class(), Class::Fatal);
    assert_eq!(
        fs::read(dest.join("a.bin")).unwrap(),
        b"the player's own file"
    );
}

#[test]
fn compat_remote_root_is_honored_but_must_be_a_relative_path() {
    let a = body(1, 100);
    let manifest = compat("g1", Some("/custom/root/"), &[("a.bin", &a)]);
    let source = source_at("custom/root", None, &[("a.bin", &a)]);
    let dest = folder("compat-root").join("game");
    download_game_files(&source, &manifest, &dest, 1024, &[]).unwrap();
    assert_eq!(fs::read(dest.join("a.bin")).unwrap(), a);
    for root in ["../up", "a/../b", "a\\b", "C:x"] {
        let bad = compat("g1", Some(root), &[("a.bin", &a)]);
        assert!(bad.validate().is_err(), "{root}");
    }
}

#[test]
fn compat_manifest_rejects_what_the_engine_rejects() {
    let a = body(1, 10);
    for path in ["../x", "a/con", ".save-relay-game-files/x", "a/../b"] {
        assert!(
            compat("g1", None, &[(path, &a)]).validate().is_err(),
            "{path}"
        );
    }
    assert!(compat("Bad/Id", None, &[("a", &a)]).validate().is_err());
    // Upper-case digests stay accepted, as in #324.
    let mut upper = compat("g1", None, &[("a", &a)]);
    upper.files[0].sha256 = upper.files[0].sha256.to_uppercase();
    upper.validate().unwrap();
    assert!(GameFilesManifest::parse("{").is_err());
}

#[test]
fn a_nested_state_folder_is_only_allowed_for_the_inline_layout_at_its_fixed_name() {
    let a = body(1, 10);
    let source = source_at(
        "game-files/g1",
        Some(manifest("g1", &[("a", &a)])),
        &[("a", &a)],
    );
    let dest = folder("nested").join("game");
    // The standard layout still refuses a state folder inside the game folder.
    let err = run(&source, "g1", &dest, &dest.join("state"), Layout::Standard).unwrap_err();
    assert!(matches!(err, Error::DestinationConflict(_)), "{err:?}");
    // The inline layout accepts only its own fixed folder.
    let inline = Layout::Inline {
        manifest: manifest("g1", &[("a", &a)]),
        prefix: "game-files/g1".into(),
    };
    let err = run(
        &source,
        "g1",
        &dest,
        &dest.join("elsewhere"),
        inline.clone(),
    )
    .unwrap_err();
    assert!(matches!(err, Error::DestinationConflict(_)), "{err:?}");
    let ok = run(
        &source,
        "g1",
        &dest,
        &dest.join(".save-relay-game-files"),
        inline,
    )
    .unwrap();
    assert_eq!(ok.state, State::Complete);
}

/// The compatibility wrapper fills `Options::protected` from its `protected` argument: a game
/// folder that is or sits inside a saves folder is refused before anything is written. One that
/// holds a saves folder is accepted when no manifest path lands in or above it.
#[test]
fn compat_download_refuses_a_destination_in_a_saves_folder() {
    let a = body(3, 1500);
    let manifest = compat("protected-game", None, &[("a.bin", &a)]);
    let source = source_at("game-files/protected-game", None, &[("a.bin", &a)]);
    let root = folder("compat-protected");
    let saves = root.join("saves");
    fs::create_dir_all(&saves).unwrap();
    for dest in [saves.clone(), saves.join("game")] {
        let error = download_game_files(
            &source,
            &manifest,
            &dest,
            1024,
            std::slice::from_ref(&saves),
        )
        .unwrap_err();
        assert_eq!(error.code(), "destination_conflict", "{dest:?}: {error:?}");
    }
    assert_eq!(fs::read_dir(&saves).unwrap().count(), 0, "nothing written");
    assert!(source.fetches().is_empty(), "no request before the refusal");
    // A folder of its own still works with the same list.
    let ok = download_game_files(
        &source,
        &manifest,
        &root.join("game"),
        1024,
        std::slice::from_ref(&saves),
    )
    .unwrap();
    assert!(ok.complete);
    // A folder that holds the saves folder works while the manifest keeps clear of it...
    let holder = folder("compat-protected-holder");
    let held = holder.join("saves");
    fs::create_dir_all(&held).unwrap();
    let ok = download_game_files(
        &source,
        &manifest,
        &holder,
        1024,
        std::slice::from_ref(&held),
    )
    .unwrap();
    assert!(ok.complete);
    // ...and is refused when a manifest path lands in it.
    let inside = compat("protected-game", None, &[("saves/a.bin", &a)]);
    let inside_source = source_at("game-files/protected-game", None, &[("saves/a.bin", &a)]);
    let other = folder("compat-protected-hit");
    let held = other.join("saves");
    fs::create_dir_all(&held).unwrap();
    let error = download_game_files(
        &inside_source,
        &inside,
        &other,
        1024,
        std::slice::from_ref(&held),
    )
    .unwrap_err();
    assert_eq!(error.code(), "destination_conflict", "{error:?}");
    assert_eq!(fs::read_dir(&held).unwrap().count(), 0, "nothing written");
}
