//! Independent integrity, migration rollback and SQLite backup acceptance.
use rusqlite::{params, Connection};
use serde_json::json;
use sha2::{Digest, Sha256};
use weave_contract::{GraphData, Program, QueryPlan, VERSION};
use weave_engine::{Engine, HostContext};
fn host() -> HostContext {
    HostContext::new("owner", ["g".into()])
}
fn data() -> GraphData {
    serde_json::from_value(json!({"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"e","predicate":"p","from":"a","to":"b","valid_time":{"start":0},"properties":{"value":1}}]})).unwrap()
}
fn query() -> QueryPlan {
    serde_json::from_value(json!({"graph_id":"g"})).unwrap()
}
fn commit(e: &mut Engine, batch: bool) {
    let cmd = if batch {
        json!({"op":"commit_batch","batch_id":"batch","commits":[{"graph_id":"g","data":data()}]})
    } else {
        json!({"op":"commit","graph_id":"g","data":data()})
    };
    let program: Program =
        serde_json::from_value(json!({"version":VERSION,"commands":[cmd]})).unwrap();
    e.execute(&program, &host()).unwrap();
}
#[test]
fn modified_content_and_parent_fail_for_hashed_and_logical_revisions() {
    for batch in [false, true] {
        for column in ["data", "parent"] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("store.db");
            let mut e = Engine::open(&path).unwrap();
            commit(&mut e, batch);
            assert_eq!(e.query(&query(), &host()).unwrap().graph.edges.len(), 1);
            let c = Connection::open(&path).unwrap();
            if column == "data" {
                let mut corrupt = data();
                corrupt.edges[0].properties.insert("value".into(), json!(2));
                c.execute(
                    "UPDATE revisions SET data=?1",
                    [serde_json::to_string(&corrupt).unwrap()],
                )
                .unwrap();
            } else {
                c.execute("UPDATE revisions SET parent='invented-parent'", [])
                    .unwrap();
            }
            assert_eq!(e.query(&query(), &host()).unwrap_err().code, "E_INTEGRITY");
            assert_eq!(e.event_count().unwrap(), 1);
        }
    }
}
#[test]
fn logical_manifest_and_integrity_index_are_required_and_bound() {
    for target in ["manifest", "index"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("store.db");
        let mut e = Engine::open(&path).unwrap();
        commit(&mut e, true);
        let c = Connection::open(&path).unwrap();
        if target == "manifest" {
            let s: String = c
                .query_row("SELECT manifest FROM snapshot_manifests", [], |r| r.get(0))
                .unwrap();
            let mut v: serde_json::Value = serde_json::from_str(&s).unwrap();
            v["members"][0]["branch_id"] = json!("tampered");
            c.execute("UPDATE snapshot_manifests SET manifest=?1", [v.to_string()])
                .unwrap();
        } else {
            c.execute("DELETE FROM revision_integrity", []).unwrap();
        }
        assert_eq!(e.query(&query(), &host()).unwrap_err().code, "E_INTEGRITY");
    }
}
#[test]
fn future_schema_is_refused_without_creating_tables_or_changing_version() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("future.db");
    let c = Connection::open(&path).unwrap();
    c.pragma_update(None, "user_version", 999).unwrap();
    let error = match Engine::open(&path) {
        Ok(_) => panic!("future version accepted"),
        Err(e) => e,
    };
    assert_eq!(error.code, "E_STORAGE_VERSION");
    assert_eq!(
        c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        999
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn failed_legacy_backfill_rolls_back_all_schema_and_identity_changes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("legacy.db");
    let c = Connection::open(&path).unwrap();
    c.execute_batch("CREATE TABLE revisions(revision TEXT PRIMARY KEY,graph_id TEXT NOT NULL,branch_id TEXT NOT NULL,parent TEXT,recorded_at INTEGER NOT NULL,data TEXT NOT NULL);").unwrap();
    let original = data();
    let mut conflicting = original.clone();
    conflicting.edges[0].from = "b".into();
    conflicting.edges[0].to = "a".into();
    for d in [original, conflicting] {
        let revision = format!(
            "sha256:{:x}",
            Sha256::digest(
                serde_json::to_vec(&("weave-revision-v0.1", "g", "main", None::<String>, &d))
                    .unwrap()
            )
        );
        c.execute(
            "INSERT INTO revisions VALUES (?1,'g','main',NULL,0,?2)",
            params![revision, serde_json::to_string(&d).unwrap()],
        )
        .unwrap();
    }
    for _ in 0..2 {
        let error = match Engine::open(&path) {
            Ok(_) => panic!("identity rebind accepted"),
            Err(e) => e,
        };
        assert_eq!(error.code, "E_EDGE_IDENTITY");
        assert_eq!(
            c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM revisions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}
#[test]
fn sqlite_backup_restores_same_pins_results_events_and_independent_future() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("active.db");
    let backup = temp.path().join("backup.db");
    let mut e = Engine::open(&path).unwrap();
    commit(&mut e, true);
    let before = e.query(&query(), &host()).unwrap();
    let events = e.events().unwrap();
    // SQLite snapshots committed WAL content; copying only the main file is insufficient.
    Connection::open(&path)
        .unwrap()
        .execute("VACUUM INTO ?1", [backup.to_str().unwrap()])
        .unwrap();
    let mut restored = Engine::open(&backup).unwrap();
    assert_eq!(restored.query(&query(), &host()).unwrap(), before);
    assert_eq!(
        serde_json::to_value(restored.events().unwrap()).unwrap(),
        serde_json::to_value(events).unwrap()
    );
    let mut changed = data();
    changed.edges[0].properties.insert("value".into(), json!(3));
    let program = serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"g","expected_head":restored.head("g","main").unwrap(),"data":changed}]})).unwrap();
    restored.execute(&program, &host()).unwrap();
    assert_eq!(e.query(&query(), &host()).unwrap(), before);
    assert_ne!(
        restored.head("g", "main").unwrap(),
        e.head("g", "main").unwrap()
    );
}

#[test]
fn legacy_corruption_cannot_poison_backfill_and_repaired_fixture_upgrades() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("old.db");
    let mut e = Engine::open(&path).unwrap();
    commit(&mut e, false);
    let expected = e.query(&query(), &host()).unwrap();
    drop(e);
    let c = Connection::open(&path).unwrap();
    let original: String = c
        .query_row("SELECT data FROM revisions", [], |r| r.get(0))
        .unwrap();
    c.execute("DELETE FROM edge_structures", []).unwrap();
    c.pragma_update(None, "user_version", 5).unwrap();
    let mut bad = data();
    bad.edges[0].predicate = "poison".into();
    c.execute(
        "UPDATE revisions SET data=?1",
        [serde_json::to_string(&bad).unwrap()],
    )
    .unwrap();
    let error = match Engine::open(&path) {
        Ok(_) => panic!("corrupt migration accepted"),
        Err(e) => e,
    };
    assert_eq!(error.code, "E_INTEGRITY");
    assert_eq!(
        c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        5
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM edge_structures", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    c.execute("UPDATE revisions SET data=?1", [original])
        .unwrap();
    let upgraded = Engine::open(&path).unwrap();
    assert_eq!(upgraded.query(&query(), &host()).unwrap(), expected);
    assert_eq!(
        c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        16
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM edge_structures", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn oversized_corrupt_cell_is_rejected_by_bounded_sql_extraction() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("oversized.db");
    let mut e = Engine::open(&path).unwrap();
    commit(&mut e, false);
    let c = Connection::open(&path).unwrap();
    c.execute("UPDATE revisions SET data=zeroblob(16777217)", [])
        .unwrap();
    assert_eq!(e.query(&query(), &host()).unwrap_err().code, "E_INTEGRITY");
}
