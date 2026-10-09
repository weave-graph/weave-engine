//! Explicit whole-owner rebuild and hidden compaction noninterference oracles.
use serde_json::json;
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;
fn owner() -> HostContext {
    HostContext::new("alice", ["source".into()])
}
fn write(engine: &mut Engine, graph: &str, value: i64, host: &HostContext, readers: Vec<String>) {
    let expected = engine.head(graph, "main").unwrap();
    engine.execute(&serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":expected,"data":{"nodes":[{"id":"n","entity_id":"e","space_id":"s","readers":readers,"properties":{"value":value}}]}}]})).unwrap(),host).unwrap();
}
fn enable(engine: &mut Engine, id: &str) {
    engine
        .register_view(
            &ViewDefinition {
                id: id.into(),
                clock: ViewClock::Fixed,
                expression: GraphExpression::Query {
                    query: serde_json::from_value(json!({"graph_id":"source"})).unwrap(),
                },
            },
            None,
            &owner(),
        )
        .unwrap();
    engine.enroll_incremental_view(id, &owner()).unwrap();
    engine.enable_view_schedule(id, &owner()).unwrap();
    engine.drain_view_work(&owner()).unwrap().unwrap();
}
fn scan() -> ViewScanBudget {
    ViewScanBudget {
        max_events: 100,
        max_views: 100,
    }
}
fn projection() -> AdapterManifest {
    AdapterManifest {
        id: "projector".into(),
        version: "1".into(),
        artifact_digest: format!("sha256:{}", "b".repeat(64)),
        config_revision: "1".into(),
        principal: "alice".into(),
        subscriptions: vec![SubscriptionScope {
            graph_id: "source".into(),
            branch_id: "main".into(),
        }],
        output_graphs: vec![],
        effect_destinations: vec![],
        max_attempts: 3,
        lease_ms: 100,
        max_pending_events: 100,
        projection_replay: true,
    }
}
#[test]
fn whole_owner_rebuild_uses_current_full_inputs_and_hidden_gc_keeps_the_epoch() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    write(&mut engine, "source", 1, &owner(), vec![]);
    let secret = HostContext::new("secret", ["garbage".into()]);
    write(&mut engine, "garbage", 99, &secret, vec!["secret".into()]);
    enable(&mut engine, "left");
    enable(&mut engine, "right");
    clock.set(20);
    write(&mut engine, "source", 2, &owner(), vec![]);
    clock.set(30);
    let policy = RetentionPolicy {
        history_before_ms: 25,
        replay_through_sequence: 3,
    };
    let plan = engine.plan_retention(&policy).unwrap();
    engine.compact_retention(&plan).unwrap();
    assert_eq!(
        engine.scan_view_work(&owner(), scan()).unwrap_err().code,
        "E_CHECKPOINT_EXPIRED"
    );
    assert_eq!(
        engine.drain_view_work(&owner()).unwrap_err().code,
        "E_CHECKPOINT_EXPIRED"
    );
    assert_eq!(
        engine
            .enable_view_schedule("left", &owner())
            .unwrap_err()
            .code,
        "E_CHECKPOINT_EXPIRED"
    );
    let stranger = HostContext::new("stranger", Vec::<String>::new());
    assert!(engine
        .rebase_view_schedule_for(&stranger)
        .unwrap()
        .is_empty());
    assert_eq!(
        engine.scan_view_work(&owner(), scan()).unwrap_err().code,
        "E_CHECKPOINT_EXPIRED"
    );
    let rebuilt = engine.rebase_view_schedule_for(&owner()).unwrap();
    assert_eq!(rebuilt.len(), 2);
    for view in rebuilt {
        assert!(view.current);
        assert!(
            view.work.fallback_runs > 0,
            "a full oracle checks each cold rebuild"
        );
        assert_eq!(
            engine
                .read_view(&view.view_id, None, ViewFreshness::RequireCurrent, &owner())
                .unwrap()
                .result
                .graph
                .nodes[0]
                .properties["value"],
            json!(2)
        );
    }
    engine.install_adapter(&projection(), &owner()).unwrap();
    engine.set_adapter_state("projector", "running").unwrap();
    let inputs = engine
        .projection_rebase_inputs_for("projector", &owner())
        .unwrap();
    let epoch = inputs.epoch.clone();
    engine
        .rebase_projection_for(
            &ProjectionRebaseRequest {
                inputs,
                state_revision: "rebuilt".into(),
                state: json!({}),
            },
            &owner(),
        )
        .unwrap();
    let garbage = engine.head("garbage", "main").unwrap().unwrap();
    engine
        .release_branch_for("garbage", "main", &garbage, &secret)
        .unwrap();
    let plan = engine.plan_retention(&policy).unwrap();
    assert_eq!(plan.collect.len(), 1);
    engine.compact_retention(&plan).unwrap();
    assert_eq!(
        engine
            .projection_rebase_inputs_for("projector", &owner())
            .unwrap()
            .epoch,
        epoch,
        "private garbage collection does not change observer epoch"
    );
    assert!(engine
        .poll_adapter_for("projector", &owner())
        .unwrap()
        .is_none());
    assert_eq!(
        engine.scan_view_work(&owner(), scan()).unwrap(),
        ViewScanProgress::default()
    );
    clock.set(40);
    write(&mut engine, "source", 3, &owner(), vec![]);
    engine.scan_view_work(&owner(), scan()).unwrap();
    assert_eq!(
        engine.drain_view_work(&owner()).unwrap().unwrap().view_id,
        "left"
    );
    engine.drain_view_work(&owner()).unwrap().unwrap();
    assert!(
        engine
            .read_view("right", None, ViewFreshness::RequireCurrent, &owner())
            .unwrap()
            .current
    );
}
#[cfg(feature = "recovery-testing")]
#[test]
fn interrupted_owned_view_rebuild_never_acknowledges_the_lost_window() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("views.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    write(&mut engine, "source", 1, &owner(), vec![]);
    enable(&mut engine, "view");
    clock.set(20);
    write(&mut engine, "source", 2, &owner(), vec![]);
    clock.set(30);
    let policy = RetentionPolicy {
        history_before_ms: 25,
        replay_through_sequence: 2,
    };
    let plan = engine.plan_retention(&policy).unwrap();
    engine.compact_retention(&plan).unwrap();
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine.rebase_view_schedule_test_before_commit(&owner(), || panic!("before commit"))
    }));
    assert!(stopped.is_err());
    drop(engine);
    let mut engine = Engine::open_with_clock(&path, clock).unwrap();
    assert_eq!(
        engine.scan_view_work(&owner(), scan()).unwrap_err().code,
        "E_CHECKPOINT_EXPIRED"
    );
    let old = engine
        .read_view("view", None, ViewFreshness::AllowStale, &owner())
        .unwrap();
    assert_eq!(old.result.graph.nodes[0].properties["value"], json!(1));
    assert_eq!(engine.rebase_view_schedule_for(&owner()).unwrap().len(), 1);
    assert_eq!(
        engine
            .read_view("view", None, ViewFreshness::RequireCurrent, &owner())
            .unwrap()
            .result
            .graph
            .nodes[0]
            .properties["value"],
        json!(2)
    );
}
