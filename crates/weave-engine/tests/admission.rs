use ed25519_dalek::SigningKey;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeSet;
use weave_contract::{Command, GraphData, GraphRef, Program, QueryPlan, SnapshotCommit};
use weave_engine::{Engine, HostContext};
use weave_policy::*;
struct Fixture {
    root: SigningKey,
    user: SigningKey,
    context: AdmissionContext,
}
impl Fixture {
    fn new() -> Self {
        let root = SigningKey::from_bytes(&[11; 32]);
        let user = SigningKey::from_bytes(&[22; 32]);
        let context = AdmissionContext {
            audience: "test-replica".into(),
            now_ms: 200,
            policy_epoch: "epoch:1".into(),
            roots: vec![RootAuthority {
                issuer: public_key(&root),
                audience: "test-replica".into(),
                policy_revision: "policy:1".into(),
                scopes: scopes(&["g", "evidence", "typed"]),
                not_before_ms: 0,
                expires_at_ms: 10000,
                max_delegations: 1,
            }],
            revoked_capabilities: BTreeSet::new(),
            revoked_keys: BTreeSet::new(),
            consumed_nonces: BTreeSet::new(),
        };
        Self {
            root,
            user,
            context,
        }
    }
    fn proof(
        &self,
        body: &impl Serialize,
        action: Action,
        graph: &str,
        grants: Vec<Scope>,
        nonce: u8,
    ) -> AdmissionProof {
        let cap = sign_capability(
            Capability {
                version: VERSION.into(),
                issuer: public_key(&self.root),
                subject: public_key(&self.user),
                audience: self.context.audience.clone(),
                policy_revision: "policy:1".into(),
                scopes: grants,
                not_before_ms: 10,
                expires_at_ms: 9000,
                delegations_remaining: 0,
                parent: None,
            },
            &self.root,
        )
        .unwrap();
        let request = sign_request(
            Request {
                version: REQUEST_VERSION.into(),
                subject: public_key(&self.user),
                audience: self.context.audience.clone(),
                capability_id: capability_id(&cap).unwrap(),
                nonce: format!("{nonce:02x}").repeat(32),
                issued_at_ms: 100,
                expires_at_ms: 1000,
                operation: Operation {
                    action,
                    graph_id: graph.into(),
                    branch_id: "main".into(),
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
    fn install(&self, e: &Engine) {
        e.install_admission_policy(&self.context).unwrap();
    }
}
fn scopes(graphs: &[&str]) -> Vec<Scope> {
    let mut result: Vec<_> = graphs
        .iter()
        .map(|g| Scope {
            graph_id: (*g).into(),
            branch_id: "main".into(),
            actions: [
                Action::Read,
                Action::Traverse,
                Action::Propose,
                Action::Publish,
            ]
            .into(),
        })
        .collect();
    result.sort();
    result
}
fn data() -> GraphData {
    serde_json::from_value(json!({"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"e","predicate":"p","from":"a","to":"b","valid_time":{"start":0}}]})).unwrap()
}
fn write(e: &mut Engine, graph: &str, branch: &str, data: GraphData) -> String {
    let expected_head = e.head(graph, branch).unwrap();
    e.execute(
        &Program {
            version: weave_contract::VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: graph.into(),
                branch_id: branch.into(),
                expected_head,
                data,
            }],
        },
        &HostContext::new("local", [graph.into()]),
    )
    .unwrap();
    e.head(graph, branch).unwrap().unwrap()
}
fn query() -> QueryPlan {
    serde_json::from_value(json!({"graph_id":"g","include_metadata":true})).unwrap()
}
fn private(mut d: GraphData, f: &Fixture) -> GraphData {
    let p = public_key(&f.user);
    for n in &mut d.nodes {
        n.readers = vec![p.clone()];
    }
    for e in &mut d.edges {
        e.readers = vec![p.clone()];
    }
    d
}
#[test]
fn signed_read_receipt_is_pinned_durable_and_nonce_bound() {
    let f = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let mut e = Engine::open(&db).unwrap();
    f.install(&e);
    let first = write(&mut e, "g", "main", data());
    let q = query();
    let p = f.proof(&q, Action::Read, "g", scopes(&["g"]), 1);
    let r = e.admit_query(&p, &q, 200).unwrap();
    assert!(!r.duplicate);
    assert_eq!(r.result.input_snapshots[0].revision, first);
    let mut changed = data();
    changed.nodes[0].properties.insert("v".into(), json!(2));
    write(&mut e, "g", "main", changed);
    drop(e);
    let mut e = Engine::open(&db).unwrap();
    let replay = e.admit_query(&p, &q, 201).unwrap();
    assert!(replay.duplicate);
    assert_eq!(replay.result, r.result);
    let mut changed = q.clone();
    changed.predicate = Some("other".into());
    let p2 = f.proof(&changed, Action::Read, "g", scopes(&["g"]), 1);
    assert_eq!(
        e.admit_query(&p2, &changed, 202).unwrap_err().code,
        "E_REPLAY"
    );
    assert_eq!(e.event_count().unwrap(), 2);
}
#[test]
fn pinned_read_cannot_escape_branch_but_accepted_fork_history_is_valid() {
    let f = Fixture::new();
    let mut e = Engine::memory().unwrap();
    f.install(&e);
    write(&mut e, "g", "main", data());
    let mut secret = data();
    secret.nodes[0]
        .properties
        .insert("secret".into(), json!(true));
    let hidden = write(&mut e, "g", "private", secret);
    let mut q = query();
    q.revision = Some(hidden);
    let p = f.proof(&q, Action::Read, "g", scopes(&["g"]), 2);
    assert_eq!(e.admit_query(&p, &q, 200).unwrap_err().code, "E_SCOPE");
    let mut other = Engine::memory().unwrap();
    f.install(&other);
    let rev = write(&mut other, "g", "private", data());
    other
        .fork_branch(
            &GraphRef {
                graph_id: "g".into(),
                revision: rev.clone(),
            },
            "main",
            &HostContext::new("local", ["g".into()]),
        )
        .unwrap();
    q.revision = Some(rev);
    let p = f.proof(&q, Action::Read, "g", scopes(&["g"]), 2);
    assert!(other.admit_query(&p, &q, 200).is_ok());
}
#[test]
fn metadata_and_provenance_require_scopes_and_narrow_retry_cannot_recover_broad_receipt() {
    let f = Fixture::new();
    let mut e = Engine::memory().unwrap();
    f.install(&e);
    let evidence = write(&mut e, "evidence", "main", data());
    let mut d = data();
    d.nodes[0].metadata.push(GraphRef {
        graph_id: "evidence".into(),
        revision: evidence.clone(),
    });
    d.edges[0].derived_from.push(weave_contract::AssertionRef {
        graph_id: "evidence".into(),
        revision: evidence,
        assertion_id: "e".into(),
    });
    write(&mut e, "g", "main", d);
    let q = query();
    let narrow = f.proof(&q, Action::Read, "g", scopes(&["g"]), 3);
    assert_eq!(e.admit_query(&narrow, &q, 200).unwrap_err().code, "E_SCOPE");
    let broad = f.proof(&q, Action::Read, "g", scopes(&["g", "evidence"]), 3);
    let result = e.admit_query(&broad, &q, 200).unwrap();
    assert_eq!(result.result.graph.edges.len(), 1);
    assert!(!result.duplicate);
    assert_eq!(e.admit_query(&narrow, &q, 201).unwrap_err().code, "E_SCOPE");
}
#[test]
fn policy_changes_invalidate_receipts_and_epochs_cannot_be_reactivated() {
    let mut f = Fixture::new();
    let mut e = Engine::memory().unwrap();
    f.install(&e);
    write(&mut e, "g", "main", data());
    let q = query();
    let p = f.proof(&q, Action::Read, "g", scopes(&["g"]), 4);
    e.admit_query(&p, &q, 200).unwrap();
    f.context.policy_epoch = "epoch:2".into();
    f.install(&e);
    assert_eq!(
        e.admit_query(&p, &q, 201).unwrap_err().code,
        "E_POLICY_CHANGED"
    );
    f.context.policy_epoch = "epoch:1".into();
    assert_eq!(
        e.install_admission_policy(&f.context).unwrap_err().code,
        "E_POLICY_EPOCH"
    );
    f.context.policy_epoch = "epoch:3".into();
    f.context.revoked_keys.insert(public_key(&f.user));
    f.install(&e);
    assert!(e.admit_query(&p, &q, 202).is_err());
}
#[test]
fn publication_egress_is_atomic_subject_scoped_and_replay_safe() {
    let f = Fixture::new();
    let mut e = Engine::memory().unwrap();
    f.install(&e);
    let mut c = SnapshotCommit {
        graph_id: "g".into(),
        branch_id: "main".into(),
        expected_head: None,
        data: data(),
    };
    let p = f.proof(&c, Action::Publish, "g", scopes(&["g"]), 5);
    assert_eq!(e.admit_publish(&p, &c, 200).unwrap_err().code, "E_EGRESS");
    assert_eq!(e.event_count().unwrap(), 0);
    assert_eq!(e.head("g", "main").unwrap(), None);
    c.data = private(c.data, &f);
    let p = f.proof(&c, Action::Publish, "g", scopes(&["g"]), 5);
    let r = e.admit_publish(&p, &c, 200).unwrap();
    assert!(!r.duplicate);
    assert!(e.admit_publish(&p, &c, 201).unwrap().duplicate);
    assert_eq!(e.event_count().unwrap(), 1);
    assert!(e
        .query(&query(), &HostContext::new("outsider", []))
        .unwrap()
        .graph
        .nodes
        .is_empty());
}
#[test]
fn new_schema_cannot_enter_through_remote_publication() {
    let f = Fixture::new();
    let mut e = Engine::memory().unwrap();
    f.install(&e);
    let schema =
        serde_json::from_value(json!({"id":"schema","revision":"1","nodes":{},"edges":{}}))
            .unwrap();
    let mut c = SnapshotCommit {
        graph_id: "typed".into(),
        branch_id: "main".into(),
        expected_head: None,
        data: GraphData::default(),
    };
    c.data.schema = Some(schema);
    let p = f.proof(&c, Action::Publish, "typed", scopes(&["typed"]), 6);
    assert_eq!(
        e.admit_publish(&p, &c, 200).unwrap_err().code,
        "E_SCHEMA_AUTHORITY"
    );
    assert_eq!(e.event_count().unwrap(), 0);
    write(&mut e, "g", "main", c.data.clone());
    assert!(e.admit_publish(&p, &c, 200).is_ok());
}
#[test]
fn isolated_proposal_cannot_poison_structure_registry_or_heads() {
    let f = Fixture::new();
    let mut donor = Engine::memory().unwrap();
    let rev = write(&mut donor, "g", "main", data());
    let capsule = donor
        .export_capsule(
            &GraphRef {
                graph_id: "g".into(),
                revision: rev,
            },
            &HostContext::new("local", []),
        )
        .unwrap();
    let mut e = Engine::memory().unwrap();
    f.install(&e);
    let p = f.proof(&capsule, Action::Propose, "g", scopes(&["g"]), 7);
    let r = e.admit_proposal(&p, &capsule, 200).unwrap();
    assert!(!r.duplicate);
    assert!(e.admit_proposal(&p, &capsule, 201).unwrap().duplicate);
    assert_eq!(e.head("g", "main").unwrap(), None);
    assert_eq!(e.event_count().unwrap(), 0);
    let mut different = data();
    different.edges[0].predicate = "different".into();
    write(&mut e, "g", "main", different);
    assert_eq!(e.event_count().unwrap(), 1);
}
#[test]
fn publication_does_not_smuggle_out_of_scope_provenance() {
    let f = Fixture::new();
    let mut e = Engine::memory().unwrap();
    f.install(&e);
    let evidence = write(&mut e, "evidence", "main", data());
    let mut d = private(data(), &f);
    d.edges[0].derived_from.push(weave_contract::AssertionRef {
        graph_id: "evidence".into(),
        revision: evidence,
        assertion_id: "e".into(),
    });
    let c = SnapshotCommit {
        graph_id: "g".into(),
        branch_id: "main".into(),
        expected_head: None,
        data: d,
    };
    let p = f.proof(&c, Action::Publish, "g", scopes(&["g"]), 8);
    assert_eq!(e.admit_publish(&p, &c, 200).unwrap_err().code, "E_SCOPE");
    assert_eq!(e.event_count().unwrap(), 1);
    assert_eq!(e.head("g", "main").unwrap(), None);
}
#[test]
fn receipt_capacity_failure_rolls_back_publication_and_nonce() {
    let f = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("quota.db");
    let mut e = Engine::open(&db).unwrap();
    f.install(&e);
    // Fill the configured host receipt quota without paying signature work for fixture setup.
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) INSERT INTO admission_receipts SELECT CAST(x AS TEXT),?1,'fixture','digest','operation','{}' FROM n",[public_key(&f.user)]).unwrap();
    let c = SnapshotCommit {
        graph_id: "g".into(),
        branch_id: "main".into(),
        expected_head: None,
        data: private(data(), &f),
    };
    let p = f.proof(&c, Action::Publish, "g", scopes(&["g"]), 9);
    assert_eq!(
        e.admit_publish(&p, &c, 200).unwrap_err().code,
        "E_BACKPRESSURE"
    );
    assert_eq!(e.event_count().unwrap(), 0);
    assert_eq!(e.head("g", "main").unwrap(), None);
    conn.execute("DELETE FROM admission_receipts WHERE epoch='fixture'", [])
        .unwrap();
    assert!(!e.admit_publish(&p, &c, 201).unwrap().duplicate);
    assert_eq!(e.event_count().unwrap(), 1);
}
#[test]
fn unsupported_live_publication_and_expired_read_retry_fail_closed() {
    let f = Fixture::new();
    let mut e = Engine::memory().unwrap();
    f.install(&e);
    write(&mut e, "g", "main", data());
    let q = query();
    let p = f.proof(&q, Action::Read, "g", scopes(&["g"]), 10);
    e.admit_query(&p, &q, 200).unwrap();
    assert!(e.admit_query(&p, &q, 1000).is_err());
    let mut d = private(data(), &f);
    d.attachments.push(serde_json::from_value(json!({"id":"live","host":{"kind":"graph"},"key":"evidence","value":{"kind":"live_graph","graph_id":"evidence","branch_id":"main"},"valid_time":{"start":0},"readers":[public_key(&f.user)]})).unwrap());
    let c = SnapshotCommit {
        graph_id: "g".into(),
        branch_id: "main".into(),
        expected_head: e.head("g", "main").unwrap(),
        data: d,
    };
    let p = f.proof(&c, Action::Publish, "g", scopes(&["g"]), 11);
    assert_eq!(
        e.admit_publish(&p, &c, 200).unwrap_err().code,
        "E_UNSUPPORTED"
    );
    assert_eq!(e.event_count().unwrap(), 1);
}
#[test]
fn hidden_objects_do_not_influence_scope_checks_or_visible_payload() {
    let f = Fixture::new();
    let mut baseline = Engine::memory().unwrap();
    f.install(&baseline);
    write(&mut baseline, "g", "main", data());
    let q = query();
    let p = f.proof(&q, Action::Read, "g", scopes(&["g"]), 12);
    let plain = baseline.admit_query(&p, &q, 200).unwrap().result;
    let mut d = data();
    let mut hidden = d.nodes[0].clone();
    hidden.id = "hidden".into();
    hidden.readers = vec!["private-principal".into()];
    hidden.metadata.push(GraphRef {
        graph_id: "forbidden".into(),
        revision: "missing".into(),
    });
    d.nodes.push(hidden);
    let mut edge = d.edges[0].clone();
    edge.id = "hidden-edge".into();
    edge.readers = vec!["private-principal".into()];
    edge.derived_from.push(weave_contract::AssertionRef {
        graph_id: "forbidden".into(),
        revision: "missing".into(),
        assertion_id: "secret".into(),
    });
    d.edges.push(edge);
    d.attachments.push(serde_json::from_value(json!({"id":"hidden-attachment","host":{"kind":"graph"},"key":"private","value":{"kind":"live_graph","graph_id":"forbidden","branch_id":"secret"},"valid_time":{"start":0},"readers":["private-principal"]})).unwrap());
    let mut with_hidden = Engine::memory().unwrap();
    f.install(&with_hidden);
    write(&mut with_hidden, "g", "main", d);
    let result = with_hidden.admit_query(&p, &q, 200).unwrap().result;
    assert_eq!(result.graph, plain.graph);
    assert_eq!(result.coverage, plain.coverage);
    assert_eq!(result.metadata_graphs, plain.metadata_graphs);
    assert_eq!(result.input_snapshots.len(), plain.input_snapshots.len());
    // Immutable whole-snapshot revision pins differ, intentionally; no hidden dependency identifiers escape.
    assert!(!serde_json::to_string(&result)
        .unwrap()
        .contains("forbidden"));
}
#[test]
fn cached_live_read_retains_original_dependency_closure_after_heads_advance() {
    let f = Fixture::new();
    let mut e = Engine::memory().unwrap();
    f.install(&e);
    let old = write(&mut e, "evidence", "main", data());
    let mut d = data();
    d.attachments.push(serde_json::from_value(json!({"id":"live","host":{"kind":"graph"},"key":"evidence","value":{"kind":"live_graph","graph_id":"evidence","branch_id":"main"},"valid_time":{"start":0}})).unwrap());
    write(&mut e, "g", "main", d);
    let q = query();
    let broad = f.proof(&q, Action::Read, "g", scopes(&["g", "evidence"]), 13);
    let original = e.admit_query(&broad, &q, 200).unwrap().result;
    assert!(original
        .input_snapshots
        .iter()
        .any(|r| r.graph_id == "evidence" && r.revision == old));
    let mut replacement = data();
    replacement.nodes[0]
        .properties
        .insert("new".into(), json!(true));
    write(&mut e, "evidence", "main", replacement);
    let narrow = f.proof(&q, Action::Read, "g", scopes(&["g"]), 13);
    assert_eq!(e.admit_query(&narrow, &q, 201).unwrap_err().code, "E_SCOPE");
    assert_eq!(e.admit_query(&broad, &q, 201).unwrap().result, original);
}

#[test]
fn signed_node_only_scalar_requires_its_private_proof_scope_on_read_and_retry() {
    let f = Fixture::new();
    let mut e = Engine::memory().unwrap();
    f.install(&e);
    let revision = write(&mut e, "evidence", "main", private(data(), &f));
    let graph:GraphData=serde_json::from_value(json!({"nodes":[{"id":"scalar","entity_id":"result","space_id":"analysis","properties":{"value":42},"derived_from":[{"graph_id":"evidence","revision":revision,"assertion_id":"e"}]}]})).unwrap();
    write(&mut e, "g", "main", graph);
    let q = query();
    let narrow = f.proof(&q, Action::Read, "g", scopes(&["g"]), 22);
    assert_eq!(e.admit_query(&narrow, &q, 200).unwrap_err().code, "E_SCOPE");
    let broad = f.proof(&q, Action::Read, "g", scopes(&["g", "evidence"]), 22);
    let result = e.admit_query(&broad, &q, 200).unwrap();
    assert_eq!(result.result.graph.nodes[0].properties["value"], 42);
    assert!(result
        .result
        .input_snapshots
        .iter()
        .any(|r| r.graph_id == "evidence"));
    assert_eq!(e.admit_query(&narrow, &q, 201).unwrap_err().code, "E_SCOPE");
    assert!(e.admit_query(&broad, &q, 201).unwrap().duplicate);
}
