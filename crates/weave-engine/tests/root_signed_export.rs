//! Independent export closure, persisted-receipt integrity and replay boundaries.
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use weave_contract::{Command, GraphRef, Program};
use weave_engine::{
    CapsuleExportExpectation, CapsuleExportRequest, CapsuleExportSigner, Engine, HostContext,
    ManualClock,
};
use weave_policy::*;

struct Fixture {
    issuer: SigningKey,
    user: SigningKey,
    server: SigningKey,
    clock: Arc<ManualClock>,
}
fn scopes(graphs: &[&str]) -> Vec<Scope> {
    let mut values: Vec<_> = graphs
        .iter()
        .map(|g| Scope {
            graph_id: (*g).into(),
            branch_id: "main".into(),
            actions: [Action::Read, Action::Traverse].into(),
        })
        .collect();
    values.sort();
    values
}
impl Fixture {
    fn new() -> Self {
        Self {
            issuer: SigningKey::from_bytes(&[61; 32]),
            user: SigningKey::from_bytes(&[62; 32]),
            server: SigningKey::from_bytes(&[63; 32]),
            clock: Arc::new(ManualClock::new(200)),
        }
    }
    fn install(&self, e: &Engine) {
        e.install_admission_policy(&AdmissionContext {
            audience: "root-export-server".into(),
            now_ms: 200,
            policy_epoch: "root-export-epoch".into(),
            roots: vec![RootAuthority {
                issuer: public_key(&self.issuer),
                audience: "root-export-server".into(),
                policy_revision: "root-export-policy".into(),
                scopes: scopes(&["A", "B", "g"]),
                not_before_ms: 0,
                expires_at_ms: 10000,
                max_delegations: 0,
            }],
            revoked_capabilities: BTreeSet::new(),
            revoked_keys: BTreeSet::new(),
            consumed_nonces: BTreeSet::new(),
        })
        .unwrap();
    }
    fn signer(&self) -> CapsuleExportSigner {
        CapsuleExportSigner::new(
            self.server.clone(),
            "root-export-server".into(),
            BTreeMap::from([(
                public_key(&self.user),
                BTreeSet::from(["root-export-recipient".into()]),
            )]),
        )
        .unwrap()
    }
    fn request(&self, root: GraphRef) -> CapsuleExportRequest {
        CapsuleExportRequest {
            format: "weave-capsule-export-request-0.1".into(),
            root,
            branch_id: "main".into(),
            server_key: public_key(&self.server),
            response_audience: "root-export-recipient".into(),
        }
    }
    fn proof(
        &self,
        body: &CapsuleExportRequest,
        graphs: &[&str],
        nonce: u8,
        issued: i64,
        expires: i64,
    ) -> AdmissionProof {
        let cap = sign_capability(
            Capability {
                version: VERSION.into(),
                issuer: public_key(&self.issuer),
                subject: public_key(&self.user),
                audience: "root-export-server".into(),
                policy_revision: "root-export-policy".into(),
                scopes: scopes(graphs),
                not_before_ms: 0,
                expires_at_ms: 9000,
                delegations_remaining: 0,
                parent: None,
            },
            &self.issuer,
        )
        .unwrap();
        let request = sign_request(
            Request {
                version: REQUEST_VERSION.into(),
                subject: public_key(&self.user),
                audience: "root-export-server".into(),
                capability_id: capability_id(&cap).unwrap(),
                nonce: format!("{nonce:02x}").repeat(32),
                issued_at_ms: issued,
                expires_at_ms: expires,
                operation: Operation {
                    action: Action::Read,
                    graph_id: body.root.graph_id.clone(),
                    branch_id: body.branch_id.clone(),
                },
                body_digest: body_digest(&serde_json::to_vec(body).unwrap()),
            },
            &self.user,
        )
        .unwrap();
        AdmissionProof {
            chain: vec![cap],
            request,
        }
    }
    fn expectation(
        &self,
        request: &CapsuleExportRequest,
        proof: &AdmissionProof,
    ) -> CapsuleExportExpectation {
        CapsuleExportExpectation::new(
            public_key(&self.server),
            "root-export-server".into(),
            public_key(&self.user),
            "root-export-recipient".into(),
            request.clone(),
            proof.request.request.clone(),
        )
        .unwrap()
    }
}
fn execute(e: &mut Engine, commands: Vec<Command>) {
    e.execute(
        &Program {
            version: weave_contract::VERSION.into(),
            source_revisions: vec![],
            commands,
        },
        &HostContext::new("root-test", ["A".into(), "B".into(), "g".into()]),
    )
    .unwrap();
}
fn write(e: &mut Engine, graph: &str, data: Value) -> GraphRef {
    let expected_head = e.head(graph, "main").unwrap();
    execute(
        e,
        vec![Command::Commit {
            graph_id: graph.into(),
            branch_id: "main".into(),
            expected_head,
            data: serde_json::from_value(data).unwrap(),
        }],
    );
    GraphRef {
        graph_id: graph.into(),
        revision: e.head(graph, "main").unwrap().unwrap(),
    }
}
fn data() -> Value {
    json!({"nodes":[{"id":"n","entity_id":"n","space_id":"s"}]})
}

#[test]
fn root_logical_siblings_and_cached_closure_metadata_are_not_ambient_authority() {
    let f = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.sqlite");
    let mut e = Engine::open_with_clock(&db, f.clock.clone()).unwrap();
    f.install(&e);
    let command:Command=serde_json::from_value(json!({"op":"commit_batch","batch_id":"root-export-batch","commits":[{"graph_id":"A","data":data()},{"graph_id":"B","data":data()}]})).unwrap();
    execute(&mut e, vec![command]);
    let request = f.request(GraphRef {
        graph_id: "A".into(),
        revision: e.head("A", "main").unwrap().unwrap(),
    });
    let narrow = f.proof(&request, &["A"], 1, 100, 1000);
    assert_eq!(
        e.admit_capsule_export(&narrow, &request, &f.signer())
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
    let wide = f.proof(&request, &["A", "B"], 1, 100, 1000);
    let admitted = e
        .admit_capsule_export(&wide, &request, &f.signer())
        .unwrap();
    assert!(!admitted.duplicate);
    assert_eq!(admitted.result.capsule.revisions.len(), 2);
    assert!(e
        .admit_capsule_export(&narrow, &request, &f.signer())
        .is_err());
    let original_events = e.event_count().unwrap();
    drop(e);
    let sql = rusqlite::Connection::open(&db).unwrap();
    let text: String = sql
        .query_row("SELECT response FROM admission_receipts", [], |r| r.get(0))
        .unwrap();
    let mut stored: Value = serde_json::from_str(&text).unwrap();
    assert!(stored["dependencies"].as_array().unwrap().len() >= 2);
    stored["dependencies"] = json!([]);
    sql.execute(
        "UPDATE admission_receipts SET response=?1",
        [serde_json::to_string(&stored).unwrap()],
    )
    .unwrap();
    drop(sql);
    let mut e = Engine::open_with_clock(&db, f.clock.clone()).unwrap();
    assert!(
        e.admit_capsule_export(&wide, &request, &f.signer())
            .is_err(),
        "trimmed private closure metadata must not authorize authenticated response"
    );
    assert_eq!(e.event_count().unwrap(), original_events);
}

#[test]
fn root_empty_whole_value_influence_requires_complete_export_scope() {
    let f = Fixture::new();
    let mut e = Engine::memory_with_clock(f.clock.clone()).unwrap();
    f.install(&e);
    let evidence = write(&mut e, "B", data());
    let selected = write(
        &mut e,
        "g",
        json!({"influence":{"nodes":[{"graph_id":"B","revision":evidence.revision,"node_id":"n"}]}}),
    );
    let request = f.request(selected);
    let narrow = f.proof(&request, &["g"], 2, 100, 1000);
    assert_eq!(
        e.admit_capsule_export(&narrow, &request, &f.signer())
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
    let proof = f.proof(&request, &["B", "g"], 2, 100, 1000);
    let response = e
        .admit_capsule_export(&proof, &request, &f.signer())
        .unwrap();
    assert_eq!(response.result.capsule.revisions.len(), 2);
    assert!(response.result.capsule.external_dependencies.is_empty());
    let root = response
        .result
        .capsule
        .revisions
        .iter()
        .find(|r| r.graph_id == "g")
        .unwrap();
    assert!(root.data.nodes.is_empty());
    assert!(root.data.influence.is_some());
}

#[test]
fn root_refreshed_proof_replays_historical_bytes_without_refreshing_remote_attestation() {
    let f = Fixture::new();
    let mut e = Engine::memory_with_clock(f.clock.clone()).unwrap();
    f.install(&e);
    let root = write(&mut e, "g", data());
    let request = f.request(root);
    let original = f.proof(&request, &["g"], 3, 100, 1000);
    let first = e
        .admit_capsule_export(&original, &request, &f.signer())
        .unwrap();
    let bytes = serde_json::to_vec(&first.result).unwrap();
    let events = e.event_count().unwrap();
    f.clock.set(1500);
    assert!(e
        .admit_capsule_export(&original, &request, &f.signer())
        .is_err());
    assert!(e
        .verify_capsule_export_response(&first.result, &f.expectation(&request, &original))
        .is_err());
    let fresh = f.proof(&request, &["g"], 3, 1400, 2000);
    let replay = e
        .admit_capsule_export(&fresh, &request, &f.signer())
        .unwrap();
    assert!(replay.duplicate);
    assert_eq!(serde_json::to_vec(&replay.result).unwrap(), bytes);
    assert_eq!(replay.result.binding.served_at_ms, 200);
    e.verify_capsule_export_response(&replay.result, &f.expectation(&request, &fresh))
        .unwrap();
    let mut altered = replay.result.clone();
    altered.binding.served_at_ms = 1500;
    assert!(e
        .verify_capsule_export_response(&altered, &f.expectation(&request, &fresh))
        .is_err());
    let mut altered = replay.result.clone();
    altered.capsule.revisions[0].data.nodes[0]
        .properties
        .insert("tampered".into(), json!(true));
    assert!(e
        .verify_capsule_export_response(&altered, &f.expectation(&request, &fresh))
        .is_err());
    assert_eq!(e.event_count().unwrap(), events);
}
