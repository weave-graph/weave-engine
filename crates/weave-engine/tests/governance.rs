use ed25519_dalek::SigningKey;
use serde_json::json;
use weave_contract::{Command, GraphData, GraphRef, Program, VERSION};
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("collector", ["source".into()])
}
fn keys() -> [SigningKey; 3] {
    [
        SigningKey::from_bytes(&[31; 32]),
        SigningKey::from_bytes(&[32; 32]),
        SigningKey::from_bytes(&[33; 32]),
    ]
}
fn policy(threshold: usize) -> GovernancePolicy {
    GovernancePolicy {
        view_id: "team".into(),
        reference: GovernancePolicyRef {
            id: "policy".into(),
            revision: "1".into(),
        },
        members: keys()[..2].iter().map(weave_policy::public_key).collect(),
        threshold,
        proposers: vec!["collector".into()],
        readers: vec!["reader".into()],
        allowed_sources: vec![GovernanceSourceScope {
            graph_id: "source".into(),
            branch_id: "main".into(),
        }],
        not_before_ms: 0,
        expires_at_ms: 10000,
    }
}
fn seed(e: &mut Engine, threshold: usize) -> GraphRef {
    e.install_governance_root(&policy(threshold)).unwrap();
    let data: GraphData =
        serde_json::from_value(json!({"nodes":[{"id":"n","entity_id":"E","space_id":"s"}]}))
            .unwrap();
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "source".into(),
                branch_id: "main".into(),
                expected_head: None,
                data,
            }],
        },
        &host(),
    )
    .unwrap();
    GraphRef {
        graph_id: "source".into(),
        revision: e.head("source", "main").unwrap().unwrap(),
    }
}
fn proposal(id: &str, source: GraphRef, expected_head: Option<String>) -> GovernanceProposal {
    GovernanceProposal {
        id: id.into(),
        view_id: "team".into(),
        policy: policy(2).reference,
        expected_head,
        expires_at_ms: 9000,
        action: GovernanceAction::Publish {
            source,
            branch_id: "main".into(),
        },
    }
}
fn signed(
    q: &GovernanceProposal,
    hash: &str,
    key: &SigningKey,
    nonce: &str,
) -> SignedGovernanceApproval {
    sign_governance_approval(
        GovernanceApproval {
            proposal_id: q.id.clone(),
            proposal_digest: hash.into(),
            view_id: q.view_id.clone(),
            policy: q.policy.clone(),
            expected_head: q.expected_head.clone(),
            member: weave_policy::public_key(key),
            issued_at_ms: 10,
            expires_at_ms: 8000,
            nonce: nonce.into(),
        },
        key,
    )
    .unwrap()
}
fn request(id: &str, nonce: &str) -> GovernanceDecisionRequest {
    GovernanceDecisionRequest {
        proposal_id: id.into(),
        nonce: nonce.into(),
    }
}
fn quorum(e: &Engine, q: &GovernanceProposal) -> GovernanceProposalReceipt {
    let r = e.propose_governance(q, 20, &host()).unwrap();
    for (i, key) in keys()[..2].iter().enumerate() {
        e.record_governance_approval(
            &signed(q, &r.digest, key, &format!("{}-{i}", q.id)),
            20,
            &host(),
        )
        .unwrap();
    }
    r
}
#[test]
fn isolated_proposals_threshold_distinct_signers_cas_and_durable_exact_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("governance.db");
    let mut e = Engine::open(&path).unwrap();
    let source = seed(&mut e, 2);
    let a = proposal("a", source.clone(), None);
    let b = proposal("b", source, None);
    let receipt = e.propose_governance(&a, 20, &host()).unwrap();
    assert!(e
        .inspect_governance_head("team", 20, &host())
        .unwrap()
        .decision_id
        .is_none());
    assert_eq!(e.governance_event_count().unwrap(), 0);
    let approval = signed(&a, &receipt.digest, &keys()[0], "one");
    assert!(e
        .record_governance_approval(&approval, 20, &host())
        .unwrap());
    assert!(!e
        .record_governance_approval(&approval, 20, &host())
        .unwrap());
    assert_eq!(
        e.accept_governance(&request("a", "accept"), 20, &host())
            .unwrap_err()
            .code,
        "E_GOV_QUORUM"
    );
    assert!(e
        .record_governance_approval(
            &signed(&a, &receipt.digest, &keys()[0], "second-nonce"),
            20,
            &host()
        )
        .is_err());
    e.record_governance_approval(&signed(&a, &receipt.digest, &keys()[1], "two"), 20, &host())
        .unwrap();
    quorum(&e, &b);
    let accepted = e
        .accept_governance(&request("a", "accept"), 20, &host())
        .unwrap();
    assert_eq!(
        e.accept_governance(&request("b", "other"), 20, &host())
            .unwrap_err()
            .code,
        "E_CAS"
    );
    assert_eq!(e.governance_event_count().unwrap(), 1);
    assert_eq!(e.event_count().unwrap(), 1); // Graph source event remains independent.
    drop(e);
    let e = Engine::open(&path).unwrap();
    let retry = e
        .accept_governance(&request("a", "accept"), 21, &host())
        .unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.decision_id, accepted.decision_id);
    assert_eq!(e.governance_event_count().unwrap(), 1);
    assert_eq!(
        e.accept_governance(&request("b", "accept"), 21, &host())
            .unwrap_err()
            .code,
        "E_GOV_REPLAY"
    );
}
#[test]
fn signatures_bind_proposal_policy_head_and_expiry_and_owner_profile_works() {
    let mut e = Engine::memory().unwrap();
    let source = seed(&mut e, 1);
    let q = proposal("owner", source, None);
    let r = e.propose_governance(&q, 20, &host()).unwrap();
    let good = signed(&q, &r.digest, &keys()[0], "owner-vote");
    let mut forged = good.clone();
    forged.approval.expected_head = Some("different".into());
    assert!(e.record_governance_approval(&forged, 20, &host()).is_err());
    let mut forged = good.clone();
    forged.signature.replace_range(..2, "00");
    assert_eq!(
        e.record_governance_approval(&forged, 20, &host())
            .unwrap_err()
            .code,
        "E_GOV_SIGNATURE"
    );
    assert!(e
        .record_governance_approval(&signed(&q, &r.digest, &keys()[2], "outsider"), 20, &host())
        .is_err());
    e.record_governance_approval(&good, 20, &host()).unwrap();
    e.accept_governance(&request("owner", "accepted"), 20, &host())
        .unwrap();
    assert!(e
        .accept_governance(&request("owner", "accepted"), 8001, &host())
        .is_err());
    assert!(e.install_governance_root(&policy(1)).is_err());
}
#[test]
fn new_membership_cannot_authorize_itself_and_old_epoch_retries_are_rejected() {
    let mut e = Engine::memory().unwrap();
    let source = seed(&mut e, 2);
    let q = proposal("first", source, None);
    quorum(&e, &q);
    let first = e
        .accept_governance(&request("first", "first"), 20, &host())
        .unwrap();
    let mut next = policy(1);
    next.reference.revision = "2".into();
    next.members = vec![weave_policy::public_key(&keys()[2])];
    let change = GovernanceProposal {
        id: "policy-change".into(),
        view_id: "team".into(),
        policy: policy(2).reference,
        expected_head: Some(first.decision_id),
        expires_at_ms: 9000,
        action: GovernanceAction::ReplacePolicy {
            policy: next.clone(),
        },
    };
    let r = e.propose_governance(&change, 20, &host()).unwrap();
    assert!(e
        .record_governance_approval(
            &signed(&change, &r.digest, &keys()[2], "self-install"),
            20,
            &host()
        )
        .is_err());
    quorum(&e, &change);
    let changed = e
        .accept_governance(&request("policy-change", "change"), 20, &host())
        .unwrap();
    assert_eq!(changed.policy, next.reference);
    assert_eq!(
        e.inspect_governance_head("team", 20, &host())
            .unwrap()
            .policy,
        next.reference
    );
    assert!(e
        .accept_governance(&request("first", "first"), 21, &host())
        .is_err());
    assert!(e.install_governance_root(&next).is_err());
    assert_eq!(e.governance_event_count().unwrap(), 2);
}
#[test]
fn current_source_privacy_and_branch_scope_are_checked_at_proposal_and_inspection() {
    let mut e = Engine::memory().unwrap();
    let source = seed(&mut e, 1);
    let q = proposal("visible", source.clone(), None);
    quorum(&e, &q);
    e.accept_governance(&request("visible", "accept"), 20, &host())
        .unwrap();
    assert!(e
        .inspect_governance_head("team", 20, &HostContext::new("outsider", []))
        .is_err());
    assert!(e
        .inspect_governance_head("team", 20, &HostContext::new("reader", []))
        .is_ok());
    let mut invalid = proposal("private-branch", source, None);
    if let GovernanceAction::Publish { branch_id, .. } = &mut invalid.action {
        *branch_id = "private".into();
    }
    assert_eq!(
        e.propose_governance(&invalid, 20, &host())
            .unwrap_err()
            .code,
        "E_GOV_SOURCE"
    );
}

#[test]
fn owner_and_expired_extra_vote_profiles() {
    let mut e = Engine::memory().unwrap();
    let mut owner = policy(1);
    owner.members.truncate(1);
    e.install_governance_root(&owner).unwrap();
    // Source bootstrap without changing the installed policy.
    e.execute(&serde_json::from_value(json!({
        "version": VERSION, "commands":[{"op":"commit","graph_id":"source","branch_id":"main","expected_head":null,"data":{}}]
    })).unwrap(), &host()).unwrap();
    let source = GraphRef {
        graph_id: "source".into(),
        revision: e.head("source", "main").unwrap().unwrap(),
    };
    let q = proposal("owner", source, None);
    let receipt = e.propose_governance(&q, 20, &host()).unwrap();
    e.record_governance_approval(
        &signed(&q, &receipt.digest, &keys()[0], "owner-vote"),
        20,
        &host(),
    )
    .unwrap();
    e.accept_governance(&request("owner", "owner-accept"), 20, &host())
        .unwrap();

    let mut e = Engine::memory().unwrap();
    let source = seed(&mut e, 1);
    let q = proposal("extra", source, None);
    let receipt = e.propose_governance(&q, 20, &host()).unwrap();
    let mut expired = signed(&q, &receipt.digest, &keys()[0], "expired").approval;
    expired.expires_at_ms = 30;
    e.record_governance_approval(
        &sign_governance_approval(expired, &keys()[0]).unwrap(),
        20,
        &host(),
    )
    .unwrap();
    e.record_governance_approval(
        &signed(&q, &receipt.digest, &keys()[1], "valid"),
        20,
        &host(),
    )
    .unwrap();
    assert_eq!(
        e.record_governance_approval(
            &signed(&q, &receipt.digest, &keys()[0], "renew"),
            40,
            &host()
        )
        .unwrap_err()
        .code,
        "E_GOV_APPROVAL"
    );
    e.accept_governance(&request("extra", "extra-accept"), 40, &host())
        .unwrap();
}

#[test]
fn two_connections_competing_quorums_have_one_atomic_winner() {
    use std::sync::{Arc, Barrier};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("race.db");
    let mut e = Engine::open(&path).unwrap();
    let source = seed(&mut e, 2);
    for id in ["a", "b"] {
        quorum(&e, &proposal(id, source.clone(), None));
    }
    drop(e);
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|id| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let e = Engine::open(&path).unwrap();
                barrier.wait();
                for _ in 0..20 {
                    match e.accept_governance(&request(id, id), 20, &host()) {
                        Ok(r) => return Ok(r),
                        Err(error)
                            if error.code == "E_STORAGE" && error.message.contains("locked") =>
                        {
                            std::thread::sleep(std::time::Duration::from_millis(5))
                        }
                        Err(error) => return Err(error),
                    }
                }
                panic!("bounded contention retries exhausted")
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert!(
        results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .all(|e| e.code == "E_CAS"),
        "{results:?}"
    );
    let c = rusqlite::Connection::open(path).unwrap();
    for table in [
        "governance_decisions",
        "governance_receipts",
        "governance_events",
    ] {
        assert_eq!(
            c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn copied_node_proofs_remain_a_current_source_gate_for_collectors_and_readers() {
    let mut engine = Engine::memory().unwrap();
    let mut p = policy(1);
    p.proposers.push("other".into());
    p.readers.clear();
    engine.install_governance_root(&p).unwrap();
    let writer = HostContext::new("collector", ["evidence".into(), "source".into()]);
    let evidence = json!({"nodes":[{"id":"n","entity_id":"E","space_id":"s"}],
        "edges":[{"id":"proof","from":"n","to":"n","predicate":"approval",
        "valid_time":{"start":0,"end":null},"readers":["collector"]}]});
    engine.execute(&serde_json::from_value(json!({"version":VERSION,"commands":[
        {"op":"commit","graph_id":"evidence","branch_id":"main","expected_head":null,"data":evidence}
    ]})).unwrap(), &writer).unwrap();
    let revision = engine.head("evidence", "main").unwrap().unwrap();
    let source = json!({"nodes":[{"id":"n","entity_id":"E","space_id":"s","properties":{"value":"private conclusion"},
        "derived_from":[{"graph_id":"evidence","revision":revision,"assertion_id":"proof"}]}]});
    engine.execute(&serde_json::from_value(json!({"version":VERSION,"commands":[
        {"op":"commit","graph_id":"source","branch_id":"main","expected_head":null,"data":source}
    ]})).unwrap(), &writer).unwrap();
    let q = proposal(
        "private",
        GraphRef {
            graph_id: "source".into(),
            revision: engine.head("source", "main").unwrap().unwrap(),
        },
        None,
    );
    let r = engine.propose_governance(&q, 20, &host()).unwrap();
    engine
        .record_governance_approval(
            &signed(&q, &r.digest, &keys()[0], "private-vote"),
            20,
            &host(),
        )
        .unwrap();
    engine
        .accept_governance(&request("private", "accept"), 20, &host())
        .unwrap();
    let other = HostContext::new("other", []);
    assert_eq!(
        engine
            .inspect_governance_head("team", 21, &other)
            .unwrap_err()
            .code,
        "E_GOV_UNAVAILABLE"
    );
    assert_eq!(
        engine
            .accept_governance(&request("private", "other"), 21, &other)
            .unwrap_err()
            .code,
        "E_GOV_UNAVAILABLE"
    );
    assert!(engine.inspect_governance_head("team", 21, &host()).is_ok());
    assert_eq!(engine.governance_event_count().unwrap(), 1);
}
