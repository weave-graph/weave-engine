//! Independent persisted lineage, feedback, privacy and atomicity oracles.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use weave_contract::{CommandResult, Program};
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("owner", ["a".into(), "b".into(), "private".into()])
}
fn data(value: i64, reader: &str) -> Value {
    json!({"nodes":[{"id":"n","entity_id":"e","space_id":"s","properties":{"value":value},"readers":[reader]}]})
}
fn program(e: &Engine, graph: &str, value: i64, reader: &str) -> Program {
    serde_json::from_value(json!({"version":weave_contract::VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":e.head(graph,"main").unwrap(),"data":data(value,reader)}]})).unwrap()
}
fn write(e: &mut Engine, graph: &str, value: i64, reader: &str) {
    e.execute(&program(e, graph, value, reader), &host())
        .unwrap();
}
fn install(e: &Engine, id: &str, input: &str, output: &str, depth: u32) {
    let manifest = manifest(id, input, output);
    e.install_adapter(&manifest, &host()).unwrap();
    e.set_causal_dispatch_policy_for(id, &CausalDispatchPolicy { max_depth: depth }, &host())
        .unwrap();
    e.set_adapter_state_for(id, "running", &host()).unwrap();
}
fn manifest(id: &str, input: &str, output: &str) -> AdapterManifest {
    serde_json::from_value(json!({"id":id,"version":"1","artifact_digest":format!("sha256:{}","a".repeat(64)),"config_revision":"1","principal":"owner","subscriptions":[{"graph_id":input,"branch_id":"main"}],"output_graphs":[output],"effect_destinations":["sink"],"max_attempts":3,"lease_ms":1000,"max_pending_events":1000,"projection_replay":false})).unwrap()
}
#[test]
fn corrupt_policy_storage_cannot_leave_a_partially_installed_registration() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("install.sqlite");
    let e = Engine::open(&path).unwrap();
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute("DROP TABLE dispatch_causal_policies", [])
        .unwrap();
    assert!(e
        .install_adapter(&manifest("A", "a", "b"), &host())
        .is_err());
    assert_eq!(
        c.query_row("SELECT count(*) FROM dispatch_adapters", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(e);
    assert_eq!(
        Engine::open(&path).err().unwrap().code,
        "E_CAUSAL_INTEGRITY"
    );
}
#[test]
fn lag_reports_a_visible_lower_bound_without_mutating_delivery() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("lag.sqlite");
    let mut e = Engine::open(&path).unwrap();
    for value in 0..257 {
        write(&mut e, "a", value, "owner");
    }
    install(&e, "A", "a", "b", 4);
    let status = e.adapter_lag_status_for("A", &host()).unwrap();
    assert_eq!(status.visible_backlog_lower_bound, 256);
    assert!(status.backlog_truncated);
    assert!(status.pending.is_none());
    let c = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='A'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    write(&mut e, "private", 900, "other");
    write(&mut e, "a", 901, "other");
    assert_eq!(e.adapter_lag_status_for("A", &host()).unwrap(), status);
    assert_eq!(
        c.query_row("SELECT count(*) FROM dispatch_pending", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn oversized_causal_cells_fail_before_creating_a_lease_or_advancing_a_checkpoint() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("large.sqlite");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, "a", 1, "owner");
    install(&e, "A", "a", "b", 4);
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute(
        "UPDATE event_causation SET body=?1",
        [" ".repeat(16 * 1024 + 1)],
    )
    .unwrap();
    assert_eq!(
        e.poll_adapter_for("A", &host()).unwrap_err().code,
        "E_BUDGET"
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM dispatch_pending", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='A'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM dispatch_circuits", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
fn row(path: &std::path::Path, query: &str) -> Value {
    let c = rusqlite::Connection::open(path).unwrap();
    let s: String = c.query_row(query, [], |r| r.get(0)).unwrap();
    serde_json::from_str(&s).unwrap()
}
#[test]
fn two_actual_adapters_suspend_without_acknowledging_and_owner_can_bound_more_work() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("loops.sqlite");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, "a", 0, "owner");
    install(&e, "A", "a", "b", 4);
    install(&e, "B", "b", "a", 4);
    let mut completed = None;
    for value in 1..=4 {
        let (adapter, out) = if value % 2 == 1 {
            ("A", "b")
        } else {
            ("B", "a")
        };
        let event = e.poll_adapter_for(adapter, &host()).unwrap().unwrap();
        let p = program(&e, out, value, "owner");
        let receipt = e
            .complete_handler(adapter, &event.id, &event.lease, &p)
            .unwrap();
        assert!(!receipt.duplicate);
        if value == 3 {
            completed = Some((event, p));
        }
    }
    let c = rusqlite::Connection::open(&path).unwrap();
    let before: i64 = c
        .query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='A'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        e.poll_adapter_for("A", &host()).unwrap_err().code,
        "E_CIRCUIT_OPEN"
    );
    let status = e.adapter_lag_status_for("A", &host()).unwrap();
    assert_eq!(status.lifecycle, "paused");
    assert!(status.circuit_open);
    assert_eq!(status.visible_backlog_lower_bound, 1);
    assert!(status.pending.is_none());
    let after: i64 = c
        .query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='A'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(after >= before);
    let source: i64 = c
        .query_row(
            "SELECT max(sequence) FROM events WHERE graph_id='a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(after < source);
    assert_eq!(
        row(
            &path,
            "SELECT body FROM event_causation ORDER BY rowid DESC LIMIT 1"
        )["depth"],
        4
    );
    drop(c);
    drop(e);
    let mut e = Engine::open(&path).unwrap();
    assert_eq!(e.adapter_lag_status_for("A", &host()).unwrap(), status);
    e.set_adapter_state_for("A", "running", &host()).unwrap();
    let (old, p) = completed.unwrap();
    let count = e.event_count().unwrap();
    assert!(
        e.complete_handler("A", &old.id, &old.lease, &p)
            .unwrap()
            .duplicate
    );
    assert_eq!(e.event_count().unwrap(), count);
    assert_eq!(
        e.poll_adapter_for("A", &host()).unwrap_err().code,
        "E_CIRCUIT_OPEN"
    );
    e.set_causal_dispatch_policy_for("A", &CausalDispatchPolicy { max_depth: 5 }, &host())
        .unwrap();
    e.set_adapter_state_for("A", "running", &host()).unwrap();
    let event = e.poll_adapter_for("A", &host()).unwrap().unwrap();
    e.complete_handler("A", &event.id, &event.lease, &program(&e, "b", 5, "owner"))
        .unwrap();
    assert_eq!(
        e.poll_adapter_for("B", &host()).unwrap_err().code,
        "E_CIRCUIT_OPEN"
    );
    assert_eq!(e.event_count().unwrap(), 6);
    let plan = e.plan_retention(&RetentionPolicy::default()).unwrap();
    e.compact_retention(&plan).unwrap();
}
#[test]
fn no_op_and_existing_logical_batch_do_not_acquire_a_new_cause() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, "a", 1, "owner");
    let batch:Program=serde_json::from_value(json!({"version":weave_contract::VERSION,"commands":[{"op":"commit_batch","batch_id":"external-batch","commits":[{"graph_id":"b","expected_head":null,"data":data(2,"owner")}]}]})).unwrap();
    e.execute(&batch, &host()).unwrap();
    install(&e, "A", "a", "b", 2);
    let event = e.poll_adapter_for("A", &host()).unwrap().unwrap();
    let receipt = e
        .complete_handler("A", &event.id, &event.lease, &batch)
        .unwrap();
    assert!(matches!(
        receipt.results[0],
        CommandResult::BatchCommitted { .. }
    ));
    assert_eq!(e.event_count().unwrap(), 2);
    assert!(
        e.complete_handler("A", &event.id, &event.lease, &batch)
            .unwrap()
            .duplicate
    );
    write(&mut e, "a", 3, "owner");
    let event = e.poll_adapter_for("A", &host()).unwrap().unwrap();
    let no_op = program(&e, "b", 2, "owner");
    let receipt = e
        .complete_handler("A", &event.id, &event.lease, &no_op)
        .unwrap();
    assert!(matches!(
        receipt.results[0],
        CommandResult::Unchanged { .. }
    ));
    assert_eq!(e.event_count().unwrap(), 3);
    let plan = e.plan_retention(&RetentionPolicy::default()).unwrap();
    e.compact_retention(&plan).unwrap();
}
#[test]
fn scoped_lag_ignores_private_and_unsubscribed_occurrences_and_denies_foreign_hosts() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, "a", 1, "owner");
    install(&e, "A", "a", "b", 4);
    let first = e.adapter_lag_status_for("A", &host()).unwrap();
    assert_eq!(first.visible_backlog_lower_bound, 1);
    write(&mut e, "private", 42, "other");
    write(&mut e, "a", 43, "other");
    assert_eq!(e.adapter_lag_status_for("A", &host()).unwrap(), first);
    let foreign = HostContext::new("other", ["b".into()]);
    assert_eq!(
        e.adapter_lag_status_for("A", &foreign).unwrap_err().code,
        "E_HOST_AUTH"
    );
    let event = e.poll_adapter_for("A", &host()).unwrap().unwrap();
    let status = e.adapter_lag_status_for("A", &host()).unwrap();
    assert_eq!(status.pending.unwrap().attempts, 1);
    e.request_effect("A", &event.id, &event.lease, "sink", "action", json!(1))
        .unwrap();
    let effect = e
        .request_effect("A", &event.id, &event.lease, "sink", "action", json!(1))
        .unwrap();
    e.begin_effect_dispatch(&effect.id).unwrap();
    assert_eq!(
        e.adapter_lag_status_for("A", &host())
            .unwrap()
            .visible_unknown_effects,
        1
    );
}
#[test]
fn pending_work_cannot_have_its_budget_reconfigured_or_unknown_outcome_erased() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, "a", 1, "owner");
    install(&e, "A", "a", "b", 4);
    let event = e.poll_adapter_for("A", &host()).unwrap().unwrap();
    e.set_adapter_state_for("A", "paused", &host()).unwrap();
    assert_eq!(
        e.set_causal_dispatch_policy_for("A", &CausalDispatchPolicy { max_depth: 1 }, &host())
            .unwrap_err()
            .code,
        "E_CAUSAL_POLICY"
    );
    e.set_adapter_state_for("A", "running", &host()).unwrap();
    let intent = e
        .request_effect("A", &event.id, &event.lease, "sink", "intent", json!(1))
        .unwrap();
    e.begin_effect_dispatch(&intent.id).unwrap();
    e.set_adapter_state_for("A", "paused", &host()).unwrap();
    assert_eq!(
        e.set_causal_dispatch_policy_for("A", &CausalDispatchPolicy { max_depth: 64 }, &host())
            .unwrap_err()
            .code,
        "E_CAUSAL_POLICY"
    );
    assert_eq!(
        e.effect_intent(&intent.id).unwrap().unwrap().state,
        "unknown"
    );
    assert_eq!(
        e.set_causal_dispatch_policy_for("A", &CausalDispatchPolicy { max_depth: 65 }, &host())
            .unwrap_err()
            .code,
        "E_CAUSAL_POLICY"
    );
}
#[test]
fn panic_rolls_back_child_event_lineage_output_receipt_and_checkpoint() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("panic.sqlite");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, "a", 1, "owner");
    install(&e, "A", "a", "b", 4);
    let event = e.poll_adapter_for("A", &host()).unwrap().unwrap();
    let p = program(&e, "b", 2, "owner");
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        e.complete_handler_test_before_commit("A", &event.id, &event.lease, &p, || {
            panic!("after actual child binding")
        })
    }));
    assert!(caught.is_err());
    assert_eq!(e.event_count().unwrap(), 1);
    assert!(e.head("b", "main").unwrap().is_none());
    let c = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM event_causation", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM handler_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    e.complete_handler("A", &event.id, &event.lease, &p)
        .unwrap();
    assert_eq!(
        row(
            &path,
            "SELECT body FROM event_causation ORDER BY rowid DESC LIMIT 1"
        )["parent"],
        event.id
    );
}
#[test]
fn actual_lineage_corruption_and_missing_modern_cells_fail_closed_before_collection() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("corrupt.sqlite");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, "a", 1, "owner");
    install(&e, "A", "a", "b", 4);
    let event = e.poll_adapter_for("A", &host()).unwrap().unwrap();
    e.complete_handler("A", &event.id, &event.lease, &program(&e, "b", 2, "owner"))
        .unwrap();
    let c = rusqlite::Connection::open(&path).unwrap();
    let original: String = c.query_row("SELECT body FROM event_causation WHERE event_id=(SELECT event_id FROM events WHERE graph_id='b')", [], |r| r.get(0)).unwrap();
    let rewritten = original.replace("\"depth\":1", "\"depth\":9");
    assert_ne!(rewritten, original);
    let digest = format!("sha256:{:x}", Sha256::digest(rewritten.as_bytes()));
    c.execute("UPDATE event_causation SET body=?1,digest=?2 WHERE event_id=(SELECT event_id FROM events WHERE graph_id='b')", rusqlite::params![rewritten,digest]).unwrap();
    assert_eq!(
        e.plan_retention(&RetentionPolicy::default())
            .unwrap_err()
            .code,
        "E_CAUSAL_INTEGRITY"
    );
    c.execute("DELETE FROM event_causation WHERE event_id=(SELECT event_id FROM events WHERE graph_id='b')",[]).unwrap();
    drop(e);
    assert_eq!(
        Engine::open(&path).err().unwrap().code,
        "E_CAUSAL_INTEGRITY"
    );
}
#[test]
fn retry_and_dead_letter_status_are_current_and_owner_scoped() {
    let clock = Arc::new(ManualClock::new(100));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    write(&mut e, "a", 1, "owner");
    install(&e, "A", "a", "b", 4);
    for attempt in 1..=3 {
        let event = e.poll_adapter_for("A", &host()).unwrap().unwrap();
        e.fail_handler("A", &event.id, &event.lease).unwrap();
        let status = e
            .adapter_lag_status_for("A", &host())
            .unwrap()
            .pending
            .unwrap();
        assert_eq!(status.attempts, attempt);
        assert_eq!(
            status.state,
            if attempt == 3 { "dead_letter" } else { "retry" }
        );
        clock.set(status.retry_at_ms + 1);
    }
    assert!(e.poll_adapter_for("A", &host()).unwrap().is_none());
    assert_eq!(
        e.adapter_lag_status_for("A", &host())
            .unwrap()
            .visible_backlog_lower_bound,
        1
    );
}
