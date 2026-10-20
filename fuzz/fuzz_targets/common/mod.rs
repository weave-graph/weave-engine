//! A small seeded fact graph shared by the Phase A fuzz targets (A6).
//!
//! Graph `fuzz` gets three fact writes at recorded times 10, 20 and 30:
//! public nodes `a`, `b`, `c` linked by `feeds` edges, an embedding space
//! `notes` with one embedding per node, a later correction, a later edge and a
//! retraction. Every fact whose ID or content contains `secret` is readable
//! only by `admin`, so the reader `analyst` must never see that word in any
//! result or error, unless the fuzz input itself contains it.
#![allow(dead_code)]

use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{Command, GraphExpression, Program, VERSION};
use weave_engine::{Engine, HostContext, ManualClock};

pub const GRAPH: &str = "fuzz";
pub const WRITER: &str = "fuzz-writer";
pub const ADMIN: &str = "admin";
pub const ANALYST: &str = "analyst";
pub const MARKER: &[u8] = b"secret";
pub const MAX_INPUT: usize = 131_072;
/// The recorded time the engine's clock rests at after seeding.
pub const NOW: i64 = 40;

pub fn writer() -> HostContext {
    HostContext::new(WRITER, [GRAPH.to_string()])
}
pub fn reader(principal: &str) -> HostContext {
    HostContext::new(principal, Vec::<String>::new())
}

fn node(id: &str, readers: &[&str], properties: Value) -> Value {
    json!({"kind": "node", "id": id, "entity_id": id.to_uppercase(), "space_id": "s",
        "properties": properties, "metadata": [], "readers": readers})
}
fn edge(id: &str, predicate: &str, from: &str, to: &str, readers: &[&str]) -> Value {
    json!({"kind": "edge", "id": id, "predicate": predicate, "from": from, "to": to,
        "valid_time": {"start": 0, "end": null}, "polarity": "positive", "properties": {},
        "metadata": [], "readers": readers, "derived_from": []})
}
fn embedding(
    id: &str,
    subject: &str,
    vector: [f32; 3],
    end: Option<i64>,
    readers: &[&str],
) -> Value {
    json!({"kind": "embedding", "id": id, "space": {"id": "notes", "revision": "1"},
        "subject": {"kind": "node", "node_id": subject}, "vector": vector,
        "valid_time": {"start": 0, "end": end}, "readers": readers,
        "provenance": {"method": {"kind": "model", "model": "fuzz-encoder", "version": "1"}}})
}
fn assert(fact: Value) -> Value {
    json!({"change": "assert", "fact": fact})
}

/// The three seeding writes, in order, with their recorded times.
pub fn seed_writes() -> Vec<(i64, Value)> {
    let admin = [ADMIN];
    vec![
        (
            10,
            json!([
                assert(
                    json!({"kind": "embedding_space", "id": "notes", "revision": "1",
                    "encoder": "fuzz-encoder", "encoder_version": "1", "preprocessing": "none",
                    "dimensions": 3, "metric": "cosine_distance"})
                ),
                assert(node("a", &[], json!({}))),
                assert(node("b", &[], json!({}))),
                assert(node("c", &[], json!({}))),
                assert(node("secret-unit", &admin, json!({"note": "secret"}))),
                assert(edge("e-ab", "feeds", "a", "b", &[])),
                assert(edge("e-bc", "feeds", "b", "c", &[])),
                assert(edge("secret-edge", "feeds", "c", "secret-unit", &admin)),
                assert(embedding("va", "a", [1.0, 0.0, 0.0], None, &[])),
                assert(embedding("vb", "b", [0.5, 0.5, 0.0], Some(100), &[])),
                assert(embedding("vc", "c", [0.0, 1.0, 0.0], None, &[])),
                assert(embedding(
                    "secret-vec",
                    "secret-unit",
                    [1.0, 0.01, 0.0],
                    None,
                    &admin
                )),
            ]),
        ),
        (
            20,
            json!([
                {"change": "correct", "fact": node("a", &[], json!({"v": 2}))},
                assert(edge("e-ac", "located_at", "a", "c", &[])),
                {"change": "correct", "fact": node("secret-unit", &admin, json!({"note": "secret-2"}))},
            ]),
        ),
        (
            30,
            json!([
                {"change": "retract", "target": {"kind": "edge", "id": "e-bc"}},
                assert(edge("secret-edge-2", "feeds", "a", "secret-unit", &admin)),
            ]),
        ),
    ]
}

pub fn write_program(changes: &Value) -> Program {
    serde_json::from_value(json!({"version": VERSION,
        "commands": [{"op": "write", "graph_id": GRAPH, "changes": changes}]}))
    .expect("seed write decodes")
}

/// A fresh in-memory engine holding the seeded graph, with its clock.
pub fn seeded() -> (Engine, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::new(0));
    let mut engine = Engine::memory_with_clock(clock.clone()).expect("in-memory test storage");
    seed(&mut engine, &clock);
    (engine, clock)
}

pub fn seed(engine: &mut Engine, clock: &ManualClock) {
    for (at, changes) in seed_writes() {
        clock.set(at);
        engine
            .execute(&write_program(&changes), &writer())
            .expect("seed write succeeds");
    }
    clock.set(NOW);
}

/// The store state a read must leave untouched.
pub fn state(engine: &Engine) -> (Option<String>, u64) {
    (
        engine.head(GRAPH, "main").expect("head"),
        engine.event_count().expect("event count"),
    )
}

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle))
}

/// The `traversal_plans` target: A3 `expand`, `as_of` and `changes`.
pub fn traversal(expression: &GraphExpression) -> bool {
    matches!(
        expression,
        GraphExpression::Expand(_) | GraphExpression::AsOf { .. } | GraphExpression::Changes(_)
    )
}

/// The `vector_plans` target: A4 `nearest` and `similar_candidates`.
pub fn vector(expression: &GraphExpression) -> bool {
    matches!(
        expression,
        GraphExpression::Nearest(_) | GraphExpression::SimilarCandidates(_)
    )
}

/// Decode a program whose every command is an `evaluate` accepted by `allow`.
pub fn read_program(data: &[u8], allow: fn(&GraphExpression) -> bool) -> Option<Program> {
    if data.len() > MAX_INPUT {
        return None;
    }
    let program = weave_contract::decode_program(data).ok()?;
    let only_reads = !program.commands.is_empty()
        && program
            .commands
            .iter()
            .all(|c| matches!(c, Command::Evaluate { value } if allow(value)));
    only_reads.then_some(program)
}

/// The fuzz-target body for read-only plans: evaluating as each reader leaves
/// the store unchanged and gives the same outcome twice, and the analyst never
/// sees a hidden fact (unless the input names one itself).
pub fn check_read(engine: &mut Engine, data: &[u8], program: &Program) {
    let before = state(engine);
    for principal in [ANALYST, ADMIN] {
        let host = reader(principal);
        let first = outcome(engine.execute(program, &host));
        let second = outcome(engine.execute(program, &host));
        assert_eq!(first, second, "evaluation is deterministic for {principal}");
        assert_eq!(state(engine), before, "evaluation changed the store");
        if principal == ANALYST && !contains(data, MARKER) {
            assert!(
                !contains(first.as_bytes(), MARKER),
                "hidden fact reached the analyst: {first}"
            );
        }
    }
}

pub fn outcome<T: serde::Serialize>(result: weave_engine::Result<T>) -> String {
    match result {
        Ok(value) => serde_json::to_string(&value).expect("results serialize"),
        Err(error) => format!("error {}: {}", error.code, error.message),
    }
}
