//! Independent operation-boundary regressions; no caller-selected authority timestamp.
use serde_json::json;
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("alice", ["input".into(), "output".into()])
}
fn program(graph: &str, private: bool) -> Program {
    serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"data":{"nodes":[{"id":"n","entity_id":"entity","space_id":"s","readers":if private {vec!["alice"]} else {vec![]}}]}}]})).unwrap()
}
fn identity_setup() -> (Engine, Arc<ManualClock>, IdentityCandidate) {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    engine.execute(&program("input", false), &host()).unwrap();
    let policy = IdentityPolicy {
        reference: IdentityPolicyRef {
            id: "policy".into(),
            revision: "1".into(),
        },
        proposers: vec!["alice".into()],
        approvers: vec!["alice".into()],
        readers: vec![],
        allowed_spaces: vec!["s".into()],
        max_members: 2,
    };
    engine.install_identity_policy(&policy).unwrap();
    let candidate = IdentityCandidate {
        id: "candidate".into(),
        mapping_id: "mapping".into(),
        policy: policy.reference,
        groups: vec![vec![NodeRef {
            graph_id: "input".into(),
            revision: engine.head("input", "main").unwrap().unwrap(),
            node_id: "n".into(),
        }]],
        evidence: vec![],
        valid_time: Interval {
            start: 0,
            end: Some(100),
        },
        context: None,
    };
    (engine, clock, candidate)
}
#[test]
fn candidate_submission_samples_once_and_failed_clock_cannot_store_a_proposal() {
    let (mut engine, clock, mut candidate) = identity_setup();
    let samples = clock.samples();
    clock.set(20);
    assert!(engine
        .submit_identity_candidate(&candidate, &host())
        .unwrap());
    assert_eq!(
        clock.samples(),
        samples + 1,
        "native candidate admission must have an operation boundary"
    );
    candidate.id = "after-clock-failure".into();
    clock.set(-1);
    let error = engine
        .submit_identity_candidate(&candidate, &host())
        .unwrap_err();
    assert_eq!(error.code, "E_CLOCK_UNAVAILABLE");
    clock.set(30);
    assert!(
        engine
            .submit_identity_candidate(&candidate, &host())
            .unwrap(),
        "failed clock must not have persisted this ID"
    );
}
#[cfg(feature = "recovery-testing")]
#[test]
fn observer_unwind_rolls_back_transaction_and_next_operation_gets_new_scope() {
    let clock = Arc::new(ManualClock::new(0));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    engine.execute(&program("input", false), &host()).unwrap();
    engine
        .install_adapter(
            &AdapterManifest {
                id: "adapter".into(),
                version: "1".into(),
                artifact_digest: format!("sha256:{}", "a".repeat(64)),
                config_revision: "1".into(),
                principal: "alice".into(),
                subscriptions: vec![SubscriptionScope {
                    graph_id: "input".into(),
                    branch_id: "main".into(),
                }],
                output_graphs: vec!["output".into()],
                effect_destinations: vec![],
                max_attempts: 3,
                lease_ms: 100,
                max_pending_events: 10,
                projection_replay: false,
            },
            &host(),
        )
        .unwrap();
    engine.set_adapter_state("adapter", "running").unwrap();
    let delivery = engine.poll_adapter("adapter").unwrap().unwrap();
    let before = engine.event_count().unwrap();
    clock.set(10);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = engine.complete_handler_test_before_commit(
            "adapter",
            &delivery.id,
            &delivery.lease,
            &program("output", true),
            || panic!("observer unwind"),
        );
    }));
    assert!(panic.is_err());
    assert!(
        engine.head("output", "main").unwrap().is_none(),
        "unwinding observer must not leave readable uncommitted output"
    );
    assert_eq!(engine.event_count().unwrap(), before);
    clock.set(20);
    let samples = clock.samples();
    engine
        .complete_handler(
            "adapter",
            &delivery.id,
            &delivery.lease,
            &program("output", true),
        )
        .unwrap();
    assert_eq!(clock.samples(), samples + 1);
    let revision = engine.head("output", "main").unwrap().unwrap();
    assert_eq!(engine.recorded_at(&revision).unwrap(), 20);
}

#[test]
fn failed_clock_is_checked_before_first_identity_candidate_write() {
    let (mut engine, clock, candidate) = identity_setup();
    clock.set(-1);
    let result = engine.submit_identity_candidate(&candidate, &host());
    assert_eq!(result.unwrap_err().code, "E_CLOCK_UNAVAILABLE");
    clock.set(20);
    assert!(engine
        .submit_identity_candidate(&candidate, &host())
        .unwrap());
}

#[test]
fn effect_fence_checks_clock_before_transition_and_reuses_one_sample() {
    let clock = Arc::new(ManualClock::new(0));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    engine.execute(&program("input", false), &host()).unwrap();
    engine
        .install_adapter(
            &AdapterManifest {
                id: "effects".into(),
                version: "1".into(),
                artifact_digest: format!("sha256:{}", "b".repeat(64)),
                config_revision: "1".into(),
                principal: "alice".into(),
                subscriptions: vec![SubscriptionScope {
                    graph_id: "input".into(),
                    branch_id: "main".into(),
                }],
                output_graphs: vec![],
                effect_destinations: vec!["mock://sink".into()],
                max_attempts: 3,
                lease_ms: 100,
                max_pending_events: 10,
                projection_replay: false,
            },
            &host(),
        )
        .unwrap();
    engine.set_adapter_state("effects", "running").unwrap();
    let delivery = engine.poll_adapter("effects").unwrap().unwrap();
    clock.set(10);
    let intent = engine
        .request_effect(
            "effects",
            &delivery.id,
            &delivery.lease,
            "mock://sink",
            "key",
            json!({"message":"fixture"}),
        )
        .unwrap();
    clock.set(-1);
    assert_eq!(
        engine.begin_effect_dispatch(&intent.id).unwrap_err().code,
        "E_CLOCK_UNAVAILABLE"
    );
    assert_eq!(
        engine.effect_intent(&intent.id).unwrap().unwrap().state,
        "pending"
    );
    clock.set(20);
    let before = clock.samples();
    assert_eq!(
        engine.begin_effect_dispatch(&intent.id).unwrap().state,
        "unknown"
    );
    assert_eq!(clock.samples(), before + 1);
    clock.set(30);
    assert_eq!(
        engine.begin_effect_dispatch(&intent.id).unwrap_err().code,
        "E_EFFECT_UNKNOWN"
    );
    assert_eq!(
        engine.effect_intent(&intent.id).unwrap().unwrap().state,
        "unknown"
    );
}

#[test]
fn contended_writer_does_not_sample_clock_and_retry_captures_fresh_time() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("contended.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    engine.execute(&program("input", false), &host()).unwrap();
    let other = rusqlite::Connection::open(&path).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let samples = clock.samples();
    clock.set(20);
    assert!(engine.execute(&program("output", false), &host()).is_err());
    assert_eq!(
        clock.samples(),
        samples,
        "reserve the writer before sampling authority time"
    );
    assert!(engine.head("output", "main").unwrap().is_none());
    other.execute_batch("ROLLBACK").unwrap();
    clock.set(30);
    engine.execute(&program("output", false), &host()).unwrap();
    assert_eq!(clock.samples(), samples + 1);
    let revision = engine.head("output", "main").unwrap().unwrap();
    assert_eq!(engine.recorded_at(&revision).unwrap(), 30);
}
