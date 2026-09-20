use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("alice", ["g".into(), "saved".into()])
}
fn data(n: i64) -> GraphData {
    serde_json::from_value(json!({"nodes":[{"id":"a","entity_id":"A","space_id":"s","properties":{"n":n}},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"e","predicate":"p","from":"a","to":"b","valid_time":{"start":0,"end":10}}]})).unwrap()
}
fn write(e: &mut Engine, mut d: GraphData, n: i64) {
    if n < 0 {
        d.edges.clear();
    }
    let expected_head = e.head("g", "main").unwrap();
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "g".into(),
                branch_id: "main".into(),
                expected_head,
                data: d,
            }],
        },
        &host(),
    )
    .unwrap();
}
fn query() -> GraphExpression {
    GraphExpression::Query {
        query: serde_json::from_value(json!({"graph_id":"g"})).unwrap(),
    }
}
fn enroll(e: &mut Engine, id: &str, clock: ViewClock, expression: GraphExpression) {
    let tick = (clock == ViewClock::Tick).then_some(0);
    e.register_view(
        &ViewDefinition {
            id: id.into(),
            clock,
            expression,
        },
        tick,
        &host(),
    )
    .unwrap();
    e.enroll_incremental_view(id, &host()).unwrap();
    e.enable_view_schedule(id, &host()).unwrap();
}
fn scan(e: &mut Engine, views: usize) -> ViewScanProgress {
    e.scan_view_work(
        &host(),
        ViewScanBudget {
            max_events: 16,
            max_views: views,
        },
    )
    .unwrap()
}
fn drain(e: &mut Engine) -> ViewWorkOutcome {
    e.drain_view_work(&host()).unwrap().unwrap()
}
#[test]
fn paged_fanout_coalescing_and_restart_keep_latest_state_without_extra_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.db");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, data(0), 0);
    for i in 0..3 {
        enroll(&mut e, &format!("v{i}"), ViewClock::Tick, query());
        drain(&mut e);
    }
    write(&mut e, data(1), 1);
    write(&mut e, data(2), -1);
    let p = scan(&mut e, 1);
    assert_eq!(p.view_notifications, 1);
    assert_eq!(p.events_completed, 0);
    drop(e);
    let mut e = Engine::open(&path).unwrap();
    for _ in 0..8 {
        scan(&mut e, 1);
    }
    e.request_view_tick("v0", 5, &host()).unwrap();
    e.request_view_tick("v0", 11, &host()).unwrap();
    assert_eq!(
        e.read_view("v0", Some(0), ViewFreshness::RequireCurrent, &host())
            .unwrap_err()
            .code,
        "E_FRESHNESS"
    );
    assert_eq!(
        e.read_view("v0", Some(11), ViewFreshness::RequireCurrent, &host())
            .unwrap_err()
            .code,
        "E_FRESHNESS"
    );
    for _ in 0..3 {
        let out = drain(&mut e);
        assert_eq!(out.generation, 2);
        assert!(out.current);
        let tick = if out.view_id == "v0" { 11 } else { 0 };
        let v = e
            .read_view(
                &out.view_id,
                Some(tick),
                ViewFreshness::RequireCurrent,
                &host(),
            )
            .unwrap();
        assert!(v.result.graph.edges.is_empty());
        let mut q: QueryPlan = serde_json::from_value(json!({"graph_id":"g"})).unwrap();
        q.valid_at = Some(tick);
        assert_eq!(v.result, e.query(&q, &host()).unwrap());
    }
    assert!(e.drain_view_work(&host()).unwrap().is_none());
    assert_eq!(e.event_count().unwrap(), 3);
    let c = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM view_schedules WHERE pending=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let manifest: String = c
        .query_row(
            "SELECT processed_manifest FROM view_schedules WHERE id='v0'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let m: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(m["tick"], 11);
    assert_eq!(m["generation"], 2);
    assert_eq!(
        m["input_snapshots"][0]["revision"],
        e.head("g", "main").unwrap().unwrap()
    );
}
#[test]
fn failing_oldest_rotates_durably_and_manual_tick_cannot_poison_queue() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.db");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, data(0), 0);
    enroll(
        &mut e,
        "bad",
        ViewClock::Fixed,
        GraphExpression::Project {
            input: Box::new(query()),
            node_ids: vec![],
            edge_ids: vec!["e".into()],
        },
    );
    drain(&mut e);
    enroll(&mut e, "good", ViewClock::Tick, query());
    drain(&mut e);
    write(&mut e, data(1), -1);
    scan(&mut e, 16);
    assert_eq!(
        e.drain_view_work(&host()).unwrap_err().code,
        "E_PROJECT_MEMBER"
    );
    drop(e);
    let mut e = Engine::open(&path).unwrap();
    e.request_view_tick("good", 5, &host()).unwrap();
    e.refresh_view("good", Some(11), &host()).unwrap();
    let out = drain(&mut e);
    assert_eq!(out.view_id, "good");
    assert_eq!(out.generation, 2);
    assert_eq!(
        e.read_view("good", Some(11), ViewFreshness::RequireCurrent, &host())
            .unwrap()
            .tick,
        Some(11)
    );
    let c = rusqlite::Connection::open(path).unwrap();
    let row: (i64, i64, Option<String>) = c
        .query_row(
            "SELECT pending,attempts,processed_manifest FROM view_schedules WHERE id='bad'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((row.0, row.1), (1, 1));
    let m: serde_json::Value = serde_json::from_str(&row.2.unwrap()).unwrap();
    assert_eq!(m["generation"], 1);
}
struct Counting(AtomicUsize);
impl TrustedClock for Counting {
    fn unix_millis(&self) -> std::result::Result<i64, weave_engine::Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(20)
    }
}
#[test]
fn one_successful_publication_captures_one_clock_and_two_workers_publish_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.db");
    let clock = Arc::new(Counting(AtomicUsize::new(0)));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    write(&mut e, data(0), 0);
    enroll(&mut e, "v", ViewClock::Fixed, query());
    clock.0.store(0, Ordering::SeqCst);
    drain(&mut e);
    assert_eq!(clock.0.load(Ordering::SeqCst), 1);
    write(&mut e, data(1), 1);
    scan(&mut e, 16);
    drop(e);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let threads = (0..2)
        .map(|_| {
            let path = path.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                let mut e = Engine::open(path).unwrap();
                b.wait();
                e.drain_view_work(&host()).unwrap().map(|o| o.generation)
            })
        })
        .collect::<Vec<_>>();
    let outcomes = threads
        .into_iter()
        .filter_map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes, [2]);
}
#[test]
fn pinned_sources_ignore_head_notifications_and_owner_lookup_isolated() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, data(0), 0);
    let pin = e.head("g", "main").unwrap().unwrap();
    let expression = GraphExpression::Query {
        query: serde_json::from_value(json!({"graph_id":"g","revision":pin})).unwrap(),
    };
    enroll(&mut e, "v", ViewClock::Fixed, expression);
    drain(&mut e);
    write(&mut e, data(1), 1);
    assert_eq!(scan(&mut e, 16).view_notifications, 0);
    assert!(e.drain_view_work(&host()).unwrap().is_none());
    assert!(e
        .drain_view_work(&HostContext::new("bob", []))
        .unwrap()
        .is_none());
    assert_eq!(
        e.enable_view_schedule("v", &HostContext::new("bob", []))
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
    assert_eq!(
        e.read_view("v", None, ViewFreshness::RequireCurrent, &host())
            .unwrap()
            .generation,
        1
    );
}
#[cfg(feature = "recovery-testing")]
#[test]
fn observer_unwind_rolls_back_publication_and_preserves_pending_work() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, data(0), 0);
    enroll(&mut e, "v", ViewClock::Fixed, query());
    drain(&mut e);
    write(&mut e, data(1), 1);
    scan(&mut e, 16);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || e.drain_view_work_test_before_commit(&host(), || panic!("observer"))
    ))
    .is_err());
    assert_eq!(
        e.read_view("v", None, ViewFreshness::AllowStale, &host())
            .unwrap()
            .generation,
        1
    );
    assert_eq!(drain(&mut e).generation, 2);
    assert!(e.drain_view_work(&host()).unwrap().is_none());
}

#[test]
fn queued_fallback_rechecks_governance_expiry_without_head_change() {
    use ed25519_dalek::SigningKey;
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    write(&mut e, data(0), 0);
    let key = SigningKey::from_bytes(&[91; 32]);
    let reference = GovernancePolicyRef {
        id: "policy".into(),
        revision: "1".into(),
    };
    e.install_governance_root(&GovernancePolicy {
        view_id: "team".into(),
        reference: reference.clone(),
        members: vec![weave_policy::public_key(&key)],
        threshold: 1,
        proposers: vec!["alice".into()],
        readers: vec![],
        allowed_sources: vec![GovernanceSourceScope {
            graph_id: "g".into(),
            branch_id: "main".into(),
        }],
        not_before_ms: 0,
        expires_at_ms: 100,
    })
    .unwrap();
    let p = GovernanceProposal {
        id: "publish".into(),
        view_id: "team".into(),
        policy: reference.clone(),
        expected_head: None,
        expires_at_ms: 90,
        action: GovernanceAction::Publish {
            source: GraphRef {
                graph_id: "g".into(),
                revision: e.head("g", "main").unwrap().unwrap(),
            },
            branch_id: "main".into(),
        },
    };
    let receipt = e.propose_governance(&p, &host()).unwrap();
    let signed = sign_governance_approval(
        GovernanceApproval {
            proposal_id: p.id.clone(),
            proposal_digest: receipt.digest,
            view_id: p.view_id,
            policy: reference,
            expected_head: None,
            member: weave_policy::public_key(&key),
            issued_at_ms: 0,
            expires_at_ms: 80,
            nonce: "vote".into(),
        },
        &key,
    )
    .unwrap();
    e.record_governance_approval(&signed, &host()).unwrap();
    e.accept_governance(
        &GovernanceDecisionRequest {
            proposal_id: p.id,
            nonce: "accept".into(),
        },
        &host(),
    )
    .unwrap();
    let accepted = e
        .query_accepted_view(
            &AcceptedViewSelection {
                view_id: "team".into(),
                decision_id: None,
            },
            &host(),
        )
        .unwrap();
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "saved".into(),
                branch_id: "main".into(),
                expected_head: None,
                data: accepted.graph,
            }],
        },
        &host(),
    )
    .unwrap();
    enroll(
        &mut e,
        "v",
        ViewClock::Fixed,
        GraphExpression::Query {
            query: serde_json::from_value(json!({"graph_id":"saved"})).unwrap(),
        },
    );
    let head = e.head("saved", "main").unwrap();
    let events = e.event_count().unwrap();
    clock.set(100);
    assert!(e
        .read_view("v", None, ViewFreshness::AllowStale, &host())
        .is_err());
    let out = drain(&mut e);
    assert!(!out.current);
    assert_eq!(out.work.fallback_runs, 1);
    assert!(e
        .read_view("v", None, ViewFreshness::RequireCurrent, &host())
        .is_err());
    assert!(e.view_changes("v", 1, &host()).is_err());
    assert_eq!(e.head("saved", "main").unwrap(), head);
    assert_eq!(e.event_count().unwrap(), events);
}
