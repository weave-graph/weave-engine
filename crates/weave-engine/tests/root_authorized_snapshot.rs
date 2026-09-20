//! Independent whole-snapshot authorization regression. No runtime internals.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;

fn host() -> HostContext {
    HostContext::new("alice", ["Proofs", "Secret", "Saved"].map(String::from))
}
fn commit(e: &mut Engine, id: &str, data: GraphData) -> GraphRef {
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: id.into(),
                branch_id: "main".into(),
                expected_head: None,
                data,
            }],
        },
        &host(),
    )
    .unwrap();
    GraphRef {
        graph_id: id.into(),
        revision: e.head(id, "main").unwrap().unwrap(),
    }
}
fn source(readers: Value) -> GraphData {
    serde_json::from_value(json!({
        "nodes":[{"id":"n","entity_id":"n","space_id":"s","readers":readers}],
        "edges":[
            {"id":"a","predicate":"p","from":"n","to":"n","valid_time":{"start":0},"readers":readers},
            {"id":"z","predicate":"p","from":"n","to":"n","valid_time":{"start":0},"readers":readers}
        ]
    })).unwrap()
}
fn assertion(r: &GraphRef, id: &str) -> Value {
    json!({"graph_id":r.graph_id,"revision":r.revision,"assertion_id":id})
}
fn group(premises: Vec<Value>, node_premises: Vec<Value>, snapshots: Vec<GraphRef>) -> Value {
    json!({"operator":"independent-proof","premises":premises,
        "node_premises":node_premises,"input_snapshots":snapshots,
        "parameters":{"exact":"preserve this explanation"}})
}
fn saved(explicit: bool, flat: Vec<Value>, groups: Vec<Value>) -> GraphData {
    let mut data = json!({"nodes":[{"id":"n","entity_id":"result","space_id":"s"}]});
    if explicit {
        data["profile"] = json!("explicit");
        data["structural_edges"] =
            json!([{"id":"relation","predicate":"conclusion","from":"n","to":"n"}]);
        data["assertions"] = json!([{"id":"derived","edge_id":"relation","source":"fixture",
            "valid_time":{"start":0},"derived_from":flat,"derivations":groups}]);
    } else {
        data["edges"] = json!([{"id":"derived","predicate":"conclusion","from":"n","to":"n",
            "valid_time":{"start":0},"derived_from":flat,"derivations":groups}]);
    }
    serde_json::from_value(data).unwrap()
}
fn query(e: &Engine, r: &GraphRef) -> QueryResult {
    let q: QueryPlan =
        serde_json::from_value(json!({"graph_id":r.graph_id,"revision":r.revision})).unwrap();
    e.query(&q, &host()).unwrap()
}
fn delivery(e: &mut Engine) -> Option<DispatchEnvelope> {
    e.install_adapter(
        &AdapterManifest {
            id: "snapshot-reader".into(),
            version: "1".into(),
            artifact_digest: format!("sha256:{}", "a".repeat(64)),
            config_revision: "1".into(),
            principal: "alice".into(),
            subscriptions: vec![SubscriptionScope {
                graph_id: "Saved".into(),
                branch_id: "main".into(),
            }],
            output_graphs: vec![],
            effect_destinations: vec![],
            max_attempts: 3,
            lease_ms: 100,
            max_pending_events: 100,
            projection_replay: true,
        },
        &host(),
    )
    .unwrap();
    e.set_adapter_state("snapshot-reader", "running").unwrap();
    e.poll_adapter("snapshot-reader").unwrap()
}
fn engine() -> Engine {
    Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap()
}

#[test]
fn harmless_flat_index_order_and_duplicates_preserve_exact_transport_bytes() {
    for explicit in [false, true] {
        let mut e = engine();
        let proof = commit(&mut e, "Proofs", source(json!([])));
        let a = assertion(&proof, "a");
        let z = assertion(&proof, "z");
        // Group traversal is z,a while the stored compatibility index is a,z,a.
        let data = saved(
            explicit,
            vec![a.clone(), z.clone(), a.clone()],
            vec![
                group(vec![z], vec![], vec![proof.clone()]),
                group(vec![a], vec![], vec![proof]),
            ],
        );
        let original = serde_json::to_vec(&data).unwrap();
        let root = commit(&mut e, "Saved", data);
        let result = query(&e, &root);
        assert_eq!(result.coverage, Coverage::Complete);
        assert_eq!(result.graph.edges.len(), 1);
        let capsule = e.export_capsule(&root, &host()).unwrap();
        let record = capsule
            .revisions
            .iter()
            .find(|r| r.graph_id == "Saved")
            .unwrap();
        assert_eq!(serde_json::to_vec(&record.data).unwrap(), original);
        assert_eq!(record.revision, root.revision);
        assert_eq!(delivery(&mut e).unwrap().graph, root);
        let mut dest = engine();
        dest.receive_capsule(&capsule, &host()).unwrap();
        let again = dest.export_capsule(&root, &host()).unwrap();
        assert_eq!(
            again
                .revisions
                .iter()
                .find(|r| r.graph_id == "Saved")
                .unwrap(),
            record
        );
    }
}

#[test]
fn a_withheld_alternative_is_not_whole_snapshot_authorization() {
    for explicit in [false, true] {
        for node_only in [false, true] {
            let mut e = engine();
            let proof = commit(&mut e, "Proofs", source(json!([])));
            let hidden = commit(&mut e, "Secret", source(json!(["bob"])));
            let public = assertion(&proof, "a");
            let secret = assertion(&hidden, "z");
            let (hidden_assertions, hidden_nodes) = if node_only {
                (
                    vec![],
                    vec![
                        json!({"graph_id":hidden.graph_id,"revision":hidden.revision,"node_id":"n"}),
                    ],
                )
            } else {
                (vec![secret.clone()], vec![])
            };
            let mut flat = vec![public.clone()];
            if !node_only {
                flat.push(secret);
            }
            let data = saved(
                explicit,
                flat,
                vec![
                    group(vec![public], vec![], vec![proof]),
                    group(hidden_assertions, hidden_nodes, vec![hidden]),
                ],
            );
            let root = commit(&mut e, "Saved", data);
            let result = query(&e, &root);
            assert_eq!(result.coverage, Coverage::Partial);
            assert_eq!(
                result.graph.edges.len(),
                1,
                "public alternative remains a query result"
            );
            assert_eq!(result.graph.edges[0].derivations.len(), 1);
            assert_eq!(
                e.export_capsule(&root, &host()).unwrap_err().code,
                "E_UNAVAILABLE"
            );
            assert!(
                delivery(&mut e).is_none(),
                "whole occurrence may not disclose the hidden explanation"
            );
        }
    }
}

#[test]
fn a_withheld_joint_premise_blocks_export_and_delivery() {
    for explicit in [false, true] {
        let mut e = engine();
        let proof = commit(&mut e, "Proofs", source(json!([])));
        let hidden = commit(&mut e, "Secret", source(json!(["bob"])));
        let public = assertion(&proof, "a");
        let secret = assertion(&hidden, "z");
        let data = saved(
            explicit,
            vec![public.clone(), secret.clone()],
            vec![group(vec![secret, public], vec![], vec![proof, hidden])],
        );
        let root = commit(&mut e, "Saved", data);
        let result = query(&e, &root);
        assert_eq!(result.coverage, Coverage::Partial);
        assert!(result.graph.edges.is_empty());
        assert_eq!(
            e.export_capsule(&root, &host()).unwrap_err().code,
            "E_UNAVAILABLE"
        );
        assert!(delivery(&mut e).is_none());
    }
}

#[test]
fn pruning_nonpremise_snapshot_is_not_harmless_index_normalization() {
    for explicit in [false, true] {
        let mut e = engine();
        let proof = commit(&mut e, "Proofs", source(json!([])));
        let unrelated = commit(&mut e, "Secret", source(json!([])));
        let public = assertion(&proof, "a");
        let data = saved(
            explicit,
            vec![public.clone()],
            vec![group(vec![public], vec![], vec![proof, unrelated.clone()])],
        );
        let root = commit(&mut e, "Saved", data);
        let result = query(&e, &root);
        assert_eq!(result.coverage, Coverage::Complete);
        assert!(
            !result.graph.edges[0].derivations[0].input_snapshots.contains(&unrelated),
            "unrelated original explanation snapshot was pruned; materialization may add its own pin"
        );
        assert_eq!(
            e.export_capsule(&root, &host()).unwrap_err().code,
            "E_UNAVAILABLE"
        );
        assert!(
            delivery(&mut e).is_none(),
            "a changed explanation is not the entire stored snapshot"
        );
    }
}
