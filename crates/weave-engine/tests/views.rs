use serde_json::json;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("alice", ["g".into(), "meta".into()])
}
fn data() -> GraphData {
    serde_json::from_value(json!({"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"edge","predicate":"related","from":"a","to":"b","valid_time":{"start":0,"end":10}}]})).unwrap()
}
fn q() -> QueryPlan {
    serde_json::from_value(json!({"graph_id":"g","include_metadata":true})).unwrap()
}
fn write(e: &mut Engine, graph: &str, data: GraphData) {
    let expected_head = e.head(graph, "main").unwrap();
    e.execute(
        &Program {
            version: VERSION.into(),
            commands: vec![Command::Commit {
                graph_id: graph.into(),
                branch_id: "main".into(),
                expected_head,
                data,
            }],
        },
        &host(),
    )
    .unwrap();
}
fn def(clock: ViewClock) -> ViewDefinition {
    ViewDefinition {
        id: "live".into(),
        expression: GraphExpression::Query { query: q() },
        clock,
    }
}
#[test]
fn correction_deletion_and_restart_match_full_recomputation() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("views.db");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, "g", data());
    let initial = e
        .register_view(&def(ViewClock::Fixed), None, &host())
        .unwrap();
    assert_eq!(initial.result, e.query(&q(), &host()).unwrap());
    let mut edited = data();
    edited.edges[0]
        .properties
        .insert("corrected".into(), true.into());
    write(&mut e, "g", edited);
    assert_eq!(
        e.read_view("live", None, ViewFreshness::RequireCurrent, &host())
            .unwrap_err()
            .code,
        "E_FRESHNESS"
    );
    assert!(
        !e.read_view("live", None, ViewFreshness::AllowStale, &host())
            .unwrap()
            .current
    );
    let corrected = e.refresh_view("live", None, &host()).unwrap();
    assert_eq!(corrected.result, e.query(&q(), &host()).unwrap());
    assert_eq!(
        e.view_changes("live", initial.generation, &host())
            .unwrap()
            .unwrap()
            .changed_edges,
        ["edge"]
    );
    let mut removed = data();
    removed.edges.clear();
    write(&mut e, "g", removed);
    let gone = e.refresh_view("live", None, &host()).unwrap();
    assert_eq!(gone.result, e.query(&q(), &host()).unwrap());
    assert_eq!(
        e.view_changes("live", corrected.generation, &host())
            .unwrap()
            .unwrap()
            .removed_edges,
        ["edge"]
    );
    assert_eq!(
        e.view_changes("live", 0, &host()).unwrap_err().code,
        "E_REPLAY_WINDOW"
    );
    drop(e);
    let e = Engine::open(path).unwrap();
    assert_eq!(
        e.read_view("live", None, ViewFreshness::RequireCurrent, &host())
            .unwrap()
            .result,
        gone.result
    );
    assert_eq!(
        e.read_view(
            "live",
            None,
            ViewFreshness::AllowStale,
            &HostContext::new("bob", [])
        )
        .unwrap_err()
        .code,
        "E_UNAVAILABLE"
    );
}
#[test]
fn explicit_tick_retracts_expired_edge_without_graph_change() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, "g", data());
    let initial = e
        .register_view(&def(ViewClock::Tick), Some(9), &host())
        .unwrap();
    assert_eq!(initial.result.graph.edges.len(), 1);
    assert_eq!(e.event_count().unwrap(), 1);
    let expired = e.refresh_view("live", Some(10), &host()).unwrap();
    assert!(expired.result.graph.edges.is_empty());
    assert_eq!(e.event_count().unwrap(), 1);
    assert_eq!(
        e.view_changes("live", 1, &host())
            .unwrap()
            .unwrap()
            .removed_edges,
        ["edge"]
    );
    assert_eq!(
        e.refresh_view("live", Some(9), &host()).unwrap_err().code,
        "E_CLOCK"
    );
    assert_eq!(
        e.refresh_view("live", Some(10), &host())
            .unwrap()
            .generation,
        expired.generation
    );
}
#[test]
fn live_metadata_invalidates_but_pinned_metadata_does_not_follow_future_head() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, "meta", GraphData::default());
    let mut source = data();
    source.attachments.push(serde_json::from_value(json!({"id":"evidence","host":{"kind":"graph"},"key":"evidence","value":{"kind":"live_graph","graph_id":"meta"},"valid_time":{"start":0}})).unwrap());
    write(&mut e, "g", source);
    let initial = e
        .register_view(&def(ViewClock::Fixed), None, &host())
        .unwrap();
    write(&mut e, "meta", data());
    assert_eq!(
        e.read_view("live", None, ViewFreshness::RequireCurrent, &host())
            .unwrap_err()
            .code,
        "E_FRESHNESS"
    );
    let next = e.refresh_view("live", None, &host()).unwrap();
    assert_ne!(initial.result.input_snapshots, next.result.input_snapshots);
    assert_eq!(next.result, e.query(&q(), &host()).unwrap());
    let pinned_meta = e.head("meta", "main").unwrap().unwrap();
    let mut pinned_source = data();
    pinned_source.nodes[0].metadata.push(GraphRef {
        graph_id: "meta".into(),
        revision: pinned_meta,
    });
    write(&mut e, "g", pinned_source);
    let mut pinned_query = q();
    pinned_query.revision = e.head("g", "main").unwrap();
    let pinned_def = ViewDefinition {
        id: "pinned".into(),
        expression: GraphExpression::Query {
            query: pinned_query,
        },
        clock: ViewClock::Fixed,
    };
    let pinned = e.register_view(&pinned_def, None, &host()).unwrap();
    let mut later = data();
    later.edges.clear();
    write(&mut e, "meta", later);
    assert_eq!(
        e.read_view("pinned", None, ViewFreshness::RequireCurrent, &host())
            .unwrap()
            .result,
        pinned.result
    );
}
#[test]
fn incomplete_view_remains_explicit_and_failed_refresh_preserves_previous_snapshot() {
    let mut e = Engine::memory().unwrap();
    let mut source = data();
    source.nodes[0].metadata.push(GraphRef {
        graph_id: "missing".into(),
        revision: "missing".into(),
    });
    write(&mut e, "g", source);
    let view = e
        .register_view(&def(ViewClock::Fixed), None, &host())
        .unwrap();
    assert_eq!(view.result.coverage, Coverage::Partial);
    assert!(!view.current);
    assert_eq!(
        e.read_view("live", None, ViewFreshness::RequireCurrent, &host())
            .unwrap_err()
            .code,
        "E_FRESHNESS"
    );
    let mut other = def(ViewClock::Fixed);
    other.expression = GraphExpression::Reference {
        name: "unbound".into(),
    };
    assert_eq!(
        e.register_view(&other, None, &host()).unwrap_err().code,
        "E_VIEW_CONFLICT"
    );
    assert_eq!(
        e.read_view("live", None, ViewFreshness::AllowStale, &host())
            .unwrap()
            .result,
        view.result
    );
}

#[test]
fn clocked_metadata_selection_filters_target_evidence_at_tick() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, "meta", data());
    let mut source = data();
    source.edges.clear();
    source.attachments.push(serde_json::from_value(json!({"id":"attachment","host":{"kind":"graph"},"key":"evidence","value":{"kind":"graph","reference":{"graph_id":"meta","revision":e.head("meta","main").unwrap().unwrap()}},"valid_time":{"start":0}})).unwrap());
    write(&mut e, "g", source);
    let definition = ViewDefinition {
        id: "metadata-expiry".into(),
        expression: GraphExpression::Metadata {
            input: Box::new(GraphExpression::Query { query: q() }),
            host: MetadataHost::Graph,
            key: "evidence".into(),
        },
        clock: ViewClock::Tick,
    };
    assert_eq!(
        e.register_view(&definition, Some(9), &host())
            .unwrap()
            .result
            .graph
            .edges
            .len(),
        1
    );
    assert!(e
        .refresh_view("metadata-expiry", Some(10), &host())
        .unwrap()
        .result
        .graph
        .edges
        .is_empty());
}
