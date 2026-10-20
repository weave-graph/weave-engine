//! Store 31 → 32 (B1, ADR-0007): the principal registry lives as facts in
//! `weave:registry`, and fact records may carry `reader_groups`. No table
//! changes and no row is rewritten; the marker moves so that a store-31
//! binary, which knows neither groups nor graph ACLs, refuses a store whose
//! records it would authorize too widely. Genuine old binaries and real
//! process kills are covered by `scripts/check_registry_store_migration.py`.
use rusqlite::Connection;
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{Program, VERSION};
use weave_engine::{Engine, HostContext, ManualClock, RegistryChange, STORAGE_VERSION};

fn host(who: &str) -> HostContext {
    HostContext::new(who, ["g".to_string()])
}
fn program(version: &str, commands: Value) -> Program {
    serde_json::from_value(json!({"version": version, "commands": commands})).unwrap()
}
/// A store as a store-31 binary leaves it: snapshot, fact and embedding revisions.
fn populate(e: &mut Engine) -> Vec<String> {
    let mut revisions = vec![];
    for commands in [
        json!([{"op": "commit", "graph_id": "g", "data": {"nodes": [{"id": "a", "entity_id": "A", "space_id": "s"}]}}]),
        json!([{"op": "write", "graph_id": "g", "changes": [
            {"change": "assert", "fact": {"kind": "node", "id": "b", "entity_id": "B", "space_id": "s", "readers": ["bob"]}},
            {"change": "assert", "fact": {"kind": "edge", "id": "e", "predicate": "p", "from": "a", "to": "b", "valid_time": {"start": 0}}},
            {"change": "assert", "fact": {"kind": "embedding_space", "id": "sp", "revision": "1", "encoder": "e",
                "encoder_version": "v", "preprocessing": "p", "dimensions": 2, "metric": "euclidean"}},
            {"change": "assert", "fact": {"kind": "embedding", "id": "va", "space": {"id": "sp", "revision": "1"},
                "subject": {"kind": "node", "node_id": "a"}, "vector": [1.0, 2.0], "valid_time": {"start": 0}}}]}]),
    ] {
        let out = serde_json::to_value(
            e.execute(&program("1.0.0", commands), &host("alice"))
                .unwrap(),
        )
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
                    &program("1.0.0", query),
                    &HostContext::new(principal, Vec::<String>::new()),
                )
                .unwrap();
            out.push(serde_json::to_string(&result).unwrap());
        }
    }
    out
}
type Rows = (i64, Vec<Vec<String>>);
fn rows(path: &std::path::Path) -> Rows {
    let db = Connection::open(path).unwrap();
    let marker = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    let mut all = vec![];
    for table in [
        "revisions",
        "heads",
        "events",
        "fact_lines",
        "fact_revisions",
        "fact_log",
        "fact_index",
    ] {
        let mut statement = db.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let columns = statement.column_count();
        let mut table_rows: Vec<String> = statement
            .query_map([], |r| {
                Ok((0..columns)
                    .map(|i| format!("{:?}", r.get::<_, rusqlite::types::Value>(i).unwrap()))
                    .collect::<Vec<_>>()
                    .join("|"))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        table_rows.sort();
        all.push(table_rows);
    }
    (marker, all)
}
fn downgrade_to_31(path: &std::path::Path) {
    Connection::open(path)
        .unwrap()
        .pragma_update(None, "user_version", 31)
        .unwrap();
}

#[test]
fn populated_store_31_upgrades_without_rewriting_rows_and_takes_a_registry() {
    assert_eq!(STORAGE_VERSION, 32);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let revisions = populate(&mut e);
    let before = pinned(&mut e, &revisions);
    drop(e);
    downgrade_to_31(&path);
    let old = rows(&path);
    assert_eq!(old.0, 31);
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let new = rows(&path);
    assert_eq!(new.0, 32);
    assert_eq!(new.1, old.1, "no row changes");
    assert_eq!(pinned(&mut e, &revisions), before, "pinned reads unchanged");
    // The upgraded store enables the registry and takes group readers on its line.
    clock.set(20);
    e.enable_registry(&HostContext::new("root", Vec::<String>::new()))
        .unwrap();
    let changes: Vec<RegistryChange> = serde_json::from_value(json!([
        {"change": "add_principal", "id": "alice", "kind": "user"},
        {"change": "add_principal", "id": "bob", "kind": "user"},
        {"change": "add_principal", "id": "team", "kind": "group"},
        {"change": "add_member", "group": "team", "member": "bob"},
        {"change": "grant", "graph_id": "g", "subject": "alice", "permission": "write"},
        {"change": "grant", "graph_id": "g", "subject": "team", "permission": "read"}
    ]))
    .unwrap();
    clock.set(30);
    e.apply_registry(&changes, &HostContext::new("root", Vec::<String>::new()))
        .unwrap();
    clock.set(40);
    let write = json!([{"op": "write", "graph_id": "g", "changes": [
        {"change": "assert", "fact": {"kind": "node", "id": "t", "entity_id": "T", "space_id": "s", "reader_groups": ["team"]}}]}]);
    e.execute(&program(VERSION, write), &host("root")).unwrap();
    let nodes = |e: &mut Engine, who: &str| -> Vec<String> {
        let out = e
            .execute(
                &program(
                    VERSION,
                    json!([{"op": "query", "query": {"graph_id": "g"}}]),
                ),
                &host(who),
            )
            .unwrap();
        let v = serde_json::to_value(out).unwrap();
        let mut ids: Vec<String> = v[0]["result"]["graph"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap().into())
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(nodes(&mut e, "alice"), ["a"]);
    assert_eq!(nodes(&mut e, "bob"), ["a", "b", "t"]);
    drop(e);
    assert_eq!(rows(&path).0, 32);
}

#[cfg(feature = "recovery-testing")]
#[test]
fn interrupted_upgrade_leaves_store_31_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let mut e = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    let revisions = populate(&mut e);
    let before = pinned(&mut e, &revisions);
    drop(e);
    downgrade_to_31(&path);
    let old = rows(&path);
    let crashed = std::panic::catch_unwind(|| {
        let _ = Engine::open_test_before_schema_commit(&path, || panic!("killed before commit"));
    });
    assert!(crashed.is_err());
    assert_eq!(rows(&path), old, "nothing of the upgrade is visible");
    let mut e = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    assert_eq!(rows(&path).0, 32);
    assert_eq!(pinned(&mut e, &revisions), before);
}

#[test]
fn a_store_32_binary_refuses_newer_stores() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("future.db");
    drop(Engine::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", STORAGE_VERSION + 1)
        .unwrap();
    assert_eq!(Engine::open(&path).err().unwrap().code, "E_STORAGE_VERSION");
}
