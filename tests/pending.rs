use save_relay::local::LocalStore;
use save_relay::pending::{self, Answer, ChoiceKind, PendingChoice, ANSWER_WAIT_SECS, POLL_SECS};
use save_relay::platform::AppDirs;
use save_relay::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const HELD: &str = r#"{
  "format": 1,
  "game": "totk",
  "kind": "held",
  "server": null,
  "local": null,
  "server_card": null,
  "local_card": null,
  "held": {
    "device": "other-device",
    "since": "2026-11-01T19:12:00Z"
  },
  "time": "2026-11-01T19:13:00Z",
  "answer": null
}
"#;

// Deserialize through the derive, independently of the parser under test.
fn held() -> PendingChoice {
    serde_json::from_str(HELD).unwrap()
}

fn conflict() -> PendingChoice {
    serde_json::from_value(serde_json::json!({
        "format": 1, "game": "uncharted", "kind": "conflict",
        "server": {"device": "server-device", "time": "2026-11-01T19:12:00Z", "revision": 7},
        "local": {"device": "local-device", "time": "2026-11-01T19:13:00Z", "revision": null},
        "server_card": {"format": 1, "device": "server-device", "time": "2026-11-01T19:12:00Z",
            "playtime_secs": 60, "place": "Chapter 2", "picture": null},
        "local_card": {"format": 1, "device": "local-device", "time": "2026-11-01T19:13:00Z",
            "playtime_secs": null, "place": null, "picture": null},
        "held": null, "time": "2026-11-01T19:13:00Z", "answer": null
    }))
    .unwrap()
}

struct Fixture {
    root: PathBuf,
    pending: PathBuf,
    store: LocalStore,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::current_dir()
            .unwrap()
            .join(format!(".pending-tests-{name}-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config").join("save-relay"),
            data: root.join("data").join("save-relay"),
        };
        Self {
            pending: dirs.data.join("pending"),
            root,
            store: LocalStore::new(dirs),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn answers_fit_all_eight_pairs() {
    for answer in [
        Answer::KeepServer,
        Answer::KeepLocal,
        Answer::PlayAnyway,
        Answer::Wait,
    ] {
        let conflict = matches!(answer, Answer::KeepServer | Answer::KeepLocal);
        assert_eq!(answer.fits(ChoiceKind::Conflict), conflict);
        assert_eq!(answer.fits(ChoiceKind::Held), !conflict);
    }
}

#[test]
fn pretty_json_has_declared_order_nulls_snake_case_and_one_newline() {
    assert_eq!(held().to_json(), HELD);
    let mut choice = conflict();
    let unanswered = choice.to_json();
    choice.answer = Some(Answer::KeepServer);
    assert_eq!(
        choice.to_json(),
        unanswered.replace("\"answer\": null", "\"answer\": \"keep_server\"")
    );
}

#[test]
fn choices_round_trip_with_sides_cards_and_every_fitting_answer() {
    for mut choice in [held(), conflict()] {
        assert_eq!(PendingChoice::from_json(&choice.to_json()).unwrap(), choice);
        for answer in [
            Answer::KeepServer,
            Answer::KeepLocal,
            Answer::PlayAnyway,
            Answer::Wait,
        ] {
            if answer.fits(choice.kind) {
                choice.answer = Some(answer);
                assert_eq!(PendingChoice::from_json(&choice.to_json()).unwrap(), choice);
            }
        }
    }
}

#[test]
fn invalid_json_format_game_time_and_mismatched_answers_are_state_invalid() {
    for text in [
        HELD.replace("\"format\": 1", "\"format\": 2"),
        HELD.replace("\"game\": \"totk\"", "\"game\": \"\""),
        HELD.replace("2026-11-01T19:13:00Z", "now"),
        HELD.replace("\"answer\": null", "\"answer\": \"keep_local\""),
        conflict()
            .to_json()
            .replace("\"answer\": null", "\"answer\": \"wait\""),
        "{".into(),
    ] {
        assert!(
            matches!(PendingChoice::from_json(&text), Err(Error::StateInvalid(_))),
            "{text}"
        );
    }
}

#[test]
fn store_saves_replaces_lists_in_game_order_and_clears_without_temp_files() {
    let f = Fixture::new("store");
    assert_eq!(f.store.load_pending("totk").unwrap(), None);
    assert_eq!(f.store.list_pending().unwrap(), vec![]);
    f.store.clear_pending("totk").unwrap();
    f.store.save_pending(&conflict()).unwrap();
    f.store.save_pending(&held()).unwrap();
    assert_eq!(
        fs::read_to_string(f.pending.join("totk.json")).unwrap(),
        HELD
    );
    let mut replacement = held();
    replacement.answer = Some(Answer::Wait);
    f.store.save_pending(&replacement).unwrap();
    assert_eq!(
        f.store.load_pending("totk").unwrap(),
        Some(replacement.clone())
    );
    let mut names: Vec<_> = fs::read_dir(&f.pending)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names, vec!["totk.json", "uncharted.json"]);
    fs::write(f.pending.join("ignored.json.tmp"), "{").unwrap();
    fs::write(f.pending.join("ignored.txt"), "{").unwrap();
    assert_eq!(
        f.store.list_pending().unwrap(),
        vec![replacement, conflict()]
    );
    f.store.clear_pending("totk").unwrap();
    f.store.clear_pending("totk").unwrap();
    assert_eq!(f.store.load_pending("totk").unwrap(), None);
    assert_eq!(f.store.list_pending().unwrap(), vec![conflict()]);
}

#[test]
fn store_rejects_broken_invalid_and_unreadable_pending_files() {
    let f = Fixture::new("broken");
    f.store.save_pending(&held()).unwrap();
    let path = f.pending.join("totk.json");
    for text in [
        "{".to_string(),
        HELD.replace("\"format\": 1", "\"format\": 2"),
    ] {
        fs::write(&path, text).unwrap();
        assert!(matches!(
            f.store.load_pending("totk"),
            Err(Error::StateInvalid(_))
        ));
        assert!(matches!(
            f.store.list_pending(),
            Err(Error::StateInvalid(_))
        ));
    }
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(matches!(
        f.store.load_pending("totk"),
        Err(Error::StateInvalid(_))
    ));
}

#[test]
fn answer_requires_a_pending_choice_and_a_fitting_answer_then_persists_it() {
    let f = Fixture::new("answer");
    assert_eq!(
        pending::answer(&f.store, "totk", Answer::Wait),
        Err(Error::NoPendingChoice("totk".into()))
    );
    for choice in [held(), conflict()] {
        f.store.save_pending(&choice).unwrap();
        let path = f.pending.join(format!("{}.json", choice.game));
        let before = fs::read(&path).unwrap();
        let (wrong, right, kind) = match choice.kind {
            ChoiceKind::Held => (Answer::KeepServer, Answer::PlayAnyway, "held"),
            ChoiceKind::Conflict => (Answer::Wait, Answer::KeepLocal, "conflict"),
        };
        assert_eq!(
            pending::answer(&f.store, &choice.game, wrong),
            Err(Error::Usage(format!(
                "{} does not answer a {kind} choice",
                wrong.key()
            )))
        );
        assert_eq!(fs::read(path).unwrap(), before);
        let mut expected = choice;
        expected.answer = Some(right);
        assert_eq!(
            pending::answer(&f.store, &expected.game, right).unwrap(),
            expected
        );
        assert_eq!(
            f.store.load_pending(&expected.game).unwrap(),
            Some(expected)
        );
    }
}

/// Two `answer` calls on one conflict at the same moment. Before each write had a temp file of
/// its own, one rename found no file (os error 2) or published the other's bytes. Whichever
/// answer lands last is the one on record, so the check is "one of the two", not a fixed one.
#[test]
fn two_answers_at_once_neither_fails_and_one_of_them_is_on_record() {
    const ROUNDS: usize = 100;
    let f = Fixture::new("race");
    fs::create_dir_all(&f.pending).unwrap();
    for round in 0..ROUNDS {
        f.store.save_pending(&conflict()).unwrap();
        let (first, second) = std::thread::scope(|scope| {
            let first = scope.spawn(|| pending::answer(&f.store, "uncharted", Answer::KeepServer));
            let second = scope.spawn(|| pending::answer(&f.store, "uncharted", Answer::KeepLocal));
            (first.join().unwrap(), second.join().unwrap())
        });
        for result in [&first, &second] {
            assert!(
                !matches!(result, Err(Error::Io(_))),
                "round {round}: {result:?}"
            );
        }
        let stored = f.store.load_pending("uncharted").unwrap().unwrap().answer;
        assert!(
            stored == Some(Answer::KeepServer) || stored == Some(Answer::KeepLocal),
            "round {round}: stored {stored:?}"
        );
    }
    let leftovers: Vec<_> = fs::read_dir(&f.pending)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
}

/// A file named for one game that holds a choice for another is refused, and not rewritten.
#[test]
fn answer_refuses_a_stored_choice_for_another_game_and_changes_nothing() {
    let f = Fixture::new("answer-mismatch");
    fs::create_dir_all(&f.pending).unwrap();
    let path = f.pending.join("totk.json");
    fs::write(&path, conflict().to_json()).unwrap();
    let before = fs::read(&path).unwrap();
    assert_eq!(
        pending::answer(&f.store, "totk", Answer::KeepLocal),
        Err(Error::NoPendingChoice("totk".into()))
    );
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn wait_times_out_after_exactly_120_sleeps_and_keeps_the_file() {
    static NOW: AtomicU64 = AtomicU64::new(700);
    fn now() -> u64 {
        NOW.load(Ordering::SeqCst)
    }
    let f = Fixture::new("timeout");
    f.store.save_pending(&held()).unwrap();
    let sleep = |seconds| {
        assert_eq!(seconds, POLL_SECS);
        NOW.fetch_add(1, Ordering::SeqCst);
    };
    assert_eq!(
        pending::wait_for_answer(&f.store, "totk", ANSWER_WAIT_SECS, now, &sleep).unwrap(),
        None
    );
    assert_eq!(now(), 820);
    assert_eq!(f.store.load_pending("totk").unwrap(), Some(held()));
}

#[test]
fn wait_returns_answer_written_on_fifth_sleep_and_keeps_it() {
    static NOW: AtomicU64 = AtomicU64::new(0);
    fn now() -> u64 {
        NOW.load(Ordering::SeqCst)
    }
    let f = Fixture::new("wait-answer");
    f.store.save_pending(&held()).unwrap();
    let sleep = |seconds| {
        assert_eq!(seconds, POLL_SECS);
        if NOW.fetch_add(1, Ordering::SeqCst) + 1 == 5 {
            pending::answer(&f.store, "totk", Answer::Wait).unwrap();
        }
    };
    assert_eq!(
        pending::wait_for_answer(&f.store, "totk", 120, now, &sleep).unwrap(),
        Some(Answer::Wait)
    );
    assert_eq!(now(), 5);
    assert_eq!(
        f.store.load_pending("totk").unwrap().unwrap().answer,
        Some(Answer::Wait)
    );
}

#[test]
fn wait_returns_none_when_file_is_removed_on_third_sleep() {
    static NOW: AtomicU64 = AtomicU64::new(0);
    fn now() -> u64 {
        NOW.load(Ordering::SeqCst)
    }
    let f = Fixture::new("wait-removed");
    f.store.save_pending(&held()).unwrap();
    let sleep = |seconds| {
        assert_eq!(seconds, POLL_SECS);
        if NOW.fetch_add(1, Ordering::SeqCst) + 1 == 3 {
            f.store.clear_pending("totk").unwrap();
        }
    };
    assert_eq!(
        pending::wait_for_answer(&f.store, "totk", 120, now, &sleep).unwrap(),
        None
    );
    assert_eq!(now(), 3);
}

#[test]
fn wait_without_a_file_returns_none_without_sleeping() {
    static NOW: AtomicU64 = AtomicU64::new(0);
    fn now() -> u64 {
        NOW.load(Ordering::SeqCst)
    }
    let f = Fixture::new("wait-none");
    assert_eq!(
        pending::wait_for_answer(&f.store, "totk", 120, now, &|_| panic!("slept")).unwrap(),
        None
    );
}

#[test]
fn wait_with_an_answer_returns_it_without_sleeping_even_at_zero_timeout() {
    static NOW: AtomicU64 = AtomicU64::new(0);
    fn now() -> u64 {
        NOW.load(Ordering::SeqCst)
    }
    let f = Fixture::new("wait-answered");
    let mut choice = held();
    choice.answer = Some(Answer::PlayAnyway);
    f.store.save_pending(&choice).unwrap();
    assert_eq!(
        pending::wait_for_answer(&f.store, "totk", 0, now, &|_| panic!("slept")).unwrap(),
        Some(Answer::PlayAnyway)
    );
    assert_eq!(f.store.load_pending("totk").unwrap(), Some(choice));
}

#[test]
fn wait_uses_a_tiny_timeout_without_real_sleep() {
    static NOW: AtomicU64 = AtomicU64::new(0);
    fn now() -> u64 {
        NOW.load(Ordering::SeqCst)
    }
    let f = Fixture::new("wait-tiny");
    f.store.save_pending(&held()).unwrap();
    assert_eq!(
        pending::wait_for_answer(&f.store, "totk", 1, now, &|seconds| {
            NOW.fetch_add(seconds, Ordering::SeqCst);
        })
        .unwrap(),
        None
    );
    assert_eq!(now(), 1);
}
