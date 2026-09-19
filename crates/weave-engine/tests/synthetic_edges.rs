use serde_json::json;
use weave_contract::{algebra, AlgebraContext, CommandResult, GraphData, Program, VERSION};
use weave_engine::{Engine, HostContext};

fn commit(engine: &mut Engine, name: &str, data: GraphData) {
    let plan: Program = serde_json::from_value(
        json!({"version":VERSION,"commands":[{"op":"commit","graph_id":name,"data":data}]}),
    )
    .unwrap();
    assert!(matches!(
        engine
            .execute(&plan, &HostContext::new("reader", [name.into()]))
            .unwrap()[0],
        CommandResult::Committed { .. }
    ));
}
#[test]
fn node_grounded_navigation_relations_gain_real_assertion_origins_after_persistence() {
    let mut engine = Engine::memory().unwrap();
    let source=serde_json::from_value(json!({"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[]})).unwrap();
    commit(&mut engine, "source", source);
    let host = HostContext::new("reader", []);
    let mut value = engine
        .query(
            &serde_json::from_value(json!({"graph_id":"source"})).unwrap(),
            &host,
        )
        .unwrap();
    value.graph.edges.push(serde_json::from_value(json!({"id":"navigation","predicate":"weave:cluster:frontier","from":"a","to":"b","valid_time":{"start":0,"end":1}})).unwrap());
    value.edge_origins.insert("navigation".into(), vec![]);
    let ctx = AlgebraContext {
        principal: "reader".into(),
        max_objects: 100,
        max_output_bytes: 1_000_000,
    };
    let result = algebra::union(value.clone(), value, &ctx).unwrap();
    assert!(result.edge_origins.values().all(Vec::is_empty));
    commit(&mut engine, "saved", result.graph);
    let read = engine
        .query(
            &serde_json::from_value(json!({"graph_id":"saved"})).unwrap(),
            &host,
        )
        .unwrap();
    assert!(read
        .edge_origins
        .values()
        .all(|origins| origins.len() == 1 && origins[0].graph_id == "saved"));
    let support = algebra::support(
        read,
        "weave:cluster:frontier",
        &weave_contract::EntitySpace {
            entity_id: "A".into(),
            space_id: "s".into(),
        },
        &weave_contract::EntitySpace {
            entity_id: "B".into(),
            space_id: "s".into(),
        },
        0,
        &ctx,
    )
    .unwrap();
    assert_eq!(support.graph.nodes[0].properties["state"], "supported");
    assert!(support
        .graph
        .edges
        .iter()
        .all(|e| !e.derived_from.is_empty()));
}
