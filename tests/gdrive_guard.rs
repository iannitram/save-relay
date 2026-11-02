use save_relay::gdrive::guard::{
    check_written, decide, make_etag, parse_etag, Claim, Step, LEASE_SECS, MAX_HOPS, SLOW_SECS,
};
use save_relay::Error;

#[test]
fn etags_carry_version_and_claim() {
    assert_eq!(make_etag(12, "1AbC"), "v12.1AbC");
    assert_eq!(make_etag(0, ""), "v0.");
    assert_eq!(parse_etag("v12.1AbC"), Some((12, "1AbC".to_string())));
    assert_eq!(parse_etag("v12."), Some((12, String::new())));
    assert_eq!(parse_etag("v7.a.b"), Some((7, "a.b".to_string())));
    for bad in [
        "", "12.a", "v.a", "vx.a", "v12", "V12.a", "v-1.a", "v1 .a", "\"v1.a\"",
    ] {
        assert_eq!(parse_etag(bad), None, "{bad}");
    }
}

fn taken(age_secs: u64, next: Option<&str>) -> Claim {
    Claim::Taken {
        age_secs,
        next: next.map(String::from),
    }
}

#[test]
fn a_changed_version_always_loses() {
    for claim in [Claim::Won, taken(0, Some("n")), taken(999, Some("n"))] {
        assert_eq!(decide(12, 13, &claim, 0, 0), Step::Changed, "{claim:?}");
    }
}

#[test]
fn a_won_claim_writes_unless_too_slow() {
    assert_eq!((LEASE_SECS, SLOW_SECS, MAX_HOPS), (120, 60, 3));
    assert_eq!(decide(12, 12, &Claim::Won, 0, 0), Step::Write);
    assert_eq!(decide(12, 12, &Claim::Won, SLOW_SECS - 1, 2), Step::Write);
    assert_eq!(decide(12, 12, &Claim::Won, SLOW_SECS, 0), Step::TooSlow);
}

#[test]
fn a_live_claim_means_another_machine_is_writing() {
    let live = taken(LEASE_SECS - 1, Some("n"));
    assert_eq!(decide(12, 12, &live, 0, 0), Step::Changed);
}

#[test]
fn a_stale_claim_is_broken_by_claiming_its_next_id() {
    let stale = taken(LEASE_SECS, Some("N2"));
    assert_eq!(decide(12, 12, &stale, 0, 0), Step::Break("N2".into()));
    assert_eq!(
        decide(12, 12, &stale, 0, MAX_HOPS - 1),
        Step::Break("N2".into())
    );
    assert_eq!(decide(12, 12, &stale, 0, MAX_HOPS), Step::Changed);
    assert_eq!(decide(12, 12, &taken(9999, None), 0, 0), Step::Changed);
}

#[test]
fn the_check_after_the_write() {
    assert_eq!(check_written(13, "N1", 13, Some("N1")), Ok(()));
    let err = |v: u64| {
        let m = format!("drive: state changed during the write (version {v}): run again");
        Err(Error::Server(m))
    };
    assert_eq!(check_written(13, "N1", 14, Some("N9")), err(14));
    assert_eq!(check_written(13, "N1", 13, Some("N9")), err(13));
    assert_eq!(check_written(13, "N1", 13, None), err(13));
}
