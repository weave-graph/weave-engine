#![no_main]
//! A3 operators: decode a program of `evaluate` commands whose expressions
//! are `expand`, `as_of` or `changes` (nested expressions are free), and
//! evaluate it over the seeded fact graph with history. Evaluation is
//! read-only and deterministic, and hidden facts never reach the analyst.
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
    if let Some(program) = read_program(data, traversal) {
        ENGINE.with_borrow_mut(|engine| check_read(engine, data, &program));
    }
});
