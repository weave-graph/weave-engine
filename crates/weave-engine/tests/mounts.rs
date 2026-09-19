use serde_json::json;
use weave_contract::*;
use weave_engine::*;
fn host(actor: &str) -> HostContext {
    HostContext::new(actor, ["source".into(), "copy".into()])
}
fn source(e: &mut Engine) -> GraphRef {
    e.execute(&serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"source","data":{"nodes":[{"id":"a","entity_id":"a","space_id":"physical"},{"id":"b","entity_id":"b","space_id":"operations"}]}}]})).unwrap(),&host("alice")).unwrap();
    GraphRef {
        graph_id: "source".into(),
        revision: e.head("source", "main").unwrap().unwrap(),
    }
}
#[test]
fn received_mount_routes_persist_detach_and_never_accept_or_delete_graphs() {
    let mut sender = Engine::memory().unwrap();
    let root = source(&mut sender);
    let capsule = sender.export_capsule(&root, &host("alice")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("receiver.db");
    let mut receiver = Engine::open(&path).unwrap();
    receiver.receive_capsule(&capsule, &host("alice")).unwrap();
    let spec = MountSpec {
        id: "working-set".into(),
        reference: root.clone(),
    };
    let first = receiver
        .attach_mount(&spec, None, "attach", &host("alice"))
        .unwrap();
    assert_eq!(first.generation, 1);
    assert_eq!(receiver.event_count().unwrap(), 0);
    assert!(receiver.head("source", "main").unwrap().is_none());
    assert!(
        receiver
            .attach_mount(&spec, None, "attach", &host("alice"))
            .unwrap()
            .duplicate
    );
    assert_eq!(
        receiver
            .query_mount("working-set", &host("alice"))
            .unwrap()
            .graph
            .nodes
            .len(),
        2
    );
    drop(receiver);
    let mut receiver = Engine::open(&path).unwrap();
    assert_eq!(
        receiver
            .query_mount("working-set", &host("alice"))
            .unwrap()
            .graph
            .nodes
            .len(),
        2
    );
    assert_eq!(
        receiver
            .detach_mount("working-set", 0, "stale", &host("alice"))
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    let detached = receiver
        .detach_mount("working-set", 1, "detach", &host("alice"))
        .unwrap();
    assert_eq!(detached.generation, 2);
    assert_eq!(
        receiver
            .query_mount("working-set", &host("alice"))
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
    assert!(
        receiver
            .detach_mount("working-set", 1, "detach", &host("alice"))
            .unwrap()
            .duplicate
    );
    let direct: QueryPlan =
        serde_json::from_value(json!({"graph_id":root.graph_id,"revision":root.revision})).unwrap();
    assert_eq!(
        receiver
            .query(&direct, &host("alice"))
            .unwrap()
            .graph
            .nodes
            .len(),
        2
    );
    let reattached = receiver
        .attach_mount(&spec, Some(2), "reattach", &host("alice"))
        .unwrap();
    assert_eq!(reattached.generation, 3);
    assert_eq!(
        receiver
            .mount_changes(0, 100, &host("alice"))
            .unwrap()
            .len(),
        3
    );
    assert_eq!(receiver.event_count().unwrap(), 0);
}
#[test]
fn routes_receipts_and_lifecycle_cursors_are_principal_local_and_source_is_immutable() {
    let mut e = Engine::memory().unwrap();
    let root = source(&mut e);
    let spec = MountSpec {
        id: "route".into(),
        reference: root,
    };
    e.attach_mount(&spec, None, "nonce", &host("alice"))
        .unwrap();
    assert!(e.mount_changes(0, 100, &host("bob")).unwrap().is_empty());
    assert_eq!(
        e.query_mount("route", &host("bob")).unwrap_err().code,
        "E_UNAVAILABLE"
    );
    e.attach_mount(&spec, None, "nonce", &host("bob")).unwrap();
    assert_eq!(e.mount_changes(0, 100, &host("bob")).unwrap()[0].cursor, 1);
    assert_eq!(
        e.detach_mount("route", 1, "nonce", &host("alice"))
            .unwrap_err()
            .code,
        "E_REPLAY"
    );
    e.detach_mount("route", 1, "detach", &host("alice"))
        .unwrap();
    let mut changed = spec.clone();
    changed.reference.graph_id = "copy".into();
    assert!(e
        .attach_mount(&changed, Some(2), "replace", &host("alice"))
        .is_err());
    assert_eq!(e.mount_changes(1, 1, &host("alice")).unwrap()[0].cursor, 2);
    assert_eq!(
        e.mount_changes(0, 101, &host("alice")).unwrap_err().code,
        "E_BUDGET"
    );
}
#[test]
fn route_read_and_attach_retry_recheck_original_proof_after_policy_revocation() {
    let mut e = Engine::memory().unwrap();
    let root = source(&mut e);
    let policy = IdentityPolicy {
        reference: IdentityPolicyRef {
            id: "policy".into(),
            revision: "1".into(),
        },
        proposers: vec!["alice".into()],
        approvers: vec!["alice".into()],
        readers: vec![],
        allowed_spaces: vec!["physical".into(), "operations".into()],
        max_members: 2,
    };
    e.install_identity_policy(&policy).unwrap();
    let candidate = IdentityCandidate {
        id: "candidate".into(),
        mapping_id: "mapping".into(),
        policy: policy.reference.clone(),
        groups: vec![vec![
            NodeRef {
                graph_id: root.graph_id.clone(),
                revision: root.revision.clone(),
                node_id: "a".into(),
            },
            NodeRef {
                graph_id: root.graph_id,
                revision: root.revision,
                node_id: "b".into(),
            },
        ]],
        evidence: vec![],
        valid_time: Interval {
            start: 0,
            end: None,
        },
        context: None,
    };
    e.submit_identity_candidate(&candidate, &host("alice"))
        .unwrap();
    let accepted = e
        .accept_identity_candidate(
            &IdentityDecisionRequest {
                candidate_id: candidate.id,
                expected_head: None,
                nonce: "decision".into(),
            },
            &host("alice"),
        )
        .unwrap();
    let data=e.query(&serde_json::from_value(json!({"graph_id":accepted.reference.graph_id,"revision":accepted.reference.revision})).unwrap(),&host("alice")).unwrap().graph;
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "copy".into(),
                branch_id: "main".into(),
                expected_head: None,
                data,
            }],
        },
        &host("alice"),
    )
    .unwrap();
    let spec = MountSpec {
        id: "derived-route".into(),
        reference: GraphRef {
            graph_id: "copy".into(),
            revision: e.head("copy", "main").unwrap().unwrap(),
        },
    };
    e.attach_mount(&spec, None, "attach", &host("alice"))
        .unwrap();
    e.revoke_identity_policy(&policy.reference).unwrap();
    assert_eq!(
        e.query_mount(&spec.id, &host("alice")).unwrap_err().code,
        "E_UNAVAILABLE"
    );
    assert_eq!(
        e.attach_mount(&spec, None, "attach", &host("alice"))
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
    assert!(
        !e.detach_mount(&spec.id, 1, "cleanup", &host("alice"))
            .unwrap()
            .active
    );
}
