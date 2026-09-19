use ed25519_dalek::SigningKey;
use serde_json::json;
use weave_contract::{GraphRef, QueryPlan, VERSION};
use weave_engine::*;
use weave_policy::{
    Action, AdmissionContext, AdmissionProof, Capability, Operation, Request, RootAuthority, Scope,
};
fn host() -> HostContext {
    HostContext::new("reviewer", ["source".into()])
}
struct Peer {
    root: SigningKey,
    user: SigningKey,
    context: AdmissionContext,
}
impl Peer {
    fn new() -> Self {
        let root = SigningKey::from_bytes(&[21; 32]);
        let user = SigningKey::from_bytes(&[22; 32]);
        let scopes = vec![Scope {
            graph_id: "source".into(),
            branch_id: "main".into(),
            actions: [Action::Propose].into(),
        }];
        let context = AdmissionContext {
            audience: "receiver".into(),
            now_ms: 200,
            policy_epoch: "epoch1".into(),
            roots: vec![RootAuthority {
                issuer: weave_policy::public_key(&root),
                audience: "receiver".into(),
                policy_revision: "1".into(),
                scopes,
                not_before_ms: 0,
                expires_at_ms: 10000,
                max_delegations: 1,
            }],
            revoked_capabilities: Default::default(),
            revoked_keys: Default::default(),
            consumed_nonces: Default::default(),
        };
        Self {
            root,
            user,
            context,
        }
    }
    fn proof(&self, capsule: &Capsule) -> AdmissionProof {
        let cap = weave_policy::sign_capability(
            Capability {
                version: weave_policy::VERSION.into(),
                issuer: weave_policy::public_key(&self.root),
                subject: weave_policy::public_key(&self.user),
                audience: "receiver".into(),
                policy_revision: "1".into(),
                scopes: self.context.roots[0].scopes.clone(),
                not_before_ms: 10,
                expires_at_ms: 9000,
                delegations_remaining: 0,
                parent: None,
            },
            &self.root,
        )
        .unwrap();
        let request = weave_policy::sign_request(
            Request {
                version: weave_policy::REQUEST_VERSION.into(),
                subject: weave_policy::public_key(&self.user),
                audience: "receiver".into(),
                capability_id: weave_policy::capability_id(&cap).unwrap(),
                nonce: "ab".repeat(32),
                issued_at_ms: 100,
                expires_at_ms: 1000,
                operation: Operation {
                    action: Action::Propose,
                    graph_id: "source".into(),
                    branch_id: "main".into(),
                },
                body_digest: weave_policy::body_digest(&serde_json::to_vec(capsule).unwrap()),
            },
            &self.user,
        )
        .unwrap();
        AdmissionProof {
            chain: vec![cap],
            request,
        }
    }
}
fn source(readers: serde_json::Value) -> (Engine, Capsule) {
    let mut e = Engine::memory().unwrap();
    e.execute(&serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"source","data":{"nodes":[{"id":"a","entity_id":"a","space_id":"s","readers":readers},{"id":"b","entity_id":"b","space_id":"s","readers":readers}],"edges":[{"id":"link","from":"a","to":"b","predicate":"connected","valid_time":{"start":0},"readers":readers}]}}]})).unwrap(),&host()).unwrap();
    let root = GraphRef {
        graph_id: "source".into(),
        revision: e.head("source", "main").unwrap().unwrap(),
    };
    let capsule = e.export_capsule(&root, &host()).unwrap();
    (e, capsule)
}
fn decision(id: String) -> IntegrationRequest {
    IntegrationRequest {
        proposal_id: id,
        branch_id: "offline".into(),
        expected_head: None,
        nonce: "approval".into(),
    }
}
fn query() -> QueryPlan {
    serde_json::from_value(json!({"graph_id":"source","branch_id":"offline"})).unwrap()
}
#[test]
fn two_engine_signed_receipt_stays_isolated_until_explicit_restart_safe_integration() {
    let (sender, capsule) = source(json!([]));
    let peer = Peer::new();
    let proof = peer.proof(&capsule);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("receiver.db");
    let mut receiver = Engine::open(&path).unwrap();
    receiver.install_admission_policy(&peer.context).unwrap();
    let proposed = receiver.admit_proposal(&proof, &capsule, 200).unwrap();
    let request = decision(proposed.result.id);
    assert!(receiver.head("source", "offline").unwrap().is_none());
    assert_eq!(receiver.event_count().unwrap(), 0);
    assert!(receiver
        .query(
            &serde_json::from_value(json!({"graph_id":"source","revision":capsule.root.revision}))
                .unwrap(),
            &host()
        )
        .is_err());
    assert_eq!(
        receiver
            .integrate_proposal(&request, &proof, 201, &HostContext::new("reviewer", []))
            .unwrap_err()
            .code,
        "E_FORBIDDEN"
    );
    let accepted = receiver
        .integrate_proposal(&request, &proof, 202, &host())
        .unwrap();
    assert!(accepted.event_id.is_some());
    assert!(!accepted.duplicate);
    assert_eq!(
        accepted.source_subject,
        weave_policy::public_key(&peer.user)
    );
    assert_eq!(
        receiver.query(&query(), &host()).unwrap().graph.edges.len(),
        1
    );
    assert_eq!(receiver.event_count().unwrap(), 1);
    assert!(receiver.head("source", "main").unwrap().is_none());
    drop(receiver);
    let mut receiver = Engine::open(&path).unwrap();
    let replay = receiver
        .integrate_proposal(&request, &proof, 203, &host())
        .unwrap();
    assert!(replay.duplicate);
    assert_eq!(replay.event_id, accepted.event_id);
    assert_eq!(receiver.event_count().unwrap(), 1);
    let mut changed = request.clone();
    changed.branch_id = "other".into();
    assert_eq!(
        receiver
            .integrate_proposal(&changed, &proof, 204, &host())
            .unwrap_err()
            .code,
        "E_REPLAY"
    );
    assert_eq!(
        sender.head("source", "main").unwrap(),
        Some(capsule.root.revision)
    );
    let mut revoked = peer.context.clone();
    revoked.policy_epoch = "epoch2".into();
    revoked
        .revoked_keys
        .insert(weave_policy::public_key(&peer.user));
    receiver.install_admission_policy(&revoked).unwrap();
    assert!(receiver
        .integrate_proposal(&request, &proof, 205, &host())
        .is_err());
    assert_eq!(receiver.event_count().unwrap(), 1);
}
#[test]
fn stale_branch_cas_and_private_data_do_not_promote_isolated_proposals() {
    let (_, capsule) = source(json!([]));
    let peer = Peer::new();
    let proof = peer.proof(&capsule);
    let mut receiver = Engine::memory().unwrap();
    receiver.install_admission_policy(&peer.context).unwrap();
    let proposed = receiver.admit_proposal(&proof, &capsule, 200).unwrap();
    let mut request = decision(proposed.result.id);
    request.expected_head = Some("absent".into());
    assert_eq!(
        receiver
            .integrate_proposal(&request, &proof, 201, &host())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    assert_eq!(receiver.event_count().unwrap(), 0);
    assert!(receiver
        .query(
            &serde_json::from_value(json!({"graph_id":"source","revision":capsule.root.revision}))
                .unwrap(),
            &host()
        )
        .is_err());
    // The signed sender has proposal scope; that alone cannot let a local reviewer read private content.
    let (_, private) = source(json!(["reviewer"]));
    let private_proof = peer.proof(&private);
    let mut receiver = Engine::memory().unwrap();
    receiver.install_admission_policy(&peer.context).unwrap();
    let proposed = receiver
        .admit_proposal(&private_proof, &private, 200)
        .unwrap();
    let request = decision(proposed.result.id);
    assert_eq!(
        receiver
            .integrate_proposal(
                &request,
                &private_proof,
                201,
                &HostContext::new("other-reviewer", ["source".into()])
            )
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
    assert_eq!(receiver.event_count().unwrap(), 0);
    assert!(receiver.head("source", "offline").unwrap().is_none());
    assert!(receiver
        .query(
            &serde_json::from_value(json!({"graph_id":"source","revision":private.root.revision}))
                .unwrap(),
            &host()
        )
        .is_err());
}
#[test]
fn structural_identity_conflicts_rollback_import_without_overwriting_local_branch() {
    let (_, capsule) = source(json!([]));
    let peer = Peer::new();
    let proof = peer.proof(&capsule);
    let mut receiver = Engine::memory().unwrap();
    receiver.install_admission_policy(&peer.context).unwrap();
    let proposed = receiver.admit_proposal(&proof, &capsule, 200).unwrap();
    receiver.execute(&serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"source","data":{"nodes":[{"id":"a","entity_id":"a","space_id":"s"},{"id":"b","entity_id":"b","space_id":"s"}],"edges":[{"id":"link","from":"a","to":"b","predicate":"different","valid_time":{"start":0}}]}}]})).unwrap(),&host()).unwrap();
    let before = receiver.head("source", "main").unwrap();
    assert_eq!(
        receiver
            .integrate_proposal(&decision(proposed.result.id), &proof, 201, &host())
            .unwrap_err()
            .code,
        "E_EDGE_IDENTITY"
    );
    assert_eq!(receiver.head("source", "main").unwrap(), before);
    assert!(receiver.head("source", "offline").unwrap().is_none());
    assert_eq!(receiver.event_count().unwrap(), 1);
}

#[test]
fn export_and_accept_use_verified_storage_before_releasing_or_publishing_content() {
    let (_, capsule) = source(json!([]));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt.db");
    let mut receiver = Engine::open(&path).unwrap();
    receiver.receive_capsule(&capsule, &host()).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE revisions SET data=json_set(data,'$.nodes[0].entity_id','tampered')",
        [],
    )
    .unwrap();
    assert_eq!(
        receiver
            .export_capsule(&capsule.root, &host())
            .unwrap_err()
            .code,
        "E_INTEGRITY"
    );
    assert_eq!(
        receiver
            .accept_revision(&capsule.root, "offline", None, &host())
            .unwrap_err()
            .code,
        "E_INTEGRITY"
    );
    assert_eq!(receiver.event_count().unwrap(), 0);
    assert!(receiver.head("source", "offline").unwrap().is_none());
}
