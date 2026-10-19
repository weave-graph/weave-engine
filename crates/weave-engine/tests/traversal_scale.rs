//! A3 acceptance: `expand` returns the exact k-hop neighborhood (k = 1..4) of
//! a 100,000-fact graph, matching an independent breadth-first search, with
//! valid-time and as-of filters and per-record readers.
//!
//! `WEAVE_A3_FACTS` sets the graph size: 100,000 by default in optimized
//! builds, which the release CI job (`ci.yml`, `fact-scale-1m`) runs on every
//! change, and 20,000 in the default debug suite. The graph is
//! written as fact revisions in two recorded steps, so `as_of` between them
//! reads the first state through the recorded-time index.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use weave_contract::{CommandResult, Program, QueryResult, VERSION};
use weave_engine::{Engine, HostContext, ManualClock};

fn facts() -> usize {
    std::env::var("WEAVE_A3_FACTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(if cfg!(debug_assertions) {
            20_000
        } else {
            100_000
        })
}

struct Rng(u64);
impl Rng {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % bound
    }
}

#[derive(Clone)]
struct Edge {
    id: String,
    from: usize,
    to: usize,
    predicate: &'static str,
    start: i64,
    end: Option<i64>,
    restricted: bool,
    step: u8,
}

fn write(e: &mut Engine, changes: Vec<Value>) {
    let program: Program = serde_json::from_value(
        json!({"version": VERSION, "commands": [{"op": "write", "graph_id": "fleet", "changes": changes}]}),
    )
    .unwrap();
    let result = e
        .execute(&program, &HostContext::new("loader", ["fleet".to_string()]))
        .unwrap();
    assert!(matches!(result[0], CommandResult::Committed { .. }));
}

fn expand(e: &mut Engine, principal: &str, plan: Value) -> QueryResult {
    let program: Program = serde_json::from_value(
        json!({"version": VERSION, "commands": [{"op": "evaluate", "value": plan}]}),
    )
    .unwrap();
    match e
        .execute(&program, &HostContext::new(principal, Vec::<String>::new()))
        .unwrap()
        .remove(0)
    {
        CommandResult::Queried { result } => *result,
        other => panic!("{other:?}"),
    }
}

struct Query<'a> {
    seed: usize,
    hops: u32,
    direction: &'a str,
    predicates: &'a [&'a str],
    valid_at: Option<i64>,
    as_of_first: bool,
    principal: &'a str,
}

/// Independent BFS over the edges the principal may read.
fn oracle(edges: &[Edge], q: &Query) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut out: BTreeMap<usize, Vec<&Edge>> = BTreeMap::new();
    let mut inc: BTreeMap<usize, Vec<&Edge>> = BTreeMap::new();
    for edge in edges {
        let usable = (!q.as_of_first || edge.step == 0)
            && (!edge.restricted || q.principal == "ops")
            && (q.predicates.is_empty() || q.predicates.contains(&edge.predicate))
            && q.valid_at
                .is_none_or(|t| edge.start <= t && edge.end.is_none_or(|end| t < end));
        if usable {
            out.entry(edge.from).or_default().push(edge);
            inc.entry(edge.to).or_default().push(edge);
        }
    }
    let mut depth = BTreeMap::from([(q.seed, 0u32)]);
    let mut queue = VecDeque::from([q.seed]);
    let mut kept = BTreeSet::new();
    while let Some(node) = queue.pop_front() {
        if depth[&node] >= q.hops {
            continue;
        }
        let mut steps = vec![];
        if q.direction != "in" {
            steps.extend(out.get(&node).into_iter().flatten().map(|e| (*e, e.to)));
        }
        if q.direction != "out" {
            steps.extend(inc.get(&node).into_iter().flatten().map(|e| (*e, e.from)));
        }
        for (edge, next) in steps {
            kept.insert(edge.id.clone());
            if !depth.contains_key(&next) {
                depth.insert(next, depth[&node] + 1);
                queue.push_back(next);
            }
        }
    }
    (depth.keys().map(|n| format!("unit-{n}")).collect(), kept)
}

#[test]
fn expand_matches_bfs_on_a_large_fact_graph() {
    let total = facts();
    let units = total / 5;
    let mut rng = Rng(2026);
    let mut edges = Vec::new();
    for i in 0..units * 4 {
        let start = rng.next(100) as i64;
        edges.push(Edge {
            id: format!("link-{i}"),
            from: i / 4,
            to: rng.next(units as u64) as usize,
            predicate: ["installed_in", "feeds", "monitors"][rng.next(3) as usize],
            start,
            end: (rng.next(2) == 0).then_some(start + 40),
            restricted: rng.next(10) == 0,
            step: (rng.next(2)) as u8,
        });
    }
    let clock = Arc::new(ManualClock::new(10));
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::open_with_clock(dir.path().join("fleet.db"), clock.clone()).unwrap();
    let started = Instant::now();
    let edge_fact = |edge: &Edge| {
        json!({"change": "assert", "fact": {"kind": "edge", "id": edge.id, "predicate": edge.predicate,
            "from": format!("unit-{}", edge.from), "to": format!("unit-{}", edge.to),
            "valid_time": {"start": edge.start, "end": edge.end},
            "readers": if edge.restricted { json!(["ops"]) } else { json!([]) }}})
    };
    let nodes: Vec<Value> = (0..units)
        .map(|i| {
            json!({"change": "assert", "fact": {"kind": "node", "id": format!("unit-{i}"),
            "entity_id": format!("U-{i}"), "space_id": "fleet"}})
        })
        .collect();
    for chunk in nodes.chunks(100_000) {
        write(&mut e, chunk.to_vec());
    }
    for step in 0..2u8 {
        clock.set(10 + 10 * step as i64);
        let batch: Vec<Value> = edges
            .iter()
            .filter(|e| e.step == step)
            .map(edge_fact)
            .collect();
        for chunk in batch.chunks(100_000) {
            write(&mut e, chunk.to_vec());
        }
    }
    clock.set(40);
    eprintln!("loaded {} facts in {:?}", units * 5, started.elapsed());
    let (mut checked, mut nonempty) = (0, 0);
    let started = Instant::now();
    for hops in 1..=4u32 {
        for (i, direction) in ["out", "in", "both"].into_iter().enumerate() {
            for (j, valid_at) in [None, Some(55)].into_iter().enumerate() {
                for as_of_first in [false, true] {
                    for principal in ["reader", "ops"] {
                        let seed = (hops as usize * 7919 + i * 104_729 + j * 31) % units;
                        let predicates: &[&str] = if (i + j) % 2 == 0 {
                            &[]
                        } else {
                            &["feeds", "monitors"]
                        };
                        // Keep both directions at four hops to a bounded neighborhood.
                        let hops = if direction == "both" {
                            hops.min(3)
                        } else {
                            hops
                        };
                        let q = Query {
                            seed,
                            hops,
                            direction,
                            predicates,
                            valid_at,
                            as_of_first,
                            principal,
                        };
                        let mut plan = json!({"kind": "expand", "graph_id": "fleet", "seeds": [format!("unit-{seed}")],
                            "hops": hops, "direction": direction, "limit": 200_000});
                        if !predicates.is_empty() {
                            plan["predicates"] = json!(predicates);
                        }
                        if let Some(t) = valid_at {
                            plan["valid_at"] = json!(t);
                        }
                        if as_of_first {
                            plan["as_of"] = json!(15);
                        }
                        let result = expand(&mut e, principal, plan.clone());
                        let (want_nodes, want_edges) = oracle(&edges, &q);
                        let got_nodes: BTreeSet<String> =
                            result.graph.nodes.iter().map(|n| n.id.clone()).collect();
                        let got_edges: BTreeSet<String> =
                            result.graph.edges.iter().map(|e| e.id.clone()).collect();
                        assert_eq!(got_nodes, want_nodes, "{plan}");
                        assert_eq!(got_edges, want_edges, "{plan}");
                        assert_eq!(
                            result.coverage,
                            weave_contract::Coverage::Complete,
                            "{plan}"
                        );
                        nonempty += usize::from(want_edges.len() > 10);
                        checked += 1;
                    }
                }
            }
        }
    }
    eprintln!("checked {checked} expansions in {:?}", started.elapsed());
    assert_eq!(checked, 4 * 3 * 2 * 2 * 2);
    assert!(
        nonempty >= checked / 4,
        "{nonempty} of {checked} expansions are nontrivial"
    );
}
