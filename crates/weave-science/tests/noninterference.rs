//! B2 (#5, ADR-0008 section 7): the noninterference harness for every
//! science analysis. It uses the engine harness's generated store pairs
//! (`weave-engine/tests/support/noninterference.rs`): for each pair, each
//! analysis of a generated authorized input must give each reader under test
//! identical responses (analysis values, semantics, the retained input,
//! coverage and errors) with and without the hidden facts, and `mallory`, who
//! sees the hidden facts, must get a different response on some cases.
//!
//! `WEAVE_NI_CASES` sets the cases per analysis (default 40; CI runs 500 per
//! PR and 10,000 in the scheduled full-count job, optimized).
#[path = "../../weave-engine/tests/support/noninterference.rs"]
mod ni;

use ni::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use weave_contract::VERSION;
use weave_engine::{Engine, HostContext};
use weave_science::{Request, ScienceSession};

const ANALYSES: [&str; 5] = [
    "degree",
    "components",
    "shortest_paths",
    "pagerank",
    "nearest_vectors",
];

/// One generated authorized input: a read expression over the pair.
fn input(spec: &Spec, rng: &mut Rng) -> Value {
    let node = |rng: &mut Rng| {
        if rng.chance(1, 4) {
            rng.pick(&spec.hidden_nodes).clone()
        } else {
            rng.pick(&spec.visible_nodes).clone()
        }
    };
    let graph = if rng.chance(1, 4) { "snap" } else { "g" };
    let mut query = json!({"graph_id": graph});
    if rng.chance(1, 3) {
        query["predicate"] = json!(rng.pick_str(&PREDICATES));
    }
    if rng.chance(1, 3) && graph == "g" {
        query["revision"] = json!(format!("~R{}~", rng.range(1, STEP_TIMES.len() as i64)));
    }
    match rng.below(5) {
        0 | 1 => json!({"kind": "query", "query": query}),
        2 => {
            query.as_object_mut().unwrap().remove("revision");
            json!({"kind": "as_of", "query": query, "unix_millis": *rng.pick(&[10, 20, 30, 40, 999])})
        }
        3 => json!({"kind": "expand", "graph_id": "g", "seeds": [node(rng), node(rng)],
            "hops": rng.range(1, 3), "direction": rng.pick_str(&["out", "in", "both"])}),
        _ => json!({"kind": "filter", "input": {"kind": "query", "query": {"graph_id": "g"}},
            "predicate": rng.pick_str(&PREDICATES), "valid_at": rng.range(0, 40)}),
    }
}

fn analysis(name: &str, spec: &Spec, rng: &mut Rng) -> Value {
    let valid_at = if rng.chance(1, 2) {
        json!(rng.range(0, 40))
    } else {
        Value::Null
    };
    let node = |rng: &mut Rng| {
        if rng.chance(1, 4) {
            rng.pick(&spec.hidden_nodes).clone()
        } else {
            rng.pick(&spec.visible_nodes).clone()
        }
    };
    match name {
        "degree" => json!({"algorithm": "degree", "valid_at": valid_at}),
        "components" => json!({"algorithm": "components",
            "mode": rng.pick_str(&["weak", "strong"]), "valid_at": valid_at}),
        "shortest_paths" => {
            let mut a = json!({"algorithm": "shortest_paths", "source": node(rng),
                "directed": rng.chance(1, 2), "valid_at": valid_at});
            if rng.chance(1, 2) {
                a["target"] = json!(node(rng));
            }
            a
        }
        "pagerank" => json!({"algorithm": "pagerank", "valid_at": valid_at,
            "max_iterations": rng.range(5, 60)}),
        _ => {
            let query: Vec<f64> = if rng.chance(1, 2) && !spec.vectors.is_empty() {
                rng.pick(&spec.vectors).iter().map(|x| *x as f64).collect()
            } else {
                (0..DIMS).map(|_| rng.range(1, 4) as f64).collect()
            };
            json!({"algorithm": "nearest_vectors", "space_id": rng.pick_str(&SPACES),
                "query": query, "metric": rng.pick_str(&["cosine", "euclidean"]),
                "k": rng.range(1, 6)})
        }
    }
}

/// One store per reader: a science session's host is fixed at creation.
struct Pair {
    plain: (ScienceSession, World),
    mixed: (ScienceSession, World),
}

fn session(spec: &Spec, hidden: bool, principal: &str) -> (ScienceSession, World) {
    let mut world = World::build(spec, hidden);
    world.at(FINAL_TIME);
    let engine = std::mem::replace(&mut world.e, Engine::memory().unwrap());
    (
        ScienceSession::new(engine, HostContext::new(principal, Vec::<String>::new())),
        world,
    )
}

fn respond(store: &mut (ScienceSession, World), request: &Value) -> String {
    let request = store.1.substitute(request);
    let request: Request = serde_json::from_value(request).unwrap();
    let outcome = store
        .0
        .handle(request)
        .map_err(|error| (error.code, error.message));
    canonical(&store.1, outcome)
}

#[derive(Default, serde::Serialize)]
struct Counts {
    cases: usize,
    ok: usize,
    errors: usize,
    sensitive: usize,
    error_codes: BTreeMap<String, usize>,
}

fn run_world(seed: u64, plans: usize, counts: &Mutex<BTreeMap<&'static str, Counts>>) {
    let profile = if seed.is_multiple_of(2) {
        Profile::Local
    } else {
        Profile::Registry
    };
    let spec = Spec::new(seed, profile);
    let mut pairs: Vec<(&str, Pair)> = readers(profile)
        .iter()
        .chain([&HIDDEN_READER])
        .map(|principal| {
            (
                *principal,
                Pair {
                    plain: session(&spec, false, principal),
                    mixed: session(&spec, true, principal),
                },
            )
        })
        .collect();
    for (index, name) in ANALYSES.iter().enumerate() {
        let mut local = Counts::default();
        for case in 0..plans {
            let mut rng =
                Rng::new(seed.wrapping_mul(7_368_787) ^ ((index as u64) << 40) ^ case as u64);
            let request = json!({"operation": "analyze",
                "program": {"version": VERSION, "commands": [{"op": "evaluate", "value": input(&spec, &mut rng)}]},
                "analysis": analysis(name, &spec, &mut rng),
                "allow_partial": rng.chance(2, 3)});
            local.cases += 1;
            for (principal, pair) in pairs.iter_mut() {
                let a = respond(&mut pair.plain, &request);
                let b = respond(&mut pair.mixed, &request);
                if *principal == HIDDEN_READER {
                    local.sensitive += usize::from(a != b);
                    continue;
                }
                assert!(
                    a == b,
                    "hidden facts changed {principal}'s {name} analysis (seed {seed}, {profile:?}, case {case})\nrequest {request}\nwithout hidden: {a}\nwith hidden:    {b}"
                );
                if let Some(rest) = a.strip_prefix("error ") {
                    let code = rest.split(' ').next().unwrap_or_default().to_string();
                    *local.error_codes.entry(code).or_default() += 1;
                    local.errors += 1;
                } else {
                    local.ok += 1;
                }
            }
        }
        let mut counts = counts.lock().unwrap();
        let entry = counts.entry(*name).or_default();
        entry.cases += local.cases;
        entry.ok += local.ok;
        entry.errors += local.errors;
        entry.sensitive += local.sensitive;
        for (code, n) in local.error_codes {
            *entry.error_codes.entry(code).or_default() += n;
        }
    }
}

#[test]
fn hidden_facts_never_change_any_science_analysis() {
    let target = cases_per_operator();
    let plans = 10usize;
    let worlds = target.div_ceil(plans).max(2);
    let counts = Mutex::new(BTreeMap::new());
    let next = AtomicUsize::new(0);
    let threads = threads();
    let base = first_seed();
    let started = std::time::Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let world = next.fetch_add(1, Ordering::SeqCst);
                if world >= worlds {
                    break;
                }
                run_world(base + world as u64, plans, &counts);
            });
        }
    });
    let counts = counts.into_inner().unwrap();
    println!(
        "NONINTERFERENCE_SCIENCE {}",
        json!({"cases_per_analysis_target": target, "worlds": worlds,
            "seconds": started.elapsed().as_secs_f64(), "analyses": counts})
    );
    for name in ANALYSES {
        let c = &counts[name];
        assert!(c.cases >= target, "{name}: {} cases", c.cases);
        assert!(c.ok > 0, "{name}: no successful outcome");
        assert!(
            c.sensitive > 0,
            "{name}: hidden facts never reached the analysis"
        );
    }
}
