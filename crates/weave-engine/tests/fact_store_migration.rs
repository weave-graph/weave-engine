//! Store 29 → 30 (ADR-0004 section 10): the fact store is added empty and
//! nothing existing is rewritten. Genuine old binaries and real process kills
//! are covered by `scripts/check_fact_store_migration.py` in CI.
use rusqlite::Connection;
use serde_json::{json, Value};
use weave_contract::Program;
use weave_engine::{Engine, HostContext, STORAGE_VERSION};

const FACT_TABLES: [&str; 5] = [
    "fact_lines",
    "fact_revisions",
    "fact_log",
    "fact_index",
    "fact_checkpoints",
];

fn host() -> HostContext {
    HostContext::new("alice", ["g".to_string(), "h".to_string()])
}
fn program(version: &str, commands: Value) -> Program {
    serde_json::from_value(json!({"version": version, "commands": commands})).unwrap()
}
fn populate(e: &mut Engine) -> Vec<String> {
    let data = json!({"nodes": [{"id": "a", "entity_id": "A", "space_id": "s"}, {"id": "b", "entity_id": "B", "space_id": "s", "readers": ["bob"]}],
        "edges": [{"id": "e", "predicate": "p", "from": "a", "to": "b", "valid_time": {"start": 0, "end": 9}}]});
    let mut revisions = vec![];
    for commands in [
        json!([{"op": "commit", "graph_id": "g", "data": data}]),
        json!([{"op": "commit_batch", "batch_id": "b", "commits": [{"graph_id": "h", "data": {"nodes": [{"id": "x", "entity_id": "X", "space_id": "s"}]}}]}]),
    ] {
        let out = serde_json::to_value(e.execute(&program("0.21.0", commands), &host()).unwrap())
            .unwrap();
        let revision = out[0]["revision"]
            .as_str()
            .or(out[0]["commits"][0]["revision"].as_str());
        revisions.push(revision.unwrap().to_string());
    }
    revisions
}
fn pinned(e: &mut Engine, revisions: &[String]) -> Vec<String> {
    let mut out = vec![];
    for (graph, revision) in [("g", &revisions[0]), ("h", &revisions[1])] {
        for principal in ["alice", "bob"] {
            let query = json!([{"op": "query", "query": {"graph_id": graph, "revision": revision, "valid_at": 3}}]);
            let result = e
                .execute(
                    &program("0.21.0", query),
                    &HostContext::new(principal, Vec::<String>::new()),
                )
                .unwrap();
            out.push(serde_json::to_string(&result).unwrap());
        }
    }
    out
}
fn downgrade_to_29(path: &std::path::Path) {
    let db = Connection::open(path).unwrap();
    for table in FACT_TABLES.iter().rev() {
        db.execute(&format!("DROP TABLE {table}"), []).unwrap();
    }
    db.pragma_update(None, "user_version", 29).unwrap();
}
fn snapshot(path: &std::path::Path) -> (i64, Vec<String>, Vec<(String, String)>) {
    let db = Connection::open(path).unwrap();
    let marker = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    let tables = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'fact_%' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let revisions = db
        .prepare("SELECT revision,data FROM revisions ORDER BY revision")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    (marker, tables, revisions)
}

#[test]
fn populated_store_29_upgrades_without_rewriting_revisions() {
    assert_eq!(STORAGE_VERSION, 30);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let mut e = Engine::open(&path).unwrap();
    let revisions = populate(&mut e);
    let before = pinned(&mut e, &revisions);
    drop(e);
    downgrade_to_29(&path);
    let (marker, tables, rows) = snapshot(&path);
    assert_eq!((marker, tables.len()), (29, 0));
    let mut e = Engine::open(&path).unwrap();
    let (marker, tables, after_rows) = snapshot(&path);
    assert_eq!(marker, 30);
    let mut expected: Vec<String> = FACT_TABLES.iter().map(|t| t.to_string()).collect();
    expected.sort();
    assert_eq!(tables, expected);
    assert_eq!(
        after_rows, rows,
        "every pre-migration revision row is unchanged"
    );
    assert_eq!(
        pinned(&mut e, &revisions),
        before,
        "pinned results are byte-identical"
    );
    // A fact write on top of a migrated snapshot head works.
    let write = json!([{"op": "write", "graph_id": "g", "changes": [{"change": "assert", "fact": {"kind": "node", "id": "c", "entity_id": "C", "space_id": "s"}}]}]);
    e.execute(&program(weave_contract::VERSION, write), &host())
        .unwrap();
    assert_eq!(pinned(&mut e, &revisions), before);
}

#[cfg(feature = "recovery-testing")]
#[test]
fn interrupted_upgrade_leaves_store_29_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let mut e = Engine::open(&path).unwrap();
    let revisions = populate(&mut e);
    let before = pinned(&mut e, &revisions);
    drop(e);
    downgrade_to_29(&path);
    let old = snapshot(&path);
    let crashed = std::panic::catch_unwind(|| {
        let _ = Engine::open_test_before_schema_commit(&path, || panic!("killed before commit"));
    });
    assert!(crashed.is_err());
    assert_eq!(snapshot(&path), old, "nothing of the upgrade is visible");
    let mut e = Engine::open(&path).unwrap();
    assert_eq!(snapshot(&path).0, 30);
    assert_eq!(pinned(&mut e, &revisions), before);
}

#[test]
fn partial_or_downgraded_fact_schema_and_future_stores_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    // A store-29 marker with fact tables is a downgrade, not a legacy store.
    let path = dir.path().join("downgraded.db");
    drop(Engine::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", 29)
        .unwrap();
    assert_eq!(Engine::open(&path).err().unwrap().code, "E_INTEGRITY");
    // A store-30 marker missing a fact table is incomplete.
    let path = dir.path().join("partial.db");
    drop(Engine::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute("DROP TABLE fact_checkpoints", [])
        .unwrap();
    assert_eq!(Engine::open(&path).err().unwrap().code, "E_INTEGRITY");
    // A newer store is refused (the store-29 binary refuses store 30 the same way).
    let path = dir.path().join("future.db");
    drop(Engine::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", STORAGE_VERSION + 1)
        .unwrap();
    assert_eq!(Engine::open(&path).err().unwrap().code, "E_STORAGE_VERSION");
}
