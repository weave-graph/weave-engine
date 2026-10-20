//! The protocol 1.0.0 cut (A6) replaced every version-list gate with
//! `weave_contract::version_at_least(version, introduced)`. This table is the
//! list each gate had before, copied from the 0.24.0 source (where `VERSION`
//! was 0.24.0), and the test requires that every gate accepts exactly the same
//! versions as before, for every version in `SUPPORTED_VERSIONS`, with 1.0.0
//! and every later version accepted wherever the current version was. A
//! future version bump touches
//! the version table, not this list.
//!
//! The behavioral test below runs one program per gated feature under every
//! supported version against a real engine.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{version_at_least, Program, SUPPORTED_VERSIONS, VERSION};
use weave_engine::{Engine, HostContext, ManualClock};

struct Gate {
    site: &'static str,
    introduced: &'static str,
    old: &'static [&'static str],
    old_current: bool,
}

const GATES: &[Gate] = &[
    Gate {
        site: "handler_registration.rs:214",
        introduced: "0.18.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0",
        ],
        old_current: true,
    },
    Gate {
        site: "handler_registration.rs:271",
        introduced: "0.19.0",
        old: &["0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0"],
        old_current: true,
    },
    Gate {
        site: "handler_registration.rs:341",
        introduced: "0.19.0",
        old: &["0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0"],
        old_current: true,
    },
    Gate {
        site: "view_registration.rs:80",
        introduced: "0.16.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0",
        ],
        old_current: true,
    },
    Gate {
        site: "view_registration.rs:119",
        introduced: "0.20.0",
        old: &["0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0"],
        old_current: true,
    },
    Gate {
        site: "capsule_export.rs:276",
        introduced: "0.16.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0",
        ],
        old_current: true,
    },
    Gate {
        site: "capsule_export.rs:280",
        introduced: "0.19.0",
        old: &["0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0"],
        old_current: true,
    },
    Gate {
        site: "compiled_handlers.rs:371",
        introduced: "0.18.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0",
        ],
        old_current: true,
    },
    Gate {
        site: "compiled_handlers.rs:402",
        introduced: "0.19.0",
        old: &["0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs:334",
        introduced: "0.3.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
            "0.7.0", "0.6.0", "0.5.0", "0.4.0", "0.3.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:347",
        introduced: "0.4.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
            "0.7.0", "0.6.0", "0.5.0", "0.4.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:348",
        introduced: "0.5.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
            "0.7.0", "0.6.0", "0.5.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:369",
        introduced: "0.6.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
            "0.7.0", "0.6.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:388",
        introduced: "0.8.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:410",
        introduced: "0.9.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:431",
        introduced: "0.11.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:451",
        introduced: "0.12.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:469",
        introduced: "0.14.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:487",
        introduced: "0.15.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:505",
        introduced: "0.17.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:522",
        introduced: "0.19.0",
        old: &["0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs:564",
        introduced: "0.22.0",
        old: &["0.22.0", "0.23.0", "0.24.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs:589",
        introduced: "0.21.0",
        old: &["0.21.0", "0.22.0", "0.23.0", "0.24.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs:610",
        introduced: "0.21.0",
        old: &["0.21.0", "0.22.0", "0.23.0", "0.24.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs:627",
        introduced: "0.6.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
            "0.7.0", "0.6.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:700",
        introduced: "0.4.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
            "0.7.0", "0.6.0", "0.5.0", "0.4.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:799",
        introduced: "0.22.0",
        old: &["0.22.0", "0.23.0", "0.24.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3008",
        introduced: "0.23.0",
        old: &["0.23.0", "0.24.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3048",
        introduced: "0.21.0",
        old: &["0.21.0", "0.22.0", "0.23.0", "0.24.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3061",
        introduced: "0.20.0",
        old: &["0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3077",
        introduced: "0.19.0",
        old: &["0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3086",
        introduced: "0.19.0",
        old: &["0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3096",
        introduced: "0.16.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3109",
        introduced: "0.14.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3120",
        introduced: "0.13.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3133",
        introduced: "0.10.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3147",
        introduced: "0.9.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3163",
        introduced: "0.9.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3177",
        introduced: "0.8.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3192",
        introduced: "0.7.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
            "0.7.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3208",
        introduced: "0.5.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
            "0.7.0", "0.6.0", "0.5.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3221",
        introduced: "0.5.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
            "0.7.0", "0.6.0", "0.5.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3234",
        introduced: "0.6.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
            "0.7.0", "0.6.0",
        ],
        old_current: true,
    },
    Gate {
        site: "lib.rs:3246",
        introduced: "0.4.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
            "0.7.0", "0.6.0", "0.5.0", "0.4.0",
        ],
        old_current: true,
    },
    Gate {
        site: "projection_migration.rs:75",
        introduced: "0.21.0",
        old: &["0.21.0", "0.22.0", "0.23.0", "0.24.0"],
        old_current: true,
    },
    Gate {
        site: "projection_migration.rs:297",
        introduced: "0.21.0",
        old: &["0.21.0", "0.22.0", "0.23.0", "0.24.0"],
        old_current: true,
    },
    Gate {
        site: "recorded_actors.rs:140",
        introduced: "0.21.0",
        old: &["0.21.0", "0.22.0", "0.23.0", "0.24.0"],
        old_current: true,
    },
    Gate {
        site: "retained_cluster.rs:43",
        introduced: "0.19.0",
        old: &["0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0"],
        old_current: true,
    },
    Gate {
        site: "retained_cluster.rs:337",
        introduced: "0.19.0",
        old: &["0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0"],
        old_current: true,
    },
    // `program.version == LEGACY_VERSION` rejected join only under 0.1.0.
    Gate {
        site: "lib.rs: join",
        introduced: "0.2.0",
        old: &[
            "0.24.0", "0.23.0", "0.22.0", "0.21.0", "0.20.0", "0.19.0", "0.18.0", "0.17.0",
            "0.16.0", "0.15.0", "0.14.0", "0.13.0", "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0",
            "0.7.0", "0.6.0", "0.5.0", "0.4.0", "0.3.0", "0.2.0",
        ],
        old_current: true,
    },
    // Four `version != VERSION` gates (embedding writes, 0.23 change sets, seeds_from, nearest): only 0.24.0 had the feature.
    Gate {
        site: "lib.rs: embedding writes",
        introduced: "0.24.0",
        old: &["0.24.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs: change-set embedding kinds",
        introduced: "0.24.0",
        old: &["0.24.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs: expand seeds_from",
        introduced: "0.24.0",
        old: &["0.24.0"],
        old_current: true,
    },
    Gate {
        site: "lib.rs: nearest and similar_candidates",
        introduced: "0.24.0",
        old: &["0.24.0"],
        old_current: true,
    },
];

#[test]
fn every_gate_accepts_exactly_the_versions_it_accepted_before() {
    // `old_current` gates accepted the current version at the 1.0.0 cut, so
    // they accept 1.0.0 and every later version (1.1.0, B1).
    assert!(version_at_least(VERSION, "1.0.0"));
    for gate in GATES {
        for version in SUPPORTED_VERSIONS {
            let before = gate.old.contains(version)
                || (gate.old_current && version_at_least(version, "1.0.0"));
            assert_eq!(
                version_at_least(version, gate.introduced),
                before,
                "{} under {version}",
                gate.site
            );
        }
    }
    assert_eq!(GATES.len(), 54);
}

/// The capsule check `contract_version == "0.16.0"` became
/// `!version_at_least(v, "0.17.0")`. They differ only below 0.16.0, which the
/// clause before it in the same condition already rejects.
#[test]
fn the_capsule_exact_version_check_is_unchanged_where_it_applies() {
    for version in SUPPORTED_VERSIONS {
        if version_at_least(version, "0.16.0") {
            assert_eq!(*version == "0.16.0", !version_at_least(version, "0.17.0"));
        }
    }
}

fn run(version: &str, commands: Value, setup: &[Value]) -> Result<Vec<Value>, String> {
    let mut engine = Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap();
    let host = HostContext::new("alice", ["g".to_string()]);
    for step in setup {
        let program: Program =
            serde_json::from_value(json!({"version": VERSION, "commands": step})).unwrap();
        engine.execute(&program, &host).unwrap();
    }
    let program: Program =
        serde_json::from_value(json!({"version": version, "commands": commands})).unwrap();
    engine
        .execute(&program, &host)
        .map(|r| r.iter().map(|c| serde_json::to_value(c).unwrap()).collect())
        .map_err(|e| e.code)
}

#[test]
fn each_gated_feature_runs_exactly_from_its_version() {
    let node = |id: &str| json!({"kind": "node", "id": id, "entity_id": id.to_uppercase(), "space_id": "s"});
    let space = json!({"kind": "embedding_space", "id": "sp", "revision": "1", "encoder": "e",
        "encoder_version": "1", "preprocessing": "none", "dimensions": 2, "metric": "euclidean"});
    let setup = vec![json!([{"op": "write", "graph_id": "g", "changes": [
        {"change": "assert", "fact": node("a")}, {"change": "assert", "fact": node("b")},
        {"change": "assert", "fact": {"kind": "edge", "id": "e", "predicate": "p", "from": "a", "to": "b", "valid_time": {"start": 0}}},
        {"change": "assert", "fact": space},
        {"change": "assert", "fact": {"kind": "embedding", "id": "va", "space": {"id": "sp", "revision": "1"},
            "subject": {"kind": "node", "node_id": "a"}, "vector": [1.0, 0.0], "valid_time": {"start": 0}}}]}])];
    let features: Vec<(&str, &str, Value)> = vec![
        (
            "evaluate",
            "0.3.0",
            json!([{"op": "evaluate", "value": {"kind": "query", "query": {"graph_id": "g"}}}]),
        ),
        (
            "write",
            "0.22.0",
            json!([{"op": "write", "graph_id": "g", "changes": [{"change": "assert", "fact": node("c")}]}]),
        ),
        (
            "expand",
            "0.23.0",
            json!([{"op": "evaluate", "value": {"kind": "expand", "graph_id": "g", "seeds": ["a"], "hops": 1}}]),
        ),
        (
            "changes",
            "0.23.0",
            json!([{"op": "evaluate", "value": {"kind": "changes", "graph_id": "g", "from": {"kind": "as_of", "unix_millis": 0}}}]),
        ),
        (
            "nearest",
            "0.24.0",
            json!([{"op": "evaluate", "value": {"kind": "nearest", "graph_id": "g",
            "space": {"id": "sp", "revision": "1"}, "query": {"kind": "vector", "vector": [1.0, 0.0]}, "k": 1}}]),
        ),
        (
            "embedding write",
            "0.24.0",
            json!([{"op": "write", "graph_id": "g", "changes": [{"change": "assert", "fact":
            {"kind": "embedding", "id": "vb", "space": {"id": "sp", "revision": "1"},
             "subject": {"kind": "node", "node_id": "b"}, "vector": [0.0, 1.0], "valid_time": {"start": 0}}}]}]),
        ),
        (
            "seeds_from",
            "0.24.0",
            json!([{"op": "evaluate", "value": {"kind": "expand", "graph_id": "g", "seeds": [], "hops": 1,
            "seeds_from": {"kind": "nearest", "graph_id": "g", "space": {"id": "sp", "revision": "1"},
                "query": {"kind": "vector", "vector": [1.0, 0.0]}, "k": 1}}}]),
        ),
    ];
    for (name, introduced, commands) in &features {
        for version in SUPPORTED_VERSIONS {
            let outcome = run(version, commands.clone(), &setup);
            if version_at_least(version, introduced) {
                assert!(outcome.is_ok(), "{name} under {version}: {outcome:?}");
            } else {
                assert_eq!(outcome.unwrap_err(), "E_VERSION", "{name} under {version}");
            }
        }
    }
    // Results of 0.22.0 and later programs carry per-fact provenance; older ones do not.
    let read = json!([{"op": "query", "query": {"graph_id": "g"}}]);
    for version in SUPPORTED_VERSIONS {
        let result = &run(version, read.clone(), &setup).unwrap()[0]["result"];
        assert_eq!(
            result.get("fact_provenance").is_some(),
            version_at_least(version, "0.22.0"),
            "provenance under {version}"
        );
        assert_eq!(
            result["version"], VERSION,
            "results report the producing engine"
        );
    }
    // Change sets of programs older than 0.24.0 omit embedding kinds.
    let changes = json!([{"op": "evaluate", "value": {"kind": "changes", "graph_id": "g", "from": {"kind": "as_of", "unix_millis": 0}}}]);
    for version in SUPPORTED_VERSIONS
        .iter()
        .filter(|v| version_at_least(v, "0.23.0"))
    {
        let result = &run(version, changes.clone(), &setup).unwrap()[0]["result"];
        let kinds: Vec<&str> = result["changes"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["kind"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds.contains(&"embedding"),
            version_at_least(version, "0.24.0"),
            "change-set kinds under {version}: {kinds:?}"
        );
    }
}
