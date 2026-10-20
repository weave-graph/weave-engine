#![no_main]
//! A4 operators: decode a program of `evaluate` commands whose expressions
//! are `nearest` or `similar_candidates` (nested candidates and seeds are
//! free), and evaluate it over the seeded graph's embeddings. Evaluation is
//! read-only and deterministic, and hidden embeddings never reach the analyst.
use libfuzzer_sys::fuzz_target;
use std::cell::RefCell;
use weave_engine::Engine;

#[path = "common/mod.rs"]
mod common;
use common::*;

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(seeded().0);
}

fuzz_target!(|data: &[u8]| {
    if let Some(program) = read_program(data, vector) {
        ENGINE.with_borrow_mut(|engine| check_read(engine, data, &program));
    }
});
