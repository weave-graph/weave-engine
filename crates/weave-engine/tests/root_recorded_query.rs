use serde_json::json;
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;

fn host() -> HostContext {
    HostContext::new("alice", ["g".into(), "other".into()])
}
fn write(e: &mut Engine, value: i64) {
    let p = serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"g","expected_head":e.head("g","main").unwrap(),"data":{"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"e","from":"a","to":"b","predicate":"p","valid_time":{"start":5,"end":8},"properties":{"value":value}}]}}]})).unwrap();
    e.execute(&p, &host()).unwrap();
}
fn expression(time: i64) -> GraphExpression {
    serde_json::from_value(json!({"kind":"recorded_query","query":{"graph_id":"g","valid_at":7},"selection":{"kind":"local_time","unix_millis":time}})).unwrap()
}
fn evaluate(e: &mut Engine, value: GraphExpression) -> QueryResult {
    let p = Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands: vec![Command::Evaluate { value }],
    };
    let results = e.execute(&p, &host()).unwrap();
    let CommandResult::Queried { result } = results.into_iter().next().unwrap() else {
        panic!("evaluation missing");
    };
    *result
}

#[test]
fn canonical_queries_preserve_both_historical_observations_through_composition() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    write(&mut e, 1);
    clock.set(20);
    write(&mut e, 2);
    clock.set(30);
    let old = evaluate(&mut e, expression(15));
    let new = evaluate(&mut e, expression(25));
    assert_eq!(old.graph.edges[0].properties["value"], json!(1));
    assert_eq!(new.graph.edges[0].properties["value"], json!(2));
    let union = evaluate(
        &mut e,
        GraphExpression::Union {
            left: Box::new(expression(15)),
            right: Box::new(expression(25)),
        },
    );
    assert_eq!(union.recorded_observations.len(), 2);
    assert!(union
        .recorded_observations
        .contains(&old.recorded_observations[0]));
    assert!(union
        .recorded_observations
        .contains(&new.recorded_observations[0]));
    for observation in &union.recorded_observations {
        assert!(union.input_snapshots.contains(&observation.graph));
    }
    let exact = &old.recorded_observations[0];
    let checkpoint = GraphExpression::RecordedQuery {
        query: serde_json::from_value(json!({"graph_id":"g","valid_at":7})).unwrap(),
        selection: RecordedSelection::Checkpoint {
            observer: exact.observer.clone(),
            checkpoint: exact.checkpoint.clone(),
        },
    };
    assert_eq!(evaluate(&mut e, checkpoint), old);
    let clipped = evaluate(
        &mut e,
        GraphExpression::Window {
            input: Box::new(expression(15)),
            window: Interval {
                start: 6,
                end: Some(8),
            },
        },
    );
    assert_eq!(clipped.recorded_observations, old.recorded_observations);
}

#[test]
fn old_protocol_and_invalid_selectors_cannot_publish_an_earlier_write() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::memory_with_clock(clock).unwrap();
    let mut p:Program=serde_json::from_value(json!({"version":"0.19.0","commands":[{"op":"commit","graph_id":"other","data":{}},{"op":"evaluate","value":expression(10)}]})).unwrap();
    assert_eq!(e.execute(&p, &host()).unwrap_err().code, "E_VERSION");
    assert_eq!(e.head("other", "main").unwrap(), None);
    assert_eq!(e.event_count().unwrap(), 0);
    p.version = VERSION.into();
    p.commands[1] = Command::Evaluate {
        value: expression(-1),
    };
    assert_eq!(
        e.execute(&p, &host()).unwrap_err().code,
        "E_HISTORY_SELECTOR"
    );
    assert_eq!(e.head("other", "main").unwrap(), None);
}

#[test]
fn fixed_recorded_view_recomputes_equal_time_appends_and_revalidates_cached_witness() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store");
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    write(&mut e, 1);
    let definition = ViewDefinition {
        id: "history".into(),
        expression: expression(10),
        clock: ViewClock::Fixed,
    };
    let original = e.register_view(&definition, None, &host()).unwrap();
    write(&mut e, 2);
    assert!(e
        .read_view("history", None, ViewFreshness::RequireCurrent, &host())
        .is_err());
    let updated = e.refresh_view("history", None, &host()).unwrap();
    assert_eq!(updated.result.graph.edges[0].properties["value"], json!(2));
    assert_ne!(
        updated.result.recorded_observations,
        original.result.recorded_observations
    );
    let observation = updated.result.recorded_observations[0].clone();
    drop(e);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE head_observations SET recorded_at_ms=9 WHERE id=?1",
            [observation.checkpoint],
        )
        .unwrap();
    drop(connection);
    clock.set(20);
    let e = Engine::open_with_clock(&path, clock).unwrap();
    assert!(e
        .read_view("history", None, ViewFreshness::AllowStale, &host())
        .is_err());
}

#[test]
fn compiled_recorded_view_keeps_recorded_criterion_separate_from_tick_valid_time() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    write(&mut e, 1);
    clock.set(20);
    write(&mut e, 2);
    clock.set(30);
    let template = weave_contract::view_registration::seal_template(CompiledViewTemplate {
        format: weave_contract::view_registration::VIEW_TEMPLATE_FORMAT.into(),
        protocol: VERSION.into(),
        name: "history".into(),
        revision: "1".into(),
        expression: expression(15),
        clock: ViewClock::Tick,
        source_revisions: vec![],
        definition_digest: String::new(),
    })
    .unwrap();
    let registered = e
        .register_compiled_view("history", &template, Some(7), &host())
        .unwrap();
    assert_eq!(
        registered.result.graph.edges[0].properties["value"],
        json!(1)
    );
    let expired = e.refresh_view("history", Some(8), &host()).unwrap();
    assert!(expired.result.graph.edges.is_empty());
    assert_eq!(
        expired.result.recorded_observations,
        registered.result.recorded_observations
    );
}
