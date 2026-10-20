#![no_main]
//! A2 fact `write`: decode a program of `write` commands and apply it to the
//! seeded fact graph. A failed write leaves the store unchanged and the line
//! still accepts a clean write; a successful one leaves a graph both readers
//! can read; the same program on a second seeded store has the same outcome.
use libfuzzer_sys::fuzz_target;
use serde_json::json;
use weave_contract::{decode_program, Command, Program, VERSION};

#[path = "common/mod.rs"]
mod common;
use common::*;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    let Ok(program) = decode_program(data) else {
        return;
    };
    if program.commands.is_empty()
        || !program
            .commands
            .iter()
            .all(|c| matches!(c, Command::Write { .. }))
    {
        return;
    }
    let (mut engine, _clock) = seeded();
    let before = state(&engine);
    let first = engine.execute(&program, &writer());
    match &first {
        Err(_) => {
            assert_eq!(state(&engine), before, "a failed write changed the store");
            let clean = write_program(&json!([{"change": "assert", "fact": {
                "kind": "node", "id": "fuzz-fresh", "entity_id": "FRESH", "space_id": "s",
                "properties": {}, "metadata": [], "readers": []}}]));
            engine
                .execute(&clean, &writer())
                .expect("a failed write left the line unwritable");
        }
        Ok(_) => {
            let read: Program = serde_json::from_value(json!({"version": VERSION,
                "commands": [{"op": "query", "query": {"graph_id": GRAPH}}]}))
            .unwrap();
            for principal in [ANALYST, ADMIN] {
                engine
                    .execute(&read, &reader(principal))
                    .expect("a written graph reads");
            }
        }
    }
    let (mut twin, _clock) = seeded();
    assert_eq!(
        outcome(twin.execute(&program, &writer())),
        outcome(first),
        "the same write on the same store has the same outcome"
    );
});
