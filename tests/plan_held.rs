//! Locked tests for F49: the `Held` row of the plan and the mark's path. Do not edit.

use save_relay::layout::Layout;
use save_relay::plan::{decide, decide_with_mark, Decision, Intent, ServerView};
use save_relay::presence::Hold;
use save_relay::scan::{FileEntry, SaveSet};
use save_relay::state::LastSynced;
use Intent::{Exit, Launch};

/// One save file `save1.dat` whose hash is `hash` x 64; `' '` is the empty set.
fn set(hash: char) -> SaveSet {
    let file = |h: char| FileEntry {
        path: "save1.dat".to_string(),
        size: 4,
        mtime: 1_793_476_260,
        sha256: h.to_string().repeat(64),
    };
    let files = if hash == ' ' {
        vec![]
    } else {
        vec![file(hash)]
    };
    SaveSet { files }
}

fn last(revision: u64, hash: char) -> Option<LastSynced> {
    let (files, game) = (set(hash).files, "uncharted".to_string());
    let time = "2026-10-31T19:51:00Z".to_string();
    Some(LastSynced {
        format: 1,
        game,
        revision,
        time,
        saves_folder: None,
        files,
    })
}

fn at(revision: u64, hash: char) -> ServerView {
    ServerView::At {
        revision,
        files: set(hash),
    }
}

fn elsewhere() -> Hold {
    let (device, since) = (
        "amd-handheld".to_string(),
        "2026-11-01T19:12:00Z".to_string(),
    );
    Hold::Elsewhere { device, since }
}

/// One of each row of the plain table that the server can be reached for.
fn cases() -> Vec<(SaveSet, Option<LastSynced>, ServerView)> {
    vec![
        (set(' '), None, ServerView::Empty),
        (set('a'), None, ServerView::Empty),
        (set(' '), None, at(3, 'a')),
        (set('a'), None, at(3, 'a')),
        (set('b'), None, at(3, 'a')),
        (set('a'), last(3, 'a'), at(3, 'a')),
        (set('b'), last(3, 'a'), at(3, 'a')),
        (set('a'), last(3, 'a'), at(4, 'b')),
        (set('b'), last(2, 'a'), at(4, 'b')),
        (set('b'), last(2, 'a'), at(4, 'c')),
    ]
}

#[test]
fn unreachable_is_offline_whoever_holds_the_game() {
    let server = ServerView::Unreachable;
    for hold in [Hold::Free, Hold::Mine, elsewhere()] {
        for (intent, anyway) in [(Launch, false), (Launch, true), (Exit, false), (Exit, true)] {
            let d = decide_with_mark(intent, &hold, anyway, &set('a'), None, &server);
            assert_eq!(d, Decision::Offline, "{intent:?} {hold:?} anyway={anyway}");
        }
    }
}

#[test]
fn launch_with_a_mark_elsewhere_is_held() {
    for (local, last, server) in cases() {
        let d = decide_with_mark(Launch, &elsewhere(), false, &local, last.as_ref(), &server);
        assert_eq!(d, Decision::Held, "{local:?} {last:?} {server:?}");
    }
}

#[test]
fn anyway_exit_free_and_mine_are_the_plain_table() {
    let holds = [(Hold::Free, false), (Hold::Free, true), (Hold::Mine, false)];
    let holds = [&holds[..], &[(Hold::Mine, true), (elsewhere(), true)]].concat();
    for (local, last, server) in cases() {
        for intent in [Launch, Exit] {
            let plain = decide(intent, &local, last.as_ref(), &server);
            assert_ne!(plain, Decision::Held);
            for (hold, anyway) in &holds {
                let d = decide_with_mark(intent, hold, *anyway, &local, last.as_ref(), &server);
                assert_eq!(d, plain, "{intent:?} {hold:?} anyway={anyway} {server:?}");
            }
        }
        let plain = decide(Exit, &local, last.as_ref(), &server);
        let d = decide_with_mark(Exit, &elsewhere(), false, &local, last.as_ref(), &server);
        assert_eq!(d, plain, "exit never checks the mark: {server:?}");
    }
}

#[test]
fn some_plain_rows_written_out() {
    let d = decide_with_mark(Launch, &Hold::Mine, false, &set(' '), None, &at(3, 'a'));
    assert_eq!(d, Decision::Pull);
    let d = decide_with_mark(
        Exit,
        &elsewhere(),
        false,
        &set('a'),
        None,
        &ServerView::Empty,
    );
    assert_eq!(d, Decision::Push);
    let d = decide_with_mark(
        Launch,
        &elsewhere(),
        true,
        &set(' '),
        None,
        &ServerView::Empty,
    );
    assert_eq!(d, Decision::Noop);
    let l = last(2, 'a');
    let d = decide_with_mark(
        Exit,
        &elsewhere(),
        false,
        &set('b'),
        l.as_ref(),
        &at(4, 'c'),
    );
    assert_eq!(d, Decision::Conflict);
    assert_eq!(serde_json::to_string(&Decision::Held).unwrap(), "\"held\"");
}

#[test]
fn playing_path() {
    let layout = Layout::new("/Games/save-relay/").unwrap();
    let want = "Games/save-relay/uncharted/playing.json";
    assert_eq!(layout.playing_path("uncharted"), want);
    let one = Layout::new("saves").unwrap();
    assert_eq!(one.playing_path("totk"), "saves/totk/playing.json");
}
