use ed25519_dalek::SigningKey;
use serde_json::json;
use std::sync::Arc;
use weave_contract::{GraphData, GraphRef, GraphSchema, Program, VERSION};
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("owner", ["request".into(), "private".into()])
}
fn schema() -> GraphSchema {
    serde_json::from_value(json!({"id":"request-schema","revision":"1","nodes":{},"edges":{}}))
        .unwrap()
}
fn policy() -> GovernancePolicy {
    GovernancePolicy {
        view_id: "requests".into(),
        reference: GovernancePolicyRef {
            id: "policy".into(),
            revision: "1".into(),
        },
        members: vec![weave_policy::public_key(&SigningKey::from_bytes(&[7; 32]))],
        threshold: 1,
        proposers: vec!["owner".into()],
        readers: vec!["owner".into()],
        allowed_sources: vec![GovernanceSourceScope {
            graph_id: "request".into(),
            branch_id: "main".into(),
        }],
        not_before_ms: 0,
        expires_at_ms: 10_000,
    }
}
fn engine() -> (Engine, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::new(10));
    let e = Engine::memory_with_clock(clock.clone()).unwrap();
    e.install_governance_root(&policy()).unwrap();
    (e, clock)
}
fn write(e: &mut Engine, graph: &str, data: GraphData) -> GraphRef {
    let p:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":e.head(graph,"main").unwrap(),"data":data}]})).unwrap();
    e.execute(&p, &host()).unwrap();
    GraphRef {
        graph_id: graph.into(),
        revision: e.head(graph, "main").unwrap().unwrap(),
    }
}
fn publish(e: &Engine, source: GraphRef, id: &str) -> GovernanceReceipt {
    let proposal = GovernanceProposal {
        id: id.into(),
        view_id: "requests".into(),
        policy: policy().reference,
        expected_head: e
            .inspect_governance_head("requests", &host())
            .unwrap()
            .decision_id,
        expires_at_ms: 9000,
        action: GovernanceAction::Publish {
            source,
            branch_id: "main".into(),
        },
    };
    decide(e, proposal)
}
fn decide(e: &Engine, proposal: GovernanceProposal) -> GovernanceReceipt {
    let id = proposal.id.clone();
    let receipt = e.propose_governance(&proposal, &host()).unwrap();
    let key = SigningKey::from_bytes(&[7; 32]);
    let approval = sign_governance_approval(
        GovernanceApproval {
            proposal_id: id.clone(),
            proposal_digest: receipt.digest,
            view_id: proposal.view_id,
            policy: proposal.policy,
            expected_head: proposal.expected_head,
            member: weave_policy::public_key(&key),
            issued_at_ms: 0,
            expires_at_ms: 8000,
            nonce: format!("approval-{id}"),
        },
        &key,
    )
    .unwrap();
    e.record_governance_approval(&approval, &host()).unwrap();
    e.accept_governance(
        &GovernanceDecisionRequest {
            proposal_id: id.clone(),
            nonce: format!("decision-{id}"),
        },
        &host(),
    )
    .unwrap()
}
fn grant(id: &str, start: EffectStart) -> GovernedEffectGrant {
    GovernedEffectGrant {
        id: "execute".into(),
        revision: "1".into(),
        principal: "owner".into(),
        view_id: "requests".into(),
        source: SubscriptionScope {
            graph_id: "request".into(),
            branch_id: "main".into(),
        },
        request_schema: schema(),
        destination_id: "reference-sink".into(),
        destination_principal: "owner".into(),
        encoder: EffectEncoder::CanonicalGraphV1,
        execution_id: id.into(),
        start,
        not_before_ms: 0,
        expires_at_ms: 5000,
    }
}
fn manifest(id: &str, g: &GovernedEffectGrant) -> AdapterManifest {
    AdapterManifest {
        id: id.into(),
        version: "1".into(),
        artifact_digest: governed_effect_grant_digest(g).unwrap(),
        config_revision: "1".into(),
        principal: "owner".into(),
        subscriptions: vec![g.source.clone()],
        output_graphs: vec![],
        effect_destinations: vec![g.destination_id.clone()],
        max_attempts: 5,
        lease_ms: 100,
        max_pending_events: 100,
        projection_replay: false,
    }
}
fn install(e: &Engine, id: &str, g: &GovernedEffectGrant) {
    e.install_governed_effect(&manifest(id, g), g, &host())
        .unwrap();
    e.set_adapter_state(id, "running").unwrap();
}
fn request(e: &mut Engine) -> GraphRef {
    write(
        e,
        "request",
        GraphData {
            schema: Some(schema()),
            ..GraphData::default()
        },
    )
}
fn enqueue(e: &Engine, id: &str) -> (GovernanceDelivery, GovernedEffectReceipt) {
    let d = e.poll_governance(id, "requests", &host()).unwrap().unwrap();
    let r = e
        .enqueue_governed_effect(id, &d.event.id, &d.lease, &host())
        .unwrap();
    (d, r)
}
fn intent(r: &GovernedEffectReceipt) -> String {
    match &r.disposition {
        GovernedEffectDisposition::Intent { intent_id } => intent_id.clone(),
        x => panic!("not intent: {x:?}"),
    }
}
fn evidence() -> SinkEvidence {
    SinkEvidence {
        receipt_id: "sink-receipt".into(),
        response_digest: format!("sha256:{}", "0".repeat(64)),
    }
}
#[test]
fn genuine_empty_request_dispatch_fence_and_raw_bypasses() {
    let (mut e, _) = engine();
    let source = request(&mut e);
    publish(&e, source, "one");
    install(
        &e,
        "bridge",
        &grant("execution", EffectStart::ReplayHistory),
    );
    let (d, r) = enqueue(&e, "bridge");
    let id = intent(&r);
    assert!(
        e.enqueue_governed_effect("bridge", &d.event.id, &d.lease, &host())
            .unwrap()
            .duplicate
    );
    let empty: Program = serde_json::from_value(json!({"version":VERSION,"commands":[]})).unwrap();
    assert_eq!(
        e.complete_handler("bridge", &d.event.id, &d.lease, &empty)
            .unwrap_err()
            .code,
        "E_EFFECT_BOUND"
    );
    assert_eq!(
        e.request_effect(
            "bridge",
            &d.event.id,
            &d.lease,
            "reference-sink",
            "key",
            json!({})
        )
        .unwrap_err()
        .code,
        "E_EFFECT_BOUND"
    );
    assert_eq!(
        e.acknowledge_governance("bridge", "requests", &d.event.id, &d.lease, &host())
            .unwrap_err()
            .code,
        "E_EFFECT_BOUND"
    );
    assert!(e
        .subscribe_governance("bridge", "requests", &host())
        .is_err());
    assert!(e.poll_adapter("bridge").is_err());
    assert!(e.effect_intent(&id).is_err());
    assert!(e.begin_effect_dispatch(&id).is_err());
    assert!(e.reconcile_effect(&id, "confirmed", json!({})).is_err());
    let ticket = e.begin_governed_effect(&id, &host()).unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&ticket.payload).unwrap();
    assert_eq!(payload["format"], "weave-governed-graph-effect/1");
    assert_eq!(
        e.begin_governed_effect(&id, &host()).unwrap_err().code,
        "E_EFFECT_UNKNOWN"
    );
    assert!(e.cancel_governed_effect(&id, &host()).is_err());
    e.revoke_governed_effect_grant("bridge", &host()).unwrap();
    assert!(e.read_governed_effect(&id, &host()).is_err());
    e.reconcile_governed_effect(
        &id,
        &ticket.attempt_id,
        ReconciledOutcome::Confirmed,
        &evidence(),
        &host(),
    )
    .unwrap();
    e.reconcile_governed_effect(
        &id,
        &ticket.attempt_id,
        ReconciledOutcome::Confirmed,
        &evidence(),
        &host(),
    )
    .unwrap();
    assert!(e
        .reconcile_governed_effect(
            &id,
            &ticket.attempt_id,
            ReconciledOutcome::Failed,
            &evidence(),
            &host()
        )
        .is_err());
}
#[test]
fn superseded_publications_advance_and_execution_namespace_is_durable() {
    let (mut e, _) = engine();
    let source = request(&mut e);
    publish(&e, source.clone(), "one");
    publish(&e, source, "two");
    let g = grant("execution", EffectStart::ReplayHistory);
    install(&e, "bridge", &g);
    let (old, skipped) = enqueue(&e, "bridge");
    assert_eq!(
        skipped.disposition,
        GovernedEffectDisposition::SupersededPublication
    );
    assert!(
        e.enqueue_governed_effect("bridge", &old.event.id, &old.lease, &host())
            .unwrap()
            .duplicate
    );
    let (_, current) = enqueue(&e, "bridge");
    assert!(matches!(
        current.disposition,
        GovernedEffectDisposition::Intent { .. }
    ));
    assert_eq!(current.ordinal, skipped.ordinal + 1);
    assert!(e
        .install_governed_effect(&manifest("another", &g), &g, &host())
        .is_err());
    e.revoke_governed_effect_grant("bridge", &host()).unwrap();
    e.set_adapter_state("bridge", "removed").unwrap();
    assert!(e
        .install_governed_effect(&manifest("another", &g), &g, &host())
        .is_err());
    install(
        &e,
        "replay",
        &grant("explicit-new-execution", EffectStart::ReplayHistory),
    );
    assert_eq!(
        enqueue(&e, "replay").1.disposition,
        GovernedEffectDisposition::SupersededPublication
    );
    assert_ne!(intent(&enqueue(&e, "replay").1), intent(&current));
}
#[test]
fn supersession_blocks_existing_intent_and_owner_can_cancel_after_revoke() {
    let (mut e, _) = engine();
    let source = request(&mut e);
    publish(&e, source.clone(), "one");
    install(&e, "bridge", &grant("exec", EffectStart::ReplayHistory));
    let (d, r) = enqueue(&e, "bridge");
    let id = intent(&r);
    publish(&e, source, "two");
    assert!(e.begin_governed_effect(&id, &host()).is_err());
    assert!(e
        .enqueue_governed_effect("bridge", &d.event.id, &d.lease, &host())
        .is_err());
    e.revoke_governed_effect_grant("bridge", &host()).unwrap();
    e.cancel_governed_effect(&id, &host()).unwrap();
    e.cancel_governed_effect(&id, &host()).unwrap();
    assert!(e
        .cancel_governed_effect(&id, &HostContext::new("other", []))
        .is_err());
}
#[test]
fn installation_checkpoint_expiry_and_pause() {
    let (mut e, clock) = engine();
    let source = request(&mut e);
    publish(&e, source.clone(), "one");
    install(&e, "bridge", &grant("exec", EffectStart::AfterInstallation));
    assert!(e
        .poll_governance("bridge", "requests", &host())
        .unwrap()
        .is_none());
    publish(&e, source, "two");
    let (_, r) = enqueue(&e, "bridge");
    let id = intent(&r);
    e.set_adapter_state("bridge", "paused").unwrap();
    assert!(e.begin_governed_effect(&id, &host()).is_err());
    e.set_adapter_state("bridge", "running").unwrap();
    clock.set(5000);
    assert!(e.begin_governed_effect(&id, &host()).is_err());
    e.cancel_governed_effect(&id, &host()).unwrap();
}
#[cfg(feature = "recovery-testing")]
#[test]
fn observer_unwinds_rollback_enqueue_dispatch_and_reconcile() {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    let (mut e, _) = engine();
    let source = request(&mut e);
    publish(&e, source, "one");
    install(&e, "bridge", &grant("exec", EffectStart::ReplayHistory));
    let d = e
        .poll_governance("bridge", "requests", &host())
        .unwrap()
        .unwrap();
    assert!(catch_unwind(AssertUnwindSafe(|| e
        .enqueue_governed_effect_test_before_commit(
            "bridge",
            &d.event.id,
            &d.lease,
            &host(),
            || panic!("observer")
        )))
    .is_err());
    let r = e
        .enqueue_governed_effect("bridge", &d.event.id, &d.lease, &host())
        .unwrap();
    assert!(!r.duplicate);
    let id = intent(&r);
    assert!(catch_unwind(AssertUnwindSafe(|| e
        .begin_governed_effect_test_before_commit(&id, &host(), || panic!(
            "observer"
        ))))
    .is_err());
    assert_eq!(
        e.read_governed_effect(&id, &host()).unwrap().state,
        "pending"
    );
    let t = e.begin_governed_effect(&id, &host()).unwrap();
    assert!(catch_unwind(AssertUnwindSafe(|| e
        .reconcile_governed_effect_test_before_commit(
            &id,
            &t.attempt_id,
            ReconciledOutcome::Confirmed,
            &evidence(),
            &host(),
            || panic!("observer")
        )))
    .is_err());
    assert_eq!(
        e.read_governed_effect(&id, &host()).unwrap().state,
        "unknown"
    );
    e.reconcile_governed_effect(
        &id,
        &t.attempt_id,
        ReconciledOutcome::Confirmed,
        &evidence(),
        &host(),
    )
    .unwrap();
}
#[test]
fn policy_control_is_no_effect_and_preserves_current_publication() {
    let (mut e, _) = engine();
    let source = request(&mut e);
    publish(&e, source, "one");
    install(&e, "bridge", &grant("exec", EffectStart::ReplayHistory));
    let (_, r) = enqueue(&e, "bridge");
    let id = intent(&r);
    let mut next = policy();
    next.reference.revision = "2".into();
    let head = e.inspect_governance_head("requests", &host()).unwrap();
    decide(
        &e,
        GovernanceProposal {
            id: "policy-change".into(),
            view_id: "requests".into(),
            policy: head.policy,
            expected_head: head.decision_id,
            expires_at_ms: 9000,
            action: GovernanceAction::ReplacePolicy { policy: next },
        },
    );
    let (d, r) = enqueue(&e, "bridge");
    assert_eq!(r.disposition, GovernedEffectDisposition::PolicyChange);
    assert_eq!(
        e.enqueue_governed_effect("bridge", &d.event.id, &d.lease, &host())
            .unwrap()
            .disposition,
        r.disposition
    );
    e.begin_governed_effect(&id, &host()).unwrap();
}
#[test]
fn lease_rotation_drain_and_malformed_request_do_not_acknowledge() {
    let (mut e, clock) = engine();
    let source = request(&mut e);
    publish(&e, source, "one");
    install(&e, "bridge", &grant("exec", EffectStart::ReplayHistory));
    let old = e
        .poll_governance("bridge", "requests", &host())
        .unwrap()
        .unwrap();
    clock.set(111);
    assert_eq!(
        e.enqueue_governed_effect("bridge", &old.event.id, &old.lease, &host())
            .unwrap_err()
            .code,
        "E_LEASE"
    );
    let fresh = e
        .poll_governance("bridge", "requests", &host())
        .unwrap()
        .unwrap();
    assert_ne!(fresh.lease, old.lease);
    e.set_adapter_state("bridge", "draining").unwrap();
    let receipt = e
        .enqueue_governed_effect("bridge", &fresh.event.id, &fresh.lease, &host())
        .unwrap();
    e.begin_governed_effect(&intent(&receipt), &host()).unwrap();
    assert!(e
        .poll_governance("bridge", "requests", &host())
        .unwrap()
        .is_none());
    // A wrong descriptor is genuine governance input, but not an executable request.
    let (mut e, clock) = engine();
    let source = write(&mut e, "request", GraphData::default());
    publish(&e, source, "wrong-schema");
    install(&e, "bridge", &grant("exec", EffectStart::ReplayHistory));
    let d = e
        .poll_governance("bridge", "requests", &host())
        .unwrap()
        .unwrap();
    assert!(e
        .enqueue_governed_effect("bridge", &d.event.id, &d.lease, &host())
        .is_err());
    clock.set(111); // Existing delivery semantics withhold an unexpired lease.
    assert_eq!(
        e.poll_governance("bridge", "requests", &host())
            .unwrap()
            .unwrap()
            .event
            .id,
        d.event.id
    );
    assert!(e
        .install_governed_effect(
            &manifest("bridge", &grant("exec", EffectStart::ReplayHistory)),
            &grant("exec", EffectStart::ReplayHistory),
            &host()
        )
        .is_ok());
}
#[test]
fn grant_never_declassifies_and_operation_clock_is_sampled_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Clock(AtomicUsize);
    impl TrustedClock for Clock {
        fn unix_millis(&self) -> weave_engine::Result<i64> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(10)
        }
    }
    let clock = Arc::new(Clock(AtomicUsize::new(0)));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    e.install_governance_root(&policy()).unwrap();
    let source = request(&mut e);
    publish(&e, source, "one");
    let mut cross = grant("cross", EffectStart::ReplayHistory);
    cross.destination_principal = "stranger".into();
    assert!(e
        .install_governed_effect(&manifest("cross", &cross), &cross, &host())
        .is_err());
    clock.0.store(0, Ordering::Relaxed);
    install(&e, "bridge", &grant("exec", EffectStart::ReplayHistory));
    assert_eq!(clock.0.load(Ordering::Relaxed), 2); // install and lifecycle transition
    clock.0.store(0, Ordering::Relaxed);
    let d = e
        .poll_governance("bridge", "requests", &host())
        .unwrap()
        .unwrap();
    assert_eq!(clock.0.load(Ordering::Relaxed), 1);
    clock.0.store(0, Ordering::Relaxed);
    let r = e
        .enqueue_governed_effect("bridge", &d.event.id, &d.lease, &host())
        .unwrap();
    assert_eq!(clock.0.load(Ordering::Relaxed), 1);
    clock.0.store(0, Ordering::Relaxed);
    e.enqueue_governed_effect("bridge", &d.event.id, &d.lease, &host())
        .unwrap();
    assert_eq!(clock.0.load(Ordering::Relaxed), 1);
    clock.0.store(0, Ordering::Relaxed);
    let ticket = e.begin_governed_effect(&intent(&r), &host()).unwrap();
    assert_eq!(clock.0.load(Ordering::Relaxed), 1);
    clock.0.store(0, Ordering::Relaxed);
    e.reconcile_governed_effect(
        &ticket.intent_id,
        &ticket.attempt_id,
        ReconciledOutcome::Confirmed,
        &evidence(),
        &host(),
    )
    .unwrap();
    assert_eq!(clock.0.load(Ordering::Relaxed), 1);
}
#[test]
fn corrupt_oversized_response_fails_bounded_and_repair_recovers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("effects.db");
    let mut e = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    e.install_governance_root(&policy()).unwrap();
    let source = request(&mut e);
    publish(&e, source, "one");
    install(&e, "bridge", &grant("exec", EffectStart::ReplayHistory));
    let (_, receipt) = enqueue(&e, "bridge");
    let id = intent(&receipt);
    let ticket = e.begin_governed_effect(&id, &host()).unwrap();
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute(
        "UPDATE effect_intents SET response=?2 WHERE id=?1",
        rusqlite::params![id, "x".repeat(65537)],
    )
    .unwrap();
    assert_eq!(
        e.read_governed_effect(&id, &host()).unwrap_err().code,
        "E_BUDGET"
    );
    assert_eq!(
        e.reconcile_governed_effect(
            &id,
            &ticket.attempt_id,
            ReconciledOutcome::Confirmed,
            &evidence(),
            &host()
        )
        .unwrap_err()
        .code,
        "E_BUDGET"
    );
    db.execute("UPDATE effect_intents SET response=NULL WHERE id=?1", [&id])
        .unwrap();
    assert_eq!(
        e.read_governed_effect(&id, &host()).unwrap().state,
        "unknown"
    );
    e.reconcile_governed_effect(
        &id,
        &ticket.attempt_id,
        ReconciledOutcome::Confirmed,
        &evidence(),
        &host(),
    )
    .unwrap();
}
