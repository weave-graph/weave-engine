//! A4 operators (protocol 0.24.0, ADR-0006): embedding facts, exact
//! `nearest`, composition with `expand` and `similar_candidates`.
//!
//! Every ranking check compares the engine with an independent oracle in
//! this file: an exhaustive scan of the visible model that computes the
//! normative distance (core v1 `embedding.space`) and sorts by
//! (score, subject, embedding ID). The paired tests build two stores that
//! differ only in facts the reader cannot see, including exact duplicates of
//! visible vectors, and require identical output after normalizing revision
//! IDs (content-derived over every fact, ADR-0001 and ADR-0004).
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use weave_contract::embedding::{EmbeddingMetric, EmbeddingSpace};
use weave_contract::provenance::{FactRef, Method};
use weave_contract::{CommandResult, Interval, Program, QueryResult, VERSION};
use weave_engine::{Engine, HostContext, ManualClock};

struct Rng(u64);
impl Rng {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % bound
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.next(items.len() as u64) as usize]
    }
}

fn writer() -> HostContext {
    HostContext::new("writer", ["g", "h"].map(String::from))
}
fn reader(principal: &str) -> HostContext {
    HostContext::new(principal, Vec::<String>::new())
}
fn run_as(
    e: &mut Engine,
    version: &str,
    host: &HostContext,
    commands: Value,
) -> weave_engine::Result<Vec<CommandResult>> {
    let program: Program =
        serde_json::from_value(json!({"version": version, "commands": commands})).unwrap();
    e.execute(&program, host)
}
fn run(
    e: &mut Engine,
    host: &HostContext,
    commands: Value,
) -> weave_engine::Result<Vec<CommandResult>> {
    run_as(e, VERSION, host, commands)
}
fn write(e: &mut Engine, changes: Vec<Value>) -> weave_engine::Result<String> {
    write_to(e, "g", changes)
}
fn write_to(e: &mut Engine, graph: &str, changes: Vec<Value>) -> weave_engine::Result<String> {
    run(
        e,
        &writer(),
        json!([{"op": "write", "graph_id": graph, "changes": changes}]),
    )
    .map(|r| match &r[0] {
        CommandResult::Committed { revision, .. } | CommandResult::Unchanged { revision } => {
            revision.clone()
        }
        other => panic!("{other:?}"),
    })
}
fn evaluate(e: &mut Engine, principal: &str, value: Value) -> weave_engine::Result<QueryResult> {
    run(
        e,
        &reader(principal),
        json!([{"op": "evaluate", "value": value}]),
    )
    .map(|mut r| match r.remove(0) {
        CommandResult::Queried { result } => *result,
        other => panic!("{other:?}"),
    })
}
fn assert(fact: Value) -> Value {
    json!({"change": "assert", "fact": fact})
}
fn space(id: &str, dims: u32, metric: &str) -> Value {
    json!({"kind": "embedding_space", "id": id, "revision": "1", "encoder": "test-encoder",
        "encoder_version": "v1", "preprocessing": "none/v1", "dimensions": dims, "metric": metric})
}
fn node(id: &str, readers: Value) -> Value {
    json!({"kind": "node", "id": id, "entity_id": id.to_uppercase(), "space_id": "s", "readers": readers})
}
fn embedding(
    id: &str,
    space: &str,
    subject: Value,
    vector: &[f32],
    valid: Value,
    readers: Value,
) -> Value {
    json!({"kind": "embedding", "id": id, "space": {"id": space, "revision": "1"}, "subject": subject,
        "vector": vector, "valid_time": valid, "readers": readers,
        "provenance": {"method": {"kind": "model", "model": "test-encoder", "version": "v1"}}})
}
fn on_node(id: &str) -> Value {
    json!({"kind": "node", "node_id": id})
}
fn always() -> Value {
    json!({"start": 0})
}
fn nearest(space: &str, vector: &[f32], k: u32) -> Value {
    json!({"kind": "nearest", "graph_id": "g", "space": {"id": space, "revision": "1"},
        "query": {"kind": "vector", "vector": vector}, "k": k})
}
fn hits(result: &QueryResult) -> Vec<(String, f64)> {
    result
        .neighbors
        .iter()
        .map(|n| (n.embedding_id.clone(), n.score))
        .collect()
}
fn admits(readers: &Value, principal: &str) -> bool {
    let readers = readers.as_array().cloned().unwrap_or_default();
    readers.is_empty() || readers.iter().any(|r| r == principal)
}

/// Normalize revision IDs, which differ whenever any fact differs.
fn normal(result: &weave_engine::Result<QueryResult>) -> String {
    let text = match result {
        Ok(result) => serde_json::to_string(result).unwrap(),
        Err(error) => return format!("error {} {}", error.code, error.message),
    };
    let mut value: Value = serde_json::from_str(&text).unwrap();
    fn scrub(value: &mut Value) {
        match value {
            Value::String(s) if s.starts_with("fact:sha256:") || s.starts_with("sha256:") => {
                *s = "REV".into()
            }
            Value::Array(items) => items.iter_mut().for_each(scrub),
            Value::Object(map) => {
                let keys: Vec<String> = map.keys().cloned().collect();
                let mut renamed = serde_json::Map::new();
                for key in keys {
                    let mut item = map.remove(&key).unwrap();
                    scrub(&mut item);
                    let key = if key.starts_with("fact:sha256:") || key.starts_with("sha256:") {
                        "REV".to_string()
                    } else {
                        key
                    };
                    renamed.insert(key, item);
                }
                *map = renamed;
            }
            _ => {}
        }
    }
    scrub(&mut value);
    value.to_string()
}

// ------------------------------------------------------------ the oracle

/// The normative distance, written independently of the engine.
fn oracle_distance(metric: &str, a: &[f32], b: &[f32]) -> f64 {
    let (mut dot, mut na, mut nb, mut sq) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for i in 0..a.len() {
        let (x, y) = (a[i] as f64, b[i] as f64);
        dot += x * y;
        na += x * x;
        nb += y * y;
        sq += (x - y) * (x - y);
    }
    match metric {
        "euclidean" => sq.sqrt(),
        _ => {
            let d = 1.0 - dot / (na.sqrt() * nb.sqrt());
            d.clamp(0.0, 2.0)
        }
    }
}

#[derive(Clone)]
struct Vector {
    id: String,
    subject_kind: u8, // 0 entity, 1 node: the subject order of rankings
    subject: String,
    vector: Vec<f32>,
    valid: (i64, Option<i64>),
    readers: Value,
}

struct Model {
    metric: &'static str,
    /// node ID -> readers
    nodes: BTreeMap<String, Value>,
    vectors: Vec<Vector>,
}

impl Model {
    fn visible_subject(&self, v: &Vector, principal: &str) -> bool {
        match v.subject_kind {
            1 => self
                .nodes
                .get(&v.subject)
                .is_some_and(|r| admits(r, principal)),
            _ => self
                .nodes
                .iter()
                .any(|(id, r)| id.to_uppercase() == v.subject && admits(r, principal)),
        }
    }
    fn nearest(
        &self,
        principal: &str,
        query: &[f32],
        k: usize,
        valid_at: Option<i64>,
    ) -> Vec<(String, f64)> {
        let mut scored: Vec<(f64, u8, String, String)> = self
            .vectors
            .iter()
            .filter(|v| admits(&v.readers, principal) && self.visible_subject(v, principal))
            .filter(|v| valid_at.is_none_or(|t| v.valid.0 <= t && v.valid.1.is_none_or(|e| t < e)))
            .map(|v| {
                (
                    oracle_distance(self.metric, query, &v.vector),
                    v.subject_kind,
                    v.subject.clone(),
                    v.id.clone(),
                )
            })
            .collect();
        scored.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        scored
            .into_iter()
            .take(k)
            .map(|(s, _, _, id)| (id, s))
            .collect()
    }
}

/// A random vector, or an adversarial copy of an earlier one: an exact
/// duplicate, a positive multiple (a cosine tie), or a one-ulp neighbor.
fn adversarial(rng: &mut Rng, dims: usize, earlier: &[Vector], metric: &str) -> Vec<f32> {
    let fresh = |rng: &mut Rng| -> Vec<f32> {
        (0..dims)
            .map(|_| (rng.next(9) as f32 - 4.0) / 2.0)
            .collect()
    };
    let mut v = if earlier.is_empty() || rng.next(3) == 0 {
        fresh(rng)
    } else {
        let base = rng.pick(earlier).vector.clone();
        match rng.next(4) {
            0 => base,
            1 if metric == "cosine_distance" => base.iter().map(|x| x * 4.0).collect(),
            2 => {
                let mut near = base.clone();
                let i = rng.next(dims as u64) as usize;
                near[i] = f32::from_bits(near[i].to_bits() + 1);
                near
            }
            _ => fresh(rng),
        }
    };
    if v.iter().all(|x| *x == 0.0) {
        v[0] = 1.0;
    }
    v
}

fn random_store(seed: u64, metric: &'static str) -> (Engine, Model) {
    let mut rng = Rng(seed * 7 + 3);
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(5))).unwrap();
    let dims = 1 + rng.next(5) as usize;
    let mut changes = vec![assert(space("sp", dims as u32, metric))];
    let mut model = Model {
        metric,
        nodes: BTreeMap::new(),
        vectors: vec![],
    };
    for i in 0..12 {
        let id = format!("n{i:02}");
        let readers = rng
            .pick(&[json!([]), json!([]), json!(["alice"]), json!(["bob"])])
            .clone();
        changes.push(assert(node(&id, readers.clone())));
        model.nodes.insert(id, readers);
    }
    for i in 0..60 {
        let vector = adversarial(&mut rng, dims, &model.vectors, metric);
        let entity = rng.next(6) == 0;
        let n = format!("n{:02}", rng.next(12));
        let (kind, subject, subject_json) = if entity {
            let entity_id = n.to_uppercase();
            (
                0,
                entity_id.clone(),
                json!({"kind": "entity", "entity_id": entity_id}),
            )
        } else {
            (1, n.clone(), on_node(&n))
        };
        let start = rng.next(20) as i64;
        let end = (rng.next(2) == 0).then_some(start + 1 + rng.next(20) as i64);
        let readers = rng
            .pick(&[
                json!([]),
                json!([]),
                json!(["alice"]),
                json!(["bob"]),
                json!(["alice", "bob"]),
            ])
            .clone();
        let id = format!("v{i:02}");
        changes.push(assert(embedding(
            &id,
            "sp",
            subject_json,
            &vector,
            json!({"start": start, "end": end}),
            readers.clone(),
        )));
        model.vectors.push(Vector {
            id,
            subject_kind: kind,
            subject,
            vector,
            valid: (start, end),
            readers,
        });
    }
    write(&mut e, changes).unwrap();
    (e, model)
}

#[test]
fn exact_nearest_equals_an_exhaustive_oracle_with_ties_and_near_duplicates() {
    let mut compared = 0;
    let mut ties = 0;
    for seed in 0..24 {
        let metric = if seed % 2 == 0 {
            "cosine_distance"
        } else {
            "euclidean"
        };
        let (mut e, model) = random_store(seed, metric);
        let mut rng = Rng(seed + 1000);
        let dims = model.vectors[0].vector.len();
        for _ in 0..6 {
            let query = adversarial(&mut rng, dims, &model.vectors, metric);
            for k in [1, 3, 10, 100] {
                for valid_at in [None, Some(5), Some(18)] {
                    for principal in ["alice", "bob", "carol"] {
                        let mut plan = nearest("sp", &query, k);
                        if let Some(t) = valid_at {
                            plan["valid_at"] = json!(t);
                        }
                        let got = evaluate(&mut e, principal, plan).unwrap();
                        let want = model.nearest(principal, &query, k as usize, valid_at);
                        assert_eq!(
                            hits(&got),
                            want,
                            "seed {seed} k {k} at {valid_at:?} as {principal}"
                        );
                        assert_eq!(
                            got.exactness,
                            Some(weave_contract::embedding::Exactness::Exact)
                        );
                        assert!(got
                            .neighbors
                            .iter()
                            .enumerate()
                            .all(|(i, n)| n.rank as usize == i + 1));
                        ties += want.windows(2).filter(|w| w[0].1 == w[1].1).count();
                        compared += 1;
                    }
                }
            }
        }
    }
    assert!(compared > 1000);
    println!("{compared} rankings, {ties} adjacent ties");
    assert!(ties > 100, "adversarial ties are exercised ({ties})");
}

#[test]
fn embeddings_are_selected_by_valid_and_recorded_time() {
    let clock = Arc::new(ManualClock::new(100));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    write(
        &mut e,
        vec![
            assert(space("sp", 2, "euclidean")),
            assert(node("a", json!([]))),
            assert(node("b", json!([]))),
            assert(embedding(
                "va",
                "sp",
                on_node("a"),
                &[1.0, 0.0],
                json!({"start": 10, "end": 20}),
                json!([]),
            )),
        ],
    )
    .unwrap();
    let at = |t: i64| {
        let mut plan = nearest("sp", &[1.0, 0.0], 5);
        plan["valid_at"] = json!(t);
        plan
    };
    let returned = |e: &mut Engine, plan: Value| -> Vec<String> {
        evaluate(e, "alice", plan)
            .unwrap()
            .neighbors
            .into_iter()
            .map(|n| n.embedding_id)
            .collect()
    };
    assert_eq!(returned(&mut e, at(15)), ["va"]);
    assert!(returned(&mut e, at(25)).is_empty());
    assert!(returned(&mut e, at(9)).is_empty());
    assert_eq!(returned(&mut e, at(10)), ["va"]);
    assert!(returned(&mut e, at(20)).is_empty(), "half-open");
    // A vector recorded after K is not returned as of K.
    clock.set(200);
    write(
        &mut e,
        vec![assert(embedding(
            "vb",
            "sp",
            on_node("b"),
            &[1.0, 0.1],
            json!({"start": 0}),
            json!([]),
        ))],
    )
    .unwrap();
    clock.set(300);
    let mut before = nearest("sp", &[1.0, 0.0], 5);
    before["as_of"] = json!(150);
    assert_eq!(returned(&mut e, before), ["va"]);
    let mut after = nearest("sp", &[1.0, 0.0], 5);
    after["as_of"] = json!(250);
    assert_eq!(returned(&mut e, after), ["va", "vb"]);
    // Before the first recorded head the branch is empty, not partial.
    let mut empty = nearest("sp", &[1.0, 0.0], 5);
    empty["as_of"] = json!(50);
    assert_eq!(
        evaluate(&mut e, "alice", empty).unwrap_err().code,
        "E_UNAVAILABLE"
    );
}

#[test]
fn mismatched_spaces_are_rejected_even_at_equal_dimension() {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(5))).unwrap();
    let base = space("base", 3, "cosine_distance");
    let mut changes = vec![
        assert(base.clone()),
        assert(node("a", json!([]))),
        assert(node("b", json!([]))),
    ];
    let variants = [
        ("encoder", json!("other-encoder")),
        ("encoder_version", json!("v2")),
        ("preprocessing", json!("lowercase/v1")),
        ("metric", json!("euclidean")),
    ];
    for (field, value) in &variants {
        let mut other = base.clone();
        other["id"] = json!(format!("by-{field}"));
        other[*field] = value.clone();
        changes.push(assert(other));
        changes.push(assert(embedding(
            &format!("q-{field}"),
            &format!("by-{field}"),
            on_node("b"),
            &[1.0, 2.0, 3.0],
            always(),
            json!([]),
        )));
    }
    changes.push(assert(embedding(
        "in-base",
        "base",
        on_node("a"),
        &[1.0, 2.0, 3.0],
        always(),
        json!([]),
    )));
    write(&mut e, changes).unwrap();
    for (field, _) in &variants {
        // A query fact from a space that differs only in `field` is rejected.
        let plan = json!({"kind": "nearest", "graph_id": "g", "space": {"id": "base", "revision": "1"},
            "query": {"kind": "embedding", "embedding_id": format!("q-{field}")}, "k": 3});
        assert_eq!(
            evaluate(&mut e, "alice", plan).unwrap_err().code,
            "E_SPACE",
            "{field}"
        );
        // Rebinding the base space name to a descriptor that differs only in `field` conflicts.
        let mut rebound = base.clone();
        rebound[*field] = variants.iter().find(|(f, _)| f == field).unwrap().1.clone();
        assert_eq!(
            write(&mut e, vec![assert(rebound.clone())])
                .unwrap_err()
                .code,
            "E_CONFLICT",
            "{field}"
        );
        assert_eq!(
            write(&mut e, vec![json!({"change": "correct", "fact": rebound})])
                .unwrap_err()
                .code,
            "E_CONFLICT",
            "{field}"
        );
    }
    // The base query fact itself works, and excludes its own subject.
    let own = json!({"kind": "nearest", "graph_id": "g", "space": {"id": "base", "revision": "1"},
        "query": {"kind": "embedding", "embedding_id": "in-base"}, "k": 3});
    assert!(evaluate(&mut e, "alice", own).unwrap().neighbors.is_empty());
    // Wrong dimension, non-finite or zero query vectors are rejected.
    assert_eq!(
        evaluate(&mut e, "alice", nearest("base", &[1.0, 2.0], 3))
            .unwrap_err()
            .code,
        "E_SPACE"
    );
    assert_eq!(
        evaluate(&mut e, "alice", nearest("base", &[0.0, 0.0, 0.0], 3))
            .unwrap_err()
            .code,
        "E_SPACE"
    );
    // Writes are checked against the declared space exactly.
    for (vector, code) in [
        (vec![1.0f32, 2.0], "E_SPACE"),
        (vec![0.0, 0.0, 0.0], "E_SPACE"),
    ] {
        let fact = embedding("bad", "base", on_node("a"), &vector, always(), json!([]));
        assert_eq!(write(&mut e, vec![assert(fact)]).unwrap_err().code, code);
    }
    let unknown = embedding(
        "bad",
        "missing",
        on_node("a"),
        &[1.0, 2.0, 3.0],
        always(),
        json!([]),
    );
    assert_eq!(
        write(&mut e, vec![assert(unknown)]).unwrap_err().code,
        "E_SPACE"
    );
    let orphan = embedding(
        "bad",
        "base",
        on_node("nowhere"),
        &[1.0, 2.0, 3.0],
        always(),
        json!([]),
    );
    assert_eq!(
        write(&mut e, vec![assert(orphan)]).unwrap_err().code,
        "E_ENDPOINT"
    );
    // A space revision may not contain the key separator.
    let mut at = base.clone();
    at["revision"] = json!("1@2");
    assert_eq!(write(&mut e, vec![assert(at)]).unwrap_err().code, "E_SPACE");
    // A plan naming an undeclared space fails the same way for every reader.
    assert_eq!(
        evaluate(&mut e, "alice", nearest("absent", &[1.0, 2.0, 3.0], 3))
            .unwrap_err()
            .code,
        "E_SPACE"
    );
    assert_eq!(
        evaluate(&mut e, "alice", nearest("base", &[1.0, 2.0, 3.0], 0))
            .unwrap_err()
            .code,
        "E_NEAREST"
    );
    assert_eq!(
        evaluate(&mut e, "alice", nearest("base", &[1.0, 2.0, 3.0], 1001))
            .unwrap_err()
            .code,
        "E_BUDGET"
    );
}

#[test]
fn embeddings_are_facts_written_through_the_fact_write_path() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    let first = write(
        &mut e,
        vec![
            assert(space("sp", 2, "euclidean")),
            assert(node("a", json!([]))),
            assert(embedding(
                "va",
                "sp",
                on_node("a"),
                &[0.1, 0.2],
                always(),
                json!([]),
            )),
        ],
    )
    .unwrap();
    // Re-asserting the identical facts is a no-op.
    clock.set(20);
    let again = write(
        &mut e,
        vec![assert(embedding(
            "va",
            "sp",
            on_node("a"),
            &[0.1, 0.2],
            always(),
            json!([]),
        ))],
    )
    .unwrap();
    assert_eq!(again, first);
    // JSON numbers are rounded to binary32 and stored rounded.
    let hit = evaluate(&mut e, "alice", nearest("sp", &[0.1, 0.2], 1)).unwrap();
    assert_eq!(hit.graph.embeddings[0].vector, vec![0.1f32, 0.2f32]);
    assert_eq!(hit.graph.embedding_spaces.len(), 1);
    // Ordinary reads never return embeddings or spaces.
    let plain = run(
        &mut e,
        &reader("alice"),
        json!([{"op": "query", "query": {"graph_id": "g"}}]),
    )
    .unwrap();
    let CommandResult::Queried { result } = &plain[0] else {
        panic!()
    };
    assert!(result.graph.embeddings.is_empty() && result.graph.embedding_spaces.is_empty());
    assert_eq!(result.graph.nodes.len(), 1);
    // A correction ends the old vector's interval and keeps history.
    clock.set(30);
    let corrected = write(&mut e, vec![json!({"change": "correct", "fact": embedding("va", "sp", on_node("a"), &[0.1, 0.2], json!({"start": 0, "end": 5}), json!([]))})]).unwrap();
    let changes = evaluate(
        &mut e,
        "alice",
        json!({"kind": "changes", "graph_id": "g", "from": {"kind": "cursor", "revision": first}}),
    )
    .unwrap();
    let entries = &changes.changes.as_ref().unwrap().entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(serde_json::to_value(entries[0].kind).unwrap(), "embedding");
    assert_eq!(
        changes.fact_provenance.len(),
        2,
        "before and after cite the embedding"
    );
    // A 0.23.0 change set keeps its meaning: it has no embedding kinds.
    let old = run_as(
        &mut e,
        "0.23.0",
        &reader("alice"),
        json!([{"op": "evaluate", "value": {"kind": "changes", "graph_id": "g", "from": {"kind": "cursor", "revision": first}}}]),
    )
    .unwrap();
    let CommandResult::Queried { result } = &old[0] else {
        panic!()
    };
    assert!(result.changes.as_ref().unwrap().entries.is_empty());
    // A correction cannot move an embedding to another subject or space.
    clock.set(40);
    write(&mut e, vec![assert(node("b", json!([])))]).unwrap();
    let moved = json!({"change": "correct", "fact": embedding("va", "sp", on_node("b"), &[0.1, 0.2], always(), json!([]))});
    assert_eq!(write(&mut e, vec![moved]).unwrap_err().code, "E_CONFLICT");
    // The subject cannot be retracted while the embedding is live.
    let retract_a = json!({"change": "retract", "target": {"kind": "node", "id": "a"}});
    assert_eq!(
        write(&mut e, vec![retract_a.clone()]).unwrap_err().code,
        "E_DEPENDENT_FACT"
    );
    // A space is immutable.
    let retract_space =
        json!({"change": "retract", "target": {"kind": "embedding_space", "id": "sp@1"}});
    assert_eq!(
        write(&mut e, vec![retract_space]).unwrap_err().code,
        "E_CONFLICT"
    );
    // Retracting the embedding, then the subject, works.
    write(
        &mut e,
        vec![
            json!({"change": "retract", "target": {"kind": "embedding", "id": "va"}}),
            retract_a,
        ],
    )
    .unwrap();
    assert!(evaluate(&mut e, "alice", nearest("sp", &[0.1, 0.2], 1))
        .unwrap()
        .neighbors
        .is_empty());
    // Embeddings share the record namespace with edges and attachments.
    let clash = json!({"kind": "attachment", "id": "vb", "host": {"kind": "node", "id": "b"}, "key": "k",
        "value": {"kind": "literal", "value": 1}, "valid_time": {"start": 0}});
    write(&mut e, vec![assert(clash)]).unwrap();
    let same_id = embedding("vb", "sp", on_node("b"), &[1.0, 1.0], always(), json!([]));
    assert_eq!(
        write(&mut e, vec![json!({"change": "correct", "fact": same_id})])
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    // Snapshots never carry embeddings; 0.23.0 programs cannot name them.
    let snapshot = json!([{"op": "commit", "graph_id": "h", "data": {"nodes": [{"id": "x", "entity_id": "X", "space_id": "s"}],
        "embedding_spaces": [{"id": "sp", "revision": "1", "encoder": "e", "encoder_version": "v", "preprocessing": "p", "dimensions": 1, "metric": "euclidean"}]}}]);
    assert_eq!(
        run(&mut e, &writer(), snapshot.clone()).unwrap_err().code,
        "E_EMBEDDING"
    );
    assert_eq!(
        run_as(&mut e, "0.23.0", &writer(), snapshot)
            .unwrap_err()
            .code,
        "E_VERSION"
    );
    let old = json!([{"op": "write", "graph_id": "g", "changes": [assert(space("sp2", 1, "euclidean"))]}]);
    assert_eq!(
        run_as(&mut e, "0.23.0", &writer(), old).unwrap_err().code,
        "E_VERSION"
    );
    let old = json!([{"op": "evaluate", "value": nearest("sp", &[1.0, 1.0], 1)}]);
    assert_eq!(
        run_as(&mut e, "0.23.0", &reader("alice"), old)
            .unwrap_err()
            .code,
        "E_VERSION"
    );
    let _ = corrected;
}

#[test]
fn nearest_hits_explain_their_source_facts() {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(7))).unwrap();
    let revision = write(
        &mut e,
        vec![
            assert(space("sp", 2, "cosine_distance")),
            assert(node("a", json!([]))),
            assert(node("b", json!([]))),
            assert(embedding(
                "va",
                "sp",
                on_node("a"),
                &[1.0, 0.0],
                always(),
                json!([]),
            )),
            assert(embedding(
                "vb",
                "sp",
                on_node("b"),
                &[0.0, 1.0],
                always(),
                json!([]),
            )),
        ],
    )
    .unwrap();
    let result = evaluate(&mut e, "alice", nearest("sp", &[1.0, 0.2], 2)).unwrap();
    assert_eq!(
        hits(&result)
            .iter()
            .map(|h| h.0.as_str())
            .collect::<Vec<_>>(),
        ["va", "vb"]
    );
    assert_eq!(
        result
            .graph
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .collect::<BTreeSet<_>>(),
        ["a", "b"].into()
    );
    let embedding_entries: Vec<_> = result
        .fact_provenance
        .iter()
        .filter(|p| matches!(p.fact, FactRef::Embedding { .. }))
        .collect();
    assert_eq!(embedding_entries.len(), 2);
    for entry in embedding_entries {
        let FactRef::Embedding { revision: r, .. } = &entry.fact else {
            unreachable!()
        };
        assert_eq!(r, &revision);
        assert_eq!(entry.provenance.writer, "writer");
        assert_eq!(entry.provenance.recorded_at_ms, 7);
        assert!(
            matches!(&entry.provenance.method, Method::Model { model, .. } if model == "test-encoder")
        );
    }
    assert!(result
        .fact_provenance
        .iter()
        .any(|p| matches!(&p.fact, FactRef::Node { node_id, .. } if node_id == "a")));
}

// ------------------------------------------------------------ composition

/// A small fleet: units at sites, maintenance-note chunks about units, and
/// a few restricted records. Returns the store and its model.
struct Fleet {
    edges: Vec<(String, String, String, Value)>, // id, from, to, readers
    nodes: BTreeMap<String, Value>,
    chunks: Vec<(String, String, Vec<f32>, Value)>, // embedding id, chunk node, vector, readers
}

fn fleet(seed: u64) -> (Engine, Fleet) {
    let mut rng = Rng(seed * 13 + 1);
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(5))).unwrap();
    let mut model = Fleet {
        edges: vec![],
        nodes: BTreeMap::new(),
        chunks: vec![],
    };
    let mut changes = vec![assert(space("notes", 4, "cosine_distance"))];
    let add_node = |changes: &mut Vec<Value>, model: &mut Fleet, id: String, readers: Value| {
        changes.push(assert(node(&id, readers.clone())));
        model.nodes.insert(id, readers);
    };
    for s in 0..3 {
        add_node(&mut changes, &mut model, format!("site{s}"), json!([]));
    }
    for u in 0..8 {
        let readers = if u == 3 { json!(["admin"]) } else { json!([]) };
        add_node(&mut changes, &mut model, format!("unit{u}"), readers);
        let site = format!("site{}", u % 3);
        let id = format!("loc{u}");
        changes.push(assert(json!({"kind": "edge", "id": id, "predicate": "located_at", "from": format!("unit{u}"), "to": site, "valid_time": {"start": 0}})));
        model.edges.push((id, format!("unit{u}"), site, json!([])));
    }
    for c in 0..30 {
        let unit = format!("unit{}", rng.next(8));
        let chunk = format!("chunk{c:02}");
        let readers = if rng.next(6) == 0 {
            json!(["admin"])
        } else {
            json!([])
        };
        let mut props = serde_json::Map::new();
        props.insert("artifact_uri".into(), json!(format!("notes://wo/{c}")));
        props.insert("start".into(), json!(0));
        props.insert("end".into(), json!(120));
        let mut chunk_node = node(&chunk, readers.clone());
        chunk_node["type_id"] = Value::Null;
        chunk_node.as_object_mut().unwrap().remove("type_id");
        chunk_node["properties"] = Value::Object(props);
        changes.push(assert(chunk_node));
        model.nodes.insert(chunk.clone(), readers.clone());
        let id = format!("about{c:02}");
        changes.push(assert(json!({"kind": "edge", "id": id, "predicate": "weave:mentions", "from": chunk, "to": unit, "valid_time": {"start": 0}})));
        model.edges.push((id, chunk.clone(), unit, json!([])));
        let vector: Vec<f32> = (0..4).map(|_| 1.0 + rng.next(5) as f32).collect();
        let vreaders = if rng.next(5) == 0 {
            json!(["admin"])
        } else {
            json!([])
        };
        let vid = format!("vec{c:02}");
        changes.push(assert(embedding(
            &vid,
            "notes",
            on_node(&chunk),
            &vector,
            always(),
            vreaders.clone(),
        )));
        model.chunks.push((vid, chunk, vector, vreaders));
    }
    write(&mut e, changes).unwrap();
    (e, model)
}

impl Fleet {
    fn visible_node(&self, id: &str, who: &str) -> bool {
        self.nodes.get(id).is_some_and(|r| admits(r, who))
    }
    fn visible_edges(&self, who: &str) -> Vec<(&str, &str, &str)> {
        self.edges
            .iter()
            .filter(|(_, f, t, r)| {
                admits(r, who) && self.visible_node(f, who) && self.visible_node(t, who)
            })
            .map(|(id, f, t, _)| (id.as_str(), f.as_str(), t.as_str()))
            .collect()
    }
    /// Breadth-first neighborhood in both directions.
    fn neighborhood(
        &self,
        seeds: &[&str],
        hops: u32,
        who: &str,
        predicates: Option<&[&str]>,
    ) -> BTreeSet<String> {
        let edges = self.visible_edges(who);
        let mut seen: BTreeSet<String> = seeds
            .iter()
            .filter(|s| self.visible_node(s, who))
            .map(|s| s.to_string())
            .collect();
        let mut queue: VecDeque<(String, u32)> = seen.iter().map(|s| (s.clone(), 0)).collect();
        while let Some((at, d)) = queue.pop_front() {
            if d == hops {
                continue;
            }
            for (id, f, t) in &edges {
                let _ = id;
                let _ = predicates;
                for (here, there) in [(f, t), (t, f)] {
                    if *here == at && seen.insert(there.to_string()) {
                        queue.push_back((there.to_string(), d + 1));
                    }
                }
            }
        }
        seen
    }
    fn nearest(
        &self,
        query: &[f32],
        k: usize,
        who: &str,
        allowed: Option<&BTreeSet<String>>,
    ) -> Vec<(String, f64)> {
        let mut scored: Vec<(f64, String, String)> = self
            .chunks
            .iter()
            .filter(|(_, chunk, _, r)| admits(r, who) && self.visible_node(chunk, who))
            .filter(|(_, chunk, _, _)| allowed.is_none_or(|a| a.contains(chunk)))
            .map(|(id, chunk, v, _)| {
                (
                    oracle_distance("cosine_distance", query, v),
                    chunk.clone(),
                    id.clone(),
                )
            })
            .collect();
        scored.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        scored
            .into_iter()
            .take(k)
            .map(|(s, _, id)| (id, s))
            .collect()
    }
}

#[test]
fn graph_to_vector_and_vector_to_graph_compose_in_one_plan() {
    for seed in 0..6 {
        let (mut e, model) = fleet(seed);
        let query = [1.0f32, 2.0, 3.0, 4.0];
        for who in ["analyst", "admin"] {
            for unit in ["unit0", "unit3", "unit5"] {
                // "2-hop neighborhood of unit U -> nearest 10 maintenance-note chunks".
                let plan = json!({"kind": "nearest", "graph_id": "g", "space": {"id": "notes", "revision": "1"},
                    "query": {"kind": "vector", "vector": query}, "k": 10,
                    "candidates": {"kind": "expand", "graph_id": "g", "seeds": [unit], "hops": 2, "direction": "both"}});
                let got = evaluate(&mut e, who, plan).unwrap();
                let region = model.neighborhood(&[unit], 2, who, None);
                let want = model.nearest(&query, 10, who, Some(&region));
                assert_eq!(hits(&got), want, "seed {seed} {who} {unit}");
                if who == "analyst" && unit == "unit3" {
                    assert!(
                        got.neighbors.is_empty(),
                        "a hidden seed hides its neighborhood"
                    );
                    assert_eq!(got.coverage, weave_contract::Coverage::Partial);
                }
            }
            // "nearest 10 chunks -> owning units -> their sites".
            let plan = json!({"kind": "expand", "graph_id": "g", "hops": 2, "direction": "out",
                "predicates": ["weave:mentions", "located_at"],
                "seeds_from": {"kind": "nearest", "graph_id": "g", "space": {"id": "notes", "revision": "1"},
                    "query": {"kind": "vector", "vector": query}, "k": 10}});
            let got = evaluate(&mut e, who, plan).unwrap();
            let top: Vec<String> = model
                .nearest(&query, 10, who, None)
                .into_iter()
                .map(|(id, _)| model.chunks.iter().find(|c| c.0 == id).unwrap().1.clone())
                .collect();
            let edges = model.visible_edges(who);
            let units: BTreeSet<String> = edges
                .iter()
                .filter(|(_, f, _)| top.iter().any(|c| c == f))
                .map(|(_, _, t)| t.to_string())
                .collect();
            let sites: BTreeSet<String> = edges
                .iter()
                .filter(|(_, f, _)| units.contains(*f))
                .map(|(_, _, t)| t.to_string())
                .collect();
            let want: BTreeSet<String> = top
                .iter()
                .cloned()
                .chain(units.clone())
                .chain(sites)
                .collect();
            let nodes: BTreeSet<String> = got.graph.nodes.iter().map(|n| n.id.clone()).collect();
            assert_eq!(nodes, want, "seed {seed} {who}");
            if who == "analyst" {
                assert!(!nodes.contains("unit3"), "restricted unit stays hidden");
            }
            assert!(got.neighbors.is_empty(), "expand output is a graph value");
        }
    }
}

// ------------------------------------------------------------ candidates

#[test]
fn candidate_relations_carry_provenance_and_stay_out_of_traversal() {
    let clock = Arc::new(ManualClock::new(9));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    let units = ["u1", "u2", "u3", "u4"];
    let mut changes = vec![assert(space("units", 3, "euclidean"))];
    for u in units {
        changes.push(assert(node(u, json!([]))));
    }
    changes.push(assert(json!({"kind": "edge", "id": "link", "predicate": "peer_of", "from": "u1", "to": "u2", "valid_time": {"start": 0}})));
    let vectors = [
        [0.0f32, 0.0, 0.0],
        [0.0, 0.0, 0.1],
        [5.0, 5.0, 5.0],
        [5.0, 5.0, 5.2],
    ];
    for (u, v) in units.iter().zip(vectors) {
        changes.push(assert(embedding(
            &format!("e-{u}"),
            "units",
            on_node(u),
            &v,
            json!({"start": 0, "end": 100}),
            json!([]),
        )));
    }
    let revision = write(&mut e, changes).unwrap();
    let plan = json!({"kind": "similar_candidates", "graph_id": "g", "space": {"id": "units", "revision": "1"}, "k": 1});
    let result = evaluate(&mut e, "alice", plan.clone()).unwrap();
    let pairs: Vec<(String, String)> = result
        .candidates
        .iter()
        .map(|c| (c.from.clone(), c.to.clone()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("u1".to_string(), "u2".to_string()),
            ("u3".into(), "u4".into())
        ]
    );
    for candidate in &result.candidates {
        assert_eq!(candidate.predicate, "weave:similar_to");
        assert_eq!(
            candidate.valid_time,
            Interval {
                start: 0,
                end: Some(100)
            }
        );
        assert_eq!(candidate.origins.len(), 2);
        assert!(candidate.id.starts_with("weave:candidate:"));
        for origin in &candidate.origins {
            let FactRef::Embedding { revision: r, .. } = origin else {
                panic!()
            };
            assert_eq!(r, &revision);
            let entry = result
                .fact_provenance
                .iter()
                .find(|p| &p.fact == origin)
                .expect("origin provenance");
            assert!(matches!(entry.provenance.method, Method::Model { .. }));
        }
    }
    assert_eq!(result.candidates[0].score, 0.1f32 as f64);
    // Relations already in the graph are dropped on request.
    let mut absent = plan.clone();
    absent["exclude_linked"] = json!(true);
    let result = evaluate(&mut e, "alice", absent).unwrap();
    let pairs: Vec<(String, String)> = result
        .candidates
        .iter()
        .map(|c| (c.from.clone(), c.to.clone()))
        .collect();
    assert_eq!(pairs, [("u3".to_string(), "u4".to_string())]);
    // Candidates are never written: the head does not move, and ordinary
    // traversal never follows them.
    let head = |e: &mut Engine| e.head("g", "main").unwrap();
    assert_eq!(head(&mut e).as_deref(), Some(revision.as_str()));
    let expanded = evaluate(
        &mut e,
        "alice",
        json!({"kind": "expand", "graph_id": "g", "seeds": ["u3"], "hops": 3, "direction": "both"}),
    )
    .unwrap();
    assert_eq!(
        expanded
            .graph
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        ["u3"]
    );
    assert!(expanded
        .graph
        .edges
        .iter()
        .all(|e| e.predicate != "weave:similar_to"));
    // max_score, sources and the comparison bound.
    let mut close = plan.clone();
    close["max_score"] = json!(0.15);
    close["sources"] = json!({"kind": "query", "query": {"graph_id": "g", "predicate": "peer_of"}});
    let result = evaluate(&mut e, "alice", close).unwrap();
    assert_eq!(result.candidates.len(), 1);
    let mut tight = plan.clone();
    tight["limit"] = json!(1);
    assert_eq!(
        evaluate(&mut e, "alice", tight).unwrap_err().code,
        "E_BUDGET"
    );
    // Persisting a candidate is an explicit write of an ordinary edge that
    // cites the two embeddings as evidence.
    let candidate = &evaluate(&mut e, "alice", plan).unwrap().candidates[1];
    clock.set(20);
    let persisted = json!({"kind": "edge", "id": "sim-u3-u4", "predicate": "similar_unit", "from": candidate.from, "to": candidate.to,
        "valid_time": {"start": 0, "end": 100},
        "provenance": {"method": {"kind": "rule", "rule_id": "similar-units", "revision": "1"},
            "evidence": candidate.origins.iter().map(|o| {
                let FactRef::Embedding { graph_id, revision, embedding_id } = o else { panic!() };
                json!({"kind": "embedding", "graph_id": graph_id, "revision": revision, "embedding_id": embedding_id})
            }).collect::<Vec<_>>()}});
    write(&mut e, vec![assert(persisted)]).unwrap();
    let read = run(
        &mut e,
        &reader("alice"),
        json!([{"op": "query", "query": {"graph_id": "g", "predicate": "similar_unit"}}]),
    )
    .unwrap();
    let CommandResult::Queried { result } = &read[0] else {
        panic!()
    };
    let entry = result.fact_provenance.iter().find(|p| matches!(&p.fact, FactRef::Assertion { assertion_id, .. } if assertion_id == "sim-u3-u4")).unwrap();
    assert_eq!(
        entry.provenance.evidence.len(),
        2,
        "visible embedding evidence is kept"
    );
}

// ------------------------------------------------------------ hidden input

/// Two stores with the same visible facts; the second also holds facts
/// only `mallory` can read: exact duplicates of visible vectors (ties),
/// closer vectors, embeddings on hidden nodes, hidden embeddings on visible
/// nodes, and a hidden edge between visible nodes.
fn paired(seed: u64) -> [Engine; 2] {
    let build = |hidden: bool| {
        let mut rng = Rng(seed * 29 + 7);
        let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(5))).unwrap();
        let mut changes = vec![
            assert(space("sp", 3, "cosine_distance")),
            assert(space("eu", 3, "euclidean")),
        ];
        let mut vectors: Vec<Vec<f32>> = vec![];
        for i in 0..10 {
            changes.push(assert(node(&format!("n{i}"), json!([]))));
        }
        for i in 0..20 {
            let v: Vec<f32> = (0..3).map(|_| 1.0 + rng.next(4) as f32).collect();
            for space in ["sp", "eu"] {
                let id = format!("{space}{i}");
                changes.push(assert(embedding(
                    &id,
                    space,
                    on_node(&format!("n{}", i % 10)),
                    &v,
                    json!({"start": rng.next(3) as i64}),
                    json!([]),
                )));
            }
            vectors.push(v);
        }
        changes.push(assert(json!({"kind": "edge", "id": "vis", "predicate": "p", "from": "n0", "to": "n1", "valid_time": {"start": 0}})));
        if hidden {
            let mallory = json!(["mallory"]);
            changes.push(assert(node("secret", mallory.clone())));
            changes.push(assert(json!({"kind": "edge", "id": "hid", "predicate": "p", "from": "n2", "to": "n3", "valid_time": {"start": 0}, "readers": mallory})));
            for (j, v) in vectors.iter().enumerate().take(12) {
                for space in ["sp", "eu"] {
                    // A duplicate on a visible node, hidden by its readers.
                    changes.push(assert(embedding(
                        &format!("{space}-dup{j}"),
                        space,
                        on_node(&format!("n{}", j % 10)),
                        v,
                        always(),
                        mallory.clone(),
                    )));
                    // A duplicate on a hidden node, public itself.
                    changes.push(assert(embedding(
                        &format!("{space}-sec{j}"),
                        space,
                        on_node("secret"),
                        v,
                        always(),
                        json!([]),
                    )));
                }
            }
            for space in ["sp", "eu"] {
                changes.push(assert(embedding(
                    &format!("{space}-q"),
                    space,
                    on_node("n4"),
                    &[1.0, 2.0, 3.0],
                    always(),
                    mallory.clone(),
                )));
            }
        }
        write(&mut e, changes).unwrap();
        e
    };
    [build(false), build(true)]
}

#[test]
fn hidden_embeddings_never_change_ranks_scores_counts_ties_or_errors() {
    for seed in 0..6 {
        let [mut a, mut b] = paired(seed);
        let mut plans = vec![];
        for space in ["sp", "eu"] {
            let sref = json!({"id": space, "revision": "1"});
            for query in [[1.0f32, 2.0, 3.0], [2.0, 2.0, 2.0], [4.0, 1.0, 1.0]] {
                for k in [1u32, 3, 7, 40] {
                    let mut plan = nearest(space, &query, k);
                    plans.push(plan.clone());
                    plan["valid_at"] = json!(1);
                    plans.push(plan.clone());
                    plan["candidates"] = json!({"kind": "expand", "graph_id": "g", "seeds": ["n0", "n2", "secret"], "hops": 1, "direction": "both"});
                    plans.push(plan);
                }
            }
            for query in [
                format!("{space}3"),
                format!("{space}-q"),
                "nowhere".to_string(),
            ] {
                plans.push(json!({"kind": "nearest", "graph_id": "g", "space": sref, "query": {"kind": "embedding", "embedding_id": query}, "k": 5}));
            }
            plans.push(nearest(space, &[1.0, 2.0], 3));
            plans.push(nearest(space, &[1.0, 2.0, 3.0], 0));
            for k in [1u32, 2] {
                for exclude in [false, true] {
                    plans.push(json!({"kind": "similar_candidates", "graph_id": "g", "space": sref, "k": k, "exclude_linked": exclude}));
                    plans.push(json!({"kind": "similar_candidates", "graph_id": "g", "space": sref, "k": k, "max_score": 0.05, "limit": 3}));
                }
            }
            plans.push(
                json!({"kind": "expand", "graph_id": "g", "hops": 1, "direction": "both",
                "seeds_from": nearest(space, &[1.0, 2.0, 3.0], 4)}),
            );
        }
        let mut errors = 0;
        for plan in &plans {
            for principal in ["alice", "carol"] {
                let x = normal(&evaluate(&mut a, principal, plan.clone()));
                let y = normal(&evaluate(&mut b, principal, plan.clone()));
                assert_eq!(x, y, "seed {seed} {principal} {plan}");
                errors += usize::from(x.starts_with("error"));
            }
        }
        assert!(
            errors > 0 && errors < plans.len(),
            "both outcomes are exercised"
        );
        // Mallory does see her duplicates, and they tie with the visible ones.
        let mallory = evaluate(&mut b, "mallory", nearest("eu", &[1.0, 2.0, 3.0], 1000)).unwrap();
        assert_eq!(mallory.neighbors[0].score, 0.0);
        assert!(mallory.neighbors.iter().any(|n| n.embedding_id == "eu-q"));
        let all = evaluate(&mut b, "mallory", nearest("sp", &[1.0, 2.0, 3.0], 1000)).unwrap();
        assert!(
            all.neighbors.len()
                > evaluate(&mut a, "alice", nearest("sp", &[1.0, 2.0, 3.0], 1000))
                    .unwrap()
                    .neighbors
                    .len()
        );
    }
}

#[test]
fn a_denied_embedding_makes_the_result_partial() {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(5))).unwrap();
    write_to(&mut e, "h", vec![assert(node("text", json!(["admin"])))]).unwrap();
    let text = e.head("h", "main").unwrap().unwrap();
    let mut gated = embedding(
        "gated",
        "sp",
        on_node("a"),
        &[1.0, 0.0],
        always(),
        json!([]),
    );
    gated["derived_nodes"] = json!([{"graph_id": "h", "revision": text, "node_id": "text"}]);
    write(
        &mut e,
        vec![
            assert(space("sp", 2, "euclidean")),
            assert(node("a", json!([]))),
            assert(node("b", json!([]))),
            assert(gated),
            assert(embedding(
                "open",
                "sp",
                on_node("b"),
                &[0.0, 1.0],
                always(),
                json!([]),
            )),
        ],
    )
    .unwrap();
    let analyst = evaluate(&mut e, "analyst", nearest("sp", &[1.0, 0.0], 5)).unwrap();
    assert_eq!(hits(&analyst).len(), 1);
    assert_eq!(analyst.coverage, weave_contract::Coverage::Partial);
    let admin = evaluate(&mut e, "admin", nearest("sp", &[1.0, 0.0], 5)).unwrap();
    assert_eq!(hits(&admin).len(), 2);
    assert_eq!(admin.coverage, weave_contract::Coverage::Complete);
}

// ------------------------------------------------------------ migration

#[test]
fn the_untyped_vector_property_migrates_only_explicitly() {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(5))).unwrap();
    let data = json!({"nodes": [
        {"id": "a", "entity_id": "A", "space_id": "s", "properties": {"vector": [0.1, 0.2, 0.3]}},
        {"id": "b", "entity_id": "B", "space_id": "s", "properties": {"vector": [1.0, 0.0, 0.5]}, "readers": ["bob"]},
        {"id": "c", "entity_id": "C", "space_id": "s"}]});
    let snapshot = match &run(
        &mut e,
        &writer(),
        json!([{"op": "commit", "graph_id": "g", "data": data}]),
    )
    .unwrap()[0]
    {
        CommandResult::Committed { revision, .. } => revision.clone(),
        other => panic!("{other:?}"),
    };
    let space: EmbeddingSpace = serde_json::from_value(json!({"id": "legacy", "revision": "1", "encoder": "unknown",
        "encoder_version": "pre-0.24", "preprocessing": "none", "dimensions": 3, "metric": "cosine_distance"})).unwrap();
    assert_eq!(space.metric, EmbeddingMetric::CosineDistance);
    let interval = Interval {
        start: 0,
        end: None,
    };
    let plan = e
        .vector_property_changes(
            "g",
            "main",
            "vector",
            &space,
            &interval,
            "vector-property",
            "1",
            &writer(),
        )
        .unwrap();
    assert_eq!(plan.head.as_deref(), Some(snapshot.as_str()));
    assert_eq!(
        plan.converted, 1,
        "the writer converts only what it can read"
    );
    assert_eq!(plan.rounded, 3, "0.1, 0.2 and 0.3 are not binary32 values");
    assert_eq!(plan.changes.len(), 2, "the space and one embedding");
    let changes = serde_json::to_value(&plan.changes).unwrap();
    assert_eq!(changes[1]["fact"]["provenance"]["method"]["kind"], "import");
    assert_eq!(
        changes[1]["fact"]["provenance"]["evidence"][0]["revision"],
        json!(snapshot)
    );
    run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "expected_head": snapshot, "changes": changes}]),
    )
    .unwrap();
    // Idempotent: nothing left to convert.
    let again = e
        .vector_property_changes(
            "g",
            "main",
            "vector",
            &space,
            &interval,
            "vector-property",
            "1",
            &writer(),
        )
        .unwrap();
    assert!(again.changes.is_empty());
    // The stored property is unchanged, still binary64.
    let read = run(
        &mut e,
        &reader("alice"),
        json!([{"op": "query", "query": {"graph_id": "g"}}]),
    )
    .unwrap();
    let CommandResult::Queried { result } = &read[0] else {
        panic!()
    };
    let a = result.graph.nodes.iter().find(|n| n.id == "a").unwrap();
    assert_eq!(a.properties["vector"], json!([0.1, 0.2, 0.3]));
    let hit = evaluate(&mut e, "alice", nearest("legacy", &[0.1, 0.2, 0.3], 1)).unwrap();
    assert_eq!(hit.graph.embeddings[0].vector, vec![0.1f32, 0.2, 0.3]);
    assert_eq!(
        hit.neighbors[0].subject,
        weave_contract::embedding::EmbeddingSubject::Node {
            node_id: "a".into()
        }
    );
    // A malformed property fails the whole conversion.
    let bad = json!([{"op": "write", "graph_id": "g", "changes": [{"change": "correct", "fact":
        {"kind": "node", "id": "c", "entity_id": "C", "space_id": "s", "properties": {"vector": [1.0, 2.0]}}}]}]);
    run(&mut e, &writer(), bad).unwrap();
    let error = e
        .vector_property_changes(
            "g",
            "main",
            "vector",
            &space,
            &interval,
            "vector-property",
            "1",
            &writer(),
        )
        .unwrap_err();
    assert_eq!(error.code, "E_SPACE");
    // Conversion needs write authority.
    let error = e
        .vector_property_changes(
            "g",
            "main",
            "vector",
            &space,
            &interval,
            "vector-property",
            "1",
            &reader("alice"),
        )
        .unwrap_err();
    assert_eq!(error.code, "E_FORBIDDEN");
}
