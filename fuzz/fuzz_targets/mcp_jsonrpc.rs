#![no_main]
//! A7 MCP JSON-RPC input: each input line is one message to a `weave-mcp`
//! server over the seeded store, as the analyst. Every response is one
//! JSON-RPC 2.0 object with exactly one of `result` and `error` and the
//! request's ID; no message changes the store; hidden facts never appear in
//! a response unless the input names them.
use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use std::cell::RefCell;
use weave_engine::Engine;
use weave_mcp::Server;
use weave_science::ScienceSession;

#[path = "common/mod.rs"]
mod common;
use common::*;

const MAX_MESSAGES: usize = 16;

struct Harness {
    _dir: tempfile::TempDir,
    server: Server,
    observer: Engine,
    before: (Option<String>, u64),
}

thread_local! {
    static HARNESS: RefCell<Harness> = RefCell::new({
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("fuzz.db");
        let clock = std::sync::Arc::new(weave_engine::ManualClock::new(0));
        let mut engine = Engine::open_with_clock(&path, clock.clone()).expect("store");
        seed(&mut engine, &clock);
        drop(engine);
        let session = ScienceSession::open(&path, reader(ANALYST)).expect("session");
        let observer = Engine::open(&path).expect("observer");
        let before = state(&observer);
        Harness { _dir: dir, server: Server::new(session), observer, before }
    });
}

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    let secret_in_input = contains(data, MARKER);
    HARNESS.with_borrow_mut(|harness| {
        for line in data.split(|b| *b == b'\n').take(MAX_MESSAGES) {
            let request: Option<Value> = serde_json::from_slice(line).ok();
            let Some(response) = harness.server.handle_message(line) else {
                continue;
            };
            let object = response.as_object().expect("a response is one object");
            assert_eq!(object.get("jsonrpc"), Some(&Value::from("2.0")));
            assert!(
                object.contains_key("result") != object.contains_key("error"),
                "exactly one of result and error: {response}"
            );
            let rejected_envelope = object
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_i64)
                .is_some_and(|code| code == -32700 || code == -32600);
            if let Some(id) = request
                .as_ref()
                .and_then(|r| r.get("id"))
                .filter(|id| id.is_string() || id.is_number())
            {
                if !rejected_envelope {
                    assert_eq!(object.get("id"), Some(id), "the response echoes the ID");
                }
            }
            if !secret_in_input {
                let text = response.to_string();
                assert!(
                    !contains(text.as_bytes(), MARKER),
                    "hidden fact reached the analyst: {text}"
                );
            }
        }
        assert_eq!(
            state(&harness.observer),
            harness.before,
            "an MCP message changed the store"
        );
    });
});
