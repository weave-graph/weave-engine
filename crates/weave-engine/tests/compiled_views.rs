use serde_json::json;
use std::sync::Arc;
use weave_contract::{view_registration::*, *};
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("alice", ["g".into(), "marker".into()])
}
fn template(clock: ViewClock) -> CompiledViewTemplate {
    seal_template(CompiledViewTemplate {
        format: VIEW_TEMPLATE_FORMAT.into(),
        protocol: VERSION.into(),
        name: "Active".into(),
        revision: "1".into(),
        expression: serde_json::from_value(
            json!({"kind":"query","query":{"graph_id":"g","valid_at":3}}),
        )
        .unwrap(),
        clock,
        source_revisions: vec![],
        definition_digest: String::new(),
    })
    .unwrap()
}
fn commit(e: &mut Engine, data: GraphData) {
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "g".into(),
                branch_id: "main".into(),
                expected_head: e.head("g", "main").unwrap(),
                data,
            }],
        },
        &host(),
    )
    .unwrap();
}
fn selection(t: &CompiledViewTemplate, time: ViewReadTime) -> CurrentViewSelection {
    CurrentViewSelection {
        view_id: "v".into(),
        definition_digest: t.definition_digest.clone(),
        time,
    }
}
#[test]
fn explicit_clock_policy_and_manifest_conflict_share_one_operation_and_rollback() {
    let clock = Arc::new(ManualClock::new(100));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    commit(&mut e, GraphData::default());
    let t = template(ViewClock::Tick);
    e.register_compiled_view("v", &t, Some(2), &host()).unwrap();
    let prior = clock.samples();
    assert!(
        e.read_current_view(&selection(&t, ViewReadTime::Tick { valid_at: 2 }), &host())
            .unwrap()
            .current
    );
    assert_eq!(clock.samples(), prior + 1);
    assert_eq!(
        e.read_current_view(&selection(&t, ViewReadTime::Fixed), &host())
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
    assert_eq!(
        e.read_current_view(&selection(&t, ViewReadTime::Tick { valid_at: 3 }), &host())
            .unwrap_err()
            .code,
        "E_FRESHNESS"
    );
    let mut conflict = t.source_revisions.clone();
    conflict[0].digest = format!("sha256:{}", "f".repeat(64));
    let program = Program {
        version: VERSION.into(),
        source_revisions: conflict,
        commands: vec![
            Command::Commit {
                graph_id: "marker".into(),
                branch_id: "main".into(),
                expected_head: None,
                data: GraphData::default(),
            },
            Command::Evaluate {
                value: GraphExpression::CurrentView {
                    selection: selection(&t, ViewReadTime::Tick { valid_at: 2 }),
                },
            },
        ],
    };
    let prior = clock.samples();
    assert_eq!(
        e.execute(&program, &host()).unwrap_err().code,
        "E_SOURCE_REVISION"
    );
    assert_eq!(clock.samples(), prior + 1);
    assert!(e.head("marker", "main").unwrap().is_none());
    clock.set(-1);
    assert_eq!(
        e.read_current_view(&selection(&t, ViewReadTime::Tick { valid_at: 2 }), &host())
            .unwrap_err()
            .code,
        "E_CLOCK_UNAVAILABLE"
    );
}
#[test]
fn source_identity_survives_kernel_fallback_and_scheduling_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.db");
    let mut e = Engine::open(&path).unwrap();
    commit(&mut e, GraphData::default());
    let t = template(ViewClock::Fixed);
    e.register_compiled_view("v", &t, None, &host()).unwrap();
    e.enroll_incremental_view("v", &host()).unwrap();
    e.enable_view_schedule("v", &host()).unwrap();
    let (_, work) = e.refresh_view_with_work("v", None, &host()).unwrap();
    assert_eq!(work.oracle_disagreements, 0);
    // An ordinary literal attachment intentionally leaves the membership kernel profile.
    let data=serde_json::from_value(json!({"attachments":[{"id":"a","host":{"kind":"graph"},"key":"literal","value":{"kind":"literal","value":"v"},"valid_time":{"start":0}}]})).unwrap();
    commit(&mut e, data);
    let (out, work) = e.refresh_view_with_work("v", None, &host()).unwrap();
    assert_eq!(work.fallback_runs, 1);
    assert_eq!(out.result.source_revisions, t.source_revisions);
    e.drain_view_work(&host()).unwrap().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let text: String = db
        .query_row(
            "SELECT processed_manifest FROM view_schedules WHERE id='v'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let processed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(processed["definition"].as_str().is_some());
    // Definition fingerprint differs when the exact compiled source binding differs.
    let mut second = template(ViewClock::Fixed);
    second.name = "Other".into();
    second.source_revisions.clear();
    let second = seal_template(second).unwrap();
    e.register_compiled_view("other", &second, None, &host())
        .unwrap();
    e.enroll_incremental_view("other", &host()).unwrap();
    e.enable_view_schedule("other", &host()).unwrap();
    e.drain_view_work(&host()).unwrap().unwrap();
    let text: String = db
        .query_row(
            "SELECT processed_manifest FROM view_schedules WHERE id='other'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let other: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_ne!(other["definition"], processed["definition"]);
}
#[test]
fn binding_corruption_fails_closed_and_registration_storage_failure_rolls_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.db");
    let mut e = Engine::open(&path).unwrap();
    commit(&mut e, GraphData::default());
    let t = template(ViewClock::Fixed);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_source BEFORE INSERT ON view_sources BEGIN SELECT RAISE(ABORT,'test rejection'); END;").unwrap();
    assert!(e.register_compiled_view("v", &t, None, &host()).is_err());
    for table in [
        "live_views",
        "view_sources",
        "live_view_changes",
        "view_dependencies",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    db.execute_batch("DROP TRIGGER fail_source").unwrap();
    e.register_compiled_view("v", &t, None, &host()).unwrap();
    db.execute("DELETE FROM view_sources WHERE id='v'", [])
        .unwrap();
    assert_eq!(
        e.read_view("v", None, ViewFreshness::AllowStale, &host())
            .unwrap_err()
            .code,
        "E_INTEGRITY"
    );
    assert_eq!(
        e.refresh_view("v", None, &host()).unwrap_err().code,
        "E_INTEGRITY"
    );
    assert_eq!(
        e.read_current_view(&selection(&t, ViewReadTime::Fixed), &host())
            .unwrap_err()
            .code,
        "E_INTEGRITY"
    );
}

#[test]
fn stored_definition_cannot_redirect_binding_to_another_instance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.db");
    let mut e = Engine::open(&path).unwrap();
    commit(&mut e, GraphData::default());
    let t = template(ViewClock::Fixed);
    for id in ["v", "other"] {
        e.register_compiled_view(id, &t, None, &host()).unwrap();
    }
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("UPDATE live_views SET definition=(SELECT definition FROM live_views WHERE id='other') WHERE id='v'",[]).unwrap();
    assert_eq!(
        e.read_current_view(&selection(&t, ViewReadTime::Fixed), &host())
            .unwrap_err()
            .code,
        "E_INTEGRITY"
    );
    assert_eq!(
        e.read_view("v", None, ViewFreshness::AllowStale, &host())
            .unwrap_err()
            .code,
        "E_INTEGRITY"
    );
    assert_eq!(
        e.refresh_view("v", None, &host()).unwrap_err().code,
        "E_INTEGRITY"
    );
    assert!(
        e.read_view("other", None, ViewFreshness::RequireCurrent, &host())
            .unwrap()
            .current
    );
}
