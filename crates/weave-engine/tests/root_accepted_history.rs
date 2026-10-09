use ed25519_dalek::SigningKey;
use serde_json::json;
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("collector", ["source".into()])
}
fn policy() -> GovernancePolicy {
    GovernancePolicy {
        view_id: "team".into(),
        reference: GovernancePolicyRef {
            id: "policy".into(),
            revision: "1".into(),
        },
        members: vec![weave_policy::public_key(&SigningKey::from_bytes(&[73; 32]))],
        threshold: 1,
        proposers: vec!["collector".into()],
        readers: vec![],
        allowed_sources: vec![GovernanceSourceScope {
            graph_id: "source".into(),
            branch_id: "main".into(),
        }],
        not_before_ms: 0,
        expires_at_ms: 10000,
    }
}
fn write(e: &mut Engine, value: i64, private: bool) -> GraphRef {
    let p=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"source","expected_head":e.head("source","main").unwrap(),"data":{"nodes":[{"id":"a","entity_id":"A","space_id":"s","properties":{"value":value},"readers":if private {vec!["collector"]} else {vec![]}}]}}]})).unwrap();
    e.execute(&p, &host()).unwrap();
    GraphRef {
        graph_id: "source".into(),
        revision: e.head("source", "main").unwrap().unwrap(),
    }
}
fn propose(e: &Engine, id: &str, action: GovernanceAction) -> GovernanceDecisionRequest {
    let head = e.inspect_governance_head("team", &host()).unwrap();
    let p = GovernanceProposal {
        id: id.into(),
        view_id: "team".into(),
        policy: head.policy,
        expected_head: head.decision_id,
        expires_at_ms: 9000,
        action,
    };
    let receipt = e.propose_governance(&p, &host()).unwrap();
    let key = SigningKey::from_bytes(&[73; 32]);
    let signed = sign_governance_approval(
        GovernanceApproval {
            proposal_id: id.into(),
            proposal_digest: receipt.digest,
            view_id: "team".into(),
            policy: p.policy,
            expected_head: p.expected_head,
            member: weave_policy::public_key(&key),
            issued_at_ms: 0,
            expires_at_ms: 8000,
            nonce: id.into(),
        },
        &key,
    )
    .unwrap();
    e.record_governance_approval(&signed, &host()).unwrap();
    GovernanceDecisionRequest {
        proposal_id: id.into(),
        nonce: id.into(),
    }
}
fn accept(e: &Engine, id: &str, source: GraphRef) -> GovernanceReceipt {
    let request = propose(
        e,
        id,
        GovernanceAction::Publish {
            source,
            branch_id: "main".into(),
        },
    );
    e.accept_governance(&request, &host()).unwrap()
}
fn observer(e: &Engine) -> String {
    e.recorded_checkpoint_for("source", "main", &host())
        .unwrap()
        .observer
}
fn at(observer: &str, time: i64) -> AcceptedViewHistoryCut {
    AcceptedViewHistoryCut::AtTime {
        observer: observer.into(),
        unix_millis: time,
    }
}
#[test]
fn acceptance_cut_tracks_genuine_view_time_separately_from_source_recording() {
    let clock = Arc::new(ManualClock::new(5));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    e.install_governance_root(&policy()).unwrap();
    let old = write(&mut e, 1, false);
    let observer = observer(&e);
    clock.set(10);
    let first = accept(&e, "first", old.clone());
    clock.set(20);
    let new = write(&mut e, 2, false);
    assert_eq!(e.recorded_at(&new.revision).unwrap(), 20);
    clock.set(50);
    let second = accept(&e, "second", new);
    clock.set(60);
    let historical = e
        .query_accepted_history_for("team", &at(&observer, 25), &host())
        .unwrap();
    assert_eq!(historical.observation.decision_id, first.decision_id);
    assert_eq!(historical.observation.accepted_at_ms, 10);
    assert_eq!(historical.observation.source, old);
    assert_eq!(
        historical.result.graph.nodes[0].properties["value"],
        json!(1)
    );
    let latest = e
        .query_accepted_history_for("team", &at(&observer, 50), &host())
        .unwrap();
    assert_eq!(latest.observation.decision_id, second.decision_id);
    assert_eq!(latest.result.graph.nodes[0].properties["value"], json!(2));
    assert_eq!(
        e.query_accepted_history_for(
            "team",
            &AcceptedViewHistoryCut::Decision {
                observer,
                decision_id: first.decision_id
            },
            &host()
        )
        .unwrap(),
        historical
    );
}
#[test]
fn half_open_range_retains_order_and_denies_overflow_or_hidden_selected_occurrences() {
    let clock = Arc::new(ManualClock::new(5));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    e.install_governance_root(&policy()).unwrap();
    let source = write(&mut e, 1, false);
    let observer = observer(&e);
    clock.set(10);
    let first = accept(&e, "first", source.clone());
    clock.set(20);
    let second = accept(&e, "second", source);
    clock.set(30);
    let range = e
        .accepted_history_range_for(
            "team",
            &observer,
            &Interval {
                start: 10,
                end: Some(20),
            },
            10,
            &host(),
        )
        .unwrap();
    assert_eq!(range.start_state.observation.decision_id, first.decision_id);
    assert_eq!(range.changes.len(), 1);
    assert_eq!(range.changes[0].observation.decision_id, first.decision_id);
    let full = e
        .accepted_history_range_for(
            "team",
            &observer,
            &Interval {
                start: 10,
                end: Some(30),
            },
            2,
            &host(),
        )
        .unwrap();
    assert_eq!(full.changes[1].observation.decision_id, second.decision_id);
    assert_eq!(
        e.accepted_history_range_for(
            "team",
            &observer,
            &Interval {
                start: 10,
                end: Some(30)
            },
            1,
            &host()
        )
        .unwrap_err()
        .code,
        "E_GOV_HISTORY_UNAVAILABLE"
    );
    let private = write(&mut e, 3, true);
    clock.set(40);
    accept(&e, "private", private);
    clock.set(50);
    let bob = HostContext::new("bob", []);
    assert!(e
        .query_accepted_history_for("team", &at(&observer, 15), &bob)
        .is_ok());
    assert_eq!(
        e.query_accepted_history_for("team", &at(&observer, 45), &bob)
            .unwrap_err()
            .code,
        "E_GOV_HISTORY_UNAVAILABLE"
    );
    assert!(e
        .accepted_history_range_for(
            "team",
            &observer,
            &Interval {
                start: 10,
                end: Some(50)
            },
            10,
            &bob
        )
        .is_err());
}
#[test]
fn equal_time_acceptance_uses_ancestry_and_regression_is_atomic() {
    let clock = Arc::new(ManualClock::new(5));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store");
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    e.install_governance_root(&policy()).unwrap();
    let source = write(&mut e, 1, false);
    let observer = observer(&e);
    clock.set(10);
    accept(&e, "first", source.clone());
    let second = accept(&e, "second", source.clone());
    assert_eq!(
        e.query_accepted_history_for("team", &at(&observer, 10), &host())
            .unwrap()
            .observation
            .decision_id,
        second.decision_id
    );
    let request = propose(
        &e,
        "third",
        GovernanceAction::Publish {
            source,
            branch_id: "main".into(),
        },
    );
    let before = e.governance_event_count().unwrap();
    drop(e);
    clock.set(9);
    let e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    assert_eq!(
        e.accept_governance(&request, &host()).unwrap_err().code,
        "E_GOV_HISTORY_TIME"
    );
    assert_eq!(e.governance_event_count().unwrap(), before);
    assert_eq!(
        e.inspect_governance_head("team", &host())
            .unwrap()
            .decision_id,
        Some(second.decision_id)
    );
    clock.set(11);
    assert!(e.accept_governance(&request, &host()).is_ok());
}
#[test]
fn foreign_missing_future_and_cross_view_selectors_never_install_authority() {
    let clock = Arc::new(ManualClock::new(5));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    e.install_governance_root(&policy()).unwrap();
    let source = write(&mut e, 1, false);
    let observer = observer(&e);
    clock.set(10);
    let first = accept(&e, "first", source);
    clock.set(20);
    assert!(e
        .query_accepted_history_for("team", &at("foreign", 15), &host())
        .is_err());
    assert!(e
        .query_accepted_history_for("team", &at(&observer, 9), &host())
        .is_err());
    assert_eq!(
        e.query_accepted_history_for("team", &at(&observer, 21), &host())
            .unwrap_err()
            .code,
        "E_GOV_HISTORY_TIME"
    );
    assert!(e
        .query_accepted_history_for(
            "other",
            &AcceptedViewHistoryCut::Decision {
                observer: observer.clone(),
                decision_id: first.decision_id
            },
            &host()
        )
        .is_err());
    assert!(e
        .query_accepted_history_for(
            "team",
            &AcceptedViewHistoryCut::Decision {
                observer,
                decision_id: "missing".into()
            },
            &host()
        )
        .is_err());
}
#[test]
fn date_walk_cannot_skip_a_corrupt_missing_or_reparented_intermediate_decision() {
    for mutation in [
        "DELETE FROM governance_decisions WHERE proposal_id='second'",
        "UPDATE governance_decisions SET parent=NULL WHERE proposal_id='second'",
        "UPDATE governance_decisions SET accepted_at_ms=11 WHERE proposal_id='second'",
        "UPDATE governance_approvals SET body='{}' WHERE proposal_id='second'",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store");
        let clock = Arc::new(ManualClock::new(5));
        let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
        e.install_governance_root(&policy()).unwrap();
        let source = write(&mut e, 1, false);
        let observer = observer(&e);
        clock.set(10);
        accept(&e, "first", source.clone());
        clock.set(20);
        accept(&e, "second", source.clone());
        clock.set(30);
        accept(&e, "third", source);
        drop(e);
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute(mutation, [])
            .unwrap();
        clock.set(40);
        let e = Engine::open_with_clock(&path, clock).unwrap();
        assert!(
            e.query_accepted_history_for("team", &at(&observer, 15), &host())
                .is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn expired_historical_approvals_do_not_replace_current_policy_authority() {
    let clock = Arc::new(ManualClock::new(5));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    e.install_governance_root(&policy()).unwrap();
    let source = write(&mut e, 1, false);
    let observer = observer(&e);
    clock.set(10);
    let first = accept(&e, "first", source);
    let reader = HostContext::new("reader", []);
    assert!(e
        .query_accepted_history_for("team", &at(&observer, 10), &reader)
        .is_ok());
    clock.set(40);
    let mut next = policy();
    next.reference.revision = "2".into();
    next.expires_at_ms = 20000;
    next.readers = vec!["collector".into()];
    let request = propose(
        &e,
        "replace",
        GovernanceAction::ReplacePolicy { policy: next },
    );
    e.accept_governance(&request, &host()).unwrap();
    clock.set(11000);
    let old = e
        .query_accepted_history_for("team", &at(&observer, 10), &host())
        .unwrap();
    assert_eq!(old.observation.decision_id, first.decision_id);
    assert_eq!(old.result.graph.nodes[0].properties["value"], json!(1));
    assert!(e
        .query_accepted_history_for("team", &at(&observer, 10), &reader)
        .is_err());
    let current = e
        .query_accepted_history_for("team", &at(&observer, 50), &host())
        .unwrap();
    assert_eq!(current.observation.accepted_at_ms, 40);
    assert_eq!(current.observation.source, old.observation.source);
    assert!(current
        .result
        .input_snapshots
        .contains(&old.observation.occurrence));
}
