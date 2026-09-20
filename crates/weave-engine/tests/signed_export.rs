//! Native signed-export pairing, complete traversal and compatibility boundaries.
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
fn pairing_rotation_subject_and_query_body_cannot_reuse_export_authority() {
    let f = Fixture::new();
    let mut e = Engine::memory_with_clock(f.clock.clone()).unwrap();
    f.install(&e);
    let root = write(&mut e, "g", data());
    let request = f.request(root.clone());
    let proof = f.proof(&request, &["g"], 20, 100, 1000);
    let before = f.clock.samples();
    let events = e.event_count().unwrap();
    let response = e
        .admit_capsule_export(&proof, &request, &f.signer())
        .unwrap();
    assert_eq!(f.clock.samples(), before + 1);
    assert_eq!(e.event_count().unwrap(), events);
    let rotated_key = SigningKey::from_bytes(&[64; 32]);
    let rotated = CapsuleExportSigner::new(
        rotated_key.clone(),
        "root-export-server".into(),
        BTreeMap::from([(
            public_key(&f.user),
            BTreeSet::from(["root-export-recipient".into()]),
        )]),
    )
    .unwrap();
    assert_eq!(
        e.admit_capsule_export(&proof, &request, &rotated)
            .unwrap_err()
            .code,
        "E_EXPORT_BINDING"
    );
    let mut new_request = request.clone();
    new_request.server_key = public_key(&rotated_key);
    let changed = f.proof(&new_request, &["g"], 20, 100, 1000);
    assert_eq!(
        e.admit_capsule_export(&changed, &new_request, &rotated)
            .unwrap_err()
            .code,
        "E_REPLAY"
    );
    let unpaired = CapsuleExportSigner::new(
        f.server.clone(),
        "root-export-server".into(),
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        e.admit_capsule_export(&proof, &request, &unpaired)
            .unwrap_err()
            .code,
        "E_EXPORT_BINDING"
    );
    assert!(CapsuleExportExpectation::new(
        public_key(&f.server),
        "root-export-server".into(),
        public_key(&f.issuer),
        "root-export-recipient".into(),
        request.clone(),
        proof.request.request.clone()
    )
    .is_err());
    let q: weave_contract::QueryPlan =
        serde_json::from_value(json!({"graph_id":"g","revision":root.revision})).unwrap();
    assert_eq!(
        e.admit_query(&proof, &q).unwrap_err().code,
        "E_REQUEST_BINDING"
    );
    assert!(serde_json::from_value::<weave_contract::QueryPlan>(
        serde_json::to_value(&request).unwrap()
    )
    .is_err());
    e.verify_capsule_export_response(&response.result, &f.expectation(&request, &proof))
        .unwrap();
    assert_eq!(e.event_count().unwrap(), events);
}

#[test]
fn structural_pin_is_exported_and_missing_or_hidden_closure_has_one_denial() {
    let f = Fixture::new();
    let mut e = Engine::memory_with_clock(f.clock.clone()).unwrap();
    f.install(&e);
    let source = write(
        &mut e,
        "B",
        json!({"profile":"explicit","nodes":[{"id":"n","entity_id":"n","space_id":"s"}],"structural_edges":[{"id":"relation","predicate":"p","from":"n","to":"n"}]}),
    );
    let root = write(
        &mut e,
        "A",
        json!({"nodes":[{"id":"n","entity_id":"n","space_id":"s"}],"edges":[{"id":"e","predicate":"p","from":"n","to":"n","valid_time":{"start":0},"structural_ref":{"graph_id":"B","revision":source.revision,"edge_id":"relation"}}]}),
    );
    let request = f.request(root);
    let narrow = f.proof(&request, &["A"], 21, 100, 1000);
    assert_eq!(
        e.admit_capsule_export(&narrow, &request, &f.signer())
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
    let wide = f.proof(&request, &["A", "B"], 21, 100, 1000);
    let response = e
        .admit_capsule_export(&wide, &request, &f.signer())
        .unwrap();
    assert_eq!(response.result.capsule.revisions.len(), 2);
    // Both absent and wholly hidden requested values use identical native denial text.
    let missing = f.request(GraphRef {
        graph_id: "g".into(),
        revision: "missing".into(),
    });
    let a = e
        .admit_capsule_export(
            &f.proof(&missing, &["g"], 22, 100, 1000),
            &missing,
            &f.signer(),
        )
        .unwrap_err();
    let hidden = write(
        &mut e,
        "g",
        json!({"nodes":[{"id":"n","entity_id":"n","space_id":"s","readers":["not-recipient"]}]}),
    );
    let hidden = f.request(hidden);
    let b = e
        .admit_capsule_export(
            &f.proof(&hidden, &["g"], 23, 100, 1000),
            &hidden,
            &f.signer(),
        )
        .unwrap_err();
    assert_eq!((a.code, a.message), (b.code, b.message));
}

#[test]
fn ancestry_and_quarantine_are_not_grants_or_accepted_history() {
    let f = Fixture::new();
    let mut e = Engine::memory_with_clock(f.clock.clone()).unwrap();
    f.install(&e);
    let parent = write(&mut e, "g", data());
    let mut changed = data();
    changed["nodes"][0]["properties"] = json!({"v":2});
    let root = write(&mut e, "g", changed);
    let request = f.request(root.clone());
    let proof = f.proof(&request, &["g"], 24, 100, 1000);
    let response = e
        .admit_capsule_export(&proof, &request, &f.signer())
        .unwrap();
    assert!(response
        .result
        .capsule
        .revisions
        .iter()
        .any(|r| r.revision == parent.revision));
    let mut peer = Engine::memory_with_clock(f.clock.clone()).unwrap();
    f.install(&peer);
    peer.receive_capsule(
        &response.result.capsule,
        &HostContext::new(public_key(&f.user), ["g".into()]),
    )
    .unwrap();
    assert_eq!(
        peer.admit_capsule_export(&proof, &request, &f.signer())
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
    assert_eq!(peer.event_count().unwrap(), 0);
    peer.accept_revision(
        &root,
        "main",
        None,
        &HostContext::new(public_key(&f.user), ["g".into()]),
    )
    .unwrap();
    assert!(peer
        .admit_capsule_export(&proof, &request, &f.signer())
        .is_ok());
}

#[test]
fn receipt_bounds_and_backpressure_fail_without_consuming_new_nonce() {
    let f = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.sqlite");
    let mut e = Engine::open_with_clock(&db, f.clock.clone()).unwrap();
    f.install(&e);
    let root = write(&mut e, "g", data());
    let request = f.request(root);
    let proof = f.proof(&request, &["g"], 25, 100, 1000);
    e.admit_capsule_export(&proof, &request, &f.signer())
        .unwrap();
    let events = e.event_count().unwrap();
    let sql = rusqlite::Connection::open(&db).unwrap();
    let original: String = sql
        .query_row("SELECT operation FROM admission_receipts", [], |r| r.get(0))
        .unwrap();
    sql.execute(
        "UPDATE admission_receipts SET operation=?1",
        ["x".repeat(16385)],
    )
    .unwrap();
    assert_eq!(
        e.admit_capsule_export(&proof, &request, &f.signer())
            .unwrap_err()
            .code,
        "E_BUDGET"
    );
    sql.execute("UPDATE admission_receipts SET operation=?1", [original])
        .unwrap();
    sql.execute_batch("WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n<9999) INSERT INTO admission_receipts SELECT 'quota-'||n,subject,epoch,body_digest,operation,response FROM numbers CROSS JOIN (SELECT * FROM admission_receipts LIMIT 1)").unwrap();
    let fresh = f.proof(&request, &["g"], 26, 100, 1000);
    assert_eq!(
        e.admit_capsule_export(&fresh, &request, &f.signer())
            .unwrap_err()
            .code,
        "E_BACKPRESSURE"
    );
    assert_eq!(
        sql.query_row("SELECT COUNT(*) FROM admission_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        10000
    );
    assert_eq!(e.event_count().unwrap(), events);
}

#[test]
fn cached_signature_does_not_bypass_current_stored_revision_integrity() {
    let f = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.sqlite");
    let mut e = Engine::open_with_clock(&db, f.clock.clone()).unwrap();
    f.install(&e);
    let root = write(&mut e, "g", data());
    let request = f.request(root);
    let proof = f.proof(&request, &["g"], 27, 100, 1000);
    e.admit_capsule_export(&proof, &request, &f.signer())
        .unwrap();
    let events = e.event_count().unwrap();
    let sql = rusqlite::Connection::open(&db).unwrap();
    sql.execute(
        "UPDATE revisions SET data=?1 WHERE graph_id='g'",
        ["{\"nodes\":[],\"edges\":[]}"],
    )
    .unwrap();
    assert_eq!(
        e.admit_capsule_export(&proof, &request, &f.signer())
            .unwrap_err()
            .code,
        "E_INTEGRITY"
    );
    assert_eq!(e.event_count().unwrap(), events);
}

#[test]
fn empty_snapshot_export_and_cached_retry_require_complete_scope() {
    let f = Fixture::new();
    let mut e = Engine::memory_with_clock(f.clock.clone()).unwrap();
    f.install(&e);
    let source = write(&mut e, "A", json!({}));
    let saved = write(&mut e, "g", json!({"influence":{"snapshots":[source]}}));
    let request = f.request(saved);
    let narrow = f.proof(&request, &["g"], 81, 100, 1000);
    assert!(e
        .admit_capsule_export(&narrow, &request, &f.signer())
        .is_err());
    let broad = f.proof(&request, &["A", "g"], 81, 100, 1000);
    let first = e
        .admit_capsule_export(&broad, &request, &f.signer())
        .unwrap();
    assert_eq!(first.result.capsule.format, "weave-capsule-0.3");
    assert_eq!(first.result.capsule.revisions.len(), 2);
    assert!(e
        .admit_capsule_export(&narrow, &request, &f.signer())
        .is_err());
    assert_eq!(
        e.admit_capsule_export(&broad, &request, &f.signer())
            .unwrap()
            .result,
        first.result
    );
}
