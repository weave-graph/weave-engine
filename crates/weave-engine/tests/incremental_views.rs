use serde_json::json;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("alice", ["g".into()])
}
fn data() -> GraphData {
    serde_json::from_value(json!({"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"},{"id":"isolated","entity_id":"I","space_id":"s"}],
    "edges":(0..40).map(|i|json!({"id":format!("e{i:02}"),"predicate":"p","from":"a","to":"b","valid_time":{"start":i*10,"end":i*10+5}})).collect::<Vec<_>>() })).unwrap()
}
fn write(e: &mut Engine, d: GraphData) {
    let expected_head = e.head("g", "main").unwrap();
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "g".into(),
                branch_id: "main".into(),
                expected_head,
                data: d,
            }],
        },
        &host(),
    )
    .unwrap();
}
fn definition(clock: ViewClock) -> ViewDefinition {
    ViewDefinition {
        id: "v".into(),
        clock,
        expression: GraphExpression::Query {
            query: serde_json::from_value(json!({"graph_id":"g","include_metadata":true})).unwrap(),
        },
    }
}
fn oracle(e: &mut Engine, t: Option<i64>) -> QueryResult {
    e.query(
        &serde_json::from_value(json!({"graph_id":"g","include_metadata":true,"valid_at":t}))
            .unwrap(),
        &host(),
    )
    .unwrap()
}
#[test]
fn membership_work_is_local_while_snapshot_hashing_and_repinning_are_not() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, data());
    e.register_view(&definition(ViewClock::Fixed), None, &host())
        .unwrap();
    e.enroll_incremental_view("v", &host()).unwrap();
    let (seed, w) = e.refresh_view_with_work("v", None, &host()).unwrap();
    assert_eq!(w.memberships_evaluated, 40);
    assert_eq!(w.fallback_runs, 1);
    assert_eq!(seed.result, oracle(&mut e, None));
    let mut d = data();
    d.edges[7].properties.insert("changed".into(), true.into());
    write(&mut e, d);
    let (next, w) = e.refresh_view_with_work("v", None, &host()).unwrap();
    assert_eq!(w.memberships_evaluated, 1);
    assert_eq!(w.hashed_records, 43);
    assert_eq!(w.output_nodes_repinned, 3);
    assert_eq!(w.output_claims_rendered, 40);
    assert_eq!(w.fallback_runs, 0);
    assert_eq!(next.result, oracle(&mut e, None));
    assert_ne!(seed.result.node_origins, next.result.node_origins);
    let (_, w) = e.refresh_view_with_work("v", None, &host()).unwrap();
    assert_eq!(w.memberships_evaluated, 0);
}
#[test]
fn tick_crossings_retractions_and_restart_use_exact_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.db");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, data());
    e.register_view(&definition(ViewClock::Tick), Some(0), &host())
        .unwrap();
    e.enroll_incremental_view("v", &host()).unwrap();
    e.refresh_view("v", Some(0), &host()).unwrap();
    let (v, w) = e.refresh_view_with_work("v", Some(4), &host()).unwrap();
    assert_eq!(w.memberships_evaluated, 0);
    assert_eq!(v.result, oracle(&mut e, Some(4)));
    drop(e);
    let mut e = Engine::open(path).unwrap();
    let (v, w) = e.refresh_view_with_work("v", Some(5), &host()).unwrap();
    assert_eq!(w.memberships_evaluated, 1);
    assert_eq!(w.crossed_boundary_candidates, 1);
    assert_eq!(v.result, oracle(&mut e, Some(5)));
    assert!(v.result.graph.edges.is_empty());
    let (v, w) = e.refresh_view_with_work("v", Some(21), &host()).unwrap();
    assert_eq!(w.memberships_evaluated, 2);
    assert_eq!(v.result, oracle(&mut e, Some(21)));
    assert_eq!(e.event_count().unwrap(), 1);
}
#[test]
fn explicit_claims_readers_order_and_deletion_match_oracle() {
    let mut d:GraphData=serde_json::from_value(json!({"profile":"explicit","nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"structural_edges":[{"id":"s","predicate":"p","from":"a","to":"b"}],"assertions":[{"id":"z","edge_id":"s","source":"one","valid_time":{"start":0}},{"id":"aa","edge_id":"s","source":"two","valid_time":{"start":0},"polarity":"negative"}]})).unwrap();
    let mut e = Engine::memory().unwrap();
    write(&mut e, d.clone());
    e.register_view(&definition(ViewClock::Fixed), None, &host())
        .unwrap();
    e.enroll_incremental_view("v", &host()).unwrap();
    e.refresh_view("v", None, &host()).unwrap();
    for i in 0..4 {
        if i == 0 {
            d.assertions.reverse();
        }
        if i == 1 {
            d.assertions[0].readers = vec!["bob".into()];
        }
        if i == 2 {
            d.nodes[0].readers = vec!["bob".into()];
        }
        if i == 3 {
            d.assertions.clear();
            d.nodes[0].readers.clear();
        }
        write(&mut e, d.clone());
        let v = e.refresh_view("v", None, &host()).unwrap();
        assert_eq!(v.result, oracle(&mut e, None));
    }
}
#[test]
fn valid_json_cache_corruption_rebuilds_and_unsupported_sources_fall_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.db");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, data());
    e.register_view(&definition(ViewClock::Fixed), None, &host())
        .unwrap();
    e.enroll_incremental_view("v", &host()).unwrap();
    e.refresh_view("v", None, &host()).unwrap();
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute(
        "UPDATE view_selection SET state=json_set(state,'$.claims.e00.selected',json('false'))",
        [],
    )
    .unwrap();
    let (v, w) = e.refresh_view_with_work("v", None, &host()).unwrap();
    assert_eq!(w.fallback_runs, 1);
    assert_eq!(v.result, oracle(&mut e, None));
    // Even a matching byte checksum does not validate inconsistent redundant indexes.
    use sha2::{Digest, Sha256};
    c.execute(
        "UPDATE view_selection SET state=json_set(state,'$.endpoints.a',999)",
        [],
    )
    .unwrap();
    let encoded: String = c
        .query_row("SELECT state FROM view_selection", [], |r| r.get(0))
        .unwrap();
    let hash = format!("{:x}", Sha256::digest(encoded.as_bytes()));
    c.execute("UPDATE view_selection SET digest=?1", [hash])
        .unwrap();
    let (v, w) = e.refresh_view_with_work("v", None, &host()).unwrap();
    assert_eq!(w.fallback_runs, 1);
    assert_eq!(v.result, oracle(&mut e, None));
    let mut d = data();
    d.nodes[0].metadata.push(GraphRef {
        graph_id: "missing".into(),
        revision: "r".into(),
    });
    write(&mut e, d);
    let (v, w) = e.refresh_view_with_work("v", None, &host()).unwrap();
    assert_eq!(w.fallback_runs, 1);
    assert_eq!(v.result, oracle(&mut e, None));
    assert_eq!(v.result.coverage, Coverage::Partial);
    assert!(c
        .query_row("SELECT state IS NULL FROM view_selection", [], |r| r
            .get::<_, bool>(0))
        .unwrap());
}
#[test]
fn auxiliary_schema_upgrade_preserves_primary_rows_and_owner_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.db");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, data());
    e.register_view(&definition(ViewClock::Fixed), None, &host())
        .unwrap();
    let pin = e.head("g", "main").unwrap();
    drop(e);
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch("DROP TABLE view_selection; PRAGMA user_version=12;")
        .unwrap();
    drop(c);
    let mut e = Engine::open(&path).unwrap();
    assert_eq!(e.head("g", "main").unwrap(), pin);
    assert_eq!(e.event_count().unwrap(), 1);
    assert!(e
        .enroll_incremental_view("v", &HostContext::new("bob", []))
        .is_err());
    e.enroll_incremental_view("v", &host()).unwrap();
    let c = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        14
    );
}
