use save_relay::http::{Request, Response, Transport};
use save_relay::platform::AppDirs;
use save_relay::steam::cloud::{Cache, CloudState, REFRESH_SECS};
use save_relay::{Error, Result};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

struct Fake {
    replies: RefCell<VecDeque<Result<Response>>>,
    requests: RefCell<Vec<Request>>,
}

impl Fake {
    fn new(replies: Vec<Result<Response>>) -> Fake {
        Fake {
            replies: RefCell::new(replies.into()),
            requests: RefCell::new(Vec::new()),
        }
    }
    fn count(&self) -> usize {
        self.requests.borrow().len()
    }
}

impl Transport for Fake {
    fn send(&self, request: &Request) -> Result<Response> {
        self.requests.borrow_mut().push(request.clone());
        self.replies
            .borrow_mut()
            .pop_front()
            .expect("unexpected request")
    }
}

fn dirs(name: &str) -> AppDirs {
    let path = std::env::temp_dir().join(format!("save-relay-cloud-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    AppDirs {
        config: path.join("config"),
        data: path.join("data/save-relay"),
    }
}

fn game(id: u32, cloud: bool) -> Response {
    let categories = if cloud { r#"[{"id":23}]"# } else { "[]" };
    Response::new(
        200,
        format!(
            r#"{{"{id}":{{"success":true,"data":{{"type":"game","categories":{categories}}}}}}}"#
        ),
    )
}

fn unknown(id: u32) -> Response {
    Response::new(200, format!(r#"{{"{id}":{{"success":false}}}}"#))
}

#[test]
fn refreshes_missing_answers_once_then_again_after_a_week() {
    let dirs = dirs("weekly");
    let mut cache = Cache::open(&dirs).unwrap();
    let fake = Fake::new(vec![Ok(game(11, true)), Ok(game(12, false))]);
    let first = cache.refresh(&fake, &[11, 12], 100, false).unwrap();
    assert_eq!(
        (first.asked, first.stored, first.stopped_early),
        (2, 2, false)
    );
    assert_eq!(cache.state(11), CloudState::Yes);
    assert_eq!(cache.state(12), CloudState::No);

    let no_work = cache
        .refresh(&Fake::new(vec![]), &[11, 12], 100 + REFRESH_SECS - 1, false)
        .unwrap();
    assert_eq!(no_work.asked, 0);

    let old = Fake::new(vec![Ok(game(11, false))]);
    let refreshed = cache
        .refresh(&old, &[11], 100 + REFRESH_SECS, false)
        .unwrap();
    assert_eq!((refreshed.asked, refreshed.stored), (1, 1));
    assert_eq!(cache.state(11), CloudState::No);

    let forced = Fake::new(vec![Ok(game(11, true))]);
    let report = cache
        .refresh(&forced, &[11], 100 + REFRESH_SECS, true)
        .unwrap();
    assert_eq!((report.asked, report.stored), (1, 1));
    assert_eq!(cache.state(11), CloudState::Yes);
}

#[test]
fn stops_at_429_and_keeps_the_answers_already_fetched() {
    let dirs = dirs("throttle");
    let mut cache = Cache::open(&dirs).unwrap();
    let fake = Fake::new(vec![
        Ok(game(21, true)),
        Ok(game(22, false)),
        Ok(Response::new(429, "")),
    ]);
    let report = cache.refresh(&fake, &[21, 22, 23], 200, false).unwrap();
    assert_eq!(
        (report.asked, report.stored, report.stopped_early),
        (3, 2, true)
    );
    assert_eq!(cache.state(21), CloudState::Yes);
    assert_eq!(cache.state(22), CloudState::No);
    assert_eq!(cache.state(23), CloudState::Unknown);
    let reopened = Cache::open(&dirs).unwrap();
    assert_eq!(reopened.state(21), CloudState::Yes);
    assert_eq!(reopened.state(22), CloudState::No);
}

#[test]
fn unknown_answer_is_cached_for_a_week_and_offline_state_uses_cache() {
    let dirs = dirs("unknown");
    let mut cache = Cache::open(&dirs).unwrap();
    let fake = Fake::new(vec![Ok(unknown(31))]);
    let report = cache.refresh(&fake, &[31], 300, false).unwrap();
    assert_eq!(report.stored, 1);
    assert_eq!(cache.state(31), CloudState::Unknown);
    let text = fs::read_to_string(dirs.data.join("steam/cloud.json")).unwrap();
    assert!(text.contains("\"answer\": \"unknown\""));

    let offline = Fake::new(vec![Err(Error::Unreachable("offline".into()))]);
    let report = cache
        .refresh(&offline, &[31, 32], 300 + REFRESH_SECS, false)
        .unwrap();
    assert_eq!((report.asked, report.stopped_early), (1, true));
    assert_eq!(cache.state(31), CloudState::Unknown);
}

#[test]
fn malformed_cache_is_state_invalid() {
    let dirs = dirs("invalid");
    let path: PathBuf = dirs.data.join("steam/cloud.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "not json").unwrap();
    assert!(matches!(Cache::open(&dirs), Err(Error::StateInvalid(_))));
}

#[test]
fn each_request_uses_only_the_public_app_id_url() {
    let dirs = dirs("request");
    let mut cache = Cache::open(&dirs).unwrap();
    let fake = Fake::new(vec![Ok(game(41, true))]);
    cache.refresh(&fake, &[41], 400, false).unwrap();
    assert_eq!(fake.count(), 1);
    let request = &fake.requests.borrow()[0];
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.url,
        "https://store.steampowered.com/api/appdetails?appids=41&filters=basic,categories"
    );
    assert!(request.headers.is_empty());
}

#[test]
fn concurrent_refreshes_merge_answers_for_different_apps() {
    struct OneAnswer(Response);
    impl Transport for OneAnswer {
        fn send(&self, _request: &Request) -> Result<Response> {
            Ok(self.0.clone())
        }
    }

    let dirs = Arc::new(dirs("concurrent"));
    let jobs = [(51, true), (52, false)].map(|(id, cloud)| {
        let dirs = Arc::clone(&dirs);
        std::thread::spawn(move || {
            let mut cache = Cache::open(&dirs).unwrap();
            cache
                .refresh(&OneAnswer(game(id, cloud)), &[id], 500, false)
                .unwrap()
        })
    });
    for job in jobs {
        assert_eq!(job.join().unwrap().stored, 1);
    }
    let cache = Cache::open(&dirs).unwrap();
    assert_eq!(cache.state(51), CloudState::Yes);
    assert_eq!(cache.state(52), CloudState::No);
}

#[test]
fn refresh_asks_at_most_one_hundred_apps() {
    let dirs = dirs("limit");
    let mut cache = Cache::open(&dirs).unwrap();
    let ids: Vec<u32> = (1..=101).collect();
    let replies: Vec<Result<Response>> = ids[..100]
        .iter()
        .copied()
        .map(|id| Ok(game(id, false)))
        .collect();
    let fake = Fake::new(replies);
    let report = cache.refresh(&fake, &ids, 600, false).unwrap();
    assert_eq!(
        (report.asked, report.stored, report.stopped_early),
        (100, 100, false)
    );
    assert_eq!(fake.count(), 100);
    assert_eq!(cache.state(101), CloudState::Unknown);
}
