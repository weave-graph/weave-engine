//! Store 32 → 33 (B5, #12): review policies, proposals, votes and decisions
//! live in five new append-only tables. The upgrade only creates them; no
//! existing row is rewritten. The marker moves because a store-32 binary
//! would ignore review policies and let writers publish directly to a
//! reviewed graph. Genuine old binaries and real process kills are covered
//! by `scripts/check_review_store_migration.py`.
use rusqlite::Connection;
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{FactChange, Program, VERSION};
use weave_engine::{
    Engine, HostContext, ManualClock, RegistryChange, ReviewMode, ReviewVerdict, ReviewVote,
    STORAGE_VERSION,
};

const REVIEW_TABLES: [&str; 5] = [
    "review_policies",
    "review_proposals",
    "review_versions",
    "review_votes",
    "review_decisions",
];

fn host(who: &str) -> HostContext {
    HostContext::new(who, Vec::<String>::new())
}
fn program(version: &str, commands: Value) -> Program {
    serde_json::from_value(json!({"version": version, "commands": commands})).unwrap()
}
/// A registry store as a store-32 binary leaves it.
fn populate(e: &mut Engine, clock: &ManualClock) -> Vec<String> {
    clock.set(10);
    e.enable_registry(&host("root")).unwrap();
    let changes: Vec<RegistryChange> = serde_json::from_value(json!([
        {"change": "add_principal", "id": "alice", "kind": "user"},
        {"change": "add_principal", "id": "bob", "kind": "user"},
        {"change": "add_principal", "id": "team", "kind": "group"},
        {"change": "add_member", "group": "team", "member": "alice"},
        {"change": "add_member", "group": "team", "member": "bob"},
        {"change": "grant", "graph_id": "kb", "subject": "team", "permission": "write"}
    ]))
    .unwrap();
    clock.set(20);
    e.apply_registry(&changes, &host("root")).unwrap();
    let mut revisions = vec![];
    for (time, commands) in [
        (
            30,
            json!([{"op": "write", "graph_id": "kb", "changes": [
            {"change": "assert", "fact": {"kind": "node", "id": "a", "entity_id": "A", "space_id": "s"}},
            {"change": "assert", "fact": {"kind": "node", "id": "b", "entity_id": "B", "space_id": "s",
                "reader_groups": ["team"]}}]}]),
        ),
        (
            40,
            json!([{"op": "write", "graph_id": "kb", "changes": [
            {"change": "assert", "fact": {"kind": "edge", "id": "e", "predicate": "p", "from": "a",
                "to": "b", "valid_time": {"start": 0}, "readers": ["bob"]}}]}]),
        ),
    ] {
        clock.set(time);
        let out = serde_json::to_value(
            e.execute(&program("1.2.0", commands), &host("alice"))
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
            let query = json!([{"op": "query", "query": {"graph_id": "kb", "revision": revision}}]);
            let result = e
                .execute(&program("1.2.0", query), &host(principal))
                .unwrap();
            out.push(serde_json::to_string(&result).unwrap());
        }
    }
    out
}
type Rows = (i64, Vec<String>, Vec<Vec<String>>);
fn rows(path: &std::path::Path) -> Rows {
    let db = Connection::open(path).unwrap();
    let marker = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    let mut tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'review_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    tables.sort();
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
    (marker, tables, all)
}
/// What a store-32 binary leaves: no review tables, marker 32.
fn downgrade_to_32(path: &std::path::Path) {
    let db = Connection::open(path).unwrap();
    for table in REVIEW_TABLES {
        db.execute_batch(&format!("DROP TABLE {table}")).unwrap();
    }
    db.pragma_update(None, "user_version", 32).unwrap();
}

#[test]
fn populated_store_32_upgrades_without_rewriting_rows_and_takes_review() {
    assert_eq!(STORAGE_VERSION, 33);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let revisions = populate(&mut e, &clock);
    let before = pinned(&mut e, &revisions);
    drop(e);
    downgrade_to_32(&path);
    let old = rows(&path);
    assert_eq!(old.0, 32);
    assert!(old.1.is_empty());
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let new = rows(&path);
    assert_eq!(new.0, 33);
    let mut expected: Vec<String> = REVIEW_TABLES.iter().map(|t| t.to_string()).collect();
    expected.sort();
    assert_eq!(new.1, expected);
    assert_eq!(new.2, old.2, "no row changes");
    assert_eq!(pinned(&mut e, &revisions), before, "pinned reads unchanged");
    // Review works on the migrated line: the graph becomes reviewed, a
    // direct write is refused, and an accepted proposal publishes.
    clock.set(50);
    let mode = ReviewMode::Reviewed {
        reviewers: vec!["team".into()],
        threshold: 1,
    };
    e.set_review_policy("kb", &mode, 0, &host("root")).unwrap();
    clock.set(60);
    let write = json!([{"op": "write", "graph_id": "kb", "changes": [
        {"change": "assert", "fact": {"kind": "node", "id": "c", "entity_id": "C", "space_id": "s"}}]}]);
    assert_eq!(
        e.execute(&program(VERSION, write), &host("alice"))
            .unwrap_err()
            .code,
        "E_REVIEW_REQUIRED"
    );
    let changes: Vec<FactChange> = serde_json::from_value(json!([{"change": "assert",
        "fact": {"kind": "node", "id": "c", "entity_id": "C", "space_id": "s"}}]))
    .unwrap();
    let receipt = e
        .propose_review("kb", "main", &changes, &host("alice"))
        .unwrap();
    clock.set(70);
    let accepted = e
        .vote_review(
            &ReviewVote {
                proposal_id: receipt.proposal_id,
                digest: receipt.digest,
                verdict: ReviewVerdict::Approve,
                note: None,
            },
            &host("bob"),
        )
        .unwrap();
    let revision = accepted.decision.unwrap().revision.unwrap();
    assert_eq!(e.head("kb", "main").unwrap(), Some(revision));
    drop(e);
    assert_eq!(rows(&path).0, 33);
}

#[cfg(feature = "recovery-testing")]
#[test]
fn interrupted_upgrade_leaves_store_32_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let revisions = populate(&mut e, &clock);
    let before = pinned(&mut e, &revisions);
    drop(e);
    downgrade_to_32(&path);
    let old = rows(&path);
    let crashed = std::panic::catch_unwind(|| {
        let _ = Engine::open_test_before_schema_commit(&path, || panic!("killed before commit"));
    });
    assert!(crashed.is_err());
    assert_eq!(rows(&path), old, "nothing of the upgrade is visible");
    let mut e = Engine::open_with_clock(&path, clock).unwrap();
    assert_eq!(rows(&path).0, 33);
    assert_eq!(pinned(&mut e, &revisions), before);
}

#[test]
fn review_records_are_append_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    populate(&mut e, &clock);
    clock.set(50);
    let mode = ReviewMode::Reviewed {
        reviewers: vec!["team".into()],
        threshold: 1,
    };
    e.set_review_policy("kb", &mode, 0, &host("root")).unwrap();
    drop(e);
    let db = Connection::open(&path).unwrap();
    for statement in [
        "UPDATE review_policies SET writer='mallory'",
        "DELETE FROM review_policies",
    ] {
        let error = db.execute_batch(statement).unwrap_err().to_string();
        assert!(error.contains("immutable"), "{statement}: {error}");
    }
}
