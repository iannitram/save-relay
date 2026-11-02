use save_relay::plan::{decide, Decision, Intent, ServerView};
use save_relay::scan::{FileEntry, SaveSet};
use save_relay::state::LastSynced;
use Decision::*;
use Intent::*;

const H1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const H2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn set(files: &[(&str, &str, u64)]) -> SaveSet {
    let files = files
        .iter()
        .map(|&(path, sha, mtime)| FileEntry {
            path: path.into(),
            size: 4,
            mtime,
            sha256: sha.into(),
        })
        .collect();
    SaveSet { files }
}

fn v1() -> SaveSet {
    set(&[("s/0.sav", H1, 100)])
}

fn v2() -> SaveSet {
    set(&[("s/0.sav", H2, 200)])
}

fn last(revision: u64, files: SaveSet) -> LastSynced {
    LastSynced {
        format: 1,
        game: "g".into(),
        revision,
        time: "2026-10-31T19:51:00Z".into(),
        files: files.files,
    }
}

fn at(revision: u64, files: SaveSet) -> ServerView {
    ServerView::At { revision, files }
}

fn both(local: &SaveSet, last: Option<&LastSynced>, server: &ServerView) -> (Decision, Decision) {
    (
        decide(Launch, local, last, server),
        decide(Exit, local, last, server),
    )
}

#[test]
fn unreachable_is_offline_whatever_else() {
    let l = last(3, v1());
    assert_eq!(
        both(&v1(), Some(&l), &ServerView::Unreachable),
        (Offline, Offline)
    );
    assert_eq!(
        both(&SaveSet::default(), None, &ServerView::Unreachable),
        (Offline, Offline)
    );
}

#[test]
fn empty_server_and_no_local_saves_is_noop() {
    assert_eq!(
        both(&SaveSet::default(), None, &ServerView::Empty),
        (Noop, Noop)
    );
    let l = last(2, v1());
    assert_eq!(
        both(&SaveSet::default(), Some(&l), &ServerView::Empty),
        (Noop, Noop)
    );
}

#[test]
fn empty_server_with_local_saves_is_push() {
    assert_eq!(both(&v1(), None, &ServerView::Empty), (Push, Push));
    let l = last(2, v1());
    assert_eq!(both(&v1(), Some(&l), &ServerView::Empty), (Push, Push));
}

#[test]
fn never_synced_with_same_content_is_adopt_even_if_mtime_differs() {
    let local = set(&[("s/0.sav", H1, 999)]);
    assert_eq!(both(&local, None, &at(5, v1())), (Adopt, Adopt));
    assert_eq!(
        both(&SaveSet::default(), None, &at(5, SaveSet::default())),
        (Adopt, Adopt)
    );
}

#[test]
fn never_synced_with_no_local_saves_is_pull() {
    assert_eq!(both(&SaveSet::default(), None, &at(5, v1())), (Pull, Pull));
}

#[test]
fn never_synced_with_different_saves_is_conflict() {
    assert_eq!(both(&v2(), None, &at(5, v1())), (Conflict, Conflict));
}

#[test]
fn nothing_changed_is_noop() {
    let l = last(5, v1());
    let local = set(&[("s/0.sav", H1, 12345)]);
    assert_eq!(both(&local, Some(&l), &at(5, v1())), (Noop, Noop));
}

#[test]
fn only_local_changed_is_push() {
    let l = last(5, v1());
    assert_eq!(both(&v2(), Some(&l), &at(5, v1())), (Push, Push));
    let extra = set(&[("s/0.sav", H1, 100), ("s/1.sav", H2, 100)]);
    assert_eq!(both(&extra, Some(&l), &at(5, v1())), (Push, Push));
    assert_eq!(
        both(&SaveSet::default(), Some(&l), &at(5, v1())),
        (Push, Push)
    );
}

#[test]
fn only_server_changed_is_pull_at_launch_and_noop_at_exit() {
    let l = last(5, v1());
    assert_eq!(both(&v1(), Some(&l), &at(6, v2())), (Pull, Noop));
    assert_eq!(both(&v1(), Some(&l), &at(4, v2())), (Pull, Noop));
}

#[test]
fn both_changed_to_the_same_files_is_adopt() {
    let l = last(5, v1());
    assert_eq!(both(&v2(), Some(&l), &at(6, v2())), (Adopt, Adopt));
}

#[test]
fn both_changed_differently_is_conflict() {
    let l = last(5, v1());
    let other = set(&[("s/0.sav", H1, 100), ("s/1.sav", H1, 100)]);
    assert_eq!(both(&v2(), Some(&l), &at(6, other)), (Conflict, Conflict));
}
