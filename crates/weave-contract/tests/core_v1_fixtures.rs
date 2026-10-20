//! Core contract v1 golden fixtures (docs/contract/core-v1.md).
//!
//! The fixtures under `fixtures/core-v1/` are language-neutral JSON. This
//! harness fails when the fixture set, a fixture's bytes, the contract types
//! or the stable error codes drift. It needs nothing outside this crate, so it
//! runs unchanged in a vendored copy (`cargo test --manifest-path
//! vendor/weave-contract/Cargo.toml --test core_v1_fixtures`).
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use weave_contract::embedding::{CandidateRelation, EmbeddingFact, EmbeddingSpace};
use weave_contract::provenance::{ClaimedProvenance, FactProvenance, ProvenanceEntry};
use weave_contract::wire::{self, CORE_PROFILE};
use weave_contract::{decode, decode_program, ContractError, GraphData, QueryResult, VERSION};

/// Every concept that core-v1.md defines normatively. Each needs a fixture.
const CONCEPTS: &[&str] = &[
    "identity.entity",
    "identity.node",
    "identity.edge",
    "identity.assertion",
    "identity.graph",
    "identity.revision",
    "fact.polarity",
    "fact.immutability",
    "fact.readers",
    "fact.write",
    "time.valid",
    "time.recorded",
    "time.as_of",
    "provenance.fact",
    "embedding.space",
    "embedding.fact",
    "embedding.candidate",
    "query.plan",
    "query.expression",
    "query.expand",
    "query.changes",
    "query.nearest",
    "query.similar_candidates",
    "result.envelope",
    "error.version",
    "error.unknown_field",
    "error.unknown_variant",
    "error.decode",
    "compat.versions",
    "compat.reserved",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/core-v1")
}

fn files(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = format!("{prefix}{}", entry.file_name().to_str().unwrap());
        if entry.file_type().unwrap().is_dir() {
            files(&entry.path(), &format!("{name}/"), out);
        } else if name != "manifest.json" {
            out.push(name);
        }
    }
}

fn fixtures() -> Vec<(String, Value)> {
    let mut names = vec![];
    files(&root(), "", &mut names);
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let bytes = fs::read(root().join(&name)).unwrap();
            let value = serde_json::from_slice(&bytes)
                .unwrap_or_else(|e| panic!("{name}: fixture file is not JSON: {e}"));
            (name, value)
        })
        .collect()
}

fn input_bytes(fixture: &Value) -> Vec<u8> {
    match (&fixture["input_text"], &fixture["input"]) {
        (Value::String(text), Value::Null) => text.as_bytes().to_vec(),
        (Value::Null, input) if !input.is_null() => serde_json::to_vec(input).unwrap(),
        _ => panic!("fixture needs exactly one of input or input_text"),
    }
}

fn canonical<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap()
}

/// Decode and validate one fixture input; return its canonical re-encoding.
fn check(fixture: &Value) -> Result<Value, ContractError> {
    let bytes = input_bytes(fixture);
    match fixture["type"].as_str().unwrap() {
        "program" => decode_program(&bytes).map(|v| canonical(&v)),
        "graph_data" => decode::<GraphData>(&bytes).map(|v| canonical(&v)),
        "query_result" => decode::<QueryResult>(&bytes).map(|v| canonical(&v)),
        "fact_provenance" => {
            let value: FactProvenance = decode(&bytes)?;
            value.validate()?;
            Ok(canonical(&value))
        }
        "claimed_provenance" => {
            let value: ClaimedProvenance = decode(&bytes)?;
            value.validate()?;
            Ok(canonical(&value))
        }
        "provenance_entry" => {
            let value: ProvenanceEntry = decode(&bytes)?;
            value.provenance.validate()?;
            Ok(canonical(&value))
        }
        "embedding_space" => {
            let value: EmbeddingSpace = decode(&bytes)?;
            value.validate()?;
            Ok(canonical(&value))
        }
        "embedding_space_pair" => {
            let [left, right]: [EmbeddingSpace; 2] = decode(&bytes)?;
            left.validate()?;
            right.validate()?;
            left.compatible(&right)?;
            Ok(canonical(&[left, right]))
        }
        "embedding_fact" => {
            let value: EmbeddingFact = decode(&bytes)?;
            let space: EmbeddingSpace = serde_json::from_value(fixture["space"].clone())
                .expect("embedding_fact fixtures name their space");
            value.validate(&space)?;
            Ok(canonical(&value))
        }
        "candidate_relation" => decode::<CandidateRelation>(&bytes).map(|v| canonical(&v)),
        other => panic!("unknown fixture type {other}"),
    }
}

#[test]
fn manifest_pins_the_exact_fixture_set_and_bytes() {
    let manifest: Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["format"], "weave-core-fixtures/1");
    assert_eq!(manifest["profile"], CORE_PROFILE);
    assert_eq!(manifest["protocol"], VERSION);
    let mut names = vec![];
    files(&root(), "", &mut names);
    let actual: BTreeMap<String, String> = names
        .into_iter()
        .map(|name| {
            let digest = Sha256::digest(fs::read(root().join(&name)).unwrap());
            let hex = digest.iter().map(|b| format!("{b:02x}")).collect();
            (name, hex)
        })
        .collect();
    let expected: BTreeMap<String, String> =
        serde_json::from_value(manifest["fixtures"].clone()).unwrap();
    assert!(
        actual == expected,
        "fixture drift: update fixtures/core-v1/manifest.json deliberately; actual set is\n{}",
        serde_json::to_string_pretty(&actual).unwrap()
    );
}

#[test]
fn fixtures_are_well_formed_and_cover_every_core_concept() {
    let mut covered = BTreeMap::<&str, usize>::new();
    for (name, fixture) in fixtures() {
        let stem = Path::new(&name).file_stem().unwrap().to_str().unwrap();
        assert_eq!(fixture["fixture"], stem, "{name}: fixture name");
        let negative = name.starts_with("negative/");
        assert!(negative || name.starts_with("positive/"), "{name}");
        let rejects = fixture["expect"].get("reject").is_some();
        let engine_rejects = fixture["engine"].get("error").is_some();
        assert_eq!(negative, rejects || engine_rejects, "{name}: directory");
        if !rejects {
            assert_eq!(fixture["expect"], json!({"accept": true}), "{name}");
        }
        let concepts = fixture["concepts"].as_array().unwrap();
        assert!(!concepts.is_empty(), "{name}: concepts");
        for concept in concepts {
            let concept = concept.as_str().unwrap();
            let known = CONCEPTS.iter().find(|c| **c == concept);
            *covered
                .entry(known.unwrap_or_else(|| panic!("{name}: unknown concept {concept}")))
                .or_default() += 1;
        }
    }
    for concept in CONCEPTS {
        assert!(covered.contains_key(concept), "no fixture for {concept}");
    }
}

#[test]
fn positive_fixtures_decode_and_reencode_canonically() {
    for (name, fixture) in fixtures() {
        if fixture["expect"].get("reject").is_some() {
            continue;
        }
        let encoded = check(&fixture).unwrap_or_else(|e| panic!("{name}: {e}"));
        // `canonical`, when present, pins the defaults a minimal `input` decodes to.
        let expected = match &fixture["canonical"] {
            Value::Null => &fixture["input"],
            canonical => canonical,
        };
        assert!(
            encoded == *expected,
            "{name}: canonical encoding drifted; contract now encodes\n{}",
            serde_json::to_string_pretty(&encoded).unwrap()
        );
    }
}

#[test]
fn negative_fixtures_fail_with_their_stable_code() {
    for (name, fixture) in fixtures() {
        let Some(code) = fixture["expect"].get("reject") else {
            continue;
        };
        match check(&fixture) {
            Ok(_) => panic!("{name}: accepted, expected {code}"),
            Err(error) => assert_eq!(error.code, code.as_str().unwrap(), "{name}: {error}"),
        }
    }
}

#[test]
fn unknown_version_fails_with_e_version_before_structure() {
    for version in [
        "0.25.0",
        "1.1.0",
        "2.0.0",
        "1.0",
        "0.21",
        "",
        "0.21.0-rc1",
        "1.0.0-rc1",
    ] {
        let program = json!({"version": version, "commands": [], "unknown": true});
        let error = decode_program(&serde_json::to_vec(&program).unwrap()).unwrap_err();
        assert_eq!(error.code, wire::E_VERSION, "{version}");
    }
    for program in [
        json!({"commands": []}),
        json!({"version": 21, "commands": []}),
    ] {
        let error = decode_program(&serde_json::to_vec(&program).unwrap()).unwrap_err();
        assert_eq!(error.code, wire::E_VERSION);
    }
    for version in wire::SUPPORTED_VERSIONS {
        let program = json!({"version": version, "commands": []});
        decode_program(&serde_json::to_vec(&program).unwrap()).unwrap();
    }
    assert_eq!(wire::SUPPORTED_VERSIONS.len(), 25);
    assert_eq!(wire::SUPPORTED_VERSIONS[0], VERSION);
}

#[test]
fn unknown_fields_fail_with_e_unknown_field_at_every_depth() {
    let programs = [
        json!({"version": VERSION, "commands": [], "extra": 1}),
        json!({"version": VERSION, "commands": [{"op": "query", "query": {"graph_id": "g", "extra": 1}}]}),
        json!({"version": VERSION, "commands": [{"op": "commit", "graph_id": "g", "extra": 1, "data": {}}]}),
        json!({"version": VERSION, "commands": [{"op": "commit", "graph_id": "g", "data": {"nodes": [
            {"id": "a", "entity_id": "A", "space_id": "s", "reader_groups": ["team"]}
        ]}}]}),
        json!({"version": VERSION, "commands": [{"op": "evaluate", "value": {"kind": "filter",
            "input": {"kind": "query", "query": {"graph_id": "g"}}, "as_of": 5}}]}),
        json!({"version": VERSION, "commands": [{"op": "evaluate", "value": {"kind": "expand",
            "graph_id": "g", "seeds": ["a"], "hops": 1, "depth": 2}}]}),
        json!({"version": VERSION, "commands": [{"op": "evaluate", "value": {"kind": "changes",
            "graph_id": "g", "from": {"kind": "cursor", "revision": "r", "at": 1}}}]}),
        json!({"version": VERSION, "commands": [{"op": "evaluate", "value": {"kind": "nearest",
            "graph_id": "g", "space": {"id": "s", "revision": "1"}, "k": 1,
            "query": {"kind": "vector", "vector": [1.0]}, "mode": "approximate"}}]}),
        json!({"version": VERSION, "commands": [{"op": "write", "graph_id": "g", "changes": [
            {"change": "assert", "fact": {"kind": "embedding", "id": "v", "space": {"id": "s", "revision": "1"},
                "subject": {"kind": "node", "node_id": "a"}, "vector": [1.0], "valid_time": {"start": 0},
                "reader_groups": ["team"]}}]}]}),
    ];

    for program in programs {
        let error = decode_program(&serde_json::to_vec(&program).unwrap()).unwrap_err();
        assert_eq!(error.code, wire::E_UNKNOWN_FIELD, "{program}: {error}");
    }
    let forged = json!({"method": {"kind": "human"}, "recorded_at_ms": 0});
    let error = decode::<ClaimedProvenance>(&serde_json::to_vec(&forged).unwrap()).unwrap_err();
    assert_eq!(error.code, wire::E_UNKNOWN_FIELD);
}

#[test]
fn unknown_variants_and_malformed_input_have_distinct_codes() {
    let variant = json!({"version": VERSION, "commands": [{"op": "expand"}]});
    let error = decode_program(&serde_json::to_vec(&variant).unwrap()).unwrap_err();
    assert_eq!(error.code, wire::E_UNKNOWN_VARIANT);
    for bytes in [&b"[]"[..], b"null", b"{", b"\xff"] {
        assert_eq!(decode_program(bytes).unwrap_err().code, wire::E_DECODE);
    }
    let large = vec![b' '; wire::MAX_DECODE_BYTES + 1];
    assert_eq!(decode_program(&large).unwrap_err().code, wire::E_BUDGET);
}

#[test]
fn engine_recorded_provenance_replaces_absent_claims_with_unspecified() {
    let recorded = FactProvenance::record("alice".into(), 10, None);
    assert_eq!(
        canonical(&recorded),
        json!({"writer": "alice", "recorded_at_ms": 10, "method": {"kind": "unspecified"}})
    );
}
