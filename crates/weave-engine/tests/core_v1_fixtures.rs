//! Runs the core contract v1 golden fixtures against the engine and checks
//! that `docs/contract/core-v1.md` names every fixture and concept.
//!
//! The contract crate's own harness checks decoding. This one checks the
//! fixtures' `engine` expectations: observable results of executing each
//! program on a fresh in-memory store with a manual trusted clock.
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::Arc;
use weave_contract::{decode_program, CommandResult, Coverage, QueryResult};
use weave_engine::{Engine, HostContext, ManualClock};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../weave-contract/fixtures/core-v1")
}

fn fixtures() -> Vec<(String, Value)> {
    let mut out = vec![];
    for dir in ["positive", "negative"] {
        let mut names: Vec<_> = fs::read_dir(root().join(dir))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        for name in names {
            let path = format!("{dir}/{name}");
            let value = serde_json::from_slice(&fs::read(root().join(&path)).unwrap()).unwrap();
            out.push((path, value));
        }
    }
    out
}

fn host(step: &Value) -> HostContext {
    let graphs: Vec<String> = step["write_graphs"]
        .as_array()
        .map(|g| g.iter().map(|g| g.as_str().unwrap().to_string()).collect())
        .unwrap_or_default();
    HostContext::new(step["principal"].as_str().unwrap(), graphs)
}

fn bytes(fixture: &Value) -> Vec<u8> {
    match &fixture["input_text"] {
        Value::String(text) => text.as_bytes().to_vec(),
        _ => serde_json::to_vec(&fixture["input"]).unwrap(),
    }
}

fn ids<'a>(values: impl Iterator<Item = &'a String>) -> Value {
    let set: BTreeSet<_> = values.collect();
    serde_json::to_value(set).unwrap()
}

fn compare_query(name: &str, index: usize, expected: &Value, result: &QueryResult) {
    let coverage = match result.coverage {
        Coverage::Complete => "complete",
        Coverage::Partial => "partial",
    };
    assert_eq!(expected["coverage"], coverage, "{name} result {index}");
    if !expected["nodes"].is_null() {
        let actual = ids(result.graph.nodes.iter().map(|n| &n.id));
        assert_eq!(expected["nodes"], actual, "{name} result {index} nodes");
    }
    if !expected["edges"].is_null() {
        let actual = ids(result.graph.edges.iter().map(|e| &e.id));
        assert_eq!(expected["edges"], actual, "{name} result {index} edges");
    }
    if let Some(properties) = expected["edge_properties"].as_object() {
        for (id, want) in properties {
            let edge = result.graph.edges.iter().find(|e| &e.id == id).unwrap();
            assert_eq!(
                serde_json::to_value(&edge.properties).unwrap(),
                *want,
                "{name} result {index} edge {id}"
            );
        }
    }
    if let Some(changes) = expected["changes"].as_array() {
        let entries = &result.changes.as_ref().expect("a change set").entries;
        let actual: BTreeSet<String> = entries
            .iter()
            .map(|e| {
                let v = serde_json::to_value(e).unwrap();
                serde_json::json!([v["change"], v["kind"], v["id"]]).to_string()
            })
            .collect();
        let want: BTreeSet<String> = changes.iter().map(Value::to_string).collect();
        assert_eq!(actual, want, "{name} result {index} changes");
    }
    if let Some(neighbors) = expected["neighbors"].as_array() {
        let actual: Vec<&str> = result
            .neighbors
            .iter()
            .map(|n| n.embedding_id.as_str())
            .collect();
        let want: Vec<&str> = neighbors.iter().map(|n| n.as_str().unwrap()).collect();
        assert_eq!(
            actual, want,
            "{name} result {index} neighbors in rank order"
        );
    }
    if let Some(candidates) = expected["candidates"].as_array() {
        let actual: Vec<Value> = result
            .candidates
            .iter()
            .map(|c| serde_json::json!([c.from, c.to]))
            .collect();
        assert_eq!(&actual, candidates, "{name} result {index} candidates");
    }
    if !expected["exactness"].is_null() {
        assert_eq!(
            serde_json::to_value(result.exactness).unwrap(),
            expected["exactness"],
            "{name} result {index} exactness"
        );
    }
    if let Some(count) = expected["recorded_observations"].as_u64() {
        assert_eq!(result.recorded_observations.len() as u64, count, "{name}");
    }
    let provenance = &expected["fact_provenance"];
    if !provenance.is_null() {
        let entries: Vec<Value> = result
            .fact_provenance
            .iter()
            .map(|p| serde_json::to_value(&p.provenance).unwrap())
            .collect();
        assert_eq!(
            provenance["count"],
            entries.len(),
            "{name} provenance count"
        );
        let writers: BTreeSet<String> = entries
            .iter()
            .map(|p| p["writer"].as_str().unwrap().to_string())
            .collect();
        let methods: BTreeSet<String> = entries
            .iter()
            .map(|p| p["method"]["kind"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            provenance["writers"],
            serde_json::to_value(writers).unwrap(),
            "{name}"
        );
        assert_eq!(
            provenance["methods"],
            serde_json::to_value(methods).unwrap(),
            "{name}"
        );
        if let Some(at) = provenance["recorded_at_ms"].as_i64() {
            assert!(entries.iter().all(|p| p["recorded_at_ms"] == at), "{name}");
        }
    }
}

#[test]
fn fixtures_execute_with_their_golden_engine_outcome() {
    let mut executed = 0;
    for (name, fixture) in fixtures() {
        let engine_spec = &fixture["engine"];
        if engine_spec.is_null() {
            continue;
        }
        let clock = Arc::new(ManualClock::new(1));
        let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
        for step in engine_spec["setup"].as_array().into_iter().flatten() {
            if let Some(ms) = step["clock_ms"].as_i64() {
                clock.set(ms);
            }
            let program = decode_program(&serde_json::to_vec(&step["program"]).unwrap())
                .unwrap_or_else(|e| panic!("{name}: setup: {e}"));
            engine
                .execute(&program, &host(step))
                .unwrap_or_else(|e| panic!("{name}: setup: {e}"));
        }
        if let Some(ms) = engine_spec["clock_ms"].as_i64() {
            clock.set(ms);
        }
        let outcome = decode_program(&bytes(&fixture))
            .map_err(|e| e.code)
            .and_then(|program| {
                engine
                    .execute(&program, &host(engine_spec))
                    .map_err(|e| e.code)
            });
        executed += 1;
        if let Some(code) = engine_spec["error"].as_str() {
            assert_eq!(outcome.err().as_deref(), Some(code), "{name}");
            continue;
        }
        let results = outcome.unwrap_or_else(|code| panic!("{name}: {code}"));
        let expected = engine_spec["results"].as_array().unwrap();
        assert_eq!(results.len(), expected.len(), "{name}: result count");
        for (index, (want, got)) in expected.iter().zip(&results).enumerate() {
            let encoded = serde_json::to_value(got).unwrap();
            assert_eq!(want["kind"], encoded["kind"], "{name} result {index}");
            if !want["revision"].is_null() {
                // Revision IDs are content-derived (ADR-0001), so they are golden.
                assert_eq!(
                    want["revision"], encoded["revision"],
                    "{name} result {index}"
                );
            }
            if let CommandResult::Queried { result } = got {
                compare_query(&name, index, want, result);
            }
        }
    }
    assert!(executed >= 10, "engine fixtures missing");
}

#[test]
fn core_v1_spec_names_every_fixture_and_concept() {
    let spec_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/contract/core-v1.md");
    let spec = fs::read_to_string(spec_path).unwrap();
    let mut concepts = BTreeSet::new();
    for (name, fixture) in fixtures() {
        assert!(
            spec.contains(&name),
            "core-v1.md does not cite fixture {name}"
        );
        for concept in fixture["concepts"].as_array().unwrap() {
            concepts.insert(concept.as_str().unwrap().to_string());
        }
    }
    for concept in concepts {
        assert!(
            spec.contains(&format!("`{concept}`")),
            "core-v1.md does not define concept {concept}"
        );
    }
}

fn run_cli(program: &str) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let plan = dir.path().join("plan.json");
    fs::write(&plan, program).unwrap();
    let output = Process::new(env!("CARGO_BIN_EXE_weave-engine"))
        .arg("run")
        .arg("--db")
        .arg(dir.path().join("store.db"))
        .arg("--actor")
        .arg("alice")
        .arg(&plan)
        .output()
        .unwrap();
    assert!(!output.status.success());
    serde_json::from_slice(&output.stderr).unwrap()
}

#[test]
fn engine_cli_reports_stable_codes_for_unknown_versions_and_fields() {
    let version = run_cli(r#"{"version":"0.25.0","commands":[]}"#);
    assert_eq!(version["code"], "E_VERSION");
    let field = run_cli(r#"{"version":"0.21.0","commands":[],"comment":"x"}"#);
    assert_eq!(field["code"], "E_UNKNOWN_FIELD");
    let variant = run_cli(r#"{"version":"0.21.0","commands":[{"op":"assert"}]}"#);
    assert_eq!(variant["code"], "E_UNKNOWN_VARIANT");
}

#[test]
fn engine_rejects_every_unsupported_version_and_accepts_every_supported_one() {
    let mut engine = Engine::memory().unwrap();
    let host = HostContext::new("alice", Vec::<String>::new());
    for version in weave_contract::SUPPORTED_VERSIONS {
        let program = weave_contract::Program {
            source_revisions: vec![],
            version: version.to_string(),
            commands: vec![],
        };
        engine.execute(&program, &host).unwrap();
    }
    for version in ["0.25.0", "1.0.0", "0.0.0"] {
        let program = weave_contract::Program {
            source_revisions: vec![],
            version: version.into(),
            commands: vec![],
        };
        assert_eq!(
            engine.execute(&program, &host).unwrap_err().code,
            "E_VERSION"
        );
    }
}
