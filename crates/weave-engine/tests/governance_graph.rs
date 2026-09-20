#[path = "support/clock.rs"]
mod test_clock;
use ed25519_dalek::SigningKey;
use serde_json::json;
use weave_contract::*;
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
        readers: vec![],
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
    let r = test_clock::at(20, || e.propose_governance(q, &host())).unwrap();
    for (i, key) in keys()[..2].iter().enumerate() {
        test_clock::at(20, || {
            e.record_governance_approval(
                &signed(q, &r.digest, key, &format!("{}-{i}", q.id)),
                &host(),
            )
        })
        .unwrap();
    }
    r
}
fn choose(decision: Option<String>) -> AcceptedViewSelection {
    AcceptedViewSelection {
        view_id: "team".into(),
        decision_id: decision,
    }
}
fn accept(e: &Engine, source: GraphRef) -> GovernanceReceipt {
    quorum(e, &proposal("expose", source, None));
    e.accept_governance(&request("expose", "accept"), &host())
        .unwrap()
}
fn writer() -> HostContext {
    HostContext::new("collector", ["saved".into(), "source".into()])
}
fn save(e: &mut Engine, data: GraphData) -> GraphRef {
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "saved".into(),
                branch_id: "main".into(),
                expected_head: None,
                data,
            }],
        },
        &writer(),
    )
    .unwrap();
    GraphRef {
        graph_id: "saved".into(),
        revision: e.head("saved", "main").unwrap().unwrap(),
    }
}
fn q(reference: &GraphRef) -> QueryPlan {
    serde_json::from_value(json!({"graph_id":reference.graph_id,"revision":reference.revision}))
        .unwrap()
}
#[test]
fn genuine_decision_atomic_identity_empty_influence_and_policy_expiry() {
    let mut e = test_clock::memory().unwrap();
    let source = seed(&mut e, 2);
    let receipt = accept(&e, source.clone());
    let reader = HostContext::new("reader", []);
    let accepted = e.query_accepted_view(&choose(None), &reader).unwrap();
    assert_eq!(
        accepted.graph.nodes[0].properties,
        e.query(&q(&source), &reader).unwrap().graph.nodes[0].properties
    );
    let gate = accepted.graph.influence.as_ref().unwrap().assertions[0].clone();
    let decision = GraphRef {
        graph_id: gate.graph_id.clone(),
        revision: gate.revision.clone(),
    };
    let record = e.query(&q(&decision), &reader).unwrap();
    assert_eq!(record.graph.edges.len(), 1);
    assert_eq!(record.graph.edges[0].id, "accepted");
    assert_eq!(record.graph.nodes[0].properties.len(), 4);
    assert_eq!(
        record.graph.nodes[0].properties["occurrence"],
        receipt.decision_id
    );
    assert!(e.resolve_assertion(&gate, &reader).unwrap().is_some());
    assert_eq!(e.event_count().unwrap(), 2);
    assert!(
        e.accept_governance(&request("expose", "accept"), &host())
            .unwrap()
            .duplicate
    );
    assert_eq!(e.event_count().unwrap(), 2);
    let mut empty = accepted.graph.clone();
    empty.nodes.clear();
    empty.edges.clear();
    empty.attachments.clear();
    empty.schema = None;
    let saved = save(&mut e, empty);
    assert_eq!(
        e.query(&q(&saved), &reader).unwrap().coverage,
        Coverage::Complete
    );
    test_clock::at(10000, || {});
    assert!(e.query_accepted_view(&choose(None), &reader).is_err());
    assert!(e.query(&q(&decision), &reader).is_err());
    assert!(e.resolve_assertion(&gate, &reader).unwrap().is_none());
    assert!(e
        .resolve_structural(
            &StructuralRef {
                graph_id: decision.graph_id.clone(),
                revision: decision.revision.clone(),
                edge_id: "acceptance".into()
            },
            &reader
        )
        .unwrap()
        .is_none());
    let denied = e.query(&q(&saved), &reader).unwrap();
    assert!(denied.graph.influence.is_none());
    assert_eq!(denied.coverage, Coverage::Partial);
    assert!(e.export_capsule(&saved, &reader).is_err());
    // The source facts themselves do not acquire a retroactive governance restriction.
    assert_eq!(e.query(&q(&source), &reader).unwrap().graph.nodes.len(), 1);
}
#[test]
fn scalar_and_explanation_proofs_survive_carrier_and_readers_removal() {
    for explain in [false, true] {
        test_clock::at(0, || {});
        let mut e = test_clock::memory().unwrap();
        let mut source = seed(&mut e, 2);
        if explain {
            let program: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"source","expected_head":source.revision,"data":{"nodes":[{"id":"n","entity_id":"E","space_id":"s"}],"edges":[{"id":"claim","from":"n","to":"n","predicate":"known","valid_time":{"start":0}}]}}]})).unwrap();
            e.execute(&program, &host()).unwrap();
            source.revision = e.head("source", "main").unwrap().unwrap();
        }
        accept(&e, source);
        let input = e.query_accepted_view(&choose(None), &host()).unwrap();
        let context = AlgebraContext {
            principal: "collector".into(),
            max_objects: 10000,
            max_output_bytes: 32 * 1024 * 1024,
        };
        let point = EntitySpace {
            entity_id: "E".into(),
            space_id: "s".into(),
        };
        let mut value = algebra::support(
            input,
            if explain { "known" } else { "unknown" },
            &point,
            &point,
            25,
            &context,
        )
        .unwrap();
        if explain {
            value = identity::explain(&value, &context).unwrap();
        }
        assert!(!value.graph.nodes.is_empty());
        value.graph.influence = None;
        value.graph.edges.clear();
        value.graph.attachments.clear();
        for node in &mut value.graph.nodes {
            node.readers.clear();
            assert!(!node.derived_from.is_empty());
        }
        let saved = save(&mut e, value.graph);
        assert!(!e.query(&q(&saved), &host()).unwrap().graph.nodes.is_empty());
        test_clock::at(10000, || {});
        assert!(e.query(&q(&saved), &host()).unwrap().graph.nodes.is_empty());
    }
}
#[test]
fn reserved_poisoning_peer_import_and_registry_mismatch_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gov.db");
    let mut e = test_clock::open(&path).unwrap();
    let source = seed(&mut e, 2);
    accept(&e, source);
    let accepted = e.query_accepted_view(&choose(None), &host()).unwrap();
    let gate = &accepted.graph.influence.unwrap().assertions[0];
    let reference = GraphRef {
        graph_id: gate.graph_id.clone(),
        revision: gate.revision.clone(),
    };
    let capsule = e.export_capsule(&reference, &host()).unwrap();
    let mut peer = test_clock::memory().unwrap();
    assert_eq!(
        peer.receive_capsule(&capsule, &host()).unwrap_err().code,
        "E_RESERVED_NAMESPACE"
    );
    assert_eq!(peer.event_count().unwrap(), 0);
    let poison = HostContext::new("collector", [reference.graph_id.clone()]);
    let program:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":reference.graph_id,"data":{}}]})).unwrap();
    assert_eq!(
        peer.execute(&program, &poison).unwrap_err().code,
        "E_RESERVED_NAMESPACE"
    );
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute("UPDATE governance_graphs SET revision='different'", [])
        .unwrap();
    assert!(e.query(&q(&reference), &host()).is_err());
    assert!(e.query_accepted_view(&choose(None), &host()).is_err());
}
#[test]
fn transition_uses_current_policy_and_original_publication_and_keeps_history() {
    let mut e = test_clock::memory().unwrap();
    let source = seed(&mut e, 2);
    let first = accept(&e, source);
    let mut next = policy(2);
    next.reference.revision = "2".into();
    next.readers = vec!["other".into()];
    let change = GovernanceProposal {
        id: "transition".into(),
        view_id: "team".into(),
        policy: policy(2).reference,
        expected_head: Some(first.decision_id.clone()),
        expires_at_ms: 9000,
        action: GovernanceAction::ReplacePolicy { policy: next },
    };
    quorum(&e, &change);
    e.accept_governance(&request("transition", "transition"), &host())
        .unwrap();
    let newreader = HostContext::new("other", []);
    let current = e.query_accepted_view(&choose(None), &newreader).unwrap();
    assert_eq!(
        current.graph.influence.as_ref().unwrap().assertions.len(),
        2
    );
    assert_eq!(
        e.query_accepted_view(&choose(Some(first.decision_id)), &newreader)
            .unwrap()
            .graph
            .influence
            .unwrap()
            .assertions
            .len(),
        1
    );
    assert!(e
        .query_accepted_view(&choose(None), &HostContext::new("reader", []))
        .is_err());
}
#[test]
fn live_dependency_rejects_acceptance_without_partial_decision_or_event() {
    let mut e = test_clock::memory().unwrap();
    e.install_governance_root(&policy(2)).unwrap();
    let p:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"source","data":{"attachments":[{"id":"live","key":"live","host":{"kind":"graph"},"value":{"kind":"live_graph","graph_id":"source","branch_id":"main"},"valid_time":{"start":0}}]}}]})).unwrap();
    e.execute(&p, &host()).unwrap();
    let source = GraphRef {
        graph_id: "source".into(),
        revision: e.head("source", "main").unwrap().unwrap(),
    };
    quorum(&e, &proposal("live", source, None));
    assert_eq!(
        e.accept_governance(&request("live", "live"), &host())
            .unwrap_err()
            .code,
        "E_GOV_LIVE"
    );
    assert_eq!(e.governance_event_count().unwrap(), 0);
    assert_eq!(e.event_count().unwrap(), 1);
    assert!(e
        .inspect_governance_head("team", &host())
        .unwrap()
        .decision_id
        .is_none());
}
fn adapter(id: &str, graph: &str) -> AdapterManifest {
    AdapterManifest {
        id: id.into(),
        version: "1".into(),
        artifact_digest: format!("sha256:{}", "0".repeat(64)),
        config_revision: "1".into(),
        principal: "reader".into(),
        subscriptions: vec![SubscriptionScope {
            graph_id: graph.into(),
            branch_id: "main".into(),
        }],
        output_graphs: vec![],
        effect_destinations: vec![],
        max_attempts: 2,
        lease_ms: 20000,
        max_pending_events: 100,
        projection_replay: false,
    }
}
#[test]
fn expired_authority_invalidates_stale_views_delivery_and_unrelated_handler_receipt() {
    let mut e = test_clock::memory().unwrap();
    let source = seed(&mut e, 2);
    accept(&e, source);
    let reader = HostContext::new("reader", []);
    let accepted = e.query_accepted_view(&choose(None), &reader).unwrap();
    let saved = save(&mut e, accepted.graph);
    e.register_view(
        &ViewDefinition {
            id: "accepted-cache".into(),
            expression: GraphExpression::Query { query: q(&saved) },
            clock: ViewClock::Fixed,
        },
        None,
        &reader,
    )
    .unwrap();
    for (id, graph) in [
        ("handler", "source"),
        ("pending", "saved"),
        ("governance", "source"),
    ] {
        e.install_adapter(&adapter(id, graph), &reader).unwrap();
        e.set_adapter_state(id, "running").unwrap();
    }
    let handler = e.poll_adapter("handler").unwrap().unwrap();
    let program = Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands: vec![Command::Query { query: q(&saved) }],
    };
    e.complete_handler("handler", &handler.id, &handler.lease, &program)
        .unwrap();
    let pending = e.poll_adapter("pending").unwrap().unwrap();
    e.subscribe_governance("governance", "team", &reader)
        .unwrap();
    let governance = e
        .poll_governance("governance", "team", &reader)
        .unwrap()
        .unwrap();
    e.acknowledge_governance(
        "governance",
        "team",
        &governance.event.id,
        &governance.lease,
        &reader,
    )
    .unwrap();
    test_clock::at(10000, || {});
    assert!(e
        .read_view("accepted-cache", None, ViewFreshness::AllowStale, &reader)
        .is_err());
    assert!(e.view_changes("accepted-cache", 0, &reader).is_err());
    assert!(e
        .complete_handler("handler", &handler.id, &handler.lease, &program)
        .is_err());
    assert!(e
        .complete_handler("pending", &pending.id, &pending.lease, &program)
        .is_err());
    assert!(e
        .acknowledge_governance(
            "governance",
            "team",
            &governance.event.id,
            &governance.lease,
            &reader
        )
        .is_err());
}
#[test]
fn signed_cached_query_receipt_rechecks_governance_expiry_before_replay() {
    use weave_policy::*;
    let mut e = test_clock::memory().unwrap();
    let source = seed(&mut e, 2);
    accept(&e, source);
    let accepted = e.query_accepted_view(&choose(None), &host()).unwrap();
    let decision = accepted.graph.influence.as_ref().unwrap().assertions[0]
        .graph_id
        .clone();
    let saved = save(&mut e, accepted.graph);
    let query = q(&saved);
    let root = SigningKey::from_bytes(&[81; 32]);
    let user = SigningKey::from_bytes(&[82; 32]);
    let mut scopes: Vec<Scope> = ["saved", "source", decision.as_str()]
        .into_iter()
        .map(|graph| Scope {
            graph_id: graph.into(),
            branch_id: "main".into(),
            actions: [Action::Read, Action::Traverse].into(),
        })
        .collect();
    scopes.sort();
    let context = AdmissionContext {
        audience: "receiver".into(),
        now_ms: 0,
        policy_epoch: "1".into(),
        roots: vec![RootAuthority {
            issuer: public_key(&root),
            audience: "receiver".into(),
            policy_revision: "1".into(),
            scopes: scopes.clone(),
            not_before_ms: 0,
            expires_at_ms: 20000,
            max_delegations: 1,
        }],
        revoked_capabilities: Default::default(),
        revoked_keys: Default::default(),
        consumed_nonces: Default::default(),
    };
    e.install_admission_policy(&context).unwrap();
    let cap = sign_capability(
        Capability {
            version: weave_policy::VERSION.into(),
            issuer: public_key(&root),
            subject: public_key(&user),
            audience: "receiver".into(),
            policy_revision: "1".into(),
            scopes,
            not_before_ms: 0,
            expires_at_ms: 20000,
            delegations_remaining: 0,
            parent: None,
        },
        &root,
    )
    .unwrap();
    let request = sign_request(
        Request {
            version: REQUEST_VERSION.into(),
            subject: public_key(&user),
            audience: "receiver".into(),
            capability_id: capability_id(&cap).unwrap(),
            nonce: "af".repeat(32),
            issued_at_ms: 10,
            expires_at_ms: 15000,
            operation: Operation {
                action: Action::Read,
                graph_id: "saved".into(),
                branch_id: "main".into(),
            },
            body_digest: body_digest(&serde_json::to_vec(&query).unwrap()),
        },
        &user,
    )
    .unwrap();
    let proof = AdmissionProof {
        chain: vec![cap],
        request,
    };
    assert!(!e.admit_query(&proof, &query).unwrap().duplicate);
    assert!(e.admit_query(&proof, &query).unwrap().duplicate);
    test_clock::at(10000, || {});
    assert!(e.admit_query(&proof, &query).is_err());
}
#[test]
fn old_sql_only_decisions_are_not_backfilled_and_next_publication_is_real() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let mut e = test_clock::open(&path).unwrap();
    let source = seed(&mut e, 2);
    let prior = accept(&e, source.clone());
    drop(e);
    let db = rusqlite::Connection::open(&path).unwrap();
    // Reconstruct the preceding marker11 storage profile: SQL decisions had no graph.
    for table in [
        "events",
        "heads",
        "revisions",
        "edge_structures",
        "assertion_structures",
    ] {
        db.execute(
            &format!("DELETE FROM {table} WHERE graph_id LIKE 'weave:governance:%'"),
            [],
        )
        .unwrap();
    }
    db.execute("DROP TABLE governance_graphs", []).unwrap();
    db.execute("DROP TABLE governance_exposure_decisions", [])
        .unwrap();
    db.execute(
        "DELETE FROM schema_registry WHERE id LIKE 'weave:governance:%'",
        [],
    )
    .unwrap();
    db.pragma_update(None, "user_version", 11).unwrap();
    let e = test_clock::open(&path).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        15
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM governance_graphs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        e.inspect_governance_head("team", &host())
            .unwrap()
            .decision_id,
        Some(prior.decision_id.clone())
    );
    assert!(e.query_accepted_view(&choose(None), &host()).is_err());
    quorum(&e, &proposal("fresh", source, Some(prior.decision_id)));
    e.accept_governance(&request("fresh", "fresh"), &host())
        .unwrap();
    assert_eq!(
        e.query_accepted_view(&choose(None), &host())
            .unwrap()
            .graph
            .influence
            .unwrap()
            .assertions
            .len(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM governance_graphs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn source_branch_reachability_and_source_proof_cycles_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("branch.db");
    let mut e = test_clock::open(&path).unwrap();
    let source = seed(&mut e, 2);
    accept(&e, source.clone());
    let db = rusqlite::Connection::open(path).unwrap();
    // Simulates explicit movement of an accepted branch to an unrelated retained revision.
    db.execute("DELETE FROM heads WHERE graph_id='source'", [])
        .unwrap();
    assert!(e.query_accepted_view(&choose(None), &host()).is_err());
    assert!(e.query(&q(&source), &host()).is_ok()); // independent pinned native fact read
    let mut cycle = test_clock::memory().unwrap();
    cycle.install_governance_root(&policy(2)).unwrap();
    let program:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit_batch","batch_id":"cycle","commits":[{"graph_id":"source","data":{"nodes":[{"id":"n","entity_id":"E","space_id":"s","derived_nodes":[{"graph_id":"source","revision":"logical:cycle:source","node_id":"n"}]}]}}]}]})).unwrap();
    cycle.execute(&program, &host()).unwrap();
    let q = proposal(
        "cycle",
        GraphRef {
            graph_id: "source".into(),
            revision: "logical:cycle:source".into(),
        },
        None,
    );
    assert!(cycle.propose_governance(&q, &host()).is_err());
    assert_eq!(cycle.governance_event_count().unwrap(), 0);
}
#[test]
fn one_host_clock_sample_covers_record_creation_and_nested_accepted_reads() {
    let clock = std::sync::Arc::new(ManualClock::new(20));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    let source = seed(&mut e, 2);
    quorum(&e, &proposal("clock", source, None));
    let count = clock.samples();
    e.accept_governance(&request("clock", "clock"), &host())
        .unwrap();
    assert_eq!(clock.samples(), count + 1);
    let count = clock.samples();
    let value = e.query_accepted_view(&choose(None), &host()).unwrap();
    assert_eq!(clock.samples(), count + 1);
    let reference = &value.graph.influence.unwrap().assertions[0];
    assert_eq!(e.recorded_at(&reference.revision).unwrap(), 20);
}
#[test]
fn current_source_policy_revocation_blocks_decision_and_saved_acceptance_without_head_change() {
    let mut e = test_clock::memory().unwrap();
    let original = seed(&mut e, 2);
    let identity = IdentityPolicy {
        reference: IdentityPolicyRef {
            id: "identity".into(),
            revision: "1".into(),
        },
        proposers: vec!["collector".into()],
        approvers: vec!["collector".into()],
        readers: vec![],
        allowed_spaces: vec!["s".into()],
        max_members: 2,
    };
    e.install_identity_policy(&identity).unwrap();
    let candidate = IdentityCandidate {
        id: "candidate".into(),
        mapping_id: "map".into(),
        policy: identity.reference.clone(),
        groups: vec![vec![NodeRef {
            graph_id: "source".into(),
            revision: original.revision.clone(),
            node_id: "n".into(),
        }]],
        evidence: vec![],
        valid_time: Interval {
            start: 0,
            end: None,
        },
        context: None,
    };
    e.submit_identity_candidate(&candidate, &host()).unwrap();
    let mapping = e
        .accept_identity_candidate(
            &IdentityDecisionRequest {
                candidate_id: "candidate".into(),
                expected_head: None,
                nonce: "map".into(),
            },
            &host(),
        )
        .unwrap();
    let result = e
        .query(
            &serde_json::from_value(
                json!({"graph_id":mapping.reference.graph_id,"revision":mapping.reference.revision}),
            )
            .unwrap(),
            &host(),
        )
        .unwrap();
    let member = result.node_origins.values().next().unwrap()[0].clone();
    let data: GraphData = serde_json::from_value(
        json!({"nodes":[{"id":"n","entity_id":"E","space_id":"s","derived_nodes":[member]}]}),
    )
    .unwrap();
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "source".into(),
                branch_id: "main".into(),
                expected_head: Some(original.revision),
                data,
            }],
        },
        &host(),
    )
    .unwrap();
    let source = GraphRef {
        graph_id: "source".into(),
        revision: e.head("source", "main").unwrap().unwrap(),
    };
    accept(&e, source);
    let accepted = e.query_accepted_view(&choose(None), &host()).unwrap();
    let premise = accepted.graph.influence.as_ref().unwrap().assertions[0].clone();
    let saved = save(&mut e, accepted.graph);
    e.revoke_identity_policy(&identity.reference).unwrap();
    assert!(e.query_accepted_view(&choose(None), &host()).is_err());
    assert!(e.resolve_assertion(&premise, &host()).unwrap().is_none());
    assert!(e.query(&q(&saved), &host()).unwrap().graph.nodes.is_empty());
    assert!(e.export_capsule(&saved, &host()).is_err());
}
#[test]
fn historical_approval_expiry_does_not_replace_current_policy_read_authority() {
    let mut e = test_clock::memory().unwrap();
    let source = seed(&mut e, 2);
    accept(&e, source);
    test_clock::at(8500, || {});
    assert!(e.query_accepted_view(&choose(None), &host()).is_ok());
    assert_eq!(
        e.accept_governance(&request("expose", "accept"), &host())
            .unwrap_err()
            .code,
        "E_GOV_QUORUM"
    );
}
#[test]
fn accepted_geometry_scalar_keeps_governance_gate_after_reader_and_carrier_stripping() {
    let mut e = test_clock::memory().unwrap();
    e.install_governance_root(&policy(2)).unwrap();
    let point = |values: Vec<f64>| json!({"kind":"coordinates","space":{"id":"world","revision":"1","geometry":{"kind":"physical3d","frame":"world","unit":"metre"}},"role":"position","values":values});
    let data:GraphData=serde_json::from_value(json!({"profile":"explicit","nodes":[{"id":"a","entity_id":"A","space_id":"world"},{"id":"b","entity_id":"B","space_id":"world"}],"structural_edges":[{"id":"sa","predicate":"coordinates","from":"a","to":"a"},{"id":"sb","predicate":"coordinates","from":"b","to":"b"}],"assertions":[{"id":"pa","edge_id":"sa","source":"sensor","properties":{"weave.geometry":point(vec![0.,0.,0.])},"valid_time":{"start":0}},{"id":"pb","edge_id":"sb","source":"sensor","properties":{"weave.geometry":point(vec![3.,4.,0.])},"valid_time":{"start":0}}]})).unwrap();
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
    let source = GraphRef {
        graph_id: "source".into(),
        revision: e.head("source", "main").unwrap().unwrap(),
    };
    accept(&e, source);
    let accepted = e.query_accepted_view(&choose(None), &host()).unwrap();
    let saved = save(&mut e, accepted.graph);
    let operand = |id: &str| GeometryOperand {
        input: Box::new(GraphExpression::Query { query: q(&saved) }),
        assertion_id: id.into(),
    };
    let results = e
        .execute(
            &Program {
                version: VERSION.into(),
                source_revisions: vec![],
                commands: vec![Command::Evaluate {
                    value: GraphExpression::Geometry {
                        operation: GeometryOperation::Distance {
                            left: operand("pa"),
                            right: operand("pb"),
                        },
                        valid_at: 25,
                    },
                }],
            },
            &host(),
        )
        .unwrap();
    let CommandResult::Queried { result } = results.into_iter().next().unwrap() else {
        panic!()
    };
    let mut scalar = result.graph;
    assert_eq!(scalar.nodes[0].properties["value"], 5.0);
    scalar.influence = None;
    scalar.edges.clear();
    scalar.attachments.clear();
    for node in &mut scalar.nodes {
        node.readers.clear();
    }
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "saved".into(),
                branch_id: "main".into(),
                expected_head: Some(saved.revision),
                data: scalar,
            }],
        },
        &writer(),
    )
    .unwrap();
    let pinned = GraphRef {
        graph_id: "saved".into(),
        revision: e.head("saved", "main").unwrap().unwrap(),
    };
    assert_eq!(e.query(&q(&pinned), &host()).unwrap().graph.nodes.len(), 1);
    test_clock::at(10000, || {});
    assert!(e
        .query(&q(&pinned), &host())
        .unwrap()
        .graph
        .nodes
        .is_empty());
}
#[test]
fn deleted_binding_does_not_downgrade_exposed_occurrence_to_legacy_native_profile() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.db");
    let mut e = test_clock::open(&path).unwrap();
    let source = seed(&mut e, 2);
    accept(&e, source);
    rusqlite::Connection::open(path)
        .unwrap()
        .execute("DELETE FROM governance_graphs", [])
        .unwrap();
    assert!(e.inspect_governance_head("team", &host()).is_err());
    assert!(e.query_accepted_view(&choose(None), &host()).is_err());
    assert!(e
        .accept_governance(&request("expose", "accept"), &host())
        .is_err());
}
#[test]
fn metadata_selection_of_real_decision_keeps_current_authority_on_node_only_copy() {
    let mut e = test_clock::memory().unwrap();
    let source = seed(&mut e, 2);
    accept(&e, source);
    let accepted = e.query_accepted_view(&choose(None), &host()).unwrap();
    let gate = &accepted.graph.influence.unwrap().assertions[0];
    let data:GraphData=serde_json::from_value(json!({"attachments":[{"id":"decision-link","key":"approved","host":{"kind":"graph"},"value":{"kind":"graph","reference":{"graph_id":gate.graph_id,"revision":gate.revision}},"valid_time":{"start":0}}]})).unwrap();
    let saved = save(&mut e, data);
    let mut query = q(&saved);
    query.include_metadata = true;
    let plan = Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands: vec![Command::Evaluate {
            value: GraphExpression::Metadata {
                input: Box::new(GraphExpression::Query {
                    query: query.clone(),
                }),
                host: MetadataHost::Graph,
                key: "approved".into(),
            },
        }],
    };
    let CommandResult::Queried { result } = e.execute(&plan, &host()).unwrap().remove(0) else {
        panic!()
    };
    let mut copy = result.graph;
    assert_eq!(copy.nodes.len(), 1);
    copy.edges.clear();
    copy.attachments.clear();
    copy.influence = None;
    for node in &mut copy.nodes {
        node.readers.clear();
    }
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "saved".into(),
                branch_id: "main".into(),
                expected_head: Some(saved.revision.clone()),
                data: copy,
            }],
        },
        &writer(),
    )
    .unwrap();
    let latest = GraphRef {
        graph_id: "saved".into(),
        revision: e.head("saved", "main").unwrap().unwrap(),
    };
    test_clock::at(10000, || {});
    assert!(e
        .query(&q(&latest), &host())
        .unwrap()
        .graph
        .nodes
        .is_empty());
    let navigation = e.query(&query, &host()).unwrap();
    assert_eq!(navigation.coverage, Coverage::Partial);
    assert!(navigation.metadata_graphs.is_empty());
}
#[test]
fn copied_decision_claim_cannot_escape_policy_via_public_replacement_endpoints() {
    let mut e = test_clock::memory().unwrap();
    let source = seed(&mut e, 2);
    accept(&e, source);
    let accepted = e.query_accepted_view(&choose(None), &host()).unwrap();
    let premise = &accepted.graph.influence.unwrap().assertions[0];
    let record = e
        .query(
            &q(&GraphRef {
                graph_id: premise.graph_id.clone(),
                revision: premise.revision.clone(),
            }),
            &host(),
        )
        .unwrap();
    let mut copy: GraphData =
        serde_json::from_value(json!({"nodes":[{"id":"public","entity_id":"P","space_id":"s"}]}))
            .unwrap();
    let mut edge = record.graph.edges[0].clone();
    edge.from = "public".into();
    edge.to = "public".into();
    edge.type_id = None;
    edge.readers.clear();
    copy.edges.push(edge);
    let saved = save(&mut e, copy);
    assert_eq!(e.query(&q(&saved), &host()).unwrap().graph.edges.len(), 1);
    test_clock::at(10000, || {});
    let result = e.query(&q(&saved), &host()).unwrap();
    assert_eq!(result.graph.nodes.len(), 1);
    assert!(result.graph.edges.is_empty());
}

#[test]
fn root_missing_empty_private_and_expired_views_have_identical_denials() {
    let mut e = test_clock::memory().unwrap();
    let outsider = HostContext::new("outsider", []);
    let deny = |e: &Engine, selection: AcceptedViewSelection, actor: &HostContext| {
        let error = e.query_accepted_view(&selection, actor).unwrap_err();
        (error.code, error.message)
    };
    let missing = deny(&e, choose(None), &outsider);
    let mut private_policy = policy(2);
    private_policy.readers = vec!["collector".into()];
    e.install_governance_root(&private_policy).unwrap();
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
    let source = GraphRef {
        graph_id: "source".into(),
        revision: e.head("source", "main").unwrap().unwrap(),
    };
    assert_eq!(deny(&e, choose(None), &outsider), missing);
    let receipt = accept(&e, source);
    assert_eq!(deny(&e, choose(None), &outsider), missing);
    assert_eq!(
        deny(&e, choose(Some(receipt.decision_id)), &outsider),
        missing
    );
    assert_eq!(
        deny(&e, choose(Some("unknown-decision".into())), &outsider),
        missing
    );
    test_clock::at(10000, || {});
    assert_eq!(deny(&e, choose(None), &host()), missing);
}

#[test]
fn accepted_graph_expression_matches_exact_native_occurrence_and_expiry_rolls_back() {
    let clock = std::sync::Arc::new(ManualClock::new(20));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    let source = seed(&mut e, 2);
    let receipt = accept(&e, source);
    let expected = e
        .query_accepted_view(&choose(Some(receipt.decision_id.clone())), &host())
        .unwrap();
    let expression = GraphExpression::AcceptedGraph {
        selection: AcceptedGraphSelection {
            view_id: "team".into(),
            decision_id: receipt.decision_id,
        },
    };
    let before = clock.samples();
    let result = e
        .execute(
            &Program {
                version: VERSION.into(),
                source_revisions: vec![],
                commands: vec![Command::Evaluate {
                    value: expression.clone(),
                }],
            },
            &host(),
        )
        .unwrap();
    assert_eq!(clock.samples(), before + 1);
    let CommandResult::Queried { result: actual } = &result[0] else {
        panic!("query expected")
    };
    assert_eq!(actual.as_ref(), &expected);
    clock.set(10000);
    let error = e
        .execute(
            &Program {
                version: VERSION.into(),
                source_revisions: vec![],
                commands: vec![
                    Command::Commit {
                        graph_id: "saved".into(),
                        branch_id: "main".into(),
                        expected_head: None,
                        data: GraphData::default(),
                    },
                    Command::Evaluate { value: expression },
                ],
            },
            &writer(),
        )
        .unwrap_err();
    assert_eq!(error.code, "E_GOV_UNAVAILABLE");
    assert!(e.head("saved", "main").unwrap().is_none());
}

#[test]
fn snapshot_gate_retains_current_governance_policy_through_cached_values() {
    let mut e = test_clock::memory().unwrap();
    let source = seed(&mut e, 2);
    accept(&e, source);
    let accepted = e.query_accepted_view(&choose(None), &host()).unwrap();
    let gate = &accepted.graph.influence.as_ref().unwrap().assertions[0];
    let snapshot = GraphRef {
        graph_id: gate.graph_id.clone(),
        revision: gate.revision.clone(),
    };
    let data: GraphData = serde_json::from_value(json!({
        "nodes":[{"id":"scalar","entity_id":"scalar","space_id":"result","derived_snapshots":[snapshot]}],
        "attachments":[{"id":"literal","host":{"kind":"graph"},"key":"count","value":{"kind":"literal","value":1},"valid_time":{"start":0},"derived_snapshots":[snapshot]}]
    })).unwrap();
    let saved = save(&mut e, data);
    let definition = ViewDefinition {
        id: "snapshot-cache".into(),
        expression: GraphExpression::Query { query: q(&saved) },
        clock: ViewClock::Fixed,
    };
    e.register_view(&definition, None, &host()).unwrap();
    assert_eq!(e.query(&q(&saved), &host()).unwrap().graph.nodes.len(), 1);
    test_clock::at(10000, || {
        let value = e.query(&q(&saved), &host()).unwrap();
        assert!(value.graph.nodes.is_empty());
        assert!(value.graph.attachments.is_empty());
        assert!(e
            .read_view("snapshot-cache", None, ViewFreshness::AllowStale, &host())
            .is_err());
    });
}
