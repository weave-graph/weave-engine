use serde_json::{json, Value};
use weave_contract::*;
use weave_engine::{Engine, HostContext};
fn unit() -> Value {
    json!({"dimension_id":"length","unit_id":"metre","revision":"1"})
}
fn graph(edge_property: bool, quantity: bool) -> GraphData {
    let kind = if quantity {
        json!({"quantity":unit()})
    } else {
        json!("decimal")
    };
    let value = if quantity {
        json!({"amount":"0.3","unit":unit()})
    } else {
        json!("0.3")
    };
    let declaration = json!({"value_type":kind,"required":true});
    let properties = if edge_property {
        json!({})
    } else {
        json!({"amount":value})
    };
    let edge_properties = if edge_property {
        json!({"amount":value})
    } else {
        json!({})
    };
    serde_json::from_value(json!({"schema":{"id":"measurement","revision":"1","nodes":{"Point":{"properties":if edge_property {json!({})} else {json!({"amount":declaration})}}},"edges":{"Measured":{"from_type":"Point","to_type":"Point","properties":if edge_property {json!({"amount":declaration})} else {json!({})}}}},"nodes":[{"id":"n","entity_id":"n","space_id":"physical","type_id":"Point","properties":properties}],"edges":[{"id":"e","from":"n","to":"n","predicate":"measured","type_id":"Measured","valid_time":{"start":0},"properties":edge_properties}]})).unwrap()
}
fn command(data: GraphData, batch: bool) -> Command {
    if batch {
        Command::CommitBatch {
            batch_id: "batch".into(),
            commits: vec![SnapshotCommit {
                graph_id: "data".into(),
                branch_id: "main".into(),
                expected_head: None,
                data,
            }],
        }
    } else {
        Command::Commit {
            graph_id: "data".into(),
            branch_id: "main".into(),
            expected_head: None,
            data,
        }
    }
}
fn host() -> HostContext {
    HostContext::new("alice", ["first".into(), "data".into()])
}
fn query() -> QueryPlan {
    serde_json::from_value(json!({"graph_id":"data"})).unwrap()
}
#[test]
fn exact_scalars_roundtrip_in_nodes_and_edges_for_single_and_batch_commits() {
    for edge in [false, true] {
        for quantity in [false, true] {
            for batch in [false, true] {
                let mut e = Engine::memory().unwrap();
                let data = graph(edge, quantity);
                e.execute(
                    &Program {
                        version: VERSION.into(),
                        source_revisions: vec![],
                        commands: vec![command(data.clone(), batch)],
                    },
                    &host(),
                )
                .unwrap();
                let found = e.query(&query(), &host()).unwrap();
                assert_eq!(found.graph.schema, data.schema);
                assert_eq!(found.graph.nodes[0].properties, data.nodes[0].properties);
                assert_eq!(found.graph.edges[0].properties, data.edges[0].properties);
                let capsule = e
                    .export_capsule(
                        &GraphRef {
                            graph_id: "data".into(),
                            revision: e.head("data", "main").unwrap().unwrap(),
                        },
                        &host(),
                    )
                    .unwrap();
                let mut receiver = Engine::memory().unwrap();
                receiver.receive_capsule(&capsule, &host()).unwrap();
                let mut pinned = query();
                pinned.revision = Some(capsule.root.revision.clone());
                assert_eq!(
                    receiver.query(&pinned, &host()).unwrap().graph.schema,
                    data.schema
                );
            }
        }
    }
}
#[test]
fn older_protocols_reject_both_new_schema_types_before_any_write() {
    for edge in [false, true] {
        for quantity in [false, true] {
            for batch in [false, true] {
                let mut e = Engine::memory().unwrap();
                let program = Program {
                    version: "0.11.0".into(),
                    source_revisions: vec![],
                    commands: vec![
                        Command::Commit {
                            graph_id: "first".into(),
                            branch_id: "main".into(),
                            expected_head: None,
                            data: GraphData::default(),
                        },
                        command(graph(edge, quantity), batch),
                    ],
                };
                assert_eq!(e.execute(&program, &host()).unwrap_err().code, "E_VERSION");
                assert_eq!(e.event_count().unwrap(), 0);
                assert!(e.head("first", "main").unwrap().is_none());
            }
        }
    }
}
#[test]
fn noncanonical_or_mismatched_exact_values_roll_back_entire_program() {
    for value in [json!(0.3), json!("0.30"), json!("1e-1")] {
        let mut e = Engine::memory().unwrap();
        let mut data = graph(false, false);
        data.nodes[0].properties.insert("amount".into(), value);
        let program = Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![
                Command::Commit {
                    graph_id: "first".into(),
                    branch_id: "main".into(),
                    expected_head: None,
                    data: GraphData::default(),
                },
                command(data, false),
            ],
        };
        assert_eq!(
            e.execute(&program, &host()).unwrap_err().code,
            "E_SCHEMA_PROPERTY_TYPE"
        );
        assert_eq!(e.event_count().unwrap(), 0);
        assert!(e.head("first", "main").unwrap().is_none());
    }
    let mut e = Engine::memory().unwrap();
    let mut data = graph(true, true);
    data.edges[0].properties.get_mut("amount").unwrap()["unit"]["revision"] = json!("2");
    assert_eq!(
        e.execute(
            &Program {
                version: VERSION.into(),
                source_revisions: vec![],
                commands: vec![command(data, true)]
            },
            &host()
        )
        .unwrap_err()
        .code,
        "E_SCHEMA_PROPERTY_TYPE"
    );
    assert_eq!(e.event_count().unwrap(), 0);
}
