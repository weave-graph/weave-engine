//! Store 30 → 31 (A4, ADR-0006): embedding spaces and embeddings become
//! fact kinds of the existing fact log. No table changes and no row is
//! rewritten; the marker moves so that a store-30 binary refuses a store
//! that may hold kinds it cannot read. Genuine old binaries and real process
//! kills are covered by `scripts/check_embedding_store_migration.py` in CI.
use rusqlite::Connection;
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{Program, VERSION};
use weave_engine::{Engine, HostContext, ManualClock, STORAGE_VERSION};

fn host() -> HostContext {
    HostContext::new("alice", ["g".to_string()])
}
fn program(version: &str, commands: Value) -> Program {
    serde_json::from_value(json!({"version": version, "commands": commands})).unwrap()
}
/// A store as a store-30 binary leaves it: snapshot and fact revisions.
fn populate(e: &mut Engine) -> Vec<String> {
    let mut revisions = vec![];
    for commands in [
        json!([{"op": "commit", "graph_id": "g", "data": {"nodes": [{"id": "a", "entity_id": "A", "space_id": "s"}]}}]),
        json!([{"op": "write", "graph_id": "g", "changes": [
            {"change": "assert", "fact": {"kind": "node", "id": "b", "entity_id": "B", "space_id": "s", "readers": ["bob"]}},
            {"change": "assert", "fact": {"kind": "edge", "id": "e", "predicate": "p", "from": "a", "to": "b", "valid_time": {"start": 0}}}]}]),
    ] {
        let out = serde_json::to_value(e.execute(&program("0.23.0", commands), &host()).unwrap())
            .unwrap();
        revisions.push(out[0]["revision"].as_str().unwrap().to_string());
    }
    revisions
}
fn pinned(e: &mut Engine, revisions: &[String]) -> Vec<String> {
    let mut out = vec![];
    for revision in revisions {
        for principal in ["alice", "bob"] {
            let query = json!([{"op": "query", "query": {"graph_id": "g", "revision": revision}}]);
            let result = e
                .execute(
                    &program("0.23.0", query),
                    &HostContext::new(principal, Vec::<String>::new()),
                )
                .unwrap();
            out.push(serde_json::to_string(&result).unwrap());
        }
    }
    out
}
type Rows = (i64, Vec<Vec<String>>, Vec<Vec<String>>, Vec<Vec<String>>);
fn rows(path: &std::path::Path) -> Rows {
    let db = Connection::open(path).unwrap();
    let marker = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    let dump = |sql: &str| -> Vec<Vec<String>> {
        let mut statement = db.prepare(sql).unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |r| {
                (0..columns)
                    .map(|i| {
                        r.get::<_, rusqlite::types::Value>(i)
                            .map(|v| format!("{v:?}"))
                    })
                    .collect()
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    (
        marker,
        dump("SELECT * FROM revisions ORDER BY revision"),
        dump("SELECT * FROM fact_log ORDER BY position"),
        dump("SELECT * FROM fact_index ORDER BY position"),
    )
}
fn downgrade_to_30(path: &std::path::Path) {
    Connection::open(path)
        .unwrap()
        .pragma_update(None, "user_version", 30)
        .unwrap();
}

#[test]
fn populated_store_30_upgrades_without_rewriting_facts() {
    assert_eq!(STORAGE_VERSION, 31);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let revisions = populate(&mut e);
    let before = pinned(&mut e, &revisions);
    drop(e);
    downgrade_to_30(&path);
    let old = rows(&path);
    assert_eq!(old.0, 30);
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let new = rows(&path);
    assert_eq!(new.0, 31);
    assert_eq!(
        (&new.1, &new.2, &new.3),
        (&old.1, &old.2, &old.3),
        "no row changes"
    );
    assert_eq!(pinned(&mut e, &revisions), before, "pinned reads unchanged");
    // The upgraded store takes embedding facts on top of its fact line.
    clock.set(20);
    let write = json!([{"op": "write", "graph_id": "g", "changes": [
        {"change": "assert", "fact": {"kind": "embedding_space", "id": "sp", "revision": "1", "encoder": "e",
            "encoder_version": "v", "preprocessing": "p", "dimensions": 2, "metric": "euclidean"}},
        {"change": "assert", "fact": {"kind": "embedding", "id": "va", "space": {"id": "sp", "revision": "1"},
            "subject": {"kind": "node", "node_id": "a"}, "vector": [1.0, 2.0], "valid_time": {"start": 0}}}]}]);
    e.execute(&program(VERSION, write), &host()).unwrap();
    let nearest = json!([{"op": "evaluate", "value": {"kind": "nearest", "graph_id": "g",
        "space": {"id": "sp", "revision": "1"}, "query": {"kind": "vector", "vector": [1.0, 2.0]}, "k": 1}}]);
    let out =
        serde_json::to_value(e.execute(&program(VERSION, nearest), &host()).unwrap()).unwrap();
    assert_eq!(out[0]["result"]["neighbors"][0]["embedding_id"], "va");
    assert_eq!(pinned(&mut e, &revisions), before);
    drop(e);
    // A store-30 binary refuses the store from here on (it refuses any marker
    // above 30 the same way, see `fact_store_migration.rs`).
    assert_eq!(rows(&path).0, 31);
}

#[cfg(feature = "recovery-testing")]
#[test]
fn interrupted_upgrade_leaves_store_30_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let mut e = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    let revisions = populate(&mut e);
    let before = pinned(&mut e, &revisions);
    drop(e);
    downgrade_to_30(&path);
    let old = rows(&path);
    let crashed = std::panic::catch_unwind(|| {
        let _ = Engine::open_test_before_schema_commit(&path, || panic!("killed before commit"));
    });
    assert!(crashed.is_err());
    assert_eq!(rows(&path), old, "nothing of the upgrade is visible");
    let mut e = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    assert_eq!(rows(&path).0, 31);
    assert_eq!(pinned(&mut e, &revisions), before);
}

#[test]
fn a_store_31_binary_refuses_newer_stores() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("future.db");
    drop(Engine::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", STORAGE_VERSION + 1)
        .unwrap();
    assert_eq!(Engine::open(&path).err().unwrap().code, "E_STORAGE_VERSION");
}
