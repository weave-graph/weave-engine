use serde_json::Value;
use weave_contract::{decode_program, Command, Program};
use weave_engine::{Capsule, Engine, HostContext};

#[path = "../fuzz_targets/common/mod.rs"]
mod common;

type Filter = fn(&weave_contract::GraphExpression) -> bool;

/// The Phase A targets' seeds (A6): every accepted seed passes its target's
/// filter and runs, so the fuzzer starts from complete workflows rather than
/// from inputs rejected while parsing.
fn check_phase_a_seeds() {
    for (seed, accepted) in [
        (
            include_str!("../seeds/fact_write/assert-correct-retract.json"),
            true,
        ),
        (
            include_str!("../seeds/fact_write/embedding-and-provenance.json"),
            true,
        ),
        (
            include_str!("../seeds/fact_write/assert-then-correct.json"),
            true,
        ),
        (
            include_str!("../seeds/fact_write/stale-expected-head.json"),
            false,
        ),
        (
            include_str!("../seeds/fact_write/dimension-mismatch-rollback.json"),
            false,
        ),
    ] {
        let program = decode_program(seed.as_bytes()).expect("fact_write seed decodes");
        assert!(program
            .commands
            .iter()
            .all(|c| matches!(c, Command::Write { .. })));
        let (mut engine, _clock) = common::seeded();
        let before = common::state(&engine);
        let result = engine.execute(&program, &common::writer());
        assert_eq!(
            result.is_ok(),
            accepted,
            "fact_write seed {seed}: {result:?}"
        );
        if !accepted {
            assert_eq!(common::state(&engine), before, "rejected seed rolled back");
        }
    }
    let reads: [(&str, Filter, &[&str]); 2] = [
        (
            "traversal_plans",
            common::traversal,
            &[
                include_str!("../seeds/traversal_plans/expand.json"),
                include_str!("../seeds/traversal_plans/as-of.json"),
                include_str!("../seeds/traversal_plans/changes.json"),
            ],
        ),
        (
            "vector_plans",
            common::vector,
            &[
                include_str!("../seeds/vector_plans/nearest.json"),
                include_str!("../seeds/vector_plans/nearest-in-neighborhood.json"),
                include_str!("../seeds/vector_plans/similar-candidates.json"),
            ],
        ),
    ];
    for (target, allow, seeds) in reads {
        let (mut engine, _clock) = common::seeded();
        let mut admin_saw_hidden = false;
        for seed in seeds {
            let program = common::read_program(seed.as_bytes(), allow)
                .unwrap_or_else(|| panic!("{target} seed passes the target filter: {seed}"));
            for principal in [common::ANALYST, common::ADMIN] {
                let output = engine
                    .execute(&program, &common::reader(principal))
                    .unwrap_or_else(|e| panic!("{target} seed runs as {principal}: {e:?}"));
                let text = serde_json::to_string(&output).unwrap();
                let hidden = common::contains(text.as_bytes(), common::MARKER);
                assert!(principal == common::ADMIN || !hidden, "{target} leaks");
                admin_saw_hidden |= hidden && principal == common::ADMIN;
            }
            common::check_read(&mut engine, seed.as_bytes(), &program);
        }
        // The leak oracle is not vacuous: the admin does see hidden facts.
        assert!(admin_saw_hidden, "{target} seeds never reach a hidden fact");
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fuzz.db");
    let clock = std::sync::Arc::new(weave_engine::ManualClock::new(0));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    common::seed(&mut engine, &clock);
    drop(engine);
    for (admin, seed, accepted) in [
        (
            false,
            include_str!("../seeds/mcp_jsonrpc/session.jsonl"),
            true,
        ),
        (
            true,
            include_str!("../seeds/mcp_jsonrpc/session.jsonl"),
            true,
        ),
        (
            false,
            include_str!("../seeds/mcp_jsonrpc/errors.jsonl"),
            false,
        ),
    ] {
        let principal = if admin {
            common::ADMIN
        } else {
            common::ANALYST
        };
        let session =
            weave_science::ScienceSession::open(&path, common::reader(principal)).unwrap();
        let mut server = weave_mcp::Server::new(session);
        let mut responses = 0;
        let mut hidden = false;
        for line in seed.lines() {
            let Some(response) = server.handle_message(line.as_bytes()) else {
                continue;
            };
            responses += 1;
            let failed = response.get("error").is_some()
                || response["result"]["isError"] == Value::Bool(true);
            assert_eq!(
                !failed, accepted,
                "mcp_jsonrpc seed line {line}: {response}"
            );
            hidden |= common::contains(response.to_string().as_bytes(), common::MARKER);
        }
        assert!(responses >= 7, "every request is answered");
        assert_eq!(
            hidden,
            admin && accepted,
            "only the admin sees hidden facts"
        );
    }
}

fn main() {
    check_phase_a_seeds();
    let host = HostContext::new("fuzz", ["fuzz".into()]);
    let mut engine = Engine::memory().unwrap();
    let plan: Program =
        serde_json::from_str(include_str!("../seeds/program_atomic/geometry.json")).unwrap();
    let output = engine
        .execute(&plan, &host)
        .expect("geometry seed executes full graph workflow");
    assert_eq!(output.len(), plan.commands.len());
    let capsule: Capsule =
        serde_json::from_str(include_str!("../seeds/capsule_receive/empty-graph.json")).unwrap();
    let mut receiver = Engine::memory().unwrap();
    assert_eq!(receiver.receive_capsule(&capsule, &host).unwrap(), 1);
    assert_eq!(receiver.receive_capsule(&capsule, &host).unwrap(), 0);
    assert!(receiver.events().unwrap().is_empty());
    let mut recorded = Engine::memory().unwrap();
    let plan: Program = serde_json::from_str(include_str!(
        "../seeds/program_atomic/recorded-rollback.json"
    ))
    .unwrap();
    assert!(recorded.execute(&plan, &host).is_err());
    assert_eq!(recorded.head("fuzz", "main").unwrap(), None);
    assert!(recorded.events().unwrap().is_empty());
    for seed in [
        include_str!("../seeds/program_atomic/accepted-rollback.json"),
        include_str!("../seeds/program_atomic/range-rollback.json"),
        include_str!("../seeds/program_atomic/accepted-range-rollback.json"),
    ] {
        let mut engine = Engine::memory().unwrap();
        let plan: Program = serde_json::from_str(seed).unwrap();
        assert!(engine.execute(&plan, &host).is_err());
        assert_eq!(engine.head("fuzz", "main").unwrap(), None);
        assert!(engine.events().unwrap().is_empty());
    }
    println!(
        "Valid seeds exercise full geometry, idempotent capsule import, recorded/accepted selection and range rollback, fact writes and rollback, A3 and A4 plans with hidden facts, and an MCP session"
    );
}
