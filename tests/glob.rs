//! Locked tests for `glob` (F01). Do not edit: the code must make these pass.

use save_relay::glob::{matches, selected};

fn yes(pattern: &str, path: &str) {
    assert!(matches(pattern, path), "{pattern:?} should match {path:?}");
}

fn no(pattern: &str, path: &str) {
    assert!(
        !matches(pattern, path),
        "{pattern:?} should not match {path:?}"
    );
}

#[test]
fn literal_matches_only_itself() {
    yes("save.dat", "save.dat");
    no("save.dat", "save.da");
    no("save.dat", "save.dat2");
    no("save.dat", "dir/save.dat");
    no("save.dat", "Save.dat");
    yes("a/b/c.bin", "a/b/c.bin");
    no("a/b", "a/b/c.bin");
    no("", "x");
}

#[test]
fn star_stays_inside_one_segment() {
    yes("*", "save.dat");
    yes("*.dat", "save.dat");
    yes("*.dat", ".dat");
    no("*.dat", "save.dat.bak");
    no("*", "a/b");
    no("*.dat", "dir/save.dat");
    yes("dir/*", "dir/save.dat");
    no("dir/*", "dir/sub/save.dat");
    yes("SAVEDATA*", "SAVEDATA00");
    yes("SAVEDATA*", "SAVEDATA");
    yes("s*e*.d*t", "save.dat");
    no("s*e*.d*t", "save.dax");
    no("*.DAT", "save.dat");
}

#[test]
fn question_mark_is_one_character_not_slash() {
    yes("slot?.bin", "slot1.bin");
    no("slot?.bin", "slot.bin");
    no("slot?.bin", "slot12.bin");
    no("a?b", "a/b");
    yes("??", "ab");
    yes("?", "é");
}

#[test]
fn double_star_spans_whole_segments() {
    yes("a/**", "a/b");
    yes("a/**", "a/b/c/d.bin");
    no("a/**", "a");
    no("a/**", "ab/c");
    no("a/**", "b/a/c");
    yes("**/x.bin", "x.bin");
    yes("**/x.bin", "a/x.bin");
    yes("**/x.bin", "a/b/c/x.bin");
    no("**/x.bin", "a/yx.bin");
    yes("a/**/z", "a/z");
    yes("a/**/z", "a/b/c/z");
    no("a/**/z", "a/b/c/zz");
    yes("**", "anything/at/all");
    yes("**/*.tmp", "deep/er/file.tmp");
    no("**/*.tmp", "deep/er/file.tmp.keep");
}

#[test]
fn uncharted_style_pattern() {
    let p = "Uncharted 1/SAVEDATA*/**";
    yes(p, "Uncharted 1/SAVEDATA00/data.bin");
    yes(p, "Uncharted 1/SAVEDATA01/sce_sys/param.sfo");
    no(p, "Uncharted 1/SAVEDATA00");
    no(p, "Uncharted 2/SAVEDATA00/data.bin");
    no(p, "Uncharted 1/shader_cache/x.bin");
    no(p, "uncharted 1/SAVEDATA00/data.bin");
}

#[test]
fn selected_needs_an_include_and_no_exclude() {
    let include = vec!["saves/**".to_string(), "*.cfg".to_string()];
    let exclude = vec!["**/*.tmp".to_string(), "saves/cache/**".to_string()];
    assert!(selected(&include, &exclude, "saves/slot1.bin"));
    assert!(selected(&include, &exclude, "game.cfg"));
    assert!(!selected(&include, &exclude, "saves/slot1.tmp"));
    assert!(!selected(&include, &exclude, "saves/cache/a.bin"));
    assert!(!selected(&include, &exclude, "other/slot1.bin"));
    assert!(!selected(&include, &exclude, "dir/game.cfg"));
    assert!(selected(&include, &[], "saves/x.tmp"));
    assert!(!selected(&[], &[], "saves/slot1.bin"));
    assert!(!selected(&[], &exclude, "game.cfg"));
}
