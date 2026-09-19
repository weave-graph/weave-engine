//! Repeated provenance work is bounded across a whole operation and resets on return.
use serde_json::json;
use weave_contract::{Command, GraphData, Program, QueryPlan, VERSION};
use weave_engine::{Engine, HostContext};
fn host() -> HostContext {
    HostContext::new(
        "reader",
        ["source".into(), "derived".into(), "marker".into()],
    )
}
fn graph() -> GraphData {
    serde_json::from_value(json!({"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"e","predicate":"p","from":"a","to":"b","valid_time":{"start":0}}]})).unwrap()
}
fn commit(e: &mut Engine, id: &str, data: GraphData) -> String {
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
    e.head(id, "main").unwrap().unwrap()
}
fn query(id: &str) -> QueryPlan {
    serde_json::from_value(json!({"graph_id":id})).unwrap()
}
fn fixture(count: usize, payload: usize) -> Engine {
    let mut e = Engine::memory().unwrap();
    let mut source = graph();
    source.nodes[0]
        .properties
        .insert("payload".into(), json!("x".repeat(payload)));
    let revision = commit(&mut e, "source", source);
    let mut derived = graph();
    derived.edges = (0..count)
        .map(|i| {
            let mut edge = graph().edges.remove(0);
            edge.id = format!("derived:{i}");
            edge.derived_from = vec![weave_contract::AssertionRef {
                graph_id: "source".into(),
                revision: revision.clone(),
                assertion_id: "e".into(),
            }];
            edge
        })
        .collect();
    commit(&mut e, "derived", derived);
    e
}
#[test]
fn repeated_large_premises_hit_shared_byte_budget_and_rollback_prior_commands() {
    let mut e = fixture(70, 2 * 1024 * 1024);
    let before = e.event_count().unwrap();
    let program = Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands: vec![
            Command::Commit {
                graph_id: "marker".into(),
                branch_id: "main".into(),
                expected_head: None,
                data: graph(),
            },
            Command::Query {
                query: query("derived"),
            },
        ],
    };
    let error = e.execute(&program, &host()).unwrap_err();
    assert_eq!(error.code, "E_BUDGET");
    assert!(error.message.contains("graph-read"));
    assert_eq!(e.event_count().unwrap(), before);
    assert!(e.head("marker", "main").unwrap().is_none());
    // The same engine remains usable after failure: scopes are not sticky.
    assert_eq!(
        e.query(&query("source"), &host())
            .unwrap()
            .graph
            .edges
            .len(),
        1
    );
}
#[test]
fn tiny_repeated_premises_hit_call_budget_while_bounded_work_succeeds() {
    let e = fixture(4200, 0);
    let error = e.query(&query("derived"), &host()).unwrap_err();
    assert_eq!(error.code, "E_BUDGET");
    assert!(error.message.contains("count"));
    assert_eq!(
        e.query(&query("source"), &host())
            .unwrap()
            .graph
            .edges
            .len(),
        1
    );
    let e = fixture(8, 1024);
    assert_eq!(
        e.query(&query("derived"), &host())
            .unwrap()
            .graph
            .edges
            .len(),
        8
    );
}
#[test]
fn engine_retains_send_for_host_actor_threads() {
    fn require_send<T: Send>() {}
    require_send::<Engine>();
    let e = fixture(1, 0);
    assert_eq!(
        std::thread::spawn(move || e
            .query(&query("derived"), &host())
            .unwrap()
            .graph
            .edges
            .len())
        .join()
        .unwrap(),
        1
    );
}
