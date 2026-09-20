#[path = "support/clock.rs"]
mod test_clock;
use ed25519_dalek::SigningKey;
use serde_json::json;
use weave_contract::{Command, GraphData, GraphRef, Program, VERSION};
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("collector", ["source".into()])
}
fn keys() -> [SigningKey; 3] {
    [
        SigningKey::from_bytes(&[31; 32]),
        SigningKey::from_bytes(&[32; 32]),
        SigningKey::from_bytes(&[33; 32]),
    ]
}
fn policy(threshold: usize) -> GovernancePolicy {
    GovernancePolicy {
        view_id: "team".into(),
        reference: GovernancePolicyRef {
            id: "policy".into(),
            revision: "1".into(),
        },
        members: keys()[..2].iter().map(weave_policy::public_key).collect(),
        threshold,
        proposers: vec!["collector".into()],
        readers: vec!["reader".into()],
        allowed_sources: vec![GovernanceSourceScope {
            graph_id: "source".into(),
            branch_id: "main".into(),
        }],
        not_before_ms: 0,
        expires_at_ms: 10000,
    }
}
fn seed(e: &mut Engine, threshold: usize) -> GraphRef {
    e.install_governance_root(&policy(threshold)).unwrap();
    let data: GraphData =
        serde_json::from_value(json!({"nodes":[{"id":"n","entity_id":"E","space_id":"s"}]}))
            .unwrap();
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "source".into(),
                branch_id: "main".into(),
                expected_head: None,
                data,
            }],
        },
        &host(),
    )
    .unwrap();
    GraphRef {
        graph_id: "source".into(),
        revision: e.head("source", "main").unwrap().unwrap(),
    }
}
fn proposal(id: &str, source: GraphRef, expected_head: Option<String>) -> GovernanceProposal {
    GovernanceProposal {
        id: id.into(),
        view_id: "team".into(),
        policy: policy(2).reference,
        expected_head,
        expires_at_ms: 9000,
        action: GovernanceAction::Publish {
            source,
            branch_id: "main".into(),
        },
    }
}
fn signed(
    q: &GovernanceProposal,
    hash: &str,
    key: &SigningKey,
    nonce: &str,
) -> SignedGovernanceApproval {
    sign_governance_approval(
        GovernanceApproval {
            proposal_id: q.id.clone(),
            proposal_digest: hash.into(),
            view_id: q.view_id.clone(),
            policy: q.policy.clone(),
            expected_head: q.expected_head.clone(),
            member: weave_policy::public_key(key),
            issued_at_ms: 10,
            expires_at_ms: 8000,
            nonce: nonce.into(),
        },
        key,
    )
    .unwrap()
}
fn request(id: &str, nonce: &str) -> GovernanceDecisionRequest {
    GovernanceDecisionRequest {
        proposal_id: id.into(),
        nonce: nonce.into(),
    }
}
fn quorum(e: &Engine, q: &GovernanceProposal) -> GovernanceProposalReceipt {
    let r = test_clock::at(20, || e.propose_governance(q, &host())).unwrap();
    for (i, key) in keys()[..2].iter().enumerate() {
        test_clock::at(20, || {
            e.record_governance_approval(
                &signed(q, &r.digest, key, &format!("{}-{i}", q.id)),
                &host(),
            )
        })
        .unwrap();
    }
    r
}

fn adapter(id: &str, principal: &str) -> AdapterManifest {
    AdapterManifest {
        id: id.into(),
        version: "1".into(),
        artifact_digest: format!("sha256:{}", "0".repeat(64)),
        config_revision: "1".into(),
        principal: principal.into(),
        subscriptions: vec![SubscriptionScope {
            graph_id: "source".into(),
            branch_id: "main".into(),
        }],
        output_graphs: vec![],
        effect_destinations: vec![],
        max_attempts: 2,
        lease_ms: 100,
        max_pending_events: 100,
        projection_replay: false,
    }
}
fn install(e: &Engine, id: &str, principal: &str) -> HostContext {
    let h = HostContext::new(principal, []);
    e.install_adapter(&adapter(id, principal), &h).unwrap();
    e.set_adapter_state(id, "running").unwrap();
    test_clock::at(20, || e.subscribe_governance(id, "team", &h)).unwrap();
    h
}
fn accepted(e: &Engine, id: &str, source: GraphRef, head: Option<String>) -> GovernanceReceipt {
    quorum(e, &proposal(id, source, head));
    test_clock::at(20, || e.accept_governance(&request(id, id), &host())).unwrap()
}
#[test]
fn delivery_restart_pause_lease_rotation_and_ack_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delivery.db");
    let mut e = test_clock::open(&path).unwrap();
    let source = seed(&mut e, 2);
    accepted(&e, "a", source, None);
    let reader = install(&e, "reader", "reader");
    let first = test_clock::at(20, || e.poll_governance("reader", "team", &reader))
        .unwrap()
        .unwrap();
    assert_eq!(first.ordinal, 1);
    assert_eq!(first.event.event_type, "view.accepted");
    e.set_adapter_state("reader", "paused").unwrap();
    assert_eq!(
        test_clock::at(21, || e.acknowledge_governance(
            "reader",
            "team",
            &first.event.id,
            &first.lease,
            &reader
        ))
        .unwrap_err()
        .code,
        "E_PAUSED"
    );
    e.set_adapter_state("reader", "running").unwrap();
    let renewed = test_clock::at(22, || e.poll_governance("reader", "team", &reader))
        .unwrap()
        .unwrap();
    assert_eq!(renewed.ordinal, first.ordinal);
    assert_ne!(renewed.lease, first.lease);
    assert_eq!(
        test_clock::at(22, || e.acknowledge_governance(
            "reader",
            "team",
            &first.event.id,
            &first.lease,
            &reader
        ))
        .unwrap_err()
        .code,
        "E_LEASE"
    );
    assert!(
        !test_clock::at(22, || e.acknowledge_governance(
            "reader",
            "team",
            &renewed.event.id,
            &renewed.lease,
            &reader
        ))
        .unwrap()
        .duplicate
    );
    drop(e);
    let e = test_clock::open(&path).unwrap();
    assert!(
        test_clock::at(23, || e.acknowledge_governance(
            "reader",
            "team",
            &renewed.event.id,
            &renewed.lease,
            &reader
        ))
        .unwrap()
        .duplicate
    );
    assert!(
        test_clock::at(23, || e.poll_governance("reader", "team", &reader))
            .unwrap()
            .is_none()
    );
    assert_eq!(e.event_count().unwrap(), 1);
    assert_eq!(e.governance_event_count().unwrap(), 1);
    e.cancel_governance_subscription("reader", "team", &reader)
        .unwrap();
    e.set_adapter_state("reader", "removed").unwrap();
    assert!(test_clock::at(24, || e.acknowledge_governance(
        "reader",
        "team",
        &renewed.event.id,
        &renewed.lease,
        &reader
    ))
    .is_err());
}
#[test]
fn current_reader_revocation_blocks_pending_and_acknowledged_payloads() {
    let mut e = test_clock::memory().unwrap();
    let source = seed(&mut e, 2);
    let first = accepted(&e, "a", source, None);
    let reader = install(&e, "pending", "reader");
    install(&e, "acked", "reader");
    let pending = test_clock::at(20, || e.poll_governance("pending", "team", &reader))
        .unwrap()
        .unwrap();
    let acked = test_clock::at(20, || e.poll_governance("acked", "team", &reader))
        .unwrap()
        .unwrap();
    test_clock::at(20, || {
        e.acknowledge_governance("acked", "team", &acked.event.id, &acked.lease, &reader)
    })
    .unwrap();
    let mut next = policy(2);
    next.reference.revision = "2".into();
    next.readers = vec!["collector".into()];
    let q = GovernanceProposal {
        id: "remove-reader".into(),
        view_id: "team".into(),
        policy: policy(2).reference,
        expected_head: Some(first.decision_id),
        expires_at_ms: 9000,
        action: GovernanceAction::ReplacePolicy { policy: next },
    };
    quorum(&e, &q);
    test_clock::at(21, || {
        e.accept_governance(&request("remove-reader", "remove-reader"), &host())
    })
    .unwrap();
    assert!(test_clock::at(22, || e.poll_governance("pending", "team", &reader)).is_err());
    for (id, delivery) in [("pending", pending), ("acked", acked)] {
        assert_eq!(
            test_clock::at(22, || e.acknowledge_governance(
                id,
                "team",
                &delivery.event.id,
                &delivery.lease,
                &reader
            ))
            .unwrap_err()
            .code,
            "E_GOV_UNAVAILABLE"
        );
    }
    // Own cleanup is possible despite revoked view authority.
    e.cancel_governance_subscription("pending", "team", &reader)
        .unwrap();
    e.set_adapter_state("pending", "removed").unwrap();
}
#[test]
fn hidden_historical_source_and_policy_occurrences_leave_no_delivery_gaps() {
    let mut e = test_clock::memory().unwrap();
    let initial = seed(&mut e, 2);
    let commit = |e: &mut Engine, prior: &str, private: bool| {
        e.execute(&serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"source","branch_id":"main","expected_head":prior,"data":{"nodes":[{"id":"n","entity_id":"E","space_id":"s","readers":if private {vec!["collector"]} else {vec![]}}]}}]})).unwrap(),&host()).unwrap();
        GraphRef {
            graph_id: "source".into(),
            revision: e.head("source", "main").unwrap().unwrap(),
        }
    };
    let private = commit(&mut e, &initial.revision, true);
    let a = accepted(&e, "private", private.clone(), None);
    let mut next = policy(2);
    next.reference.revision = "2".into();
    let q = GovernanceProposal {
        id: "policy".into(),
        view_id: "team".into(),
        policy: policy(2).reference,
        expected_head: Some(a.decision_id),
        expires_at_ms: 9000,
        action: GovernanceAction::ReplacePolicy {
            policy: next.clone(),
        },
    };
    quorum(&e, &q);
    let transition = test_clock::at(20, || {
        e.accept_governance(&request("policy", "policy"), &host())
    })
    .unwrap();
    let public = commit(&mut e, &private.revision, false);
    let mut q = proposal("public", public, Some(transition.decision_id));
    q.policy = next.reference;
    quorum(&e, &q);
    let visible = test_clock::at(20, || {
        e.accept_governance(&request("public", "public"), &host())
    })
    .unwrap();
    let reader = install(&e, "reader", "reader");
    let delivery = test_clock::at(21, || e.poll_governance("reader", "team", &reader))
        .unwrap()
        .unwrap();
    assert_eq!(delivery.event.id, visible.event_id);
    assert_eq!(delivery.ordinal, 1);
    assert_eq!(e.governance_event_count().unwrap(), 3);
}
#[test]
fn expiration_dead_letter_drain_cancel_and_removal_reject_stale_lease() {
    let mut e = test_clock::memory().unwrap();
    let source = seed(&mut e, 2);
    accepted(&e, "a", source, None);
    let reader = install(&e, "reader", "reader");
    let first = test_clock::at(20, || e.poll_governance("reader", "team", &reader))
        .unwrap()
        .unwrap();
    assert!(e.set_adapter_state("reader", "removed").is_err());
    assert_eq!(
        test_clock::at(120, || e.acknowledge_governance(
            "reader",
            "team",
            &first.event.id,
            &first.lease,
            &reader
        ))
        .unwrap_err()
        .code,
        "E_LEASE"
    );
    let second = test_clock::at(120, || e.poll_governance("reader", "team", &reader))
        .unwrap()
        .unwrap();
    assert_ne!(second.lease, first.lease);
    assert!(
        test_clock::at(220, || e.poll_governance("reader", "team", &reader))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        test_clock::at(-1, || e
            .replay_governance_dead_letter("reader", "team", &reader))
        .unwrap_err()
        .code,
        "E_CLOCK_UNAVAILABLE"
    );
    test_clock::at(221, || {
        e.replay_governance_dead_letter("reader", "team", &reader)
    })
    .unwrap();
    let replay = test_clock::at(221, || e.poll_governance("reader", "team", &reader))
        .unwrap()
        .unwrap();
    e.set_adapter_state("reader", "draining").unwrap();
    test_clock::at(222, || {
        e.acknowledge_governance("reader", "team", &replay.event.id, &replay.lease, &reader)
    })
    .unwrap();
    assert!(
        test_clock::at(222, || e.poll_governance("reader", "team", &reader))
            .unwrap()
            .is_none()
    );
    e.set_adapter_state("reader", "paused").unwrap();
    e.set_adapter_state("reader", "running").unwrap();
    assert_eq!(
        test_clock::at(223, || e.acknowledge_governance(
            "reader",
            "team",
            &replay.event.id,
            &replay.lease,
            &reader
        ))
        .unwrap_err()
        .code,
        "E_LEASE"
    );
    e.cancel_governance_subscription("reader", "team", &reader)
        .unwrap();
    assert!(test_clock::at(224, || e.subscribe_governance("reader", "team", &reader)).is_err());
    e.set_adapter_state("reader", "removed").unwrap();
}

#[test]
fn current_policy_expiry_blocks_inspection_pending_and_cached_ack_without_head_change() {
    let clock = std::sync::Arc::new(ManualClock::new(20));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    let source = seed(&mut e, 2);
    let decision = accepted(&e, "a", source, None);
    let reader = install(&e, "reader", "reader");
    let other = install(&e, "pending-reader", "reader");
    let before = clock.samples();
    let delivery = e
        .poll_governance("reader", "team", &reader)
        .unwrap()
        .unwrap();
    assert_eq!(clock.samples(), before + 1); // event inspection/source checks share one clock
    e.acknowledge_governance(
        "reader",
        "team",
        &delivery.event.id,
        &delivery.lease,
        &reader,
    )
    .unwrap();
    e.poll_governance("pending-reader", "team", &other)
        .unwrap()
        .unwrap();
    clock.set(9999);
    assert_eq!(
        e.inspect_governance_head("team", &reader)
            .unwrap()
            .decision_id,
        Some(decision.decision_id)
    );
    clock.set(10000);
    assert_eq!(
        e.inspect_governance_head("team", &reader).unwrap_err().code,
        "E_GOV_POLICY"
    );
    assert_eq!(
        e.poll_governance("pending-reader", "team", &other)
            .unwrap_err()
            .code,
        "E_GOV_POLICY"
    );
    assert_eq!(
        e.acknowledge_governance(
            "reader",
            "team",
            &delivery.event.id,
            &delivery.lease,
            &reader
        )
        .unwrap_err()
        .code,
        "E_GOV_POLICY"
    );
    assert_eq!(e.event_count().unwrap(), 1);
    assert_eq!(e.governance_event_count().unwrap(), 1);
}

#[cfg(feature = "recovery-testing")]
#[test]
fn delivery_observer_panic_rolls_back_and_preserves_retry() {
    let mut e = test_clock::memory().unwrap();
    let source = seed(&mut e, 2);
    accepted(&e, "a", source, None);
    let reader = install(&e, "reader", "reader");
    let first = e
        .poll_governance("reader", "team", &reader)
        .unwrap()
        .unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        e.acknowledge_governance_test_before_commit(
            "reader",
            "team",
            &first.event.id,
            &first.lease,
            &reader,
            || panic!("observer"),
        )
    }));
    assert!(panic.is_err());
    assert!(
        !e.acknowledge_governance("reader", "team", &first.event.id, &first.lease, &reader)
            .unwrap()
            .duplicate
    );
    assert!(
        e.acknowledge_governance("reader", "team", &first.event.id, &first.lease, &reader)
            .unwrap()
            .duplicate
    );
}
