use serde_json::json;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("alice", ["input".into(), "output".into()])
}
fn manifest() -> AdapterManifest {
    AdapterManifest {
        id: "test-adapter-v1".into(),
        version: "1".into(),
        artifact_digest: format!("sha256:{}", "a".repeat(64)),
        config_revision: "1".into(),
        principal: "alice".into(),
        subscriptions: vec![SubscriptionScope {
            graph_id: "input".into(),
            branch_id: "main".into(),
        }],
        output_graphs: vec!["output".into()],
        effect_destinations: vec!["mock://sink".into()],
        max_attempts: 3,
        lease_ms: 100,
        max_pending_events: 100,
        projection_replay: false,
    }
}
fn program(graph: &str, readers: serde_json::Value) -> Program {
    serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"data":{"nodes":[{"id":"n","entity_id":"entity","space_id":"s","readers":readers}]}}]})).unwrap()
}
fn empty() -> Program {
    Program {
        version: VERSION.into(),
        commands: vec![],
    }
}
fn setup(e: &mut Engine) {
    e.execute(&program("input", json!([])), &host()).unwrap();
    e.install_adapter(&manifest(), &host()).unwrap();
    e.set_adapter_state("test-adapter-v1", "running").unwrap();
}
#[test]
fn ordered_lease_receipt_and_output_are_atomic_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dispatch.db");
    let mut e = Engine::open(&path).unwrap();
    setup(&mut e);
    let delivery = e.poll_adapter("test-adapter-v1", 0).unwrap().unwrap();
    assert!(e.poll_adapter("test-adapter-v1", 50).unwrap().is_none());
    let output = program("output", json!(["alice"]));
    let receipt = e
        .complete_handler("test-adapter-v1", &delivery.id, &delivery.lease, &output)
        .unwrap();
    assert!(!receipt.duplicate);
    assert_eq!(e.event_count().unwrap(), 2);
    drop(e);
    let mut e = Engine::open(path).unwrap();
    assert!(
        e.complete_handler("test-adapter-v1", &delivery.id, &delivery.lease, &output)
            .unwrap()
            .duplicate
    );
    assert_eq!(e.event_count().unwrap(), 2);
    assert!(e.poll_adapter("test-adapter-v1", 200).unwrap().is_none());
    assert_eq!(
        e.complete_handler("test-adapter-v1", &delivery.id, &delivery.lease, &empty())
            .unwrap_err()
            .code,
        "E_RECEIPT_CONFLICT"
    );
}
#[test]
fn failed_handler_rolls_back_output_and_receipt_and_stale_lease_rejects() {
    let mut e = Engine::memory().unwrap();
    setup(&mut e);
    let d = e.poll_adapter("test-adapter-v1", 0).unwrap().unwrap();
    let renewed = e.poll_adapter("test-adapter-v1", 101).unwrap().unwrap();
    assert_ne!(d.lease, renewed.lease);
    assert_eq!(
        e.complete_handler("test-adapter-v1", &d.id, &d.lease, &empty())
            .unwrap_err()
            .code,
        "E_LEASE"
    );
    let mut p = program("output", json!(["alice"]));
    p.commands
        .extend(program("denied", json!(["alice"])).commands);
    assert_eq!(
        e.complete_handler("test-adapter-v1", &d.id, &renewed.lease, &p)
            .unwrap_err()
            .code,
        "E_FORBIDDEN"
    );
    assert!(e.head("output", "main").unwrap().is_none());
    assert_eq!(e.event_count().unwrap(), 1);
    assert!(
        !e.complete_handler("test-adapter-v1", &d.id, &renewed.lease, &empty())
            .unwrap()
            .duplicate
    );
}
#[test]
fn private_occurrences_and_branch_scope_never_enter_envelope() {
    let mut e = Engine::memory().unwrap();
    e.execute(&program("input", json!(["bob"])), &host())
        .unwrap();
    e.install_adapter(&manifest(), &host()).unwrap();
    e.set_adapter_state("test-adapter-v1", "running").unwrap();
    assert!(e.poll_adapter("test-adapter-v1", 0).unwrap().is_none());
    let mut p = program("input", json!([]));
    if let Command::Commit { branch_id, .. } = &mut p.commands[0] {
        *branch_id = "other".into()
    };
    e.execute(&p, &host()).unwrap();
    assert!(e.poll_adapter("test-adapter-v1", 100).unwrap().is_none());
}
#[test]
fn unknown_effect_never_automatically_retries_and_projection_replay_denies_effects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("effects.db");
    let mut e = Engine::open(&path).unwrap();
    setup(&mut e);
    let d = e.poll_adapter("test-adapter-v1", 0).unwrap().unwrap();
    let intent = e
        .request_effect(
            "test-adapter-v1",
            &d.id,
            &d.lease,
            "mock://sink",
            "key",
            json!({"value":1}),
        )
        .unwrap();
    assert_eq!(intent.state, "pending");
    let attempted = e.begin_effect_dispatch(&intent.id).unwrap();
    assert_eq!(attempted.state, "unknown");
    drop(e);
    let e = Engine::open(path).unwrap();
    assert_eq!(
        e.begin_effect_dispatch(&intent.id).unwrap_err().code,
        "E_EFFECT_UNKNOWN"
    );
    e.reconcile_effect(&intent.id, "confirmed", json!({"receipt":"remote-1"}))
        .unwrap();
    assert_eq!(
        e.effect_intent(&intent.id).unwrap().unwrap().state,
        "confirmed"
    );
    let mut replay = manifest();
    replay.id = "replay".into();
    replay.projection_replay = true;
    e.install_adapter(&replay, &host()).unwrap();
    e.set_adapter_state("replay", "running").unwrap();
    let mut e = e;
    let d = e.poll_adapter("replay", 1000).unwrap().unwrap();
    assert_eq!(
        e.request_effect("replay", &d.id, &d.lease, "mock://sink", "key", json!({}))
            .unwrap_err()
            .code,
        "E_EFFECT_AUTHORITY"
    );
}
#[test]
fn adapter_cannot_declassify_output_or_rebind_manifest() {
    let mut e = Engine::memory().unwrap();
    setup(&mut e);
    let d = e.poll_adapter("test-adapter-v1", 0).unwrap().unwrap();
    assert_eq!(
        e.complete_handler(
            "test-adapter-v1",
            &d.id,
            &d.lease,
            &program("output", json!([]))
        )
        .unwrap_err()
        .code,
        "E_EGRESS"
    );
    let mut changed = manifest();
    changed.config_revision = "2".into();
    assert_eq!(
        e.install_adapter(&changed, &host()).unwrap_err().code,
        "E_ADAPTER_VERSION"
    );
}

#[test]
fn bounded_retries_dead_letter_and_drain_are_explicit() {
    let mut e = Engine::memory().unwrap();
    setup(&mut e);
    let mut now = 0;
    for _ in 0..3 {
        let d = e.poll_adapter("test-adapter-v1", now).unwrap().unwrap();
        e.fail_handler("test-adapter-v1", &d.id, &d.lease, now)
            .unwrap();
        assert!(e
            .poll_adapter("test-adapter-v1", now + 1)
            .unwrap()
            .is_none());
        now += 1_000_000;
    }
    assert!(e.poll_adapter("test-adapter-v1", now).unwrap().is_none());
    assert_eq!(
        e.set_adapter_state("test-adapter-v1", "removed")
            .unwrap_err()
            .code,
        "E_LIFECYCLE"
    );
    e.replay_handler_dead_letter("test-adapter-v1").unwrap();
    let d = e.poll_adapter("test-adapter-v1", now).unwrap().unwrap();
    e.set_adapter_state("test-adapter-v1", "draining").unwrap();
    e.complete_handler("test-adapter-v1", &d.id, &d.lease, &empty())
        .unwrap();
    assert!(e
        .poll_adapter("test-adapter-v1", now + 1000)
        .unwrap()
        .is_none());
    e.set_adapter_state("test-adapter-v1", "removed").unwrap();
    assert_eq!(
        e.set_adapter_state("test-adapter-v1", "running")
            .unwrap_err()
            .code,
        "E_LIFECYCLE"
    );
}
