//! `exec: true` manifest entries are published executable on Unix (mode 0755) and left alone
//! elsewhere; an entry without it keeps the default mode (docs/assets.md "Manifest v1").

mod support_gamefiles;

use save_relay::gamefiles::engine::{self, Options};
use save_relay::gamefiles::State;
use std::fs;
use std::sync::atomic::AtomicBool;
use support_gamefiles::*;

const APP: &str = "game-exec";

#[test]
fn exec_entries_are_published_executable_and_others_are_not() {
    let program = body(1, 2500);
    let data = body(2, 300);
    let source = MemorySource::new(APP, &[("bin/game", &program), ("data/a.bin", &data)]);
    let json = format!(
        r#"{{"version":1,"app_id":"{APP}","build_id":"b1","nx_game_abi":"nx-1","files":[{{"path":"bin/game","size":{},"sha256":"{}","exec":true}},{{"path":"data/a.bin","size":{},"sha256":"{}"}}]}}"#,
        program.len(),
        sha(&program),
        data.len(),
        sha(&data)
    );
    source
        .files
        .lock()
        .unwrap()
        .insert(format!("game-files/{APP}/manifest.json"), json.into_bytes());
    let root = folder("exec");
    let dest = root.join("game");
    let state = root.join("state");
    let options = Options {
        chunk: 1000,
        ..Options::default()
    };
    let progress = engine::run(
        &*source,
        APP,
        &dest,
        &state,
        &options,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(progress.state, State::Complete);
    assert_eq!(fs::read(dest.join("bin/game")).unwrap(), program);
    assert_eq!(fs::read(dest.join("data/a.bin")).unwrap(), data);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &str| fs::metadata(dest.join(p)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode("bin/game"), 0o755);
        assert_ne!(
            mode("data/a.bin") & 0o111,
            0o111,
            "a plain file is not executable"
        );
    }
}

#[cfg(unix)]
#[test]
fn adopted_exec_file_without_mode_bits_becomes_executable() {
    use std::os::unix::fs::PermissionsExt;
    let program = body(1, 700);
    let source = MemorySource::new(APP, &[("bin/game", &program)]);
    let json = format!(
        r#"{{"version":1,"app_id":"{APP}","files":[{{"path":"bin/game","size":{},"sha256":"{}","exec":true}}]}}"#,
        program.len(),
        sha(&program)
    );
    source
        .files
        .lock()
        .unwrap()
        .insert(format!("game-files/{APP}/manifest.json"), json.into_bytes());
    let root = folder("exec-adopt");
    let dest = root.join("game");
    fs::create_dir_all(dest.join("bin")).unwrap();
    fs::write(dest.join("bin/game"), &program).unwrap();
    fs::set_permissions(dest.join("bin/game"), fs::Permissions::from_mode(0o644)).unwrap();
    let progress = engine::run(
        &*source,
        APP,
        &dest,
        &root.join("state"),
        &Options::default(),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(progress.state, State::Complete);
    let mode = fs::metadata(dest.join("bin/game"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o755);
}
