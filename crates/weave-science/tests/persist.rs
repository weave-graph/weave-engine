//! B2 part 3 (#5, ADR-0008 section 5): persisted analytics and rankings are
//! no broader than their inputs.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::VERSION;
use weave_engine::{Engine, HostContext, ManualClock};
use weave_science::{Request, ScienceSession};

fn request(value: Value) -> Request {
    serde_json::from_value(value).unwrap()
}
fn session(principal: &str, engine: Engine) -> ScienceSession {
    ScienceSession::new(engine, HostContext::new(principal, ["g".to_string()]))
}

/// A store as alice can see it: two restricted nodes and a restricted edge.
fn store() -> Engine {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap();
    let node = |id: &str, readers: Value, vector: Value| {
        json!({"change": "assert", "fact": {"kind": "node", "id": id, "entity_id": id.to_uppercase(),
            "space_id": "s", "readers": readers, "properties": {"vector": vector}}})
    };
    let edge = |id: &str, from: &str, to: &str, readers: Value| {
        json!({"change": "assert", "fact": {"kind": "edge", "id": id, "predicate": "p", "from": from,
            "to": to, "valid_time": {"start": 0}, "readers": readers}})
    };
    let program: weave_contract::Program = serde_json::from_value(json!({"version": VERSION, "commands": [
        {"op": "write", "graph_id": "g", "changes": [
            {"change": "assert", "fact": {"kind": "embedding_space", "id": "sp", "revision": "1",
                "encoder": "e", "encoder_version": "v", "preprocessing": "p", "dimensions": 2,
                "metric": "euclidean"}},
            node("a", json!([]), json!([1.0, 0.0])),
            node("b", json!(["alice", "bob"]), json!([0.0, 1.0])),
            node("c", json!(["alice"]), json!([1.0, 1.0])),
            edge("ab", "a", "b", json!([])),
            edge("bc", "b", "c", json!(["alice", "carol"])),
            {"change": "assert", "fact": {"kind": "embedding", "id": "va", "space": {"id": "sp", "revision": "1"},
                "subject": {"kind": "node", "node_id": "a"}, "vector": [1.0, 0.0], "valid_time": {"start": 0},
                "readers": ["alice", "bob"]}},
            {"change": "assert", "fact": {"kind": "embedding", "id": "vb", "space": {"id": "sp", "revision": "1"},
                "subject": {"kind": "node", "node_id": "b"}, "vector": [0.0, 1.0], "valid_time": {"start": 0},
                "readers": ["alice", "bob"]}}
        ]}]}))
    .unwrap();
    e.execute(&program, &HostContext::new("alice", ["g".to_string()]))
        .unwrap();
    e
}

fn query() -> Value {
    json!({"version": VERSION, "commands": [{"op": "query", "query": {"graph_id": "g"}}]})
}

/// The persisted attachments a principal can read, by node.
fn persisted(engine: Engine, principal: &str, key: &str) -> Vec<(String, Value)> {
    let mut s = ScienceSession::new(engine, HostContext::new(principal, Vec::<String>::new()));
    let out = s
        .handle(request(
            json!({"operation": "execute", "program": {"version": VERSION,
            "commands": [{"op": "query", "query": {"graph_id": "g", "include_metadata": true}}]}}),
        ))
        .unwrap();
    let mut found: Vec<(String, Value)> = out["results"][0]["result"]["graph"]["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["key"] == key)
        .map(|a| (a["host"]["id"].as_str().unwrap().to_string(), a.clone()))
        .collect();
    found.sort_by(|x, y| x.0.cmp(&y.0));
    found
}

fn readers(record: &Value) -> (Vec<String>, Vec<String>) {
    let list = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .map(|s| s.as_str().unwrap().to_string())
            .collect()
    };
    (list(&record["readers"]), list(&record["reader_groups"]))
}

#[test]
fn a_persisted_analysis_is_readable_only_by_readers_of_every_input() {
    let mut s = session("alice", store());
    let out = s
        .handle(request(json!({"operation": "persist", "program": query(),
            "analysis": {"algorithm": "pagerank"}, "graph_id": "g", "key": "pagerank"})))
        .unwrap();
    assert_eq!(out["persisted"], 3, "{out}");
    let engine = s.into_engine();
    let alice = persisted(engine, "alice", "pagerank");
    assert_eq!(alice.len(), 3);
    for (_, record) in &alice {
        // The own clause lies within every restricted input clause...
        let (r, g) = readers(record);
        assert!(g.is_empty() && !r.is_empty());
        assert!(
            r.iter().all(|p| ["alice", "bob"].contains(&p.as_str())),
            "{record}"
        );
        // ...and every input is carried.
        assert_eq!(record["derived_nodes"].as_array().unwrap().len(), 3);
        assert_eq!(record["derived_from"].as_array().unwrap().len(), 2);
        assert_eq!(
            record["provenance"]["method"]["rule_id"],
            "weave-science:pagerank"
        );
    }
    // bob reads b but not c or the edge bc; carol reads bc but not b or c.
    for who in ["bob", "carol", "dave"] {
        let mut s = session("alice", store());
        s.handle(request(json!({"operation": "persist", "program": query(),
            "analysis": {"algorithm": "pagerank"}, "graph_id": "g", "key": "pagerank"})))
            .unwrap();
        assert!(
            persisted(s.into_engine(), who, "pagerank").is_empty(),
            "{who}"
        );
    }
}

#[test]
fn a_persisted_ranking_is_no_broader_than_its_embeddings_and_subjects() {
    let mut s = session("alice", store());
    let nearest = json!({"version": VERSION, "commands": [{"op": "evaluate", "value": {"kind": "nearest",
        "graph_id": "g", "space": {"id": "sp", "revision": "1"},
        "query": {"kind": "vector", "vector": [1.0, 0.1]}, "k": 2}}]});
    let out = s
        .handle(request(
            json!({"operation": "persist", "program": nearest, "graph_id": "g", "key": "near"}),
        ))
        .unwrap();
    assert_eq!(out["persisted"], 2, "{out}");
    let engine = s.into_engine();
    let alice = persisted(engine, "alice", "near");
    assert_eq!(
        alice.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    for (_, record) in &alice {
        let (r, _) = readers(record);
        // Within the embeddings' clause [alice, bob] and b's clause.
        assert!(
            !r.is_empty() && r.iter().all(|p| ["alice", "bob"].contains(&p.as_str())),
            "{record}"
        );
        let evidence = record["provenance"]["evidence"].as_array().unwrap();
        assert_eq!(evidence.len(), 2);
        assert!(evidence.iter().all(|e| e["kind"] == "embedding"));
    }
    let mut s = session("alice", store());
    let nearest = json!({"version": VERSION, "commands": [{"op": "evaluate", "value": {"kind": "nearest",
        "graph_id": "g", "space": {"id": "sp", "revision": "1"},
        "query": {"kind": "vector", "vector": [1.0, 0.1]}, "k": 2}}]});
    s.handle(request(
        json!({"operation": "persist", "program": nearest, "graph_id": "g", "key": "near"}),
    ))
    .unwrap();
    assert!(persisted(s.into_engine(), "carol", "near").is_empty());
}

#[test]
fn persist_accepts_only_engine_computed_values_from_read_only_programs() {
    let mut s = session("alice", store());
    let write = json!({"version": VERSION, "commands": [{"op": "write", "graph_id": "g", "changes": [
        {"change": "assert", "fact": {"kind": "node", "id": "z", "entity_id": "Z", "space_id": "s"}}]}]});
    let error = s
        .handle(request(json!({"operation": "persist", "program": write,
            "analysis": {"algorithm": "degree"}, "graph_id": "g", "key": "degree"})))
        .unwrap_err();
    assert_eq!(error.code, "E_SCIENCE_READ_ONLY");
    let error = s
        .handle(request(
            json!({"operation": "persist", "program": query(), "graph_id": "g", "key": "x"}),
        ))
        .unwrap_err();
    assert_eq!(error.code, "E_SCIENCE_RESULT");
}
