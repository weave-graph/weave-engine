use serde_json::{json, Value};
use weave_contract::Program;
use weave_engine::{AdapterManifest, Engine, HostContext, SubscriptionScope};
use weave_native::{
    artifacts::ArtifactBundle,
    cluster_journal::ClusterJournal,
    host::{HostReply, HostSession},
};
fn value(reply: HostReply) -> Value {
    let v: Value = serde_json::from_slice(&reply.bytes).unwrap();
    assert!(v["ok"].as_bool().unwrap(), "{v}");
    v["value"].clone()
}
fn call(session: &mut HostSession, operation: Value) -> Value {
    value(
        session.call(
            &serde_json::to_vec(&json!({"format":"weave-host-request/1","operation":operation}))
                .unwrap(),
        ),
    )
}
fn error(reply: HostReply) -> String {
    let v: Value = serde_json::from_slice(&reply.bytes).unwrap();
    assert_eq!(v["ok"], false, "{v}");
    v["error"]["code"].as_str().unwrap().into()
}
fn artifact(program: &Program) -> ArtifactBundle {
    let artifacts = json!({"program":program,"values":{"exact":{"kind":"integer","value":9007199254740993i64}},"view_templates":{}});
    let fp = weave_contract::identity::source_fingerprint(
        &json!({"profile":"weave-compiled-artifacts-v1","artifacts":artifacts}),
    )
    .unwrap();
    ArtifactBundle::parse(&serde_json::to_vec(&json!({"format":"weave-compiler-response/1","ok":true,"artifacts":artifacts,"artifact_fingerprint":fp})).unwrap()).unwrap()
}
fn setup(path: &std::path::Path) -> (HostSession, Value, Program) {
    let mut session = HostSession::new(
        Engine::open(path).unwrap(),
        HostContext::new("alice", ["Input".into(), "Output".into()]),
    )
    .unwrap();
    let result = call(
        &mut session,
        json!({"kind":"execute","program":{"version":weave_contract::VERSION,"commands":[{"op":"commit","graph_id":"Input","data":{"nodes":[{"id":"a","entity_id":"a","space_id":"s"},{"id":"b","entity_id":"b","space_id":"s"}],"edges":[{"id":"e","from":"a","to":"b","predicate":"p","valid_time":{"start":0,"end":10}}]}}]}}),
    );
    let revision = result[0]["revision"].as_str().unwrap();
    let manifest = AdapterManifest {
        id: "cluster".into(),
        version: "1".into(),
        artifact_digest: format!("sha256:{}", "a".repeat(64)),
        config_revision: "1".into(),
        principal: "alice".into(),
        subscriptions: vec![SubscriptionScope {
            graph_id: "Input".into(),
            branch_id: "main".into(),
        }],
        output_graphs: vec!["Output".into()],
        effect_destinations: vec![],
        max_attempts: 10,
        lease_ms: 60000,
        max_pending_events: 10,
        projection_replay: false,
    };
    value(session.install_cluster_adapter(&manifest));
    value(session.set_adapter_state("cluster", "running"));
    let event = call(&mut session, json!({"kind":"poll","adapter":"cluster"}));
    let recipe=serde_json::from_value(json!({"version":weave_contract::VERSION,"commands":[{"op":"bind","name":"Clusters","value":{"kind":"cluster","selection":{"source":{"graph_id":"Input","revision":revision},"context":{"kind":"default"},"valid_at":5,"predicate":"p","levels":2}}}]})).unwrap();
    (session, event, recipe)
}
#[test]
fn immutable_journal_reopens_replays_and_detects_missing_history() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("engine.db");
    let path = temp.path().join("journal.db");
    let (mut session, event, recipe) = setup(&db);
    let bundle = artifact(&recipe);
    let mut journal = ClusterJournal::open(&path, "phone", &session, true).unwrap();
    assert!(ClusterJournal::open(&path, "phone", &session, false).is_err());
    let prepared = value(journal.prepare(
        &mut session,
        "cluster",
        event["id"].as_str().unwrap(),
        event["lease"].as_str().unwrap(),
        &bundle,
        "main",
    ));
    let id = prepared["record_id"].as_str().unwrap();
    assert_eq!(
        value(journal.prepare(
            &mut session,
            "cluster",
            event["id"].as_str().unwrap(),
            event["lease"].as_str().unwrap(),
            &bundle,
            "main"
        ))["record_id"],
        id
    );
    assert_eq!(
        value(journal.complete(&mut session, id, event["lease"].as_str().unwrap()))["duplicate"],
        false
    );
    drop(journal);
    drop(session);
    let mut session = HostSession::new(
        Engine::open(&db).unwrap(),
        HostContext::new("alice", ["Input".into(), "Output".into()]),
    )
    .unwrap();
    let mut journal = ClusterJournal::open(&path, "phone", &session, false).unwrap();
    assert_eq!(
        value(journal.complete(&mut session, id, "expired-lease"))["duplicate"],
        true
    );
    drop(journal);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute("DELETE FROM retained", []).unwrap();
    drop(connection);
    let mut journal = ClusterJournal::open(&path, "phone", &session, false).unwrap();
    assert_eq!(
        error(journal.prepare(
            &mut session,
            "cluster",
            event["id"].as_str().unwrap(),
            event["lease"].as_str().unwrap(),
            &bundle,
            "main"
        )),
        "E_HOST_JOURNAL_MISSING"
    );
}
#[test]
fn store_copy_and_altered_record_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("engine.db");
    let path = temp.path().join("journal.db");
    let (mut session, event, recipe) = setup(&db);
    let mut journal = ClusterJournal::open(&path, "phone", &session, true).unwrap();
    let prepared = value(journal.prepare(
        &mut session,
        "cluster",
        event["id"].as_str().unwrap(),
        event["lease"].as_str().unwrap(),
        &artifact(&recipe),
        "main",
    ));
    let id = prepared["record_id"].as_str().unwrap();
    drop(journal);
    assert!(ClusterJournal::open(&path, "another-store", &session, false).is_err());
    let other = HostSession::new(
        Engine::memory().unwrap(),
        HostContext::new("alice", ["Input".into(), "Output".into()]),
    )
    .unwrap();
    assert!(ClusterJournal::open(&path, "phone", &other, false).is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("UPDATE retained SET body=x'7b7d' WHERE id=?1", [id])
        .unwrap();
    drop(connection);
    let mut journal = ClusterJournal::open(&path, "phone", &session, false).unwrap();
    assert_eq!(
        error(journal.complete(&mut session, id, event["lease"].as_str().unwrap())),
        "E_HOST_JOURNAL_INTEGRITY"
    );
}

#[test]
fn journal_capacity_rejects_new_work_and_oversized_records_without_truncation() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("engine.db");
    let path = temp.path().join("journal.db");
    let (mut session, event, recipe) = setup(&db);
    let mut journal = ClusterJournal::open(&path, "phone", &session, true).unwrap();
    let record = value(journal.prepare(
        &mut session,
        "cluster",
        event["id"].as_str().unwrap(),
        event["lease"].as_str().unwrap(),
        &artifact(&recipe),
        "main",
    ));
    let id = record["record_id"].as_str().unwrap();
    value(journal.complete(&mut session, id, event["lease"].as_str().unwrap()));
    drop(journal);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let original: Vec<u8> = connection
        .query_row("SELECT body FROM retained WHERE id=?1", [id], |r| r.get(0))
        .unwrap();
    connection
        .execute(
            "UPDATE retained SET body=zeroblob(2097153) WHERE id=?1",
            [id],
        )
        .unwrap();
    drop(connection);
    let mut journal = ClusterJournal::open(&path, "phone", &session, false).unwrap();
    assert_eq!(
        error(journal.complete(&mut session, id, "expired")),
        "E_HOST_JOURNAL_BUDGET"
    );
    drop(journal);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE retained SET body=?2 WHERE id=?1",
            rusqlite::params![id, original],
        )
        .unwrap();
    for i in 0..15 {
        connection
            .execute(
                "INSERT INTO retained VALUES(?1,?2,?3,x'00',x'00')",
                rusqlite::params![
                    format!("quota-{i}"),
                    format!("quota-{i}"),
                    format!("event-{i}")
                ],
            )
            .unwrap();
    }
    drop(connection);
    let old = event["graph"]["revision"].as_str().unwrap();
    call(
        &mut session,
        json!({"kind":"execute","program":{"version":weave_contract::VERSION,"commands":[{"op":"commit","graph_id":"Input","expected_head":old,"data":{"nodes":[{"id":"a","entity_id":"a","space_id":"s","properties":{"changed":true}},{"id":"b","entity_id":"b","space_id":"s"}],"edges":[{"id":"e","from":"a","to":"b","predicate":"p","valid_time":{"start":0,"end":10}}]}}]}}),
    );
    let next = call(&mut session, json!({"kind":"poll","adapter":"cluster"}));
    let mut changed = serde_json::to_value(&recipe).unwrap();
    changed["commands"][0]["value"]["selection"]["source"] = next["graph"].clone();
    let mut journal = ClusterJournal::open(&path, "phone", &session, false).unwrap();
    assert_eq!(
        error(journal.prepare(
            &mut session,
            "cluster",
            next["id"].as_str().unwrap(),
            next["lease"].as_str().unwrap(),
            &artifact(&serde_json::from_value(changed).unwrap()),
            "main"
        )),
        "E_HOST_JOURNAL_BUDGET"
    );
    drop(journal);
    drop(session);
    let connection = rusqlite::Connection::open(&db).unwrap();
    let pending: String = connection
        .query_row(
            "SELECT event_id FROM dispatch_pending WHERE adapter='cluster'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, next["id"].as_str().unwrap());
    let receipts: i64 = connection
        .query_row(
            "SELECT count(*) FROM handler_receipts WHERE adapter='cluster'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(receipts, 1);
}
