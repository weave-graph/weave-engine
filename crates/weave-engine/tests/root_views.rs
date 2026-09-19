use serde_json::json;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("alice", ["Source".into(), "Evidence".into()])
}
fn commit(engine: &mut Engine, graph: &str, data: GraphData) {
    let head = engine.head(graph, "main").unwrap();
    engine.execute(&serde_json::from_value(json!({"version":"0.5.0","commands":[{"op":"commit","graph_id":graph,"expected_head":head,"data":data}]})).unwrap(),&host()).unwrap();
}
fn query(tick: Option<i64>) -> QueryPlan {
    serde_json::from_value(json!({"graph_id":"Source","valid_at":tick,"include_metadata":true}))
        .unwrap()
}
fn data(turn: i64) -> GraphData {
    let mut d:GraphData=serde_json::from_value(json!({"nodes":[{"id":"a","entity_id":"A","space_id":"s","properties":{"turn":turn}},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"edge","predicate":"connected","from":"a","to":"b","valid_time":{"start":0,"end":20},"properties":{"quality":turn}}]})).unwrap();
    if turn % 5 == 0 {
        d.edges.clear();
    } else if turn % 3 == 0 {
        d.edges[0].readers = vec!["bob".into()];
    }
    d
}
#[test]
fn live_snapshot_matches_fresh_query_through_mutations_ticks_and_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("root-views.db");
    let mut e = Engine::open(&path).unwrap();
    commit(&mut e, "Source", data(0));
    let def = ViewDefinition {
        id: "independent".into(),
        expression: GraphExpression::Query { query: query(None) },
        clock: ViewClock::Tick,
    };
    let first = e.register_view(&def, Some(0), &host()).unwrap();
    assert_eq!(first.result, e.query(&query(Some(0)), &host()).unwrap());
    let mut last_generation = first.generation;
    for turn in 1..25 {
        commit(&mut e, "Source", data(turn));
        let events = e.event_count().unwrap();
        assert_eq!(
            e.read_view(
                "independent",
                Some(turn),
                ViewFreshness::RequireCurrent,
                &host()
            )
            .unwrap_err()
            .code,
            "E_FRESHNESS"
        );
        if turn % 4 == 0 {
            drop(e);
            e = Engine::open(&path).unwrap();
        }
        let value = e.refresh_view("independent", Some(turn), &host()).unwrap();
        let oracle = e.query(&query(Some(turn)), &host()).unwrap();
        assert_eq!(value.result, oracle);
        assert_eq!(value.current, oracle.coverage == Coverage::Complete);
        assert_eq!(value.generation, last_generation + 1);
        let delta = e
            .view_changes("independent", last_generation, &host())
            .unwrap()
            .unwrap();
        assert_eq!(delta.result, oracle);
        assert_eq!(e.event_count().unwrap(), events);
        last_generation = value.generation;
        assert_eq!(
            e.refresh_view("independent", Some(turn), &host())
                .unwrap()
                .generation,
            last_generation
        );
    }
    assert_eq!(
        e.read_view(
            "independent",
            Some(24),
            ViewFreshness::AllowStale,
            &HostContext::new("bob", [])
        )
        .unwrap_err()
        .code,
        "E_UNAVAILABLE"
    );
}
#[test]
fn received_missing_dependency_completes_pinned_view_without_acceptance() {
    let mut remote = Engine::memory().unwrap();
    commit(&mut remote, "Evidence", data(1));
    let reference = GraphRef {
        graph_id: "Evidence".into(),
        revision: remote.head("Evidence", "main").unwrap().unwrap(),
    };
    let capsule = remote.export_capsule(&reference, &host()).unwrap();
    let mut e = Engine::memory().unwrap();
    let mut source = data(1);
    source.nodes[0].metadata.push(reference);
    commit(&mut e, "Source", source);
    let mut pinned = query(None);
    pinned.revision = e.head("Source", "main").unwrap();
    let definition = ViewDefinition {
        id: "partial".into(),
        expression: GraphExpression::Query {
            query: pinned.clone(),
        },
        clock: ViewClock::Fixed,
    };
    let first = e.register_view(&definition, None, &host()).unwrap();
    assert!(!first.current);
    assert_eq!(first.result.coverage, Coverage::Partial);
    assert_eq!(e.receive_capsule(&capsule, &host()).unwrap(), 1);
    assert_eq!(e.head("Evidence", "main").unwrap(), None);
    assert_eq!(e.event_count().unwrap(), 1);
    let complete = e.refresh_view("partial", None, &host()).unwrap();
    assert!(complete.current);
    assert_eq!(complete.result.coverage, Coverage::Complete);
    assert_eq!(complete.result, e.query(&pinned, &host()).unwrap());
    assert_eq!(complete.generation, first.generation + 1);
    assert_eq!(e.event_count().unwrap(), 1);
}
