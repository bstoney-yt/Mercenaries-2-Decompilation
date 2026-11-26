use super::*;

fn file() -> Tracker {
    Tracker::parse(include_str!("../../../docs/contracts-status.toml"))
        .expect("the checked-in file")
}

fn entry(id: &str, status: Status, evidence: Evidence) -> Contract {
    Contract {
        id: id.into(),
        status,
        note: "n".into(),
        issues: vec![],
        evidence,
        checked: (evidence != Evidence::None).then(|| "2026-11-20".into()),
        head: None,
        locked: false,
    }
}

fn tracker() -> Tracker {
    Tracker {
        updated: "2026-11-20".into(),
        contracts: CONTRACTS
            .iter()
            .map(|id| entry(id, Status::Unknown, Evidence::None))
            .collect(),
    }
}

#[test]
fn the_checked_in_file_is_valid_and_lists_every_contract() {
    let t = file();
    assert_eq!(t.contracts.len(), 52);
    let ids: Vec<&str> = t.contracts.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, CONTRACTS);
    // Nothing is playable without a recorded result.
    for c in &t.contracts {
        if c.status != Status::Unknown {
            assert_ne!(c.evidence, Evidence::None, "{}", c.id);
        }
    }
    // It round-trips.
    assert_eq!(Tracker::parse(&t.to_toml()).unwrap(), t);
}

#[test]
fn the_validator_refuses_unbacked_claims() {
    let mut t = tracker();
    assert_eq!(validate(&t), Vec::<String>::new());
    t.contracts[0].status = Status::Playable;
    assert!(
        validate(&t)[0].contains("needs a recorded"),
        "{:?}",
        validate(&t)
    );
    t.contracts[0].evidence = Evidence::Nightly;
    t.contracts[0].checked = None;
    assert!(validate(&t)[0].contains("checked date"));
    t.contracts[0].checked = Some("2026-11-21".into());
    assert!(validate(&t).is_empty());
    // A gap or a not-yet needs its note.
    t.contracts[1] = Contract {
        note: " ".into(),
        ..entry("PmcCon001", Status::NotYet, Evidence::Autoplay)
    };
    assert!(validate(&t)[0].contains("needs a note"));
}

#[test]
fn the_validator_catches_ids() {
    let mut t = tracker();
    t.contracts
        .push(entry("VzaCon001", Status::Unknown, Evidence::None));
    t.contracts
        .push(entry("FooCon001", Status::Unknown, Evidence::None));
    t.contracts.remove(1);
    let p = validate(&t);
    assert!(p.iter().any(|s| s == "VzaCon001: listed twice"), "{p:?}");
    assert!(
        p.iter()
            .any(|s| s == "FooCon001: not one of the 52 contracts"),
        "{p:?}"
    );
    assert!(p.iter().any(|s| s == "PmcCon001: missing"), "{p:?}");
    t.updated = "26-11-2026".into();
    assert!(validate(&t).iter().any(|s| s.starts_with("updated")));
}

fn night(results: serde_json::Value) -> Night {
    serde_json::from_value(serde_json::json!({
        "run_id": "20261127-0100Z",
        "head": "0123456789abcdef",
        "started_unix": 1_795_741_200u64,
        "finished_unix": 1_795_760_000u64,
        "results": results,
    }))
    .unwrap()
}

#[test]
fn a_night_moves_statuses_on() {
    let mut t = tracker();
    t.contracts[3].locked = true; // PmcCon003: a playtest's word
    t.contracts[3].evidence = Evidence::Playtest;
    t.contracts[3].status = Status::Playable;
    t.contracts[3].checked = Some("2026-11-20".into());
    let n = night(serde_json::json!([
        {"contract": "VzaCon001", "outcome": "pass", "hops": 0, "completed": true, "extra": 1},
        {"contract": "PmcCon001", "outcome": "pass", "hops": 3, "completed": true},
        {"contract": "PmcCon002", "outcome": "fail", "issue_kinds": ["fps-cliff"], "completed": true},
        {"contract": "PmcCon003", "outcome": "fail", "issue_kinds": ["stuck"], "completed": false},
        {"contract": "PmcCon004", "outcome": "fail", "issue_kinds": ["stuck", "fps-cliff"], "completed": false},
        {"contract": "PmcCon013", "outcome": "skipped"},
        {"contract": "PmcCon015", "outcome": "aborted"},
        {"contract": "NotACon001", "outcome": "pass"}
    ]));
    let changed = update_from_night(&mut t, &n);
    assert_eq!(
        changed,
        ["VzaCon001", "PmcCon001", "PmcCon002", "PmcCon004"]
    );
    let by = |id: &str| t.contracts.iter().find(|c| c.id == id).unwrap().clone();
    assert_eq!(by("VzaCon001").status, Status::Playable);
    assert_eq!(by("VzaCon001").checked.as_deref(), Some("2026-11-27"));
    assert_eq!(by("VzaCon001").head.as_deref(), Some("0123456789"));
    assert_eq!(by("VzaCon001").evidence, Evidence::Nightly);
    assert_eq!(by("PmcCon001").status, Status::PlayableWithGaps);
    assert!(by("PmcCon001").note.contains("3 test-shortcut hop(s)"));
    assert_eq!(by("PmcCon002").status, Status::PlayableWithGaps);
    assert!(by("PmcCon002").note.contains("fps-cliff"));
    assert_eq!(by("PmcCon003").status, Status::Playable, "locked");
    assert_eq!(by("PmcCon004").status, Status::NotYet);
    assert!(by("PmcCon004").note.contains("stuck, fps-cliff"));
    assert_eq!(by("PmcCon013").status, Status::Unknown);
    assert_eq!(by("PmcCon015").status, Status::Unknown);
    assert_eq!(t.updated, "2026-11-27");
    assert!(validate(&t).is_empty(), "{:?}", validate(&t));
    // The same night again changes nothing.
    assert!(update_from_night(&mut t, &n).is_empty());
}

#[test]
fn older_nights_dont_overwrite_newer_evidence() {
    let mut t = tracker();
    t.contracts[0] = Contract {
        checked: Some("2026-12-01".into()),
        ..entry("VzaCon001", Status::NotYet, Evidence::Playtest)
    };
    let n = night(serde_json::json!([{"contract": "VzaCon001", "outcome": "pass", "hops": 0}]));
    assert!(update_from_night(&mut t, &n).is_empty());
    assert_eq!(t.contracts[0].status, Status::NotYet);
}

#[test]
fn issue_numbers_are_kept_through_an_update() {
    let mut t = tracker();
    t.contracts[0].issues = vec![680, 776];
    let n = night(
        serde_json::json!([{"contract": "VzaCon001", "outcome": "fail", "issue_kinds": ["hang"]}]),
    );
    update_from_night(&mut t, &n);
    assert_eq!(t.contracts[0].issues, [680, 776]);
}

#[test]
fn dates_and_groups() {
    assert_eq!(date(0), "1970-01-01");
    assert_eq!(date(1_795_741_200), "2026-11-27");
    assert_eq!(group("PirCon001"), "Pirates");
    assert_eq!(group("VzaCon001"), "Opening");
    assert_eq!(group("x"), "Other");
}

#[test]
fn counts_add_up() {
    let t = file();
    assert_eq!(t.counts().values().sum::<usize>(), 52);
}
