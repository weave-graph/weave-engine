//! Independent orchestration acceptance tests for information flow.
use weave_contract::{Command, CommandResult, GraphData, Program, QueryPlan, VERSION};
use weave_engine::{Engine, HostContext};

fn data(source: Option<(&str, &str)>, private: bool) -> GraphData {
    let mut value = serde_json::json!({
        "nodes": [
            {"id":"a", "entity_id":"a", "space_id":"operational"},
            {"id":"b", "entity_id":"b", "space_id":"operational"}
        ],
        "edges": [{"id":"fact", "predicate":"reveals", "from":"a", "to":"b",
            "valid_time":{"start":0,"end":null},
            "readers": if private { vec!["alice"] } else { vec![] }}]
    });
    if let Some((graph, revision)) = source {
        value["edges"][0]["derived_from"] = serde_json::json!([
            {"graph_id":graph,"revision":revision,"assertion_id":"fact"}
        ]);
    }
    serde_json::from_value(value).unwrap()
}

fn commit(engine: &mut Engine, graph: &str, data: GraphData) -> String {
    let host = HostContext::new("alice", [graph.to_string()]);
    let result = engine
        .execute(
            &Program {
                version: VERSION.into(),
                commands: vec![Command::Commit {
                    graph_id: graph.into(),
                    branch_id: "main".into(),
                    expected_head: None,
                    data,
                }],
            },
            &host,
        )
        .unwrap();
    match &result[0] {
        CommandResult::Committed { revision, .. } => revision.clone(),
        _ => panic!("expected commit"),
    }
}

fn edges_for(engine: &Engine, graph: &str, principal: &str) -> usize {
    let plan: QueryPlan = serde_json::from_value(serde_json::json!({"graph_id":graph})).unwrap();
    engine
        .query(&plan, &HostContext::new(principal, Vec::<String>::new()))
        .unwrap()
        .graph
        .edges
        .len()
}

#[test]
fn derived_output_cannot_launder_restricted_evidence() {
    let mut engine = Engine::memory().unwrap();
    let source = commit(&mut engine, "private-source", data(None, true));
    commit(
        &mut engine,
        "derived",
        data(Some(("private-source", &source)), false),
    );
    assert_eq!(edges_for(&engine, "derived", "alice"), 1);
    assert_eq!(
        edges_for(&engine, "derived", "bob"),
        0,
        "removing the hidden provenance reference must not publish its derived conclusion"
    );
}

#[test]
fn restrictions_follow_multiple_derivation_steps() {
    let mut engine = Engine::memory().unwrap();
    let source = commit(&mut engine, "source", data(None, true));
    let middle = commit(
        &mut engine,
        "middle",
        data(Some(("source", &source)), false),
    );
    commit(&mut engine, "last", data(Some(("middle", &middle)), false));
    assert_eq!(edges_for(&engine, "last", "alice"), 1);
    assert_eq!(edges_for(&engine, "last", "bob"), 0);
}

#[test]
fn missing_premise_does_not_grant_public_visibility() {
    let mut engine = Engine::memory().unwrap();
    commit(
        &mut engine,
        "derived",
        data(Some(("unavailable", "missing-revision")), false),
    );
    assert_eq!(edges_for(&engine, "derived", "bob"), 0);
}
