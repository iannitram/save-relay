//! The Kotlin and Swift wrappers and the JNI shim must agree with `include/save_relay.h`
//! (`tests/ffi_header.rs` ties the header to `src/ffi.rs`). The wrappers cannot be compiled on
//! every machine this suite runs on (CI's `bindings` job does that), so this reads the sources:
//! every exported function is reached from both wrappers, the JNI shim and the Kotlin `external`
//! declarations name the same functions with the same parameter counts, and the game-files
//! interface version is the same number everywhere.

use std::collections::BTreeMap;

fn read(path: &str) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{path}: {error}"))
        .replace("\r\n", "\n")
}

/// Names of the functions the header declares (`save_relay_*`), without the ones the wrappers
/// reach in their own way.
fn header_functions() -> Vec<String> {
    let mut header = read("include/save_relay.h");
    while let Some(start) = header.find("/*") {
        let end = start + header[start..].find("*/").expect("unterminated comment") + 2;
        header.replace_range(start..end, " ");
    }
    let mut names = Vec::new();
    for statement in header.split(';') {
        let statement = statement.trim();
        let Some(open) = statement.find('(') else {
            continue;
        };
        let before = &statement[..open];
        if let Some(name) = before
            .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .next()
        {
            if name.starts_with("save_relay_") && !before.contains("typedef") {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

fn camel(snake: &str) -> String {
    snake
        .split('_')
        .enumerate()
        .map(|(i, part)| {
            if i == 0 {
                part.to_string()
            } else {
                part[..1].to_uppercase() + &part[1..]
            }
        })
        .collect()
}

#[test]
fn every_exported_function_is_called_by_the_jni_shim_and_the_swift_wrapper() {
    let jni = read("bindings/android/src/main/cpp/save_relay_jni.c");
    let swift = read("bindings/ios/SaveRelay.swift");
    let names = header_functions();
    assert!(names.len() >= 17, "{names:?}");
    // The sink function is used inside the http callbacks; `free` inside `take`.
    for name in &names {
        let call = format!("{name}(");
        assert!(jni.contains(&call), "save_relay_jni.c never calls {name}");
        assert!(swift.contains(&call), "SaveRelay.swift never calls {name}");
    }
}

/// `Java_org_saverelay_SaveRelay_<name>` -> number of Java parameters (those after env, thiz).
fn jni_natives(jni: &str) -> BTreeMap<String, usize> {
    let mut found = BTreeMap::new();
    for part in jni.split("Java_org_saverelay_SaveRelay_").skip(1) {
        let name: String = part
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let params = part.split_once('(').unwrap().1.split_once(')').unwrap().0;
        let count = params.split(',').filter(|p| !p.trim().is_empty()).count();
        found.insert(name, count - 2);
    }
    found
}

fn kotlin_externals(kotlin: &str) -> BTreeMap<String, usize> {
    let mut found = BTreeMap::new();
    for part in kotlin.split("private external fun ").skip(1) {
        let (name, rest) = part.split_once('(').unwrap();
        let params = rest.split_once(')').unwrap().0;
        let count = params.split(',').filter(|p| !p.trim().is_empty()).count();
        found.insert(name.to_string(), count);
    }
    found
}

#[test]
fn the_jni_shim_and_the_kotlin_externals_declare_the_same_natives() {
    let jni = jni_natives(&read("bindings/android/src/main/cpp/save_relay_jni.c"));
    let kotlin = kotlin_externals(&read(
        "bindings/android/src/main/kotlin/org/saverelay/SaveRelay.kt",
    ));
    assert_eq!(jni, kotlin);
}

#[test]
fn each_game_files_function_has_a_native_and_a_wrapper_in_both_languages() {
    let kotlin = read("bindings/android/src/main/kotlin/org/saverelay/SaveRelay.kt");
    let swift = read("bindings/ios/SaveRelay.swift");
    let natives = kotlin_externals(&kotlin);
    for name in header_functions() {
        let wrapped = name.strip_prefix("save_relay_").unwrap();
        if !wrapped.contains("game_files") {
            continue;
        }
        // `save_relay_game_files_plan` -> `nativeGameFilesPlan`, and the compatibility
        // `save_relay_download_game_files` -> `nativeDownloadGameFiles`.
        let native = format!(
            "native{}",
            camel(wrapped)[..1].to_uppercase() + &camel(wrapped)[1..]
        );
        assert!(natives.contains_key(&native), "Kotlin lacks {native}");
        assert!(
            kotlin.contains(&format!("{native}(")),
            "SaveRelay.kt never calls {native}"
        );
    }
    // The public names the docs promise, in both wrappers.
    for method in [
        "gameFilesPlan",
        "gameFilesDownload",
        "gameFilesProgress",
        "gameFilesCancel",
        "gameFilesCleanup",
        "downloadGameFiles",
        "gameFilesStatus",
    ] {
        assert!(
            kotlin.contains(&format!("fun {method}(")),
            "Kotlin {method}"
        );
        assert!(swift.contains(&format!("func {method}(")), "Swift {method}");
    }
    assert!(kotlin.contains("fun gameFilesVersion("));
    assert!(swift.contains("func nativeGameFilesVersion("));
}

#[test]
fn the_game_files_version_is_the_same_everywhere() {
    let rust = read("src/ffi.rs");
    let number = |text: &str, marker: &str| -> String {
        text.split(marker)
            .nth(1)
            .unwrap_or_else(|| panic!("no {marker}"))
            .trim_start_matches([' ', ':', '='])
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect()
    };
    let version = number(&rust, "GAME_FILES_ABI_VERSION: c_int");
    assert!(!version.is_empty());
    assert_eq!(
        number(
            &read("bindings/android/src/main/kotlin/org/saverelay/SaveRelay.kt"),
            "const val GAME_FILES_VERSION"
        ),
        version
    );
    assert_eq!(
        number(
            &read("bindings/ios/SaveRelay.swift"),
            "static let gameFilesVersion: Int32"
        ),
        version
    );
}
