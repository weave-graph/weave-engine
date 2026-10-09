use ed25519_dalek::SigningKey;
use serde_json::json;
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("collector", ["source".into()])
}
fn policy() -> GovernancePolicy {
    GovernancePolicy {
        view_id: "team".into(),
        reference: GovernancePolicyRef {
            id: "policy".into(),
            revision: "1".into(),
        },
        members: vec![weave_policy::public_key(&SigningKey::from_bytes(&[73; 32]))],
        threshold: 1,
        proposers: vec!["collector".into()],
        readers: vec![],
        allowed_sources: vec![GovernanceSourceScope {
            graph_id: "source".into(),
            branch_id: "main".into(),
        }],
        not_before_ms: 0,
        expires_at_ms: 10000,
    }
}
fn write(e: &mut Engine, value: i64, private: bool) -> GraphRef {
    let p=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"source","expected_head":e.head("source","main").unwrap(),"data":{"nodes":[{"id":"a","entity_id":"A","space_id":"s","properties":{"value":value},"readers":if private {vec!["collector"]} else {vec![]}},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"e","from":"a","to":"b","predicate":"p","valid_time":{"start":5,"end":8},"readers":if private {vec!["collector"]} else {vec![]}}]}}]})).unwrap();
    e.execute(&p, &host()).unwrap();
    GraphRef {
        graph_id: "source".into(),
        revision: e.head("source", "main").unwrap().unwrap(),
    }
}
fn propose(e: &Engine, id: &str, action: GovernanceAction) -> GovernanceDecisionRequest {
    let head = e.inspect_governance_head("team", &host()).unwrap();
    let p = GovernanceProposal {
        id: id.into(),
        view_id: "team".into(),
        policy: head.policy,
        expected_head: head.decision_id,
        expires_at_ms: 9000,
        action,
    };
    let receipt = e.propose_governance(&p, &host()).unwrap();
    let key = SigningKey::from_bytes(&[73; 32]);
    let signed = sign_governance_approval(
        GovernanceApproval {
            proposal_id: id.into(),
            proposal_digest: receipt.digest,
            view_id: "team".into(),
            policy: p.policy,
            expected_head: p.expected_head,
            member: weave_policy::public_key(&key),
            issued_at_ms: 0,
            expires_at_ms: 8000,
            nonce: id.into(),
        },
        &key,
    )
    .unwrap();
    e.record_governance_approval(&signed, &host()).unwrap();
    GovernanceDecisionRequest {
        proposal_id: id.into(),
        nonce: id.into(),
    }
}
fn accept(e: &Engine, id: &str, source: GraphRef) -> GovernanceReceipt {
    let request = propose(
        e,
        id,
        GovernanceAction::Publish {
            source,
            branch_id: "main".into(),
        },
    );
    e.accept_governance(&request, &host()).unwrap()
}
fn observer(e: &Engine) -> String {
    e.recorded_checkpoint_for("source", "main", &host())
        .unwrap()
        .observer
}
fn at(observer: &str, time: i64) -> AcceptedViewHistoryCut {
    AcceptedViewHistoryCut::AtTime {
        observer: observer.into(),
        unix_millis: time,
    }
}

fn evaluate(e: &mut Engine, expression: GraphExpression) -> QueryResult {
    let outputs = e
        .execute(
            &Program {
                version: VERSION.into(),
                source_revisions: vec![],
                commands: vec![Command::Evaluate { value: expression }],
            },
            &host(),
        )
        .unwrap();
    let CommandResult::Queried { result } = outputs.into_iter().next().unwrap() else {
        panic!("missing value")
    };
    *result
}
fn historical(time: i64) -> GraphExpression {
    GraphExpression::AcceptedHistory {
        view_id: "team".into(),
        selection: AcceptedSelection::LocalTime { unix_millis: time },
    }
}
#[test]
fn canonical_accepted_selection_preserves_occurrences_through_empty_composition() {
    let clock = Arc::new(ManualClock::new(5));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    e.install_governance_root(&policy()).unwrap();
    let old = write(&mut e, 1, false);
    let observer = observer(&e);
    clock.set(10);
    let first = accept(&e, "first", old);
    clock.set(20);
    let new = write(&mut e, 2, false);
    clock.set(50);
    accept(&e, "second", new);
    clock.set(60);
    let old = evaluate(&mut e, historical(25));
    let native = e
        .query_accepted_history_for("team", &at(&observer, 25), &host())
        .unwrap();
    assert_eq!(old.accepted_observations, vec![native.observation]);
    assert_eq!(old.graph, native.result.graph);
    let exact = evaluate(
        &mut e,
        GraphExpression::AcceptedHistory {
            view_id: "team".into(),
            selection: AcceptedSelection::Decision {
                observer,
                decision_id: first.decision_id,
            },
        },
    );
    assert_eq!(exact, old);
    let empty = evaluate(
        &mut e,
        GraphExpression::Filter {
            input: Box::new(historical(25)),
            predicate: None,
            valid_at: Some(8),
        },
    );
    assert!(empty.graph.nodes.is_empty() && empty.graph.edges.is_empty());
    assert_eq!(empty.accepted_observations, old.accepted_observations);
    let composed = evaluate(
        &mut e,
        GraphExpression::Project {
            input: Box::new(GraphExpression::Union {
                left: Box::new(historical(25)),
                right: Box::new(historical(50)),
            }),
            node_ids: vec![],
            edge_ids: vec![],
        },
    );
    assert!(composed.graph.nodes.is_empty());
    assert_eq!(composed.accepted_observations.len(), 2);
    for witness in &composed.accepted_observations {
        assert!(composed.input_snapshots.contains(&witness.occurrence));
        assert!(composed.input_snapshots.contains(&witness.source));
    }
}
#[test]
fn canonical_ranges_keep_authorized_start_and_all_half_open_changes_with_fact_time() {
    let clock = Arc::new(ManualClock::new(5));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    e.install_governance_root(&policy()).unwrap();
    let old = write(&mut e, 1, false);
    let observer = observer(&e);
    clock.set(10);
    let first = accept(&e, "first", old);
    clock.set(20);
    let new = write(&mut e, 2, false);
    clock.set(50);
    accept(&e, "second", new);
    clock.set(60);
    let p:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"recorded_range","name":"R","query":{"graph_id":"source","valid_at":8},"observer":observer,"interval":{"start":5,"end":20},"limit":10},{"op":"accepted_range","name":"A","view_id":"team","observer":observer,"interval":{"start":10,"end":50},"limit":10,"valid_at":8}]})).unwrap();
    let output = e.execute(&p, &host()).unwrap();
    for value in &output {
        let CommandResult::HistoryRanged { range, .. } = value else {
            panic!("missing range")
        };
        assert_eq!(range.changes.len(), 1);
        assert!(range.start_state.graph.nodes.is_empty());
        assert!(range.changes[0].graph.nodes.is_empty());
    }
    let CommandResult::HistoryRanged { range, .. } = &output[0] else {
        unreachable!()
    };
    assert_eq!(range.axis, HistoryAxis::Recorded);
    assert_eq!(range.start_state.recorded_observations[0].recorded_at_ms, 5);
    assert_eq!(
        range.changes[0].recorded_observations,
        range.start_state.recorded_observations
    );
    let CommandResult::HistoryRanged { range, .. } = &output[1] else {
        unreachable!()
    };
    assert_eq!(range.axis, HistoryAxis::Accepted);
    assert_eq!(
        range.start_state.accepted_observations[0].decision_id,
        first.decision_id
    );
    assert_eq!(
        range.changes[0].accepted_observations,
        range.start_state.accepted_observations
    );
}
#[test]
fn old_protocol_or_invalid_range_cannot_publish_earlier_commands() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::memory_with_clock(clock).unwrap();
    for new in [
        json!({"op":"evaluate","value":historical(10)}),
        json!({"op":"accepted_range","name":"R","view_id":"team","observer":"local","interval":{"start":0,"end":10},"limit":10}),
        json!({"op":"recorded_range","name":"R","query":{"graph_id":"source"},"observer":"local","interval":{"start":0,"end":10},"limit":10}),
    ] {
        let p=serde_json::from_value(json!({"version":"0.20.0","commands":[{"op":"commit","graph_id":"source","data":{}},new]})).unwrap();
        assert_eq!(e.execute(&p, &host()).unwrap_err().code, "E_VERSION");
        assert_eq!(e.head("source", "main").unwrap(), None);
        assert_eq!(e.event_count().unwrap(), 0);
    }
    let p=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"source","data":{}},{"op":"recorded_range","name":"R","query":{"graph_id":"source"},"observer":"local","interval":{"start":-1,"end":10},"limit":10}]})).unwrap();
    assert_eq!(e.execute(&p, &host()).unwrap_err().code, "E_HISTORY_RANGE");
    assert_eq!(e.head("source", "main").unwrap(), None);
}

#[test]
fn handler_range_receipts_recheck_empty_start_changes_and_current_policy() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("handler-range.db");
    let clock = Arc::new(ManualClock::new(5));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    e.install_governance_root(&policy()).unwrap();
    let source = write(&mut e, 1, false);
    let observer = observer(&e);
    clock.set(10);
    accept(&e, "first", source);
    clock.set(60);
    e.install_adapter(
        &AdapterManifest {
            id: "ranges".into(),
            version: "1".into(),
            artifact_digest: format!("sha256:{}", "c".repeat(64)),
            config_revision: "1".into(),
            principal: "collector".into(),
            subscriptions: vec![SubscriptionScope {
                graph_id: "source".into(),
                branch_id: "main".into(),
            }],
            output_graphs: vec![],
            effect_destinations: vec![],
            max_attempts: 3,
            lease_ms: 1000,
            max_pending_events: 10,
            projection_replay: true,
        },
        &HostContext::new("collector", []),
    )
    .unwrap();
    e.set_adapter_state("ranges", "running").unwrap();
    let event = e.poll_adapter("ranges").unwrap().unwrap();
    let program: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{
        "op":"accepted_range","name":"History","view_id":"team","observer":observer,"interval":{"start":10,"end":50},"limit":10,"valid_at":8
    }]})).unwrap();
    let original = e
        .complete_handler("ranges", &event.id, &event.lease, &program)
        .unwrap();
    let CommandResult::HistoryRanged { range, .. } = &original.results[0] else {
        panic!("missing range")
    };
    assert!(range.start_state.graph.nodes.is_empty() && range.changes[0].graph.nodes.is_empty());
    assert!(
        e.complete_handler("ranges", &event.id, &event.lease, &program)
            .unwrap()
            .duplicate
    );
    let sql = rusqlite::Connection::open(&path).unwrap();
    for start in [true, false] {
        let mut changed = serde_json::to_value(&original.results).unwrap();
        let value = if start {
            &mut changed[0]["range"]["start_state"]
        } else {
            &mut changed[0]["range"]["changes"][0]
        };
        value["accepted_observations"][0]["observer"] = json!("foreign");
        sql.execute(
            "UPDATE handler_receipts SET results=?1 WHERE adapter='ranges'",
            [serde_json::to_string(&changed).unwrap()],
        )
        .unwrap();
        assert!(
            e.complete_handler("ranges", &event.id, &event.lease, &program)
                .is_err(),
            "every cached range value must validate its exact witness"
        );
    }
    sql.execute(
        "UPDATE handler_receipts SET results=?1 WHERE adapter='ranges'",
        [serde_json::to_string(&original.results).unwrap()],
    )
    .unwrap();
    clock.set(10000);
    assert!(
        e.complete_handler("ranges", &event.id, &event.lease, &program)
            .is_err(),
        "expired current policy must deny even an empty historical collection"
    );
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM handler_receipts WHERE adapter='ranges'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn persisted_result_revalidates_exact_accepted_witness_even_when_graph_is_empty() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store");
    let clock = Arc::new(ManualClock::new(5));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    e.install_governance_root(&policy()).unwrap();
    let source = write(&mut e, 1, false);
    clock.set(10);
    accept(&e, "first", source);
    clock.set(20);
    let value = evaluate(
        &mut e,
        GraphExpression::Filter {
            input: Box::new(historical(10)),
            predicate: None,
            valid_at: Some(8),
        },
    );
    assert!(value.graph.nodes.is_empty());
    let definition = ViewDefinition {
        id: "cache".into(),
        expression: GraphExpression::Query {
            query: serde_json::from_value(json!({"graph_id":"source"})).unwrap(),
        },
        clock: ViewClock::Fixed,
    };
    e.register_view(&definition, None, &host()).unwrap();
    drop(e);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE live_views SET result=?1 WHERE id='cache'",
            [serde_json::to_string(&value).unwrap()],
        )
        .unwrap();
    drop(connection);
    let e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    assert_eq!(
        e.read_view("cache", None, ViewFreshness::AllowStale, &host())
            .unwrap()
            .result,
        value
    );
    drop(e);
    for change in ["time", "observer", "occurrence", "source", "missing-pin"] {
        let mut bad = value.clone();
        let witness = &mut bad.accepted_observations[0];
        match change {
            "time" => witness.accepted_at_ms += 1,
            "observer" => witness.observer = "foreign".into(),
            "occurrence" => witness.occurrence.revision = "missing".into(),
            "source" => witness.source.revision = "missing".into(),
            _ => bad.input_snapshots.clear(),
        }
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute(
                "UPDATE live_views SET result=?1 WHERE id='cache'",
                [serde_json::to_string(&bad).unwrap()],
            )
            .unwrap();
        drop(connection);
        let e = Engine::open_with_clock(&path, clock.clone()).unwrap();
        assert!(
            e.read_view("cache", None, ViewFreshness::AllowStale, &host())
                .is_err(),
            "{change}"
        );
        drop(e);
    }
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE live_views SET result=?1 WHERE id='cache'",
            [serde_json::to_string(&value).unwrap()],
        )
        .unwrap();
    drop(connection);
    clock.set(10000);
    let e = Engine::open_with_clock(&path, clock).unwrap();
    assert!(e
        .read_view("cache", None, ViewFreshness::AllowStale, &host())
        .is_err());
}
