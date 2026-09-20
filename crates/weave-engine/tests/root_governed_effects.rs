use ed25519_dalek::SigningKey;
use serde_json::json;
use std::sync::Arc;
use weave_contract::{GraphData, GraphRef, GraphSchema, Program, VERSION};
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("owner", ["request".into(), "private".into()])
}
fn schema() -> GraphSchema {
    serde_json::from_value(json!({"id":"request-schema","revision":"1","nodes":{},"edges":{}}))
        .unwrap()
}
fn policy() -> GovernancePolicy {
    GovernancePolicy {
        view_id: "requests".into(),
        reference: GovernancePolicyRef {
            id: "policy".into(),
            revision: "1".into(),
        },
        members: vec![weave_policy::public_key(&SigningKey::from_bytes(&[7; 32]))],
        threshold: 1,
        proposers: vec!["owner".into()],
        readers: vec!["owner".into()],
        allowed_sources: vec![GovernanceSourceScope {
            graph_id: "request".into(),
            branch_id: "main".into(),
        }],
        not_before_ms: 0,
        expires_at_ms: 10_000,
    }
}
fn engine(path: &std::path::Path) -> (Engine, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::new(10));
    let e = Engine::open_with_clock(path.to_str().unwrap(), clock.clone()).unwrap();
    e.install_governance_root(&policy()).unwrap();
    (e, clock)
}
fn write(e: &mut Engine, graph: &str, data: GraphData) -> GraphRef {
    let p:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":e.head(graph,"main").unwrap(),"data":data}]})).unwrap();
    e.execute(&p, &host()).unwrap();
    GraphRef {
        graph_id: graph.into(),
        revision: e.head(graph, "main").unwrap().unwrap(),
    }
}
fn publish(e: &Engine, source: GraphRef, id: &str) -> GovernanceReceipt {
    let proposal = GovernanceProposal {
        id: id.into(),
        view_id: "requests".into(),
        policy: policy().reference,
        expected_head: e
            .inspect_governance_head("requests", &host())
            .unwrap()
            .decision_id,
        expires_at_ms: 9000,
        action: GovernanceAction::Publish {
            source,
            branch_id: "main".into(),
        },
    };
    let receipt = e.propose_governance(&proposal, &host()).unwrap();
    let key = SigningKey::from_bytes(&[7; 32]);
    let approval = sign_governance_approval(
        GovernanceApproval {
            proposal_id: id.into(),
            proposal_digest: receipt.digest,
            view_id: proposal.view_id,
            policy: proposal.policy,
            expected_head: proposal.expected_head,
            member: weave_policy::public_key(&key),
            issued_at_ms: 0,
            expires_at_ms: 8000,
            nonce: format!("approval-{id}"),
        },
        &key,
    )
    .unwrap();
    e.record_governance_approval(&approval, &host()).unwrap();
    e.accept_governance(
        &GovernanceDecisionRequest {
            proposal_id: id.into(),
            nonce: format!("decision-{id}"),
        },
        &host(),
    )
    .unwrap()
}
fn grant(id: &str, start: EffectStart) -> GovernedEffectGrant {
    GovernedEffectGrant {
        id: "execute".into(),
        revision: "1".into(),
        principal: "owner".into(),
        view_id: "requests".into(),
        source: SubscriptionScope {
            graph_id: "request".into(),
            branch_id: "main".into(),
        },
        request_schema: schema(),
        destination_id: "reference-sink".into(),
        destination_principal: "owner".into(),
        encoder: EffectEncoder::CanonicalGraphV1,
        execution_id: id.into(),
        start,
        not_before_ms: 0,
        expires_at_ms: 5000,
    }
}
fn manifest(id: &str, g: &GovernedEffectGrant) -> AdapterManifest {
    AdapterManifest {
        id: id.into(),
        version: "1".into(),
        artifact_digest: governed_effect_grant_digest(g).unwrap(),
        config_revision: "1".into(),
        principal: "owner".into(),
        subscriptions: vec![g.source.clone()],
        output_graphs: vec![],
        effect_destinations: vec![g.destination_id.clone()],
        max_attempts: 5,
        lease_ms: 100,
        max_pending_events: 100,
        projection_replay: false,
    }
}
fn install(e: &Engine, id: &str, g: &GovernedEffectGrant) {
    e.install_governed_effect(&manifest(id, g), g, &host())
        .unwrap();
    e.set_adapter_state(id, "running").unwrap();
}
fn request(e: &mut Engine) -> GraphRef {
    write(
        e,
        "request",
        GraphData {
            schema: Some(schema()),
            ..GraphData::default()
        },
    )
}
fn enqueue(e: &Engine, id: &str) -> (GovernanceDelivery, GovernedEffectReceipt) {
    let d = e.poll_governance(id, "requests", &host()).unwrap().unwrap();
    let r = e
        .enqueue_governed_effect(id, &d.event.id, &d.lease, &host())
        .unwrap();
    (d, r)
}
fn intent(r: &GovernedEffectReceipt) -> String {
    match &r.disposition {
        GovernedEffectDisposition::Intent { intent_id } => intent_id.clone(),
        x => panic!("not intent: {x:?}"),
    }
}
fn evidence() -> SinkEvidence {
    SinkEvidence {
        receipt_id: "sink-receipt".into(),
        response_digest: format!("sha256:{}", "0".repeat(64)),
    }
}

fn context_hash(id: &str, canonical_body: &str) -> String {
    use sha2::{Digest, Sha256};
    let material = format!(
        "[\"weave-governed-effect-context/1\",[{},{}]]",
        serde_json::to_string(id).unwrap(),
        canonical_body
    );
    format!("sha256:{:x}", Sha256::digest(material.as_bytes()))
}
fn missing_source(db: &std::path::Path, source: &GraphRef) {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
    assert_eq!(
        conn.execute(
            "DELETE FROM revisions WHERE graph_id=?1 AND revision=?2",
            rusqlite::params![source.graph_id, source.revision]
        )
        .unwrap(),
        1
    );
}
#[test]
fn root_valid_checksum_cannot_trim_authorization_closure() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("store.sqlite");
    let (mut e, _) = engine(&db);
    let source = request(&mut e);
    publish(&e, source, "root-one");
    install(
        &e,
        "root-bridge",
        &grant("root-execution", EffectStart::ReplayHistory),
    );
    let id = intent(&enqueue(&e, "root-bridge").1);
    let conn = rusqlite::Connection::open(&db).unwrap();
    let (body, hash): (String, String) = conn
        .query_row(
            "SELECT body,digest FROM governed_effect_context WHERE intent_id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(context_hash(&id, &body), hash);
    let start = body.find("\"closure\":").unwrap() + "\"closure\":".len();
    let end = body[start..].find(",\"payload_digest\":").unwrap() + start;
    assert_ne!(&body[start..end], "[]");
    let trimmed = format!("{}[]{}", &body[..start], &body[end..]);
    conn.execute(
        "UPDATE governed_effect_context SET body=?2,digest=?3 WHERE intent_id=?1",
        rusqlite::params![id, trimmed, context_hash(&id, &trimmed)],
    )
    .unwrap();
    assert!(e.read_governed_effect(&id, &host()).is_err());
    assert!(e.begin_governed_effect(&id, &host()).is_err());
    let state: String = conn
        .query_row("SELECT state FROM effect_intents WHERE id=?1", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(state, "pending");
    conn.execute(
        "UPDATE governed_effect_context SET body=?2,digest=?3 WHERE intent_id=?1",
        rusqlite::params![id, body, hash],
    )
    .unwrap();
    assert!(!e
        .begin_governed_effect(&id, &host())
        .unwrap()
        .payload
        .is_empty());
    assert!(e.begin_governed_effect(&id, &host()).is_err());
}
#[test]
fn root_missing_empty_request_denies_dispatch_but_allows_payload_free_cancel() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("store.sqlite");
    let (mut e, _) = engine(&db);
    let source = request(&mut e);
    publish(&e, source.clone(), "root-one");
    install(
        &e,
        "root-bridge",
        &grant("root-execution", EffectStart::ReplayHistory),
    );
    let id = intent(&enqueue(&e, "root-bridge").1);
    missing_source(&db, &source);
    assert!(e.begin_governed_effect(&id, &host()).is_err());
    assert!(e.read_governed_effect(&id, &host()).is_err());
    assert_eq!(e.effect_intent(&id).unwrap_err().code, "E_EFFECT_BOUND");
    assert!(e
        .cancel_governed_effect(&id, &HostContext::new("stranger", []))
        .is_err());
    e.cancel_governed_effect(&id, &host()).unwrap();
    e.cancel_governed_effect(&id, &host()).unwrap();
    let c = rusqlite::Connection::open(&db).unwrap();
    let (state, attempt): (String,Option<String>) = c.query_row("SELECT i.state,c.attempt_id FROM effect_intents i JOIN governed_effect_context c ON c.intent_id=i.id WHERE i.id=?1", [&id], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(state, "canceled");
    assert_eq!(attempt, None);
}
#[test]
fn root_unknown_cleanup_remains_owner_only_after_expiry_and_missing_proof() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("store.sqlite");
    let (mut e, clock) = engine(&db);
    let source = request(&mut e);
    publish(&e, source.clone(), "root-one");
    install(
        &e,
        "root-bridge",
        &grant("root-execution", EffectStart::ReplayHistory),
    );
    let id = intent(&enqueue(&e, "root-bridge").1);
    let ticket = e.begin_governed_effect(&id, &host()).unwrap();
    clock.set(20_000);
    missing_source(&db, &source);
    e.revoke_governed_effect_grant("root-bridge", &host())
        .unwrap();
    assert!(e.read_governed_effect(&id, &host()).is_err());
    assert!(e.begin_governed_effect(&id, &host()).is_err());
    assert!(e.cancel_governed_effect(&id, &host()).is_err());
    assert!(e
        .reconcile_governed_effect(
            &id,
            &ticket.attempt_id,
            ReconciledOutcome::Confirmed,
            &evidence(),
            &HostContext::new("stranger", [])
        )
        .is_err());
    assert!(e
        .reconcile_governed_effect(
            &id,
            "wrong-attempt",
            ReconciledOutcome::Confirmed,
            &evidence(),
            &host()
        )
        .is_err());
    e.reconcile_governed_effect(
        &id,
        &ticket.attempt_id,
        ReconciledOutcome::Confirmed,
        &evidence(),
        &host(),
    )
    .unwrap();
    e.reconcile_governed_effect(
        &id,
        &ticket.attempt_id,
        ReconciledOutcome::Confirmed,
        &evidence(),
        &host(),
    )
    .unwrap();
    let mut changed = evidence();
    changed.receipt_id = "different-receipt".into();
    assert!(e
        .reconcile_governed_effect(
            &id,
            &ticket.attempt_id,
            ReconciledOutcome::Confirmed,
            &changed,
            &host()
        )
        .is_err());
    assert!(e.reconcile_effect(&id, "failed", json!({})).is_err());
}
