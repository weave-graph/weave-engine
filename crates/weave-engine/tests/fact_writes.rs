//! Fact-level writes, node valid time and per-fact provenance (A2, ADR-0004,
//! ADR-0005), through the public program interface.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{CommandResult, Program, QueryResult, VERSION};
use weave_engine::{Engine, HostContext, ManualClock};

fn host(principal: &str) -> HostContext {
    HostContext::new(principal, ["g".to_string()])
}
fn engine(clock: &Arc<ManualClock>) -> Engine {
    Engine::memory_with_clock(clock.clone()).unwrap()
}
fn program(version: &str, commands: Value) -> Program {
    serde_json::from_value(json!({"version": version, "commands": commands})).unwrap()
}
fn run(e: &mut Engine, who: &str, commands: Value) -> weave_engine::Result<Vec<CommandResult>> {
    e.execute(&program(VERSION, commands), &host(who))
}
fn write(changes: Value) -> Value {
    json!({"op": "write", "graph_id": "g", "changes": changes})
}
fn node(id: &str) -> Value {
    json!({"kind": "node", "id": id, "entity_id": id.to_uppercase(), "space_id": "s"})
}
fn edge(id: &str, from: &str, to: &str) -> Value {
    json!({"kind": "edge", "id": id, "predicate": "p", "from": from, "to": to, "valid_time": {"start": 0}})
}
fn assert_(fact: Value) -> Value {
    json!({"change": "assert", "fact": fact})
}
fn committed(results: &[CommandResult]) -> String {
    match results.last().unwrap() {
        CommandResult::Committed { revision, .. } => revision.clone(),
        other => panic!("expected a committed result, got {other:?}"),
    }
}
fn query(e: &mut Engine, who: &str, plan: Value) -> QueryResult {
    let results = run(e, who, json!([{"op": "query", "query": plan}])).unwrap();
    match results.into_iter().next().unwrap() {
        CommandResult::Queried { result } => *result,
        other => panic!("expected a query result, got {other:?}"),
    }
}
fn ids<T>(items: &[T], id: impl Fn(&T) -> &str) -> Vec<String> {
    let mut ids: Vec<String> = items.iter().map(|i| id(i).to_string()).collect();
    ids.sort();
    ids
}
fn code(result: weave_engine::Result<Vec<CommandResult>>) -> String {
    result.expect_err("expected an error").code
}

#[test]
fn assert_correct_retract_keep_immutable_history() {
    let clock = Arc::new(ManualClock::new(100));
    let mut e = engine(&clock);
    let r1 = committed(
        &run(
            &mut e,
            "alice",
            json!([write(json!([
                assert_(node("a")),
                assert_(node("b")),
                assert_(edge("e", "a", "b"))
            ]))]),
        )
        .unwrap(),
    );
    assert!(r1.starts_with("fact:sha256:"), "{r1}");
    clock.set(200);
    let mut corrected = node("a");
    corrected["properties"] = json!({"firmware": "2.0"});
    let r2 = committed(
        &run(
            &mut e,
            "alice",
            json!([write(json!([
                {"change": "correct", "fact": corrected},
                {"change": "retract", "target": {"kind": "edge", "id": "e"}}
            ]))]),
        )
        .unwrap(),
    );
    let head = query(&mut e, "alice", json!({"graph_id": "g"}));
    assert_eq!(head.snapshots["g"], r2);
    assert_eq!(ids(&head.graph.nodes, |n| &n.id), ["a", "b"]);
    assert!(head.graph.edges.is_empty());
    assert_eq!(
        head.graph
            .nodes
            .iter()
            .find(|n| n.id == "a")
            .unwrap()
            .properties["firmware"],
        "2.0"
    );
    // The earlier revision stays readable with its original content.
    let old = query(&mut e, "alice", json!({"graph_id": "g", "revision": r1}));
    assert_eq!(ids(&old.graph.edges, |e| &e.id), ["e"]);
    assert!(old.graph.nodes.iter().all(|n| n.properties.is_empty()));
    // Re-asserting identical content is a no-op.
    let results = run(&mut e, "alice", json!([write(json!([assert_(node("b"))]))])).unwrap();
    assert_eq!(
        results[0],
        CommandResult::Unchanged {
            revision: r2.clone()
        }
    );
    assert_eq!(e.head("g", "main").unwrap().as_deref(), Some(r2.as_str()));
}

#[test]
fn revision_ids_are_content_derived_and_writer_independent() {
    let changes = json!([
        assert_(node("a")),
        assert_(node("b")),
        assert_(edge("e", "a", "b"))
    ]);
    let reordered = json!([
        assert_(edge("e", "a", "b")),
        assert_(node("b")),
        assert_(node("a"))
    ]);
    let mut one = engine(&Arc::new(ManualClock::new(5)));
    let mut two = engine(&Arc::new(ManualClock::new(900)));
    let a = committed(&run(&mut one, "alice", json!([write(changes)])).unwrap());
    let b = committed(&run(&mut two, "bob", json!([write(reordered)])).unwrap());
    assert_eq!(
        a, b,
        "writer, recorded time and change order are not content"
    );
}

#[test]
fn optimistic_concurrency_on_head_and_facts() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = engine(&clock);
    let r1 = committed(&run(&mut e, "alice", json!([write(json!([assert_(node("a"))]))])).unwrap());
    let r2 = committed(&run(&mut e, "alice", json!([write(json!([assert_(node("b"))]))])).unwrap());
    let stale = json!({"op": "write", "graph_id": "g", "expected_head": r1, "changes": [assert_(node("c"))]});
    assert_eq!(code(run(&mut e, "alice", json!([stale]))), "E_CONFLICT");
    let new_branch = json!({"op": "write", "graph_id": "g", "expected_head": null, "changes": [assert_(node("c"))]});
    assert_eq!(
        code(run(&mut e, "alice", json!([new_branch]))),
        "E_CONFLICT"
    );
    let fresh = json!({"op": "write", "graph_id": "g", "branch_id": "other", "expected_head": null, "changes": [assert_(node("c"))]});
    run(&mut e, "alice", json!([fresh])).unwrap();
    let current = json!({"op": "write", "graph_id": "g", "expected_head": r2, "changes": [assert_(node("c"))]});
    let r3 = committed(&run(&mut e, "alice", json!([current])).unwrap());
    // Per-fact: `a` is unchanged since r1, so a correction expecting that succeeds.
    let mut a2 = node("a");
    a2["properties"] = json!({"v": 2});
    let ok = json!({"change": "correct", "fact": a2, "expected": {"kind": "unchanged_since", "revision": r1}});
    let r4 = committed(&run(&mut e, "alice", json!([write(json!([ok]))])).unwrap());
    // Now `a` changed after r3, so the same expectation fails and nothing is written.
    let mut a3 = node("a");
    a3["properties"] = json!({"v": 3});
    let late = json!({"change": "correct", "fact": a3, "expected": {"kind": "unchanged_since", "revision": r3}});
    assert_eq!(
        code(run(&mut e, "alice", json!([write(json!([late]))]))),
        "E_CONFLICT"
    );
    let absent = json!({"change": "retract", "target": {"kind": "node", "id": "c"}, "expected": {"kind": "absent"}});
    assert_eq!(
        code(run(&mut e, "alice", json!([write(json!([absent]))]))),
        "E_CONFLICT"
    );
    assert_eq!(e.head("g", "main").unwrap().as_deref(), Some(r4.as_str()));
}

#[test]
fn invalid_writes_change_nothing() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = engine(&clock);
    let r1 = committed(
        &run(
            &mut e,
            "alice",
            json!([write(json!([
                assert_(node("a")),
                assert_(node("b")),
                assert_(edge("e", "a", "b"))
            ]))]),
        )
        .unwrap(),
    );
    let events = e.event_count().unwrap();
    let mut moved = edge("e", "b", "a");
    moved["properties"] = json!({});
    let cases = [
        (
            json!([assert_(edge("x", "a", "missing"))]),
            "E_SCHEMA_ENDPOINT",
        ),
        (
            json!([{"change": "retract", "target": {"kind": "node", "id": "a"}}]),
            "E_DEPENDENT_FACT",
        ),
        (json!([{"change": "correct", "fact": moved}]), "E_CONFLICT"),
        (
            json!([
                assert_(node("a")),
                assert_(json!({"kind": "node", "id": "a", "entity_id": "Z", "space_id": "s"}))
            ]),
            "E_CONFLICT",
        ),
        (
            json!([assert_(
                json!({"kind": "node", "id": "a", "entity_id": "Z", "space_id": "s"})
            )]),
            "E_CONFLICT",
        ),
        (
            json!([{"change": "correct", "fact": node("nope")}]),
            "E_CONFLICT",
        ),
        (
            json!([{"change": "retract", "target": {"kind": "edge", "id": "a"}}]),
            "E_CONFLICT",
        ),
        (
            json!([assert_(
                json!({"kind": "node", "id": "v", "entity_id": "V", "space_id": "s", "valid_time": {"start": 5, "end": 5}})
            )]),
            "E_INTERVAL",
        ),
        (json!([]), "E_FACT_WRITE"),
    ];
    for (changes, expected) in cases {
        assert_eq!(
            code(run(&mut e, "alice", json!([write(changes.clone())]))),
            expected,
            "{changes}"
        );
    }
    // Retracting the edge and both nodes together is consistent.
    let all = json!([
        {"change": "retract", "target": {"kind": "edge", "id": "e"}},
        {"change": "retract", "target": {"kind": "node", "id": "a"}},
        {"change": "retract", "target": {"kind": "node", "id": "b"}}
    ]);
    // A later failing command rolls back the whole program, including the write.
    let failing = json!([write(all.clone()), {"op": "write", "graph_id": "g", "changes": [{"change": "correct", "fact": node("gone")}]}]);
    assert_eq!(code(run(&mut e, "alice", failing)), "E_CONFLICT");
    assert_eq!(e.head("g", "main").unwrap().as_deref(), Some(r1.as_str()));
    assert_eq!(e.event_count().unwrap(), events);
    // Authority and version gates.
    assert_eq!(
        code(e.execute(
            &program(VERSION, json!([write(all.clone())])),
            &HostContext::new("alice", [])
        )),
        "E_FORBIDDEN"
    );
    assert_eq!(
        code(e.execute(
            &program("0.21.0", json!([write(all.clone())])),
            &host("alice")
        )),
        "E_VERSION"
    );
    run(&mut e, "alice", json!([write(all)])).unwrap();
    assert!(query(&mut e, "alice", json!({"graph_id": "g"}))
        .graph
        .nodes
        .is_empty());
}

#[test]
fn fact_writes_build_on_snapshot_commits_and_snapshots_on_facts() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = engine(&clock);
    let data = json!({"nodes": [{"id": "a", "entity_id": "A", "space_id": "s"}], "edges": []});
    let s1 = match &run(
        &mut e,
        "alice",
        json!([{"op": "commit", "graph_id": "g", "data": data}]),
    )
    .unwrap()[0]
    {
        CommandResult::Committed { revision, .. } => revision.clone(),
        other => panic!("{other:?}"),
    };
    clock.set(20);
    let f1 = committed(
        &run(
            &mut e,
            "bob",
            json!([write(json!([
                assert_(node("b")),
                assert_(edge("e", "a", "b"))
            ]))]),
        )
        .unwrap(),
    );
    let head = query(&mut e, "alice", json!({"graph_id": "g"}));
    assert_eq!(ids(&head.graph.nodes, |n| &n.id), ["a", "b"]);
    // Provenance: `a` came from the snapshot commit, the rest from the fact write.
    let writer = |result: &QueryResult, id: &str| {
        result
            .fact_provenance
            .iter()
            .find(|p| {
                serde_json::to_value(&p.fact)
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .values()
                    .any(|v| v == id)
            })
            .map(|p| (p.provenance.writer.clone(), p.provenance.recorded_at_ms))
    };
    assert_eq!(writer(&head, "a"), Some(("alice".into(), 10)));
    assert_eq!(writer(&head, "b"), Some(("bob".into(), 20)));
    // A snapshot commit on top of a fact revision stays a snapshot revision.
    clock.set(30);
    let mut full = serde_json::to_value(&head.graph).unwrap();
    full["nodes"][0]["properties"] = json!({"x": 1});
    let commit = json!({"op": "commit", "graph_id": "g", "expected_head": f1, "data": full});
    let s2 = match &run(&mut e, "carol", json!([commit])).unwrap()[0] {
        CommandResult::Committed { revision, .. } => revision.clone(),
        other => panic!("{other:?}"),
    };
    assert!(s2.starts_with("sha256:") && s1.starts_with("sha256:"));
    let after = query(&mut e, "alice", json!({"graph_id": "g"}));
    assert_eq!(
        writer(&after, "b"),
        Some(("carol".into(), 30)),
        "a snapshot restates every fact"
    );
}

#[test]
fn every_returned_fact_has_engine_recorded_provenance() {
    let clock = Arc::new(ManualClock::new(1_000));
    let mut e = engine(&clock);
    let mut a = node("a");
    a["provenance"] =
        json!({"source": "erp", "method": {"kind": "import", "importer": "csv", "version": "1"}});
    let mut x = edge("x", "a", "a");
    x["provenance"] = json!({"method": {"kind": "model", "model": "extractor", "version": "7"}, "evidence": [{"kind": "artifact", "uri": "s3://doc", "digest": "sha256:00"}]});
    let attachment = json!({"kind": "attachment", "id": "fw", "host": {"kind": "node", "id": "a"}, "key": "firmware", "value": {"kind": "literal", "value": "1.0"}, "valid_time": {"start": 0}});
    run(
        &mut e,
        "agent-7",
        json!([write(json!([assert_(a), assert_(x), assert_(attachment)]))]),
    )
    .unwrap();
    let result = query(&mut e, "reader", json!({"graph_id": "g"}));
    assert_eq!(result.fact_provenance.len(), 3);
    for entry in &result.fact_provenance {
        assert_eq!(entry.provenance.writer, "agent-7");
        assert_eq!(entry.provenance.recorded_at_ms, 1_000);
    }
    let by_kind: Vec<Value> = result
        .fact_provenance
        .iter()
        .map(|p| serde_json::to_value(&p.provenance).unwrap())
        .collect();
    assert!(by_kind
        .iter()
        .any(|p| p["source"] == "erp" && p["method"]["kind"] == "import"));
    assert!(by_kind
        .iter()
        .any(|p| p["method"]["kind"] == "model" && p["evidence"][0]["uri"] == "s3://doc"));
    assert!(by_kind.iter().any(|p| p["method"]["kind"] == "unspecified"));
    assert_eq!(result.coverage, weave_contract::Coverage::Complete);
    // Older protocol versions keep their exact result shape: no provenance list.
    let legacy = e
        .execute(
            &program(
                "0.21.0",
                json!([{"op": "query", "query": {"graph_id": "g"}}]),
            ),
            &host("reader"),
        )
        .unwrap();
    let encoded = serde_json::to_value(&legacy).unwrap();
    assert!(encoded[0]["result"].get("fact_provenance").is_none());
}

#[test]
fn a_client_cannot_supply_writer_or_recorded_time() {
    for field in [json!({"writer": "mallory"}), json!({"recorded_at_ms": 1})] {
        let mut claim = json!({"method": {"kind": "human"}});
        claim
            .as_object_mut()
            .unwrap()
            .extend(field.as_object().unwrap().clone());
        let mut fact = node("a");
        fact["provenance"] = claim;
        let encoded = json!({"version": VERSION, "commands": [write(json!([assert_(fact)]))]});
        let error = weave_contract::decode_program(encoded.to_string().as_bytes()).unwrap_err();
        assert_eq!(error.code, "E_UNKNOWN_FIELD");
    }
    // A source claim must agree with an assertion's legacy source label.
    let clock = Arc::new(ManualClock::new(1));
    let mut e = engine(&clock);
    let mut x = edge("x", "a", "a");
    x["assertion_source"] = json!("sensor");
    x["provenance"] = json!({"source": "other", "method": {"kind": "human"}});
    assert_eq!(
        code(run(
            &mut e,
            "alice",
            json!([write(json!([assert_(node("a")), assert_(x)]))])
        )),
        "E_PROVENANCE"
    );
}

#[test]
fn hidden_evidence_is_removed_without_changing_visible_output() {
    let build = |hidden: bool| {
        let clock = Arc::new(ManualClock::new(50));
        let mut e = engine(&clock);
        let secret = json!({"kind": "node", "id": "secret", "entity_id": "S", "space_id": "s", "readers": ["bob"]});
        let r1 = committed(&run(&mut e, "bob", json!([write(json!([assert_(secret)]))])).unwrap());
        let mut public = node("a");
        let mut evidence = vec![json!({"kind": "artifact", "uri": "u", "digest": "d"})];
        if hidden {
            evidence.push(
                json!({"kind": "node", "graph_id": "g", "revision": r1, "node_id": "secret"}),
            );
        }
        public["provenance"] = json!({"method": {"kind": "human"}, "evidence": evidence});
        run(&mut e, "bob", json!([write(json!([assert_(public)]))])).unwrap();
        e
    };
    let mut with_hidden = build(true);
    let mut without = build(false);
    let alice = query(&mut with_hidden, "alice", json!({"graph_id": "g"}));
    let bob = query(&mut with_hidden, "bob", json!({"graph_id": "g"}));
    let bob_evidence = &bob.fact_provenance.iter().find(|p| matches!(&p.fact, weave_contract::provenance::FactRef::Node { node_id, .. } if node_id == "a")).unwrap().provenance.evidence;
    assert_eq!(
        bob_evidence.len(),
        2,
        "a permitted reader sees all evidence"
    );
    assert_eq!(alice.coverage, weave_contract::Coverage::Partial);
    assert!(alice
        .diagnostics
        .iter()
        .any(|d| d.code == "E_EVIDENCE_UNAVAILABLE"));
    // Apart from the coverage flag, alice's output is what she would see if the
    // hidden fact were absent: same records, same visible evidence.
    let plain = query(&mut without, "alice", json!({"graph_id": "g"}));
    assert_eq!(alice.graph.nodes.len(), plain.graph.nodes.len());
    let evidence = |r: &QueryResult| -> Vec<Value> {
        r.fact_provenance
            .iter()
            .map(|p| serde_json::to_value(&p.provenance.evidence).unwrap())
            .collect()
    };
    assert_eq!(evidence(&alice), evidence(&plain));
    assert!(alice.graph.nodes.iter().all(|n| n.id != "secret"));
}

#[test]
fn node_valid_time_selects_installed_units() {
    let clock = Arc::new(ManualClock::new(1));
    let mut e = engine(&clock);
    let site = node("site");
    let unit = json!({"kind": "node", "id": "unit", "entity_id": "UNIT", "space_id": "s", "valid_time": {"start": 10, "end": 50}});
    let located = json!({"kind": "edge", "id": "at", "predicate": "located_at", "from": "unit", "to": "site", "valid_time": {"start": 0}});
    let spare = json!({"kind": "node", "id": "spare", "entity_id": "SPARE", "space_id": "s", "valid_time": {"start": 10, "end": 50}});
    run(
        &mut e,
        "alice",
        json!([write(json!([
            assert_(site),
            assert_(unit),
            assert_(located),
            assert_(spare)
        ]))]),
    )
    .unwrap();
    let at = |e: &mut Engine, t: i64| {
        let result = query(e, "alice", json!({"graph_id": "g", "valid_at": t}));
        (
            ids(&result.graph.nodes, |n| &n.id),
            ids(&result.graph.edges, |e| &e.id),
        )
    };
    for t in [5, 60] {
        let (nodes, edges) = at(&mut e, t);
        assert!(!nodes.contains(&"unit".to_string()), "unit absent at {t}");
        assert!(edges.is_empty(), "no edge needs an absent unit at {t}");
    }
    let (nodes, edges) = at(&mut e, 20);
    assert_eq!(
        nodes,
        ["site", "spare", "unit"],
        "an isolated node with explicit valid time is kept"
    );
    assert_eq!(edges, ["at"]);
    // A filter expression applies the same rule.
    let filtered = |e: &mut Engine, t: i64| {
        let results = run(e, "alice", json!([{"op": "evaluate", "value": {"kind": "filter", "valid_at": t, "input": {"kind": "query", "query": {"graph_id": "g"}}}}])).unwrap();
        match &results[0] {
            CommandResult::Queried { result } => ids(&result.graph.nodes, |n| &n.id),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(filtered(&mut e, 20), ["site", "spare", "unit"]);
    assert!(!filtered(&mut e, 60).contains(&"unit".to_string()));
    // Nodes without a valid time behave exactly as before.
    let all = query(&mut e, "alice", json!({"graph_id": "g"}));
    assert_eq!(ids(&all.graph.nodes, |n| &n.id), ["site", "spare", "unit"]);
}

/// Late-arriving correction: recorded time K and valid time V are independent.
#[test]
fn late_correction_four_quadrants() {
    let clock = Arc::new(ManualClock::new(1_000));
    let mut e = engine(&clock);
    let unit = json!({"kind": "node", "id": "unit", "entity_id": "UNIT", "space_id": "s", "valid_time": {"start": 0}});
    let firmware = |value: &str, start: i64, end: Option<i64>| json!({"kind": "attachment", "id": format!("fw-{value}"), "host": {"kind": "node", "id": "unit"}, "key": "firmware", "value": {"kind": "literal", "value": value}, "valid_time": {"start": start, "end": end}});
    // K1 = 1000: we believe firmware 1 is installed from day 0 onwards.
    run(
        &mut e,
        "alice",
        json!([write(json!([
            assert_(unit),
            assert_(firmware("1", 0, None))
        ]))]),
    )
    .unwrap();
    // K2 = 2000: we learn late that firmware 2 replaced it on day 30.
    clock.set(2_000);
    run(
        &mut e,
        "alice",
        json!([write(json!([
            {"change": "correct", "fact": firmware("1", 0, Some(30))},
            assert_(firmware("2", 30, None))
        ]))]),
    )
    .unwrap();
    clock.set(3_000);
    let known = |e: &mut Engine, k: i64, v: i64| -> Vec<String> {
        let value = json!({"kind": "recorded_query", "query": {"graph_id": "g", "valid_at": v}, "selection": {"kind": "local_time", "unix_millis": k}});
        let results = run(e, "alice", json!([{"op": "evaluate", "value": value}])).unwrap();
        match &results[0] {
            CommandResult::Queried { result } => {
                let mut values: Vec<String> = result
                    .graph
                    .attachments
                    .iter()
                    .map(|a| a.id.clone())
                    .collect();
                values.sort();
                values
            }
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(
        known(&mut e, 1_500, 20),
        ["fw-1"],
        "known at K1, valid on day 20"
    );
    assert_eq!(
        known(&mut e, 1_500, 40),
        ["fw-1"],
        "known at K1, valid on day 40: not yet learned"
    );
    assert_eq!(
        known(&mut e, 2_500, 20),
        ["fw-1"],
        "known at K2, valid on day 20"
    );
    assert_eq!(
        known(&mut e, 2_500, 40),
        ["fw-2"],
        "known at K2, valid on day 40: corrected"
    );
}

#[test]
fn checkpoints_are_derived_and_droppable() {
    let clock = Arc::new(ManualClock::new(1));
    let mut e = engine(&clock);
    let r = committed(
        &run(
            &mut e,
            "alice",
            json!([write(json!([
                assert_(node("a")),
                assert_(node("b")),
                assert_(edge("e", "a", "b"))
            ]))]),
        )
        .unwrap(),
    );
    let before = serde_json::to_string(&query(&mut e, "alice", json!({"graph_id": "g"}))).unwrap();
    assert!(e.checkpoint_fact_revision("g", &r, &host("alice")).unwrap());
    let cached = serde_json::to_string(&query(&mut e, "alice", json!({"graph_id": "g"}))).unwrap();
    assert!(e.drop_fact_checkpoint(&r, &host("alice")).unwrap());
    let dropped = serde_json::to_string(&query(&mut e, "alice", json!({"graph_id": "g"}))).unwrap();
    assert_eq!(before, cached);
    assert_eq!(before, dropped);
    assert_eq!(
        e.checkpoint_fact_revision("g", &r, &HostContext::new("x", []))
            .unwrap_err()
            .code,
        "E_FORBIDDEN"
    );
}

#[test]
fn readers_restrict_fact_written_records_like_snapshot_records() {
    let clock = Arc::new(ManualClock::new(1));
    let mut e = engine(&clock);
    let hidden =
        json!({"kind": "node", "id": "h", "entity_id": "H", "space_id": "s", "readers": ["bob"]});
    run(
        &mut e,
        "bob",
        json!([write(json!([
            assert_(node("a")),
            assert_(hidden),
            assert_(edge("e", "a", "h"))
        ]))]),
    )
    .unwrap();
    let alice = query(&mut e, "alice", json!({"graph_id": "g"}));
    assert_eq!(ids(&alice.graph.nodes, |n| &n.id), ["a"]);
    assert!(
        alice.graph.edges.is_empty(),
        "an edge to a hidden node is hidden"
    );
    assert!(alice
        .fact_provenance
        .iter()
        .all(|p| !serde_json::to_string(&p.fact).unwrap().contains("\"h\"")));
    let bob = query(&mut e, "bob", json!({"graph_id": "g"}));
    assert_eq!(ids(&bob.graph.nodes, |n| &n.id), ["a", "h"]);
    assert_eq!(bob.graph.edges.len(), 1);
}

/// ADR-0007 section 13, item 1: `reader_groups` is reserved on every fact
/// kind, and records without it keep their exact bytes and revision IDs.
#[test]
fn reader_groups_stay_reserved_and_absent_from_canonical_bytes() {
    let attachment = json!({"kind": "attachment", "id": "m", "host": {"kind": "graph"}, "key": "k", "value": {"kind": "literal", "value": 1}, "valid_time": {"start": 0}});
    let structural =
        json!({"kind": "structural_edge", "id": "s", "predicate": "p", "from": "a", "to": "a"});
    let assertion = json!({"kind": "assertion", "id": "c", "edge_id": "s", "source": "x", "valid_time": {"start": 0}});
    for fact in [
        node("a"),
        edge("e", "a", "a"),
        structural,
        assertion,
        attachment,
    ] {
        let mut grouped = fact.clone();
        grouped["reader_groups"] = json!(["team"]);
        let encoded = json!({"version": VERSION, "commands": [write(json!([assert_(grouped)]))]});
        let error = weave_contract::decode_program(encoded.to_string().as_bytes()).unwrap_err();
        assert_eq!(error.code, "E_UNKNOWN_FIELD", "{fact}");
        let plain: weave_contract::Fact = serde_json::from_value(fact.clone()).unwrap();
        assert!(!serde_json::to_string(&plain)
            .unwrap()
            .contains("reader_groups"));
    }
    // The golden fact revision ID of the core-v1 fixture does not depend on B1.
    let mut e = engine(&Arc::new(ManualClock::new(100)));
    let r = committed(
        &run(
            &mut e,
            "alice",
            json!([write(json!([
                assert_(node("a")),
                assert_(node("b")),
                assert_(edge("e", "a", "b"))
            ]))]),
        )
        .unwrap(),
    );
    assert_eq!(
        r,
        "fact:sha256:000aac150911f80a1e3f5e1cf0b8cac3c5682eca7bcf6aebac903280564afa22"
    );
}

/// ADR-0007 section 13, item 3: the stored writer is the host principal
/// string, and the reserved delegation column stays empty and outside content.
#[test]
fn stored_writer_is_the_host_principal_with_reserved_delegation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("facts.db");
    let clock = Arc::new(ManualClock::new(7));
    let mut e = Engine::open_with_clock(&path, clock).unwrap();
    run(
        &mut e,
        "agent:planner-1",
        json!([write(json!([assert_(node("a"))]))]),
    )
    .unwrap();
    drop(e);
    let db = rusqlite::Connection::open(&path).unwrap();
    for table in ["fact_log", "fact_revisions"] {
        let rows: Vec<(String, Option<String>)> = db
            .prepare(&format!("SELECT writer,on_behalf_of FROM {table}"))
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, [("agent:planner-1".to_string(), None)], "{table}");
    }
}

/// ADR-0007 section 13, item 5: fact writes to engine-reserved graphs fail.
#[test]
fn fact_writes_to_weave_graphs_are_rejected() {
    let mut e = engine(&Arc::new(ManualClock::new(1)));
    for graph in ["weave:registry", "weave:identity:x", "weave:anything"] {
        let host = HostContext::new("alice", [graph.to_string()]);
        let command = json!({"op": "write", "graph_id": graph, "changes": [assert_(node("a"))]});
        let error = e
            .execute(&program(VERSION, json!([command])), &host)
            .unwrap_err();
        assert_eq!(error.code, "E_RESERVED_NAMESPACE", "{graph}");
    }
    assert_eq!(e.event_count().unwrap(), 0);
}
