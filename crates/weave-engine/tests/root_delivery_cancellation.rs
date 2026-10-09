//! Independent owner cleanup, immutable receipt, private cursor and rebuild oracles.
use serde_json::json;
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;
fn owner() -> HostContext {
    HostContext::new("owner", ["input".into(), "output".into()])
}
fn write(engine: &Engine, graph: &str, value: i64) -> Program {
    serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":engine.head(graph,"main").unwrap(),"data":{"nodes":[{"id":"n","entity_id":"e","space_id":"s","readers":["owner"],"properties":{"value":value}}]}}]})).unwrap()
}
fn install(engine: &mut Engine) {
    engine
        .execute(&write(engine, "input", 1), &owner())
        .unwrap();
    engine
        .install_adapter(
            &AdapterManifest {
                id: "worker".into(),
                version: "1".into(),
                artifact_digest: format!("sha256:{}", "a".repeat(64)),
                config_revision: "1".into(),
                principal: "owner".into(),
                subscriptions: vec![SubscriptionScope {
                    graph_id: "input".into(),
                    branch_id: "main".into(),
                }],
                output_graphs: vec!["output".into()],
                effect_destinations: vec![],
                max_attempts: 3,
                lease_ms: 100,
                max_pending_events: 100,
                projection_replay: true,
            },
            &owner(),
        )
        .unwrap();
    engine
        .set_adapter_state_for("worker", "running", &owner())
        .unwrap();
}
fn cancellation(delivery: &DispatchEnvelope) -> DeliveryCancellationRequest {
    DeliveryCancellationRequest {
        adapter: "worker".into(),
        event: delivery.id.clone(),
        expected_lease: delivery.lease.clone(),
        nonce: "cleanup1".into(),
        reason: DeliveryCancellationReason::StaleOutput,
    }
}
#[test]
fn stale_output_cleanup_is_owned_lease_fenced_immutable_and_never_reexecutes_the_occurrence() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    install(&mut engine);
    let first = engine
        .poll_adapter_for("worker", &owner())
        .unwrap()
        .unwrap();
    let stale = write(&engine, "output", 1);
    engine
        .execute(&write(&engine, "output", 99), &owner())
        .unwrap();
    assert_eq!(
        engine
            .complete_handler("worker", &first.id, &first.lease, &stale)
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    let mut request = cancellation(&first);
    let events = engine.event_count().unwrap();
    let outsider = HostContext::new("outsider", ["output".into()]);
    assert_eq!(
        engine
            .cancel_handler_delivery_for(&request, &outsider)
            .unwrap_err()
            .code,
        "E_HOST_AUTH"
    );
    clock.set(120);
    let renewed = engine
        .poll_adapter_for("worker", &owner())
        .unwrap()
        .unwrap();
    assert_eq!(
        engine
            .cancel_handler_delivery_for(&request, &owner())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    request.expected_lease = renewed.lease;
    // Cleanup is legal after expiry. It returns no original input/output payload.
    clock.set(230);
    let receipt = engine
        .cancel_handler_delivery_for(&request, &owner())
        .unwrap();
    assert!(!receipt.rebuild_required && !receipt.duplicate);
    let encoded = serde_json::to_string(&receipt).unwrap();
    for field in ["graph_id", "sequence", "properties", "state"] {
        assert!(!encoded.contains(field));
    }
    assert!(
        engine
            .cancel_handler_delivery_for(&request, &owner())
            .unwrap()
            .duplicate
    );
    let mut changed = request.clone();
    changed.reason = DeliveryCancellationReason::OwnerStop;
    assert_eq!(
        engine
            .cancel_handler_delivery_for(&changed, &owner())
            .unwrap_err()
            .code,
        "E_RECEIPT_CONFLICT"
    );
    assert_eq!(
        engine
            .complete_handler("worker", &first.id, &first.lease, &stale)
            .unwrap_err()
            .code,
        "E_DELIVERY_CANCELED"
    );
    assert_eq!(engine.event_count().unwrap(), events);
    assert!(engine
        .poll_adapter_for("worker", &owner())
        .unwrap()
        .is_none());
    engine
        .execute(&write(&engine, "input", 2), &owner())
        .unwrap();
    let next = engine
        .poll_adapter_for("worker", &owner())
        .unwrap()
        .unwrap();
    assert_ne!(next.id, first.id);
}
#[test]
fn canceled_stateful_occurrence_requires_actual_state_rebuild_before_any_new_poll() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    install(&mut engine);
    let inputs = engine
        .projection_rebase_inputs_for("worker", &owner())
        .unwrap();
    engine
        .rebase_projection_for(
            &ProjectionRebaseRequest {
                inputs,
                state_revision: "seed".into(),
                state: json!({"total":1}),
            },
            &owner(),
        )
        .unwrap();
    clock.set(20);
    engine
        .execute(&write(&engine, "input", 2), &owner())
        .unwrap();
    let delivery = engine
        .poll_adapter_for("worker", &owner())
        .unwrap()
        .unwrap();
    let request = cancellation(&delivery);
    let receipt = engine
        .cancel_handler_delivery_for(&request, &owner())
        .unwrap();
    assert!(receipt.rebuild_required);
    assert_eq!(
        engine
            .projection_state_for("worker", &owner())
            .unwrap_err()
            .code,
        "E_CHECKPOINT_EXPIRED"
    );
    assert_eq!(
        engine
            .poll_adapter_for("worker", &owner())
            .unwrap_err()
            .code,
        "E_CHECKPOINT_EXPIRED"
    );
    let inputs = engine
        .projection_rebase_inputs_for("worker", &owner())
        .unwrap();
    engine
        .rebase_projection_for(
            &ProjectionRebaseRequest {
                inputs,
                state_revision: "rebuilt".into(),
                state: json!({"total":2}),
            },
            &owner(),
        )
        .unwrap();
    assert_eq!(
        engine
            .projection_state_for("worker", &owner())
            .unwrap()
            .state,
        json!({"total":2})
    );
    assert!(engine
        .poll_adapter_for("worker", &owner())
        .unwrap()
        .is_none());
    assert!(
        engine
            .cancel_handler_delivery_for(&request, &owner())
            .unwrap()
            .duplicate
    );
    assert_eq!(
        engine
            .projection_state_for("worker", &owner())
            .unwrap()
            .state,
        json!({"total":2})
    );
}
#[test]
fn interrupted_cancellation_preserves_actual_pending_lease_and_cursor_across_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    install(&mut engine);
    let delivery = engine
        .poll_adapter_for("worker", &owner())
        .unwrap()
        .unwrap();
    let request = cancellation(&delivery);
    let sql = rusqlite::Connection::open(&path).unwrap();
    let checkpoint: i64 = sql
        .query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='worker'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let fault = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine.cancel_handler_delivery_test_before_commit(&request, &owner(), || {
            panic!("before commit")
        })
    }));
    assert!(fault.is_err());
    drop(engine);
    let engine = Engine::open_with_clock(&path, clock).unwrap();
    assert_eq!(
        sql.query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='worker'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        checkpoint
    );
    assert_eq!(
        sql.query_row(
            "SELECT lease FROM dispatch_pending WHERE adapter='worker'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        delivery.lease
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM delivery_cancellations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        !engine
            .cancel_handler_delivery_for(&request, &owner())
            .unwrap()
            .duplicate
    );
    assert!(
        engine
            .cancel_handler_delivery_for(&request, &owner())
            .unwrap()
            .duplicate
    );
}
#[test]
fn lowering_a_store23_marker_never_defaults_existing_lifecycle_records() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.db");
    let engine = Engine::open(&path).unwrap();
    drop(engine);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.pragma_update(None, "user_version", 22).unwrap();
    let error = match Engine::open(&path) {
        Ok(_) => panic!("downgrade accepted"),
        Err(error) => error,
    };
    assert_eq!(error.code, "E_LIFECYCLE_INTEGRITY");
    assert_eq!(
        sql.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        22
    );
    assert_eq!(sql.query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('delivery_cancellations','projection_rebuild_requests','projection_migrations')",[],|r|r.get::<_,i64>(0)).unwrap(),3);
}

#[test]
fn malformed_valid_json_cancellation_binding_stops_collection_before_policy_or_payload_changes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    install(&mut engine);
    let delivery = engine
        .poll_adapter_for("worker", &owner())
        .unwrap()
        .unwrap();
    let request = cancellation(&delivery);
    engine
        .cancel_handler_delivery_for(&request, &owner())
        .unwrap();
    clock.set(30);
    let policy = RetentionPolicy {
        history_before_ms: 20,
        replay_through_sequence: engine.events().unwrap().last().unwrap().sequence as i64,
    };
    let plan = engine.plan_retention(&policy).unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    let original: String = sql
        .query_row("SELECT body FROM delivery_cancellations", [], |r| r.get(0))
        .unwrap();
    let mut forged: serde_json::Value = serde_json::from_str(&original).unwrap();
    forged["receipt"]["rebuild_required"] = json!(true);
    sql.execute(
        "UPDATE delivery_cancellations SET body=?1",
        [serde_json::to_string(&forged).unwrap()],
    )
    .unwrap();
    assert_eq!(
        engine.compact_retention(&plan).unwrap_err().code,
        "E_LIFECYCLE_INTEGRITY"
    );
    assert_eq!(
        sql.query_row("SELECT generation FROM retention_policy", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM retention_tombstones", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    sql.execute("UPDATE delivery_cancellations SET body=?1", [original])
        .unwrap();
    engine.compact_retention(&plan).unwrap();
}

#[test]
fn genuine_stale_compiled_preparation_is_preserved_and_owner_cleanup_survives_source_policy_expiry()
{
    use ed25519_dalek::SigningKey;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let key = SigningKey::from_bytes(&[77; 32]);
    let policy = GovernancePolicy {
        view_id: "team".into(),
        reference: GovernancePolicyRef {
            id: "policy".into(),
            revision: "1".into(),
        },
        members: vec![weave_policy::public_key(&key)],
        threshold: 1,
        proposers: vec!["owner".into()],
        readers: vec!["owner".into()],
        allowed_sources: vec![GovernanceSourceScope {
            graph_id: "auth-source".into(),
            branch_id: "main".into(),
        }],
        not_before_ms: 0,
        expires_at_ms: 1000,
    };
    engine.install_governance_root(&policy).unwrap();
    engine
        .execute(
            &write(&engine, "auth-source", 1),
            &HostContext::new("owner", ["auth-source".into()]),
        )
        .unwrap();
    let proposal = GovernanceProposal {
        id: "publish".into(),
        view_id: "team".into(),
        policy: policy.reference.clone(),
        expected_head: None,
        expires_at_ms: 900,
        action: GovernanceAction::Publish {
            source: GraphRef {
                graph_id: "auth-source".into(),
                revision: engine.head("auth-source", "main").unwrap().unwrap(),
            },
            branch_id: "main".into(),
        },
    };
    let proposed = engine.propose_governance(&proposal, &owner()).unwrap();
    let approval = sign_governance_approval(
        GovernanceApproval {
            proposal_id: proposal.id.clone(),
            proposal_digest: proposed.digest,
            view_id: "team".into(),
            policy: policy.reference,
            expected_head: None,
            member: weave_policy::public_key(&key),
            issued_at_ms: 0,
            expires_at_ms: 800,
            nonce: "approval".into(),
        },
        &key,
    )
    .unwrap();
    engine
        .record_governance_approval(&approval, &owner())
        .unwrap();
    engine
        .accept_governance(
            &GovernanceDecisionRequest {
                proposal_id: "publish".into(),
                nonce: "decision".into(),
            },
            &owner(),
        )
        .unwrap();
    let accepted = engine
        .query_accepted_view(
            &AcceptedViewSelection {
                view_id: "team".into(),
                decision_id: None,
            },
            &owner(),
        )
        .unwrap();
    let input:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"input","expected_head":null,"data":accepted.graph}]})).unwrap();
    engine.execute(&input, &owner()).unwrap();
    let template=handler_registration::seal_handler_template(serde_json::from_value(json!({"format":"weave-handler-registration/1","protocol":VERSION,"name":"identity","revision":"1","input":{"graph_id":"input","branch_id":"main","metadata_depth":0},"event_types":["graph.accepted","graph.committed"],"recipe":{"bindings":[],"output":"$event"},"output_slot":"out","source_revisions":[],"definition_digest":""})).unwrap()).unwrap();
    let manifest = AdapterManifest {
        id: "compiled".into(),
        version: "1".into(),
        artifact_digest: template.definition_digest.clone(),
        config_revision: "1".into(),
        principal: "owner".into(),
        subscriptions: vec![SubscriptionScope {
            graph_id: "input".into(),
            branch_id: "main".into(),
        }],
        output_graphs: vec!["output".into()],
        effect_destinations: vec![],
        max_attempts: 3,
        lease_ms: 10000,
        max_pending_events: 100,
        projection_replay: true,
    };
    engine
        .install_compiled_handler(
            &manifest,
            &template,
            &HandlerOutputBinding {
                slot: "out".into(),
                graph_id: "output".into(),
                branch_id: "main".into(),
            },
            &owner(),
        )
        .unwrap();
    engine
        .set_adapter_state_for("compiled", "running", &owner())
        .unwrap();
    let delivery = engine
        .poll_adapter_for("compiled", &owner())
        .unwrap()
        .unwrap();
    let prepared = engine
        .prepare_compiled_handler_for("compiled", &delivery.id, &delivery.lease, &owner())
        .unwrap();
    engine
        .execute(&write(&engine, "output", 99), &owner())
        .unwrap();
    let output = engine.head("output", "main").unwrap();
    assert_eq!(
        engine
            .complete_prepared_handler_for(
                "compiled",
                &delivery.id,
                &delivery.lease,
                &prepared.preparation_id,
                &owner()
            )
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    let sql = rusqlite::Connection::open(&path).unwrap();
    let body: String = sql
        .query_row("SELECT body FROM handler_preparations", [], |r| r.get(0))
        .unwrap();
    clock.set(1000);
    assert!(engine
        .prepare_compiled_handler_for("compiled", &delivery.id, &delivery.lease, &owner())
        .is_err());
    let request = DeliveryCancellationRequest {
        adapter: "compiled".into(),
        event: delivery.id.clone(),
        expected_lease: delivery.lease.clone(),
        nonce: "stale-compiled".into(),
        reason: DeliveryCancellationReason::StaleOutput,
    };
    assert!(
        !engine
            .cancel_handler_delivery_for(&request, &owner())
            .unwrap()
            .duplicate
    );
    assert!(
        engine
            .cancel_handler_delivery_for(&request, &owner())
            .unwrap()
            .duplicate
    );
    assert_eq!(
        sql.query_row("SELECT body FROM handler_preparations", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        body
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM handler_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(engine.head("output", "main").unwrap(), output);
    assert_eq!(
        engine
            .complete_prepared_handler_for(
                "compiled",
                &delivery.id,
                &delivery.lease,
                &prepared.preparation_id,
                &owner()
            )
            .unwrap_err()
            .code,
        "E_DELIVERY_CANCELED"
    );
    assert_eq!(
        engine
            .prepare_compiled_handler_for("compiled", &delivery.id, &delivery.lease, &owner())
            .unwrap_err()
            .code,
        "E_DELIVERY_CANCELED"
    );
}

#[test]
fn a_real_unknown_effect_and_pending_delivery_cannot_be_disposed_by_pure_cleanup_or_state_migration(
) {
    let clock = Arc::new(ManualClock::new(10));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("effects.db");
    let mut engine = Engine::open_with_clock(&path, clock).unwrap();
    engine
        .execute(&write(&engine, "input", 1), &owner())
        .unwrap();
    let manifest = AdapterManifest {
        id: "effect".into(),
        version: "1".into(),
        artifact_digest: format!("sha256:{}", "a".repeat(64)),
        config_revision: "1".into(),
        principal: "owner".into(),
        subscriptions: vec![SubscriptionScope {
            graph_id: "input".into(),
            branch_id: "main".into(),
        }],
        output_graphs: vec!["output".into()],
        effect_destinations: vec!["sink".into()],
        max_attempts: 3,
        lease_ms: 100,
        max_pending_events: 100,
        projection_replay: false,
    };
    engine.install_adapter(&manifest, &owner()).unwrap();
    engine
        .set_adapter_state_for("effect", "running", &owner())
        .unwrap();
    let delivery = engine
        .poll_adapter_for("effect", &owner())
        .unwrap()
        .unwrap();
    let intent = engine
        .request_effect(
            "effect",
            &delivery.id,
            &delivery.lease,
            "sink",
            "intent1",
            json!({"action":"once"}),
        )
        .unwrap();
    engine.begin_effect_dispatch(&intent.id).unwrap();
    let request = DeliveryCancellationRequest {
        adapter: "effect".into(),
        event: delivery.id.clone(),
        expected_lease: delivery.lease.clone(),
        nonce: "cleanup".into(),
        reason: DeliveryCancellationReason::OwnerStop,
    };
    assert_eq!(
        engine
            .cancel_handler_delivery_for(&request, &owner())
            .unwrap_err()
            .code,
        "E_CANCELLATION_MODE"
    );
    assert_eq!(
        engine
            .projection_migration_inputs_for("effect", &owner())
            .unwrap_err()
            .code,
        "E_REBASE_MODE"
    );
    assert_eq!(
        engine.effect_intent(&intent.id).unwrap().unwrap().state,
        "unknown"
    );
    assert_eq!(engine.event_count().unwrap(), 1);
    assert!(engine
        .poll_adapter_for("effect", &owner())
        .unwrap()
        .is_none());
    let sql = rusqlite::Connection::open(&path).unwrap();
    let pending: (String, String) = sql
        .query_row(
            "SELECT event_id,lease FROM dispatch_pending WHERE adapter='effect'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(pending, (delivery.id, delivery.lease));
    assert_eq!(
        sql.query_row("SELECT count(*) FROM delivery_cancellations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
