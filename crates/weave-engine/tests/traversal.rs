//! A3 operators (protocol 0.23.0): `expand`, `changes` and `as_of`.
//!
//! Every behavioral check compares the engine with an independent model in
//! this file: a breadth-first search over the visible graph for `expand`, and
//! the fact-by-fact difference of two visible model states for `changes`.
//! The paired tests build two stores that differ only in facts the reader
//! cannot see and require identical output, after normalizing revision IDs,
//! which are content-derived over every fact (ADR-0001, ADR-0004).
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use weave_contract::{CommandResult, Program, QueryResult, VERSION};
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

const PRINCIPALS: [&str; 3] = ["alice", "bob", "carol"];

fn writer() -> HostContext {
    HostContext::new("writer", ["g", "snap", "src"].map(String::from))
}
fn reader(principal: &str) -> HostContext {
    HostContext::new(principal, Vec::<String>::new())
}
fn run(
    e: &mut Engine,
    host: &HostContext,
    commands: Value,
) -> weave_engine::Result<Vec<CommandResult>> {
    let program: Program =
        serde_json::from_value(json!({"version": VERSION, "commands": commands})).unwrap();
    e.execute(&program, host)
}
fn committed(results: weave_engine::Result<Vec<CommandResult>>) -> String {
    match &results.unwrap()[0] {
        CommandResult::Committed { revision, .. } | CommandResult::Unchanged { revision } => {
            revision.clone()
        }
        other => panic!("{other:?}"),
    }
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
fn ids<T: AsRef<str>>(items: impl IntoIterator<Item = T>) -> BTreeSet<String> {
    items.into_iter().map(|s| s.as_ref().to_string()).collect()
}
fn readers(rng: &mut Rng) -> Value {
    rng.pick(&[
        json!([]),
        json!([]),
        json!(["alice"]),
        json!(["bob"]),
        json!(["alice", "bob"]),
    ])
    .clone()
}
fn admits(record: &Value, principal: &str) -> bool {
    let readers = record["readers"].as_array().cloned().unwrap_or_default();
    readers.is_empty() || readers.iter().any(|r| r == principal)
}
fn contains(interval: &Value, t: i64) -> bool {
    interval.is_null()
        || (interval["start"].as_i64().unwrap() <= t
            && interval["end"].as_i64().is_none_or(|end| t < end))
}

// ---------------------------------------------------------------- expand

#[derive(Clone, Default)]
struct Graph {
    nodes: Vec<Value>,
    edges: Vec<Value>,
}

impl Graph {
    fn random(rng: &mut Rng, nodes: usize, edges: usize) -> Self {
        let mut graph = Graph::default();
        for i in 0..nodes {
            let mut node = json!({"id": format!("n{i}"), "entity_id": format!("E{}", i % 4),
                "space_id": "s", "readers": readers(rng)});
            if rng.next(4) == 0 {
                let start = rng.next(30) as i64;
                node["valid_time"] = json!({"start": start, "end": start + 20});
            }
            graph.nodes.push(node);
        }
        for i in 0..edges {
            let start = rng.next(30) as i64;
            let end = if rng.next(2) == 0 {
                json!(null)
            } else {
                json!(start + 12)
            };
            graph.edges.push(json!({"id": format!("e{i}"), "predicate": rng.pick(&["p", "q", "r"]),
                "from": format!("n{}", rng.next(nodes as u64)), "to": format!("n{}", rng.next(nodes as u64)),
                "valid_time": {"start": start, "end": end}, "readers": readers(rng)}));
        }
        graph
    }
    fn changes(&self) -> Vec<Value> {
        let tag = |kind: &str, r: &Value| {
            let mut fact = r.clone();
            fact["kind"] = json!(kind);
            json!({"change": "assert", "fact": fact})
        };
        self.nodes
            .iter()
            .map(|n| tag("node", n))
            .chain(self.edges.iter().map(|e| tag("edge", e)))
            .collect()
    }
    fn snapshot(&self) -> Value {
        let mut nodes = self.nodes.clone();
        let mut edges = self.edges.clone();
        nodes.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        edges.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        json!({"nodes": nodes, "edges": edges})
    }
}

/// Independent BFS over the visible graph: nodes within `hops` of a visible
/// seed and edges leaving a node fewer than `hops` away. Returns the node IDs,
/// edge IDs and whether a seed is unavailable.
fn oracle_expand(
    graph: &Graph,
    principal: &str,
    seeds: &[String],
    hops: u32,
    direction: &str,
    predicates: &[&str],
    valid_at: Option<i64>,
) -> (BTreeSet<String>, BTreeSet<String>, bool) {
    let readable: BTreeMap<&str, &Value> = graph
        .nodes
        .iter()
        .filter(|n| admits(n, principal))
        .map(|n| (n["id"].as_str().unwrap(), n))
        .collect();
    let live: BTreeSet<&str> = readable
        .iter()
        .filter(|(_, n)| valid_at.is_none_or(|t| contains(&n["valid_time"], t)))
        .map(|(id, _)| *id)
        .collect();
    let edges: Vec<&Value> = graph
        .edges
        .iter()
        .filter(|e| {
            admits(e, principal)
                && live.contains(e["from"].as_str().unwrap())
                && live.contains(e["to"].as_str().unwrap())
                && valid_at.is_none_or(|t| contains(&e["valid_time"], t))
                && (predicates.is_empty() || predicates.contains(&e["predicate"].as_str().unwrap()))
        })
        .collect();
    let unavailable = seeds.iter().any(|s| !readable.contains_key(s.as_str()));
    let mut depth: BTreeMap<String, u32> = BTreeMap::new();
    let mut queue = VecDeque::new();
    for seed in seeds {
        if live.contains(seed.as_str()) && !depth.contains_key(seed) {
            depth.insert(seed.clone(), 0);
            queue.push_back(seed.clone());
        }
    }
    let mut out = BTreeSet::new();
    while let Some(node) = queue.pop_front() {
        let d = depth[&node];
        if d >= hops {
            continue;
        }
        for edge in &edges {
            let (from, to) = (edge["from"].as_str().unwrap(), edge["to"].as_str().unwrap());
            let mut reached = vec![];
            if direction != "in" && from == node {
                reached.push(to);
            }
            if direction != "out" && to == node {
                reached.push(from);
            }
            if reached.is_empty() {
                continue;
            }
            out.insert(edge["id"].as_str().unwrap().to_string());
            for next in reached {
                if !depth.contains_key(next) {
                    depth.insert(next.to_string(), d + 1);
                    queue.push_back(next.to_string());
                }
            }
        }
    }
    (depth.into_keys().collect(), out, unavailable)
}

/// The JSON of a result with graph and revision identifiers normalized.
fn normal(result: &weave_engine::Result<QueryResult>, names: &[(&str, &str)]) -> String {
    let text = match result {
        Ok(result) => serde_json::to_string(result).unwrap(),
        Err(error) => format!("error {} {}", error.code, error.message),
    };
    let text = names
        .iter()
        .fold(text, |text, (from, to)| text.replace(from, to));
    let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
        return text;
    };
    // Revision IDs differ whenever any fact differs; compare everything else.
    fn scrub(value: &mut Value) {
        match value {
            Value::String(s) if s.starts_with("fact:sha256:") || s.starts_with("sha256:") => {
                *s = "REV".into()
            }
            Value::Array(items) => {
                items.iter_mut().for_each(scrub);
                // Entries keyed by revision may sort differently once revisions are equal.
                let mut texts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
                texts.sort();
                texts.dedup();
                *items = texts
                    .iter()
                    .map(|t| serde_json::from_str(t).unwrap())
                    .collect();
            }
            Value::Object(map) => map.values_mut().for_each(scrub),
            _ => {}
        }
    }
    scrub(&mut value);
    value.to_string()
}

fn expand(
    graph: &str,
    seeds: &[String],
    hops: u32,
    direction: &str,
    predicates: &[&str],
    valid_at: Option<i64>,
) -> Value {
    let mut plan = json!({"kind": "expand", "graph_id": graph, "seeds": seeds, "hops": hops, "direction": direction});
    if !predicates.is_empty() {
        plan["predicates"] = json!(predicates);
    }
    if let Some(t) = valid_at {
        plan["valid_at"] = json!(t);
    }
    plan
}

#[test]
fn expand_matches_an_independent_bfs_on_fact_and_snapshot_revisions() {
    let mut checked = 0;
    for seed in 0..40 {
        let mut rng = Rng(seed * 104_729 + 7);
        let graph = Graph::random(&mut rng, 12, 26);
        let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(5))).unwrap();
        let fact = committed(run(
            &mut e,
            &writer(),
            json!([{"op": "write", "graph_id": "g", "changes": graph.changes()}]),
        ));
        let snap = committed(run(
            &mut e,
            &writer(),
            json!([{"op": "commit", "graph_id": "snap", "data": graph.snapshot()}]),
        ));
        let names = [
            (fact.as_str(), "REV"),
            (snap.as_str(), "REV"),
            ("\"snap\"", "\"g\""),
        ];
        for _ in 0..12 {
            let seeds: Vec<String> = (0..1 + rng.next(3))
                .map(|_| format!("n{}", rng.next(13)))
                .collect();
            let hops = 1 + rng.next(4) as u32;
            let direction = *rng.pick(&["out", "in", "both"]);
            let predicates: Vec<&str> = rng.pick(&[vec![], vec!["p"], vec!["p", "q"]]).clone();
            let instant = rng.next(40) as i64;
            let valid_at = *rng.pick(&[None, Some(instant)]);
            for principal in PRINCIPALS {
                let (nodes, edges, unavailable) = oracle_expand(
                    &graph,
                    principal,
                    &seeds,
                    hops,
                    direction,
                    &predicates,
                    valid_at,
                );
                let plan = expand("g", &seeds, hops, direction, &predicates, valid_at);
                let result = evaluate(&mut e, principal, plan.clone()).unwrap();
                let context = format!("seed {seed} {principal} {plan}");
                assert_eq!(
                    ids(result.graph.nodes.iter().map(|n| &n.id)),
                    nodes,
                    "{context}"
                );
                assert_eq!(
                    ids(result.graph.edges.iter().map(|e| &e.id)),
                    edges,
                    "{context}"
                );
                let partial = result.coverage == weave_contract::Coverage::Partial;
                assert_eq!(partial, unavailable, "{context}: {:?}", result.diagnostics);
                // Every element is explained to its source facts with provenance.
                for node in &result.graph.nodes {
                    assert!(result.node_origins.contains_key(&node.id), "{context}");
                }
                for edge in &result.graph.edges {
                    assert!(result.edge_origins.contains_key(&edge.id), "{context}");
                }
                assert_eq!(
                    result.fact_provenance.len(),
                    nodes.len() + edges.len(),
                    "{context}"
                );
                // The snapshot revision of the same records gives the same value.
                let mut on_snapshot = plan.clone();
                on_snapshot["graph_id"] = json!("snap");
                let snapshot = evaluate(&mut e, principal, on_snapshot);
                assert_eq!(
                    normal(&Ok(result), &names),
                    normal(&snapshot, &names),
                    "{context}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 1000);
}

#[test]
fn expand_as_of_reads_the_branch_as_known_then() {
    let mut rng = Rng(99);
    let graph = Graph::random(&mut rng, 10, 24);
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    let (first, second) = graph.edges.split_at(12);
    let early = Graph {
        nodes: graph.nodes.clone(),
        edges: first.to_vec(),
    };
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "changes": early.changes()}]),
    ));
    clock.set(20);
    let later = Graph {
        nodes: vec![],
        edges: second.to_vec(),
    };
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "changes": later.changes()}]),
    ));
    clock.set(30);
    for (as_of, model) in [(10, &early), (15, &early), (20, &graph), (29, &graph)] {
        for principal in PRINCIPALS {
            for start in 0..10 {
                let seeds = vec![format!("n{start}")];
                let mut plan = expand("g", &seeds, 3, "both", &[], Some(12));
                plan["as_of"] = json!(as_of);
                let result = evaluate(&mut e, principal, plan).unwrap();
                let (nodes, edges, _) =
                    oracle_expand(model, principal, &seeds, 3, "both", &[], Some(12));
                assert_eq!(
                    ids(result.graph.nodes.iter().map(|n| &n.id)),
                    nodes,
                    "as_of {as_of}"
                );
                assert_eq!(
                    ids(result.graph.edges.iter().map(|e| &e.id)),
                    edges,
                    "as_of {as_of}"
                );
            }
        }
    }
    // Before the branch existed there is nothing to expand; a future cut is invalid.
    let mut plan = expand("g", &["n0".to_string()], 1, "out", &[], None);
    plan["as_of"] = json!(5);
    assert_eq!(
        evaluate(&mut e, "alice", plan.clone()).unwrap_err().code,
        "E_UNAVAILABLE"
    );
    plan["as_of"] = json!(31);
    assert_eq!(
        evaluate(&mut e, "alice", plan.clone()).unwrap_err().code,
        "E_HISTORY_TIME"
    );
    plan["revision"] = json!("fact:sha256:00");
    assert_eq!(
        evaluate(&mut e, "alice", plan).unwrap_err().code,
        "E_HISTORY_SELECTOR"
    );
}

#[test]
fn expand_follows_explicit_assertions() {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(5))).unwrap();
    let data = json!({"profile": "explicit",
        "nodes": [{"id": "a", "entity_id": "A", "space_id": "s"}, {"id": "b", "entity_id": "B", "space_id": "s"},
                  {"id": "c", "entity_id": "C", "space_id": "s"}],
        "structural_edges": [{"id": "ab", "predicate": "p", "from": "a", "to": "b"},
                             {"id": "bc", "predicate": "p", "from": "b", "to": "c"}],
        "assertions": [{"id": "x1", "edge_id": "ab", "source": "s1", "valid_time": {"start": 0}},
                       {"id": "x2", "edge_id": "bc", "source": "s1", "valid_time": {"start": 0}, "readers": ["bob"]}]});
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "commit", "graph_id": "g", "data": data}]),
    ));
    let alice = evaluate(
        &mut e,
        "alice",
        expand("g", &["a".into()], 3, "out", &[], None),
    )
    .unwrap();
    assert_eq!(ids(alice.graph.edges.iter().map(|e| &e.id)), ids(["x1"]));
    assert_eq!(
        ids(alice.graph.nodes.iter().map(|n| &n.id)),
        ids(["a", "b"])
    );
    let bob = evaluate(
        &mut e,
        "bob",
        expand("g", &["a".into()], 3, "out", &[], None),
    )
    .unwrap();
    assert_eq!(
        ids(bob.graph.edges.iter().map(|e| &e.id)),
        ids(["x1", "x2"])
    );
    assert_eq!(bob.coverage, weave_contract::Coverage::Complete);
}

#[test]
fn expand_coverage_keeps_unknown_partial_denied_and_empty_apart() {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(5))).unwrap();
    // `secret` exists only for bob, so an edge derived from it is denied to alice.
    let src = committed(run(
        &mut e,
        &writer(),
        json!([{"op": "commit", "graph_id": "src", "data": {
        "nodes": [{"id": "x", "entity_id": "X", "space_id": "s", "readers": ["bob"]}],
        "edges": [{"id": "secret", "predicate": "p", "from": "x", "to": "x", "valid_time": {"start": 0}, "readers": ["bob"]}]}}]),
    ));
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "changes": [
        {"change": "assert", "fact": {"kind": "node", "id": "a", "entity_id": "A", "space_id": "s"}},
        {"change": "assert", "fact": {"kind": "node", "id": "b", "entity_id": "B", "space_id": "s"}},
        {"change": "assert", "fact": {"kind": "node", "id": "lonely", "entity_id": "L", "space_id": "s"}},
        {"change": "assert", "fact": {"kind": "node", "id": "hidden", "entity_id": "H", "space_id": "s", "readers": ["bob"]}},
        {"change": "assert", "fact": {"kind": "edge", "id": "ab", "predicate": "p", "from": "a", "to": "b", "valid_time": {"start": 0}}},
        {"change": "assert", "fact": {"kind": "edge", "id": "derived", "predicate": "p", "from": "b", "to": "a", "valid_time": {"start": 0},
            "derived_from": [{"graph_id": "src", "revision": src, "assertion_id": "secret"}]}}]}]),
    ));
    let coverage = |r: &QueryResult| {
        (
            serde_json::to_value(&r.coverage).unwrap(),
            r.diagnostics
                .iter()
                .map(|d| d.code.clone())
                .collect::<Vec<_>>(),
        )
    };
    // Empty: a visible seed with no edges is a complete, empty neighborhood.
    let empty = evaluate(
        &mut e,
        "alice",
        expand("g", &["lonely".into()], 2, "both", &[], None),
    )
    .unwrap();
    assert_eq!(coverage(&empty), (json!("complete"), vec![]));
    assert!(empty.graph.edges.is_empty());
    // Unknown: missing and hidden seeds are both partial and indistinguishable.
    let missing = evaluate(
        &mut e,
        "alice",
        expand("g", &["nowhere".into()], 2, "both", &[], None),
    )
    .unwrap();
    let hidden = evaluate(
        &mut e,
        "alice",
        expand("g", &["hidden".into()], 2, "both", &[], None),
    )
    .unwrap();
    assert_eq!(
        coverage(&missing),
        (json!("partial"), vec!["E_SEED_UNAVAILABLE".to_string()])
    );
    assert_eq!(normal(&Ok(missing), &[]), normal(&Ok(hidden), &[]));
    // Partial: a reachable edge whose premise is denied.
    let denied = evaluate(
        &mut e,
        "alice",
        expand("g", &["a".into()], 2, "both", &[], None),
    )
    .unwrap();
    assert_eq!(
        coverage(&denied),
        (
            json!("partial"),
            vec!["E_DERIVATION_UNAVAILABLE".to_string()]
        )
    );
    assert_eq!(ids(denied.graph.edges.iter().map(|e| &e.id)), ids(["ab"]));
    let bob = evaluate(
        &mut e,
        "bob",
        expand("g", &["a".into()], 2, "both", &[], None),
    )
    .unwrap();
    assert_eq!(coverage(&bob), (json!("complete"), vec![]));
    assert_eq!(
        ids(bob.graph.edges.iter().map(|e| &e.id)),
        ids(["ab", "derived"])
    );
    // Denied graph: unavailable, never an empty complete answer.
    let mut gated = expand("src", &["x".into()], 1, "out", &[], None);
    assert_eq!(
        coverage(&evaluate(&mut e, "alice", gated.clone()).unwrap()).0,
        json!("partial")
    );
    gated["graph_id"] = json!("absent");
    assert_eq!(
        evaluate(&mut e, "alice", gated).unwrap_err().code,
        "E_UNAVAILABLE"
    );
}

#[test]
fn expand_bounds_fail_with_explicit_errors() {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(5))).unwrap();
    let graph = Graph {
        nodes: (0..6)
            .map(|i| json!({"id": format!("n{i}"), "entity_id": format!("E{i}"), "space_id": "s"}))
            .collect(),
        edges: (0..5)
            .map(|i| {
                json!({"id": format!("e{i}"), "predicate": "p", "from": format!("n{i}"),
            "to": format!("n{}", i + 1), "valid_time": {"start": 0}})
            })
            .collect(),
    };
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "changes": graph.changes()}]),
    ));
    let mut plan = expand("g", &["n0".into()], 5, "out", &[], None);
    plan["limit"] = json!(11);
    assert_eq!(
        evaluate(&mut e, "alice", plan.clone())
            .unwrap()
            .graph
            .edges
            .len(),
        5
    );
    plan["limit"] = json!(10);
    assert_eq!(
        evaluate(&mut e, "alice", plan.clone()).unwrap_err().code,
        "E_BUDGET"
    );
    plan["limit"] = json!(0);
    assert_eq!(
        evaluate(&mut e, "alice", plan.clone()).unwrap_err().code,
        "E_BUDGET"
    );
    plan["limit"] = json!(10_000);
    plan["hops"] = json!(17);
    assert_eq!(
        evaluate(&mut e, "alice", plan.clone()).unwrap_err().code,
        "E_BUDGET"
    );
    plan["hops"] = json!(0);
    assert_eq!(
        evaluate(&mut e, "alice", plan.clone()).unwrap_err().code,
        "E_EXPAND"
    );
    plan["hops"] = json!(1);
    plan["seeds"] = json!([]);
    assert_eq!(
        evaluate(&mut e, "alice", plan.clone()).unwrap_err().code,
        "E_EXPAND"
    );
    plan["seeds"] = json!((0..1001).map(|i| format!("n{i}")).collect::<Vec<_>>());
    assert_eq!(
        evaluate(&mut e, "alice", plan).unwrap_err().code,
        "E_BUDGET"
    );
}

#[test]
fn expand_and_changes_require_protocol_0_23() {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(5))).unwrap();
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "changes": [
        {"change": "assert", "fact": {"kind": "node", "id": "a", "entity_id": "A", "space_id": "s"}}]}]),
    ));
    for value in [
        json!({"kind": "expand", "graph_id": "g", "seeds": ["a"], "hops": 1}),
        json!({"kind": "changes", "graph_id": "g", "from": {"kind": "as_of", "unix_millis": 0}}),
    ] {
        let program: Program = serde_json::from_value(
            json!({"version": "0.22.0", "commands": [{"op": "evaluate", "value": value}]}),
        )
        .unwrap();
        assert_eq!(
            e.execute(&program, &reader("alice")).unwrap_err().code,
            "E_VERSION"
        );
    }
}

// ---------------------------------------------------------------- changes

/// The facts of one model state, keyed by `(kind, id)`.
type State = BTreeMap<(String, String), Value>;

fn visible_state(state: &State, principal: &str) -> State {
    let nodes: BTreeMap<&str, &Value> = state
        .iter()
        .filter(|((kind, _), f)| kind == "node" && admits(f, principal))
        .map(|((_, id), f)| (id.as_str(), f))
        .collect();
    let edges: BTreeSet<&str> = state
        .iter()
        .filter(|((kind, _), f)| {
            kind == "edge"
                && admits(f, principal)
                && nodes.contains_key(f["from"].as_str().unwrap())
                && nodes.contains_key(f["to"].as_str().unwrap())
        })
        .map(|((_, id), _)| id.as_str())
        .collect();
    state
        .iter()
        .filter(|((kind, id), f)| match kind.as_str() {
            "node" => nodes.contains_key(id.as_str()),
            "edge" => edges.contains(id.as_str()),
            _ => {
                admits(f, principal)
                    && match f["host"]["kind"].as_str().unwrap() {
                        "graph" => true,
                        "node" => nodes.contains_key(f["host"]["id"].as_str().unwrap()),
                        "edge" => edges.contains(f["host"]["id"].as_str().unwrap()),
                        _ => nodes.values().any(|n| n["entity_id"] == f["host"]["id"]),
                    }
            }
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// `(change, kind, id)` of the visible difference between two states.
fn oracle_changes(
    before: &State,
    after: &State,
    principal: &str,
) -> BTreeSet<(String, String, String)> {
    let (a, b) = (
        visible_state(before, principal),
        visible_state(after, principal),
    );
    let mut out = BTreeSet::new();
    for key in a.keys().chain(b.keys()) {
        let change = match (a.get(key), b.get(key)) {
            (Some(_), None) => "retracted",
            (None, Some(_)) => "asserted",
            (Some(x), Some(y)) if x != y => "corrected",
            _ => continue,
        };
        out.insert((change.to_string(), key.0.clone(), key.1.clone()));
    }
    out
}

/// A canonical fact value: its contract encoding, so model and engine compare exactly.
fn canonical(kind: &str, record: &Value) -> Value {
    let mut fact = record.clone();
    fact["kind"] = json!(kind);
    serde_json::to_value(serde_json::from_value::<weave_contract::Fact>(fact).unwrap()).unwrap()
}

/// A random history of writes over a small id space, tracked in a model.
/// Returns the store, the model state after each write and the write times.
fn random_history(rng: &mut Rng, hidden_only: Option<&str>) -> (Engine, Vec<(i64, State)>) {
    let clock = Arc::new(ManualClock::new(1));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    let mut state: State = BTreeMap::new();
    let mut history = vec![];
    let mut head: Option<String> = None;
    let mut time = 10;
    // Structural identity is permanent per graph, even after a retraction.
    let mut identity: BTreeMap<String, Value> = BTreeMap::new();
    let record_readers = |rng: &mut Rng| match hidden_only {
        Some(p) => json!([p]),
        None => readers(rng),
    };
    for _ in 0..1 + rng.next(7) {
        time += 1 + rng.next(5) as i64;
        clock.set(time);
        let mut next = state.clone();
        let mut changes = vec![];
        let mut touched = BTreeSet::new();
        for _ in 0..1 + rng.next(4) {
            let kind = *rng.pick(&["node", "node", "edge", "edge", "attachment"]);
            let id = match kind {
                "node" => format!("n{}", rng.next(5)),
                "edge" => format!("e{}", rng.next(6)),
                _ => format!("a{}", rng.next(4)),
            };
            if !touched.insert(id.clone()) {
                continue;
            }
            let key = (kind.to_string(), id.clone());
            let live = next.contains_key(&key);
            let nodes: Vec<String> = next
                .keys()
                .filter(|(k, _)| k == "node")
                .map(|(_, id)| id.clone())
                .collect();
            let record = match kind {
                "node" => Some(
                    json!({"id": id, "entity_id": format!("E{}", rng.next(3)), "space_id": "s",
                    "properties": {"v": rng.next(3)}, "readers": record_readers(rng)}),
                ),
                "edge" if !nodes.is_empty() => {
                    let (from, to) = match next.get(&key).or(identity.get(&id)) {
                        Some(old) => (
                            old["from"].as_str().unwrap().to_string(),
                            old["to"].as_str().unwrap().to_string(),
                        ),
                        None => (rng.pick(&nodes).clone(), rng.pick(&nodes).clone()),
                    };
                    let predicate = next.get(&key).or(identity.get(&id)).map_or_else(
                        || rng.pick(&["p", "q"]).to_string(),
                        |o| o["predicate"].as_str().unwrap().to_string(),
                    );
                    if touched.contains(&from)
                        || touched.contains(&to)
                        || !nodes.contains(&from)
                        || !nodes.contains(&to)
                    {
                        None
                    } else {
                        Some(
                            json!({"id": id, "predicate": predicate, "from": from, "to": to,
                            "valid_time": {"start": rng.next(5)}, "readers": record_readers(rng)}),
                        )
                    }
                }
                "attachment" => {
                    let host = match next.get(&key).or(identity.get(&id)) {
                        Some(old) => old["host"].clone(),
                        None => match rng.next(3) {
                            0 if !nodes.is_empty() => {
                                json!({"kind": "node", "id": rng.pick(&nodes)})
                            }
                            1 => json!({"kind": "entity", "id": format!("E{}", rng.next(3))}),
                            _ => json!({"kind": "graph"}),
                        },
                    };
                    let host_ok = match host["kind"].as_str().unwrap() {
                        "node" => {
                            !touched.contains(host["id"].as_str().unwrap())
                                && nodes.iter().any(|n| host["id"] == json!(n))
                        }
                        "entity" => next
                            .iter()
                            .any(|((k, _), n)| k == "node" && n["entity_id"] == host["id"]),
                        _ => true,
                    };
                    let key_name = next.get(&key).or(identity.get(&id)).map_or_else(
                        || rng.pick(&["firmware", "note"]).to_string(),
                        |o| o["key"].as_str().unwrap().to_string(),
                    );
                    host_ok.then(|| json!({"id": id, "host": host, "key": key_name,
                        "value": {"kind": "literal", "value": rng.next(3)}, "valid_time": {"start": 0},
                        "readers": record_readers(rng)}))
                }
                _ => None,
            };
            let Some(record) = record else { continue };
            // Keep entity identity stable: a correction cannot change it.
            let record = match (kind, next.get(&key).or(identity.get(&id))) {
                ("node", Some(old)) => {
                    let mut r = record;
                    r["entity_id"] = old["entity_id"].clone();
                    r
                }
                _ => record,
            };
            let retract = live && rng.next(3) == 0;
            if retract {
                // Dependents must go in the same write.
                let dependents: Vec<(String, String)> = next
                    .iter()
                    .filter(|((k, _), f)| match (kind, k.as_str()) {
                        ("node", "edge") => f["from"] == json!(id) || f["to"] == json!(id),
                        ("node", "attachment") => {
                            (f["host"]["kind"] == "node" && f["host"]["id"] == json!(id))
                                || (f["host"]["kind"] == "entity"
                                    && f["host"]["id"] == next[&key]["entity_id"]
                                    && next
                                        .iter()
                                        .filter(|((k2, _), n)| {
                                            k2 == "node"
                                                && n["entity_id"] == next[&key]["entity_id"]
                                        })
                                        .count()
                                        == 1)
                        }
                        _ => false,
                    })
                    .map(|(k, _)| k.clone())
                    .collect();
                if dependents.iter().any(|(_, d)| touched.contains(d)) {
                    continue;
                }
                for dependent in dependents {
                    touched.insert(dependent.1.clone());
                    changes.push(json!({"change": "retract", "target": {"kind": dependent.0, "id": dependent.1}}));
                    next.remove(&dependent);
                }
                changes.push(json!({"change": "retract", "target": {"kind": kind, "id": id}}));
                next.remove(&key);
            } else {
                let change = if live { "correct" } else { "assert" };
                let fact = canonical(kind, &record);
                if next.get(&key) == Some(&fact) {
                    continue;
                }
                // Random corrections may change readers either way (1.2.0).
                let mut entry = json!({"change": change, "fact": fact.clone()});
                if live {
                    entry["widen_readers"] = json!(true);
                }
                changes.push(entry);
                identity.entry(id.clone()).or_insert_with(|| fact.clone());
                next.insert(key, fact);
            }
        }
        if changes.is_empty() {
            continue;
        }
        let snapshot = rng.next(6) == 0;
        let results = if snapshot {
            let mut data = json!({"nodes": [], "edges": [], "attachments": []});
            for ((kind, _), fact) in &next {
                let mut record = fact.clone();
                record.as_object_mut().unwrap().remove("kind");
                let list = match kind.as_str() {
                    "node" => "nodes",
                    "edge" => "edges",
                    _ => "attachments",
                };
                data[list].as_array_mut().unwrap().push(record);
            }
            run(
                &mut e,
                &writer(),
                json!([{"op": "commit", "graph_id": "g", "expected_head": head, "data": data}]),
            )
        } else {
            run(
                &mut e,
                &writer(),
                json!([{"op": "write", "graph_id": "g", "changes": changes}]),
            )
        };
        match results {
            Ok(_) => {
                head = e.head("g", "main").unwrap();
                state = next;
                history.push((time, state.clone()));
            }
            Err(error) => panic!("generated write rejected: {error} {changes:?}"),
        }
    }
    clock.set(time + 10);
    (e, history)
}

fn state_at(history: &[(i64, State)], t: i64) -> State {
    history
        .iter()
        .rev()
        .find(|(at, _)| *at <= t)
        .map(|(_, s)| s.clone())
        .unwrap_or_default()
}

fn changes_of(result: &QueryResult) -> BTreeSet<(String, String, String)> {
    result
        .changes
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|entry| {
            let v = serde_json::to_value(entry).unwrap();
            (
                v["change"].as_str().unwrap().to_string(),
                v["kind"].as_str().unwrap().to_string(),
                v["id"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn changes_equal_the_difference_of_two_as_of_states_on_random_histories() {
    let mut cases = 0;
    let mut nonempty = 0;
    for seed in 0..340 {
        let mut rng = Rng(seed * 7_368_787 + 3);
        let (mut e, history) = random_history(&mut rng, None);
        if history.is_empty() {
            continue;
        }
        let end = history.last().unwrap().0 + 10;
        let mut times: Vec<i64> = history.iter().map(|(t, _)| *t).collect();
        times.extend([0, 5, end]);
        times.extend(history.iter().map(|(t, _)| t + 1));
        for _ in 0..3 {
            let (k1, k2) = (*rng.pick(&times), *rng.pick(&times));
            let (before, after) = (state_at(&history, k1), state_at(&history, k2));
            for principal in PRINCIPALS {
                let value = json!({"kind": "changes", "graph_id": "g",
                    "from": {"kind": "as_of", "unix_millis": k1}, "to": {"kind": "as_of", "unix_millis": k2}});
                let result = evaluate(&mut e, principal, value).unwrap();
                let expected = oracle_changes(&before, &after, principal);
                let context = format!("seed {seed} {principal} {k1}..{k2}");
                assert_eq!(changes_of(&result), expected, "{context}");
                assert_eq!(
                    result.coverage,
                    weave_contract::Coverage::Complete,
                    "{context}"
                );
                // Entry content is the visible fact value on each side.
                let (a, b) = (
                    visible_state(&before, principal),
                    visible_state(&after, principal),
                );
                for entry in &result.changes.as_ref().unwrap().entries {
                    let v = serde_json::to_value(entry).unwrap();
                    let key = (
                        v["kind"].as_str().unwrap().to_string(),
                        v["id"].as_str().unwrap().to_string(),
                    );
                    assert_eq!(v.get("before"), a.get(&key), "{context}");
                    assert_eq!(v.get("after"), b.get(&key), "{context}");
                }
                // Every entry side is explained by provenance.
                let sides: usize = result
                    .changes
                    .as_ref()
                    .unwrap()
                    .entries
                    .iter()
                    .map(|e| usize::from(e.before.is_some()) + usize::from(e.after.is_some()))
                    .sum();
                assert_eq!(result.fact_provenance.len(), sides, "{context}");
                nonempty += usize::from(!expected.is_empty());
                cases += 1;
            }
        }
    }
    assert!(cases >= 1000, "{cases}");
    assert!(nonempty >= 300, "{nonempty}");
}

#[test]
fn changes_cursor_continues_the_pull_feed() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    let node = |id: &str, v: i64| json!({"kind": "node", "id": id, "entity_id": id.to_uppercase(), "space_id": "s", "properties": {"v": v}});
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "changes": [{"change": "assert", "fact": node("a", 1)}]}]),
    ));
    let first = evaluate(
        &mut e,
        "alice",
        json!({"kind": "changes", "graph_id": "g", "from": {"kind": "as_of", "unix_millis": 0}}),
    )
    .unwrap();
    assert_eq!(
        changes_of(&first),
        BTreeSet::from([("asserted".into(), "node".into(), "a".into())])
    );
    let cursor = first.changes.as_ref().unwrap().to_revision.clone().unwrap();
    // Nothing new since the cursor.
    let idle = evaluate(
        &mut e,
        "alice",
        json!({"kind": "changes", "graph_id": "g", "from": {"kind": "cursor", "revision": cursor}}),
    )
    .unwrap();
    assert!(idle.changes.as_ref().unwrap().entries.is_empty());
    clock.set(20);
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "changes": [
        {"change": "correct", "fact": node("a", 2)}, {"change": "assert", "fact": node("b", 1)}]}]),
    ));
    let next = evaluate(
        &mut e,
        "alice",
        json!({"kind": "changes", "graph_id": "g", "from": {"kind": "cursor", "revision": cursor}}),
    )
    .unwrap();
    assert_eq!(
        changes_of(&next),
        BTreeSet::from([
            ("corrected".into(), "node".into(), "a".into()),
            ("asserted".into(), "node".into(), "b".into())
        ])
    );
    // Scopes keep only matching changes.
    let scoped = evaluate(&mut e, "alice", json!({"kind": "changes", "graph_id": "g", "from": {"kind": "cursor", "revision": cursor}, "entities": ["B"]})).unwrap();
    assert_eq!(
        changes_of(&scoped),
        BTreeSet::from([("asserted".into(), "node".into(), "b".into())])
    );
    // A cursor of another graph, or an unknown one, is unavailable.
    let unknown = json!({"kind": "changes", "graph_id": "g", "from": {"kind": "cursor", "revision": "fact:sha256:00"}});
    assert_eq!(
        evaluate(&mut e, "alice", unknown).unwrap_err().code,
        "E_UNAVAILABLE"
    );
    // The limit is never a truncation.
    let small = json!({"kind": "changes", "graph_id": "g", "from": {"kind": "cursor", "revision": cursor}, "limit": 1});
    assert_eq!(
        evaluate(&mut e, "alice", small).unwrap_err().code,
        "E_BUDGET"
    );
}

#[test]
fn a_change_set_is_terminal_and_not_a_view() {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap();
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "changes": [
        {"change": "assert", "fact": {"kind": "node", "id": "a", "entity_id": "A", "space_id": "s"}}]}]),
    ));
    let changes =
        json!({"kind": "changes", "graph_id": "g", "from": {"kind": "as_of", "unix_millis": 0}});
    let wrapped = json!({"kind": "filter", "input": changes, "predicate": "p"});
    assert_eq!(
        evaluate(&mut e, "alice", wrapped).unwrap_err().code,
        "E_CHANGES_OPERAND"
    );
    let bound = run(
        &mut e,
        &reader("alice"),
        json!([
        {"op": "bind", "name": "c", "value": changes},
        {"op": "evaluate", "value": {"kind": "union", "left": {"kind": "reference", "name": "c"}, "right": {"kind": "reference", "name": "c"}}}]),
    );
    assert_eq!(bound.unwrap_err().code, "E_CHANGES_OPERAND");
    // An expansion composes like any graph value.
    let composed = json!({"kind": "filter", "predicate": null,
        "input": {"kind": "expand", "graph_id": "g", "seeds": ["a"], "hops": 1}});
    assert_eq!(
        evaluate(&mut e, "alice", composed)
            .unwrap()
            .graph
            .nodes
            .len(),
        1
    );
}

// ---------------------------------------------------------------- hidden input

/// Two stores with the same visible history; the second also holds facts only
/// `mallory` can read, written at their own times, some touching visible records.
fn paired(seed: u64) -> [(Engine, Vec<i64>); 2] {
    let build = |hidden: bool| {
        let mut rng = Rng(seed * 31 + 11);
        let mut hidden_rng = Rng(seed * 17 + 5);
        let clock = Arc::new(ManualClock::new(1));
        let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
        let mut times = vec![];
        let mut visible_nodes: Vec<String> = vec![];
        let mut entities: BTreeMap<String, String> = BTreeMap::new();
        for step in 0..6 {
            let time = 10 + step * 10;
            clock.set(time);
            times.push(time);
            let mut changes = vec![];
            for i in 0..2 {
                let id = format!("v{step}{i}");
                let entity = format!("E{}", rng.next(3));
                entities.insert(id.clone(), entity.clone());
                changes.push(json!({"change": "assert", "fact": {"kind": "node", "id": id, "entity_id": entity, "space_id": "s"}}));
                if let Some(other) =
                    visible_nodes.get(rng.next(visible_nodes.len().max(1) as u64) as usize)
                {
                    changes.push(json!({"change": "assert", "fact": {"kind": "edge", "id": format!("ve{step}{i}"), "predicate": rng.pick(&["p", "q"]),
                        "from": other, "to": id, "valid_time": {"start": rng.next(5)}}}));
                }
                visible_nodes.push(id);
            }
            if step == 3 {
                changes.push(json!({"change": "correct", "fact": {"kind": "node", "id": "v00", "entity_id": entities["v00"], "space_id": "s", "properties": {"x": 1}}}));
            }
            committed(run(
                &mut e,
                &writer(),
                json!([{"op": "write", "graph_id": "g", "changes": changes}]),
            ));
            if hidden {
                clock.set(time + 5);
                let target = hidden_rng.pick(&visible_nodes).clone();
                let h = format!("h{step}");
                let mut changes = vec![
                    json!({"change": "assert", "fact": {"kind": "node", "id": h, "entity_id": "E1", "space_id": "s", "readers": ["mallory"]}}),
                    json!({"change": "assert", "fact": {"kind": "edge", "id": format!("he{step}"), "predicate": "p", "from": target, "to": h,
                        "valid_time": {"start": 0}, "readers": ["mallory"]}}),
                    json!({"change": "assert", "fact": {"kind": "edge", "id": format!("hv{step}"), "predicate": "p", "from": target, "to": target,
                        "valid_time": {"start": 0}, "readers": ["mallory"]}}),
                    json!({"change": "assert", "fact": {"kind": "attachment", "id": format!("ha{step}"), "host": {"kind": "node", "id": target},
                        "key": "note", "value": {"kind": "literal", "value": step}, "valid_time": {"start": 0}, "readers": ["mallory"]}}),
                    json!({"change": "assert", "fact": {"kind": "attachment", "id": format!("hx{step}"), "host": {"kind": "entity", "id": "E1"},
                        "key": "note", "value": {"kind": "literal", "value": step}, "valid_time": {"start": 0}, "readers": ["mallory"]}}),
                ];
                if step > 0 {
                    let previous = step - 1;
                    changes.push(json!({"change": "correct", "fact": {"kind": "node", "id": format!("h{previous}"), "entity_id": "E1", "space_id": "s",
                        "properties": {"moved": step}, "readers": ["mallory"]}}));
                }
                committed(run(
                    &mut e,
                    &writer(),
                    json!([{"op": "write", "graph_id": "g", "changes": changes}]),
                ));
            }
        }
        clock.set(200);
        (e, times)
    };
    [build(false), build(true)]
}

#[test]
fn hidden_facts_do_not_change_expand_or_changes_output() {
    for seed in 0..8 {
        let [(mut a, times), (mut b, _)] = paired(seed);
        let mut plans = vec![];
        for start in ["v00", "v11", "v31", "h2", "nowhere"] {
            for hops in [1, 2, 4] {
                for direction in ["out", "in", "both"] {
                    for as_of in [None, Some(times[2] + 2), Some(times[4])] {
                        let mut plan =
                            expand("g", &[start.to_string()], hops, direction, &[], None);
                        if let Some(t) = as_of {
                            plan["as_of"] = json!(t);
                        }
                        plans.push(plan.clone());
                        plan["limit"] = json!(3);
                        plans.push(plan);
                    }
                }
            }
        }
        for t in [times[0], times[2] + 2, times[3] + 5, times[5], 200] {
            for query in [
                json!({"graph_id": "g"}),
                json!({"graph_id": "g", "predicate": "p"}),
                json!({"graph_id": "g", "from": "v00", "valid_at": 3}),
                json!({"graph_id": "g", "to": "v11"}),
            ] {
                plans.push(json!({"kind": "as_of", "query": query, "unix_millis": t}));
            }
        }
        for (k1, k2) in [
            (0, times[2]),
            (times[1] + 2, times[3] + 2),
            (times[0], 200),
            (times[3] + 5, times[4]),
        ] {
            for entities in [json!([]), json!(["E1"])] {
                plans.push(json!({"kind": "changes", "graph_id": "g", "entities": entities,
                    "from": {"kind": "as_of", "unix_millis": k1}, "to": {"kind": "as_of", "unix_millis": k2}}));
            }
            plans.push(json!({"kind": "changes", "graph_id": "g", "from": {"kind": "as_of", "unix_millis": k1}, "limit": 2}));
        }
        let mut errors = 0;
        for plan in &plans {
            for principal in ["alice", "carol"] {
                let x = normal(&evaluate(&mut a, principal, plan.clone()), &[]);
                let y = normal(&evaluate(&mut b, principal, plan.clone()), &[]);
                assert_eq!(x, y, "seed {seed} {principal} {plan}");
                errors += usize::from(x.starts_with("error"));
            }
        }
        assert!(
            errors > 0 && errors < plans.len(),
            "both outcomes are exercised"
        );
        // Mallory does see her facts.
        let mallory = evaluate(
            &mut b,
            "mallory",
            expand("g", &["h2".into()], 1, "both", &[], None),
        )
        .unwrap();
        assert!(mallory.graph.edges.iter().any(|e| e.id == "he2"));
    }
}

#[test]
fn hidden_facts_do_not_change_change_sets_on_random_histories() {
    // Interleave the visible history with a second, hidden-only history on
    // other IDs and compare every change set of the visible principals.
    for seed in 0..60 {
        let mut rng = Rng(seed * 7_368_787 + 3);
        let (mut plain, history) = random_history(&mut rng, None);
        if history.is_empty() {
            continue;
        }
        let mut rng = Rng(seed * 7_368_787 + 3);
        let (mut mixed, _) = random_history(&mut rng, None);
        // Add hidden facts on top, at a later time, around visible nodes.
        let clock_time = history.last().unwrap().0 + 10;
        let live = mixed.head("g", "main").unwrap();
        assert!(live.is_some());
        let changes = vec![
            json!({"change": "assert", "fact": {"kind": "node", "id": "secret", "entity_id": "E0", "space_id": "s", "readers": ["mallory"]}}),
            json!({"change": "assert", "fact": {"kind": "attachment", "id": "secret-note", "host": {"kind": "entity", "id": "E0"},
                "key": "note", "value": {"kind": "literal", "value": 1}, "valid_time": {"start": 0}, "readers": ["mallory"]}}),
        ];
        committed(run(
            &mut mixed,
            &writer(),
            json!([{"op": "write", "graph_id": "g", "changes": changes}]),
        ));
        for k1 in [0, history[0].0, clock_time - 1] {
            for principal in PRINCIPALS {
                let plan = json!({"kind": "changes", "graph_id": "g", "from": {"kind": "as_of", "unix_millis": k1}});
                let x = normal(&evaluate(&mut plain, principal, plan.clone()), &[]);
                let y = normal(&evaluate(&mut mixed, principal, plan.clone()), &[]);
                assert_eq!(x, y, "seed {seed} {principal} {plan}");
            }
        }
    }
}

#[test]
fn join_results_are_explained_to_source_facts() {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(5))).unwrap();
    for (graph, from, to) in [("snap", "a", "b"), ("src", "b", "c")] {
        committed(run(
            &mut e,
            &writer(),
            json!([{"op": "write", "graph_id": graph, "changes": [
            {"change": "assert", "fact": {"kind": "node", "id": from, "entity_id": from.to_uppercase(), "space_id": "s"}},
            {"change": "assert", "fact": {"kind": "node", "id": to, "entity_id": to.to_uppercase(), "space_id": "s"}},
            {"change": "assert", "fact": {"kind": "edge", "id": format!("{from}{to}"), "predicate": "p", "from": from, "to": to,
                "valid_time": {"start": 0}, "provenance": {"method": {"kind": "human"}}}}]}]),
        ));
    }
    let joined = evaluate(&mut e, "alice", json!({"kind": "join", "output_predicate": "path", "match_on": "entity_space_to_from",
        "left": {"kind": "query", "query": {"graph_id": "snap"}}, "right": {"kind": "query", "query": {"graph_id": "src"}}})).unwrap();
    assert_eq!(joined.graph.edges.len(), 1);
    for edge in &joined.graph.edges {
        let origins = &joined.edge_origins[&edge.id];
        assert_eq!(origins.len(), 2);
        for origin in origins {
            assert!(joined.fact_provenance.iter().any(|p| matches!(&p.fact,
                weave_contract::provenance::FactRef::Assertion { assertion_id, revision, .. }
                    if assertion_id == &origin.assertion_id && revision == &origin.revision)));
        }
    }
    // `explain` of an expansion traces its derived elements to the same sources.
    let explained = evaluate(
        &mut e,
        "alice",
        json!({"kind": "explain",
        "input": {"kind": "expand", "graph_id": "snap", "seeds": ["a"], "hops": 1}}),
    )
    .unwrap();
    assert!(!explained.graph.nodes.is_empty());
}

#[test]
fn as_of_selection_does_not_walk_history() {
    // More revisions than the 4,096-read budget: a linear walk would fail.
    let clock = Arc::new(ManualClock::new(1));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    for i in 0..5_000i64 {
        clock.set(10 + i);
        let fact = json!({"kind": "node", "id": "unit", "entity_id": "U", "space_id": "s", "properties": {"v": i}});
        let change = if i == 0 { "assert" } else { "correct" };
        committed(run(
            &mut e,
            &writer(),
            json!([{"op": "write", "graph_id": "g", "changes": [{"change": change, "fact": fact}]}]),
        ));
    }
    clock.set(10_000);
    let mut plan = expand("g", &["unit".into()], 1, "out", &[], None);
    plan["as_of"] = json!(10 + 3);
    let early = evaluate(&mut e, "alice", plan).unwrap();
    assert_eq!(early.graph.nodes[0].properties["v"], json!(3));
    let span = evaluate(&mut e, "alice", json!({"kind": "changes", "graph_id": "g",
        "from": {"kind": "as_of", "unix_millis": 12}, "to": {"kind": "as_of", "unix_millis": 4_000}})).unwrap();
    assert_eq!(
        changes_of(&span),
        BTreeSet::from([("corrected".into(), "node".into(), "unit".into())])
    );
}

#[test]
fn changes_coverage_reports_only_denied_changes() {
    let clock = Arc::new(ManualClock::new(5));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    let src = committed(run(
        &mut e,
        &writer(),
        json!([{"op": "commit", "graph_id": "src", "data": {
        "nodes": [{"id": "x", "entity_id": "X", "space_id": "s", "readers": ["bob"]}],
        "edges": [{"id": "secret", "predicate": "p", "from": "x", "to": "x", "valid_time": {"start": 0}, "readers": ["bob"]}]}}]),
    ));
    let derived = |id: &str| {
        json!({"kind": "edge", "id": id, "predicate": "p", "from": "a", "to": "a", "valid_time": {"start": 0},
        "derived_from": [{"graph_id": "src", "revision": src, "assertion_id": "secret"}]})
    };
    clock.set(10);
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "changes": [
        {"change": "assert", "fact": {"kind": "node", "id": "a", "entity_id": "A", "space_id": "s"}},
        {"change": "assert", "fact": derived("old")}]}]),
    ));
    clock.set(20);
    // A visible change and a hidden one: the denied but unchanged edge does not matter.
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "changes": [
        {"change": "assert", "fact": {"kind": "node", "id": "b", "entity_id": "B", "space_id": "s"}},
        {"change": "assert", "fact": {"kind": "node", "id": "h", "entity_id": "A", "space_id": "s", "readers": ["bob"]}}]}]),
    ));
    clock.set(30);
    let since = |t: i64| json!({"kind": "changes", "graph_id": "g", "from": {"kind": "as_of", "unix_millis": t}});
    let quiet = evaluate(&mut e, "alice", since(15)).unwrap();
    assert_eq!(quiet.coverage, weave_contract::Coverage::Complete);
    assert_eq!(
        changes_of(&quiet),
        BTreeSet::from([("asserted".into(), "node".into(), "b".into())])
    );
    // A changed fact the reader is denied makes the change set partial.
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "changes": [
        {"change": "assert", "fact": derived("new")}]}]),
    ));
    clock.set(40);
    let denied = evaluate(&mut e, "alice", since(25)).unwrap();
    assert_eq!(denied.coverage, weave_contract::Coverage::Partial);
    assert!(changes_of(&denied).is_empty());
    assert_eq!(denied.diagnostics[0].code, "E_DERIVATION_UNAVAILABLE");
    let bob = evaluate(&mut e, "bob", since(25)).unwrap();
    assert_eq!(bob.coverage, weave_contract::Coverage::Complete);
    assert_eq!(
        changes_of(&bob),
        BTreeSet::from([("asserted".into(), "edge".into(), "new".into())])
    );
}

#[test]
fn as_of_reads_partially_visible_revisions_like_a_pinned_read() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    let unit = |id: &str, readers: Value| json!({"kind": "node", "id": id, "entity_id": id.to_uppercase(), "space_id": "s", "readers": readers});
    let at = |id: &str, unit: &str| json!({"kind": "edge", "id": id, "predicate": "located_at", "from": unit, "to": "site", "valid_time": {"start": 0}});
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "changes": [
        {"change": "assert", "fact": unit("site", json!([]))},
        {"change": "assert", "fact": unit("u1", json!([]))},
        {"change": "assert", "fact": unit("u2", json!(["admin"]))},
        {"change": "assert", "fact": at("a1", "u1")},
        {"change": "assert", "fact": at("a2", "u2")}]}]),
    ));
    let k1 = e.head("g", "main").unwrap().unwrap();
    clock.set(20);
    committed(run(
        &mut e,
        &writer(),
        json!([{"op": "write", "graph_id": "g", "changes": [
        {"change": "assert", "fact": unit("u3", json!([]))}, {"change": "assert", "fact": at("a3", "u3")}]}]),
    ));
    clock.set(30);
    let plan = json!({"graph_id": "g", "predicate": "located_at"});
    for (who, sees) in [("analyst", vec!["a1"]), ("admin", vec!["a1", "a2"])] {
        let as_of = evaluate(
            &mut e,
            who,
            json!({"kind": "as_of", "query": plan, "unix_millis": 15}),
        )
        .unwrap();
        let mut pinned_plan = plan.clone();
        pinned_plan["revision"] = json!(k1);
        let pinned = evaluate(&mut e, who, json!({"kind": "query", "query": pinned_plan})).unwrap();
        assert_eq!(
            ids(as_of.graph.edges.iter().map(|e| &e.id)),
            ids(sees),
            "{who}"
        );
        // Exactly the pinned read of the selected revision, coverage included.
        assert_eq!(
            serde_json::to_value(&as_of).unwrap(),
            serde_json::to_value(&pinned).unwrap(),
            "{who}"
        );
        assert_eq!(as_of.coverage, weave_contract::Coverage::Complete);
        assert!(as_of.recorded_observations.is_empty());
        // recorded_query keeps its whole-revision rule and witnesses (ADR-0002).
        let recorded = evaluate(
            &mut e,
            who,
            json!({"kind": "recorded_query", "query": plan,
            "selection": {"kind": "local_time", "unix_millis": 15}}),
        );
        match who {
            "admin" => assert_eq!(recorded.unwrap().recorded_observations.len(), 1),
            _ => assert_eq!(recorded.unwrap_err().code, "E_HISTORY_UNAVAILABLE"),
        }
    }
    let mut pinned = json!({"kind": "as_of", "query": plan, "unix_millis": 15});
    pinned["query"]["revision"] = json!(k1);
    assert_eq!(
        evaluate(&mut e, "analyst", pinned).unwrap_err().code,
        "E_HISTORY_SELECTOR"
    );
    let future = json!({"kind": "as_of", "query": plan, "unix_millis": 31});
    assert_eq!(
        evaluate(&mut e, "analyst", future).unwrap_err().code,
        "E_HISTORY_TIME"
    );
    let before = json!({"kind": "as_of", "query": plan, "unix_millis": 5});
    assert_eq!(
        evaluate(&mut e, "analyst", before).unwrap_err().code,
        "E_UNAVAILABLE"
    );
}
