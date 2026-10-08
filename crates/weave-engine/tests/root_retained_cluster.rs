//! Independent retained-completion boundary checks, using public APIs only.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{Command, CommandResult, Coverage, GraphRef, Program, VERSION};
use weave_engine::*;

fn host() -> HostContext {
    HostContext::new(
        "alice",
        ["Evidence", "Warnings", "Clusters"].map(String::from),
    )
}
fn commit(engine: &mut Engine, graph: &str, data: Value) -> GraphRef {
    let expected = engine.head(graph, "main").unwrap();
    let program: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{
        "op":"commit","graph_id":graph,"expected_head":expected,"data":data
    }]}))
    .unwrap();
    engine.execute(&program, &host()).unwrap();
    GraphRef {
        graph_id: graph.into(),
        revision: engine.head(graph, "main").unwrap().unwrap(),
    }
}
fn fixture() -> (Engine, DispatchEnvelope, Program) {
    let mut engine = Engine::memory_with_clock(Arc::new(ManualClock::new(100))).unwrap();
    let proof = commit(
        &mut engine,
        "Evidence",
        json!({
            "nodes":[{"id":"p0","entity_id":"p0","space_id":"s"},{"id":"p1","entity_id":"p1","space_id":"s"}],
            "edges":[{"id":"p","predicate":"observed","from":"p0","to":"p1","valid_time":{"start":0,"end":10},"readers":["alice"]}]
        }),
    );
    let input = commit(
        &mut engine,
        "Warnings",
        json!({
            "nodes":[{"id":"n0","entity_id":"n0","space_id":"s"},{"id":"n1","entity_id":"n1","space_id":"s"}],
            "edges":[{"id":"w","predicate":"warning","from":"n0","to":"n1","valid_time":{"start":0,"end":10},
                "derived_from":[{"graph_id":proof.graph_id,"revision":proof.revision,"assertion_id":"p"}],
                "derivations":[{"operator":"independent-observation","premises":[{"graph_id":proof.graph_id,"revision":proof.revision,"assertion_id":"p"}],"input_snapshots":[]}]}]
        }),
    );
    engine
        .install_adapter(
            &AdapterManifest {
                id: "root-cluster".into(),
                version: "1".into(),
                artifact_digest: format!("sha256:{}", "a".repeat(64)),
                config_revision: "1".into(),
                principal: "alice".into(),
                subscriptions: vec![SubscriptionScope {
                    graph_id: "Warnings".into(),
                    branch_id: "main".into(),
                }],
                output_graphs: vec!["Clusters".into()],
                effect_destinations: vec![],
                max_attempts: 3,
                lease_ms: 1000,
                max_pending_events: 10,
                projection_replay: true,
            },
            &host(),
        )
        .unwrap();
    engine.set_adapter_state("root-cluster", "running").unwrap();
    let event = engine
        .poll_adapter_for("root-cluster", &host())
        .unwrap()
        .unwrap();
    let recipe: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{
        "op":"bind","name":"ArbitraryResultName","value":{"kind":"cluster","selection":{
            "source":input,"context":{"kind":"default"},"valid_at":7,"predicate":"warning","levels":1
        }}
    }]})).unwrap();
    (engine, event, recipe)
}
fn capture(
    engine: &mut Engine,
    event: &DispatchEnvelope,
    recipe: &Program,
) -> RetainedClusterCompletion {
    engine
        .capture_retained_cluster_for(
            "root-cluster",
            &event.id,
            &event.lease,
            recipe,
            "main",
            &host(),
        )
        .unwrap()
}
fn complete(
    engine: &mut Engine,
    event: &DispatchEnvelope,
    record: &RetainedClusterCompletion,
) -> Result<HandlerReceipt> {
    engine.complete_retained_cluster_for("root-cluster", &event.id, &event.lease, record, &host())
}

#[test]
fn exact_retained_binding_is_checked_before_fresh_and_historical_completion() {
    for completed in [false, true] {
        let (mut engine, event, recipe) = fixture();
        let record = capture(&mut engine, &event, &recipe);
        if completed {
            assert!(!complete(&mut engine, &event, &record).unwrap().duplicate);
        }
        let before = engine.events().unwrap().len();
        let head = engine.head("Clusters", "main").unwrap();
        let mut changed = record.clone();
        let Command::Commit { data, .. } = &mut changed.completion.commands[0] else {
            panic!()
        };
        data.nodes[0]
            .properties
            .insert("exact".into(), json!(9007199254740993i64));
        assert!(complete(&mut engine, &event, &changed).is_err());
        let mut changed = record.clone();
        changed.runtime_source = "urn:weave:replica:another-store".into();
        assert!(complete(&mut engine, &event, &changed).is_err());
        let mut changed = record.clone();
        changed.adapter_manifest_digest = format!("sha256:{}", "0".repeat(64));
        assert!(complete(&mut engine, &event, &changed).is_err());
        let mut changed = record.clone();
        changed.closure.clear();
        assert!(complete(&mut engine, &event, &changed).is_err());
        assert_eq!(engine.events().unwrap().len(), before);
        assert_eq!(engine.head("Clusters", "main").unwrap(), head);
        assert_eq!(
            complete(&mut engine, &event, &record).unwrap().duplicate,
            completed
        );
    }
}

#[test]
fn duplicate_receipt_still_requires_current_host_scope_and_original_journal() {
    let (mut engine, event, recipe) = fixture();
    let record = capture(&mut engine, &event, &recipe);
    let first = complete(&mut engine, &event, &record).unwrap();
    let before = engine.events().unwrap().len();
    let narrowed = HostContext::new("alice", []);
    assert_eq!(
        engine
            .complete_retained_cluster_for(
                "root-cluster",
                &event.id,
                &event.lease,
                &record,
                &narrowed
            )
            .unwrap_err()
            .code,
        "E_HOST_AUTH"
    );
    let foreign = HostContext::new("bob", ["Clusters".into()]);
    assert_eq!(
        engine
            .complete_retained_cluster_for(
                "root-cluster",
                &event.id,
                &event.lease,
                &record,
                &foreign
            )
            .unwrap_err()
            .code,
        "E_HOST_AUTH"
    );
    assert!(matches!(
        engine
            .inspect_handler_slot_for("root-cluster", &event.id, &host())
            .unwrap(),
        HandlerSlotState::Completed { .. }
    ));
    assert_eq!(
        engine
            .capture_retained_cluster_for(
                "root-cluster",
                &event.id,
                &event.lease,
                &recipe,
                "main",
                &host()
            )
            .unwrap_err()
            .code,
        "E_HOST_JOURNAL_MISSING"
    );
    let replay = complete(&mut engine, &event, &record).unwrap();
    assert!(replay.duplicate);
    assert_eq!(
        serde_json::to_value(replay.results).unwrap(),
        serde_json::to_value(first.results).unwrap()
    );
    assert_eq!(engine.events().unwrap().len(), before);
}

#[test]
fn stale_cas_does_not_rebase_or_acknowledge_retained_work() {
    let (mut engine, event, recipe) = fixture();
    let record = capture(&mut engine, &event, &recipe);
    let bytes = serde_json::to_vec(&record).unwrap();
    let competing = commit(&mut engine, "Clusters", json!({"nodes":[],"edges":[]}));
    let before = engine.events().unwrap().len();
    assert_eq!(
        complete(&mut engine, &event, &record).unwrap_err().code,
        "E_CONFLICT"
    );
    assert_eq!(
        engine.head("Clusters", "main").unwrap().unwrap(),
        competing.revision
    );
    assert_eq!(
        engine
            .inspect_handler_slot_for("root-cluster", &event.id, &host())
            .unwrap(),
        HandlerSlotState::Pending
    );
    assert_eq!(serde_json::to_vec(&record).unwrap(), bytes);
    assert_eq!(engine.events().unwrap().len(), before);
}

#[test]
fn independent_runtime_identity_rejects_cross_store_completion() {
    let (mut first, event, recipe) = fixture();
    let record = capture(&mut first, &event, &recipe);
    let (mut second, other_event, _) = fixture();
    assert_ne!(
        first.runtime_source_identity().unwrap(),
        second.runtime_source_identity().unwrap()
    );
    let before = second.events().unwrap().len();
    assert!(complete(&mut second, &other_event, &record).is_err());
    assert!(second.head("Clusters", "main").unwrap().is_none());
    assert_eq!(second.events().unwrap().len(), before);
    assert_eq!(
        second
            .inspect_handler_slot_for("root-cluster", &other_event.id, &host())
            .unwrap(),
        HandlerSlotState::Pending
    );
}

#[test]
fn scoped_navigation_does_not_admit_a_partly_visible_whole_input() {
    let (mut engine, _, _) = fixture();
    let hidden = commit(
        &mut engine,
        "Warnings",
        json!({
            "nodes":[{"id":"visible","entity_id":"v","space_id":"s"},
                     {"id":"secret","entity_id":"secret","space_id":"s","readers":["bob"]}],
            "edges":[]
        }),
    );
    // A fresh adapter isolates this occurrence from the first subscribed event.
    let mut manifest = AdapterManifest {
        id: "hidden-cluster".into(),
        version: "1".into(),
        artifact_digest: format!("sha256:{}", "b".repeat(64)),
        config_revision: "1".into(),
        principal: "alice".into(),
        subscriptions: vec![SubscriptionScope {
            graph_id: "Warnings".into(),
            branch_id: "hidden".into(),
        }],
        output_graphs: vec!["Clusters".into()],
        effect_destinations: vec![],
        max_attempts: 3,
        lease_ms: 1000,
        max_pending_events: 10,
        projection_replay: true,
    };
    // Use another graph's only event to avoid granting private input after filtering.
    manifest.subscriptions[0].graph_id = "PrivateInput".into();
    manifest.subscriptions[0].branch_id = "main".into();
    let h = HostContext::new("alice", ["PrivateInput".into(), "Clusters".into()]);
    let data=serde_json::from_value(json!({"nodes":[{"id":"visible","entity_id":"v","space_id":"s"},{"id":"secret","entity_id":"secret","space_id":"s","readers":["bob"]}]})).unwrap();
    let p = Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands: vec![Command::Commit {
            graph_id: "PrivateInput".into(),
            branch_id: "main".into(),
            expected_head: None,
            data,
        }],
    };
    engine.execute(&p, &h).unwrap();
    engine.install_adapter(&manifest, &h).unwrap();
    engine.set_adapter_state(&manifest.id, "running").unwrap();
    // Dispatch itself rejects whole-input consumption rather than advertising a secret count.
    assert!(engine.poll_adapter_for(&manifest.id, &h).unwrap().is_none());
    let recipe:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"evaluate","value":{"kind":"cluster","selection":{"source":hidden,"context":{"kind":"default"},"valid_at":7,"predicate":"warning","levels":2}}}]})).unwrap();
    let navigation = engine.execute(&recipe, &host()).unwrap();
    let CommandResult::Queried { result } = &navigation[0] else {
        panic!()
    };
    assert_eq!(result.coverage, Coverage::Partial);
    assert!(!result.graph.nodes.iter().any(|n| n.id == "secret"));
}

#[test]
fn cleared_output_readers_and_envelope_keep_original_cluster_input_gates() {
    let (mut engine, event, recipe) = fixture();
    let record = capture(&mut engine, &event, &recipe);
    let mut data = record.result.graph;
    data.influence = None;
    for n in &mut data.nodes {
        n.readers.clear();
    }
    for e in &mut data.edges {
        e.readers.clear();
    }
    let saved = commit(&mut engine, "Clusters", serde_json::to_value(data).unwrap());
    let query:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"query","query":{"graph_id":"Clusters","revision":saved.revision}}]})).unwrap();
    let bob = HostContext::new("bob", []);
    let result = engine.execute(&query, &bob).unwrap();
    let CommandResult::Queried { result } = &result[0] else {
        panic!()
    };
    assert!(result.graph.nodes.is_empty());
    assert!(result.graph.edges.is_empty());
}
