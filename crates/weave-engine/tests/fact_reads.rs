//! Indexed reads of fact revisions (ADR-0004 section 7) give exactly the
//! result of the whole-snapshot pipeline. Every case stores the same records
//! twice, once as a snapshot revision and once as a fact revision, and
//! compares every filtered query for several principals after normalizing
//! graph and revision identifiers. Hidden records are part of every graph, so
//! these are also paired hidden-input tests for the new read path.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{CommandResult, Program, VERSION};
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
}

fn host() -> HostContext {
    HostContext::new(
        "writer",
        ["src", "snap", "fact", "dsnap", "dfact"].map(String::from),
    )
}
fn execute(e: &mut Engine, commands: Value) -> Vec<CommandResult> {
    let program: Program =
        serde_json::from_value(json!({"version": VERSION, "commands": commands})).unwrap();
    e.execute(&program, &host()).unwrap()
}
fn revision(results: &[CommandResult]) -> String {
    match &results[0] {
        CommandResult::Committed { revision, .. } => revision.clone(),
        other => panic!("{other:?}"),
    }
}
fn readers(rng: &mut Rng) -> Value {
    match rng.next(4) {
        0 => json!(["bob"]),
        1 => json!(["alice", "bob"]),
        _ => json!([]),
    }
}

/// A random legacy-profile graph with restricted records, node valid time,
/// several attachment hosts and influence carriers pointing at `src`.
fn records(rng: &mut Rng, src: &str) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let mut nodes = vec![];
    for i in 0..8 {
        let mut node = json!({"id": format!("n{i}"), "entity_id": format!("E{}", i % 5), "space_id": "s", "readers": readers(rng)});
        if rng.next(3) == 0 {
            let start = rng.next(20) as i64;
            node["valid_time"] = json!({"start": start, "end": start + 15});
        }
        if rng.next(6) == 0 {
            node["derived_snapshots"] = json!([{"graph_id": "src", "revision": src}]);
        }
        nodes.push(node);
    }
    let mut edges = vec![];
    for i in 0..14 {
        let start = rng.next(20) as i64;
        let predicate = ["p", "q"][rng.next(2) as usize];
        let (from, to) = (format!("n{}", rng.next(8)), format!("n{}", rng.next(8)));
        let end = if rng.next(2) == 0 {
            json!(null)
        } else {
            json!(start + 10)
        };
        let restricted = readers(rng);
        let mut edge = json!({"id": format!("e{i}"), "predicate": predicate, "from": from, "to": to,
            "valid_time": {"start": start, "end": end}, "readers": restricted});
        if rng.next(5) == 0 {
            edge["derived_from"] =
                json!([{"graph_id": "src", "revision": src, "assertion_id": "secret"}]);
        }
        edges.push(edge);
    }
    let mut attachments = vec![];
    for i in 0..10 {
        let host = match rng.next(4) {
            0 => json!({"kind": "node", "id": format!("n{}", rng.next(8))}),
            1 => json!({"kind": "edge", "id": format!("e{}", rng.next(14))}),
            2 => json!({"kind": "entity", "id": format!("E{}", rng.next(5))}),
            _ => json!({"kind": "graph"}),
        };
        let key = ["firmware", "note"][rng.next(2) as usize];
        let start = rng.next(20);
        let restricted = readers(rng);
        let mut attachment = json!({"id": format!("a{i}"), "host": host, "key": key,
            "value": {"kind": "literal", "value": i}, "valid_time": {"start": start}, "readers": restricted});
        if rng.next(4) == 0 {
            attachment["derived_from"] =
                json!([{"graph_id": "src", "revision": src, "assertion_id": "secret"}]);
        }
        attachments.push(attachment);
    }
    (nodes, edges, attachments)
}

fn as_facts(kind: &str, records: &[Value]) -> Vec<Value> {
    records
        .iter()
        .map(|r| {
            let mut fact = r.clone();
            fact["kind"] = json!(kind);
            json!({"change": "assert", "fact": fact})
        })
        .collect()
}

fn normalized(e: &mut Engine, principal: &str, plan: Value, names: &[(&str, &str)]) -> String {
    let program: Program = serde_json::from_value(
        json!({"version": VERSION, "commands": [{"op": "query", "query": plan}]}),
    )
    .unwrap();
    let result = e
        .execute(&program, &HostContext::new(principal, Vec::<String>::new()))
        .map(|r| serde_json::to_string(&r).unwrap())
        .unwrap_or_else(|error| format!("error {}", error.code));
    names
        .iter()
        .fold(result, |text, (from, to)| text.replace(from, to))
}

/// A filtered fact-revision read reports `partial` only for records the
/// filter selects. A whole-snapshot read also reports derivations denied on
/// records the filter discards. Everything else must be identical, and the
/// indexed read may never be less complete than the snapshot read.
fn coverage_scoped(snapshot: String, fact: String, context: &str) -> (String, String) {
    let (Ok(mut a), Ok(b)) = (
        serde_json::from_str::<Value>(&snapshot),
        serde_json::from_str::<Value>(&fact),
    ) else {
        return (snapshot, fact);
    };
    let (ra, rb) = (&a[0]["result"], &b[0]["result"]);
    if ra["coverage"] == "partial" && rb["coverage"] == "complete" {
        let codes: Vec<&str> = ra["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["code"].as_str().unwrap())
            .collect();
        assert!(
            codes.iter().all(|c| *c == "E_DERIVATION_UNAVAILABLE"),
            "{context}: {codes:?}"
        );
        a[0]["result"]["coverage"] = json!("complete");
        a[0]["result"]["diagnostics"] = json!([]);
    }
    assert!(
        !(rb["coverage"] == "partial" && a[0]["result"]["coverage"] == "complete"),
        "{context}: indexed read is less complete"
    );
    (a.to_string(), b.to_string())
}

fn same(a: &str, b: &str, context: &str) {
    if a != b {
        let at = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
        let start = at.saturating_sub(200);
        panic!(
            "{context}: results differ at byte {at}\n snapshot: {}\n fact:     {}",
            &a[start..(at + 300).min(a.len())],
            &b[start..(at + 300).min(b.len())]
        );
    }
}

#[test]
fn indexed_fact_reads_equal_snapshot_reads() {
    for seed in 0..12 {
        let mut rng = Rng(seed * 7919 + 1);
        let clock = Arc::new(ManualClock::new(1));
        let mut e = Engine::memory_with_clock(clock).unwrap();
        // A hidden premise that carriers and derived edges depend on.
        let src = revision(&execute(
            &mut e,
            json!([{"op": "commit", "graph_id": "src", "data": {
                "nodes": [{"id": "x", "entity_id": "X", "space_id": "s", "readers": ["bob"]}],
                "edges": [{"id": "secret", "predicate": "p", "from": "x", "to": "x", "valid_time": {"start": 0}, "readers": ["bob"]}]}}]),
        ));
        let (mut nodes, mut edges, mut attachments) = records(&mut rng, &src);
        // Fact revisions list each kind in ID order; store the snapshot the same way.
        for list in [&mut nodes, &mut edges, &mut attachments] {
            list.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        }
        let snap = revision(&execute(
            &mut e,
            json!([{"op": "commit", "graph_id": "snap", "data": {"nodes": nodes, "edges": edges, "attachments": attachments}}]),
        ));
        let mut changes = as_facts("node", &nodes);
        changes.extend(as_facts("edge", &edges));
        changes.extend(as_facts("attachment", &attachments));
        let fact = revision(&execute(
            &mut e,
            json!([{"op": "write", "graph_id": "fact", "changes": changes}]),
        ));
        assert!(fact.starts_with("fact:sha256:"));
        // Edges elsewhere whose premises live in each copy: scoped premise loads.
        let mut derived_revisions = vec![];
        for (graph, target, pin) in [("dsnap", "snap", &snap), ("dfact", "fact", &fact)] {
            let derived: Vec<Value> = (0..6)
                .map(|i| {
                    let premise = format!("{}{}", ["e", "a"][i % 2], i);
                    json!({"id": format!("d{i}"), "predicate": "derived", "from": "m", "to": "m",
                        "valid_time": {"start": 0}, "derived_from": [{"graph_id": target, "revision": pin, "assertion_id": premise}]})
                })
                .collect();
            derived_revisions.push(revision(&execute(
                &mut e,
                json!([{"op": "commit", "graph_id": graph, "data": {"nodes": [{"id": "m", "entity_id": "M", "space_id": "s"}], "edges": derived}}]),
            )));
        }
        let names = [
            (snap.as_str(), "REV"),
            (fact.as_str(), "REV"),
            (derived_revisions[0].as_str(), "DREV"),
            (derived_revisions[1].as_str(), "DREV"),
            ("\"snap\"", "\"G\""),
            ("\"fact\"", "\"G\""),
            ("\"dsnap\"", "\"D\""),
            ("\"dfact\"", "\"D\""),
        ];
        let mut plans = vec![];
        for predicate in [json!(null), json!("p"), json!("q")] {
            for from in [json!(null), json!("n1"), json!("n3")] {
                for to in [json!(null), json!("n2")] {
                    for valid_at in [json!(null), json!(12)] {
                        plans.push((predicate.clone(), from.clone(), to.clone(), valid_at));
                    }
                }
            }
        }
        let mut indexed = 0;
        for principal in ["alice", "bob", "carol"] {
            for (predicate, from, to, valid_at) in &plans {
                let plan = |graph: &str| json!({"graph_id": graph, "predicate": predicate, "from": from, "to": to, "valid_at": valid_at});
                let a = normalized(&mut e, principal, plan("snap"), &names);
                let b = normalized(&mut e, principal, plan("fact"), &names);
                let context = format!("seed {seed} {principal} {predicate} {from} {to} {valid_at}");
                let (a, b) = coverage_scoped(a, b, &context);
                same(&a, &b, &context);
                indexed += usize::from(!predicate.is_null() || !from.is_null() || !to.is_null());
            }
            let a = normalized(&mut e, principal, json!({"graph_id": "dsnap"}), &names);
            let b = normalized(&mut e, principal, json!({"graph_id": "dfact"}), &names);
            same(&a, &b, &format!("seed {seed} {principal} derived premises"));
        }
        assert!(indexed > 50);
    }
}

#[test]
fn hidden_facts_do_not_change_indexed_results() {
    // Paired stores: identical except for facts only bob can read.
    let run = |hidden: bool| {
        let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(1))).unwrap();
        let mut changes = vec![
            json!({"change": "assert", "fact": {"kind": "node", "id": "a", "entity_id": "A", "space_id": "s"}}),
            json!({"change": "assert", "fact": {"kind": "node", "id": "b", "entity_id": "B", "space_id": "s"}}),
            json!({"change": "assert", "fact": {"kind": "edge", "id": "e", "predicate": "p", "from": "a", "to": "b", "valid_time": {"start": 0}}}),
        ];
        if hidden {
            changes.push(json!({"change": "assert", "fact": {"kind": "node", "id": "h", "entity_id": "A", "space_id": "s", "readers": ["bob"]}}));
            changes.push(json!({"change": "assert", "fact": {"kind": "edge", "id": "f", "predicate": "p", "from": "a", "to": "h", "valid_time": {"start": 0}, "readers": ["bob"]}}));
            changes.push(json!({"change": "assert", "fact": {"kind": "attachment", "id": "m", "host": {"kind": "node", "id": "a"}, "key": "k", "value": {"kind": "literal", "value": 1}, "valid_time": {"start": 0}, "readers": ["bob"]}}));
        }
        let r = revision(&execute(
            &mut e,
            json!([{"op": "write", "graph_id": "fact", "changes": changes}]),
        ));
        let mut out = vec![];
        for plan in [
            json!({"graph_id": "fact", "predicate": "p"}),
            json!({"graph_id": "fact", "from": "a"}),
            json!({"graph_id": "fact", "to": "b", "valid_at": 3}),
        ] {
            out.push(normalized(&mut e, "alice", plan, &[(r.as_str(), "REV")]));
        }
        out
    };
    assert_eq!(run(true), run(false));
}
