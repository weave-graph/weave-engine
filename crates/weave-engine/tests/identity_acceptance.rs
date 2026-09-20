#[path = "support/clock.rs"]
mod test_clock;
use serde_json::json;
use weave_contract::*;
use weave_engine::*;
fn host(actor: &str) -> HostContext {
    HostContext::new(
        actor,
        [
            "physical".into(),
            "operations".into(),
            "private".into(),
            "copy".into(),
        ],
    )
}
fn write(e: &mut Engine, id: &str, entity: &str, space: &str, readers: &[&str]) -> NodeRef {
    let graph:GraphData=serde_json::from_value(json!({"nodes":[{"id":"node","entity_id":entity,"space_id":space,"readers":readers,"properties":{"local":space}}]})).unwrap();
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: id.into(),
                branch_id: "main".into(),
                expected_head: None,
                data: graph,
            }],
        },
        &host("alice"),
    )
    .unwrap();
    NodeRef {
        graph_id: id.into(),
        revision: e.head(id, "main").unwrap().unwrap(),
        node_id: "node".into(),
    }
}
fn policy() -> IdentityPolicy {
    IdentityPolicy {
        reference: IdentityPolicyRef {
            id: "review".into(),
            revision: "1".into(),
        },
        proposers: vec!["alice".into()],
        approvers: vec!["reviewer".into()],
        readers: vec![],
        allowed_spaces: vec!["physical".into(), "operations".into(), "private".into()],
        max_members: 16,
    }
}
fn setup() -> (Engine, IdentityCandidate) {
    let mut e = test_clock::memory().unwrap();
    e.install_identity_policy(&policy()).unwrap();
    let a = write(&mut e, "physical", "independent-A", "physical", &[]);
    let b = write(&mut e, "operations", "independent-B", "operations", &[]);
    let c = write(
        &mut e,
        "private",
        "pairwise-C",
        "private",
        &["alice", "reviewer"],
    );
    let candidate = IdentityCandidate {
        id: "candidate".into(),
        mapping_id: "equipment".into(),
        policy: policy().reference,
        groups: vec![vec![a, b, c]],
        evidence: vec![],
        valid_time: Interval {
            start: 0,
            end: Some(10),
        },
        context: None,
    };
    (e, candidate)
}
fn request(
    candidate: &IdentityCandidate,
    head: Option<String>,
    nonce: &str,
) -> IdentityDecisionRequest {
    IdentityDecisionRequest {
        candidate_id: candidate.id.clone(),
        expected_head: head,
        nonce: nonce.into(),
    }
}
fn selection(candidate: &IdentityCandidate, revision: &str, target: &str) -> IdentityResolve {
    IdentityResolve {
        mapping_id: candidate.mapping_id.clone(),
        revision: revision.into(),
        policy: candidate.policy.clone(),
        source: candidate.groups[0][0].clone(),
        target_space: target.into(),
        valid_at: 5,
        context: ContextSelection::Default,
    }
}
#[test]
fn candidate_receipt_does_not_accept_and_decisions_require_installed_authority_and_cas() {
    let (mut e, c) = setup();
    assert!(e.submit_identity_candidate(&c, &host("alice")).unwrap());
    assert!(!e.submit_identity_candidate(&c, &host("alice")).unwrap());
    assert!(e.identity_head(&c.mapping_id).unwrap().is_none());
    assert_eq!(e.event_count().unwrap(), 3);
    assert_eq!(
        e.accept_identity_candidate(&request(&c, None, "n"), &host("alice"))
            .unwrap_err()
            .code,
        "E_FORBIDDEN"
    );
    let accepted = e
        .accept_identity_candidate(&request(&c, None, "n"), &host("reviewer"))
        .unwrap();
    assert!(accepted.changed);
    assert!(!accepted.duplicate);
    assert!(accepted.event_id.is_some());
    assert_eq!(e.event_count().unwrap(), 4);
    let replay = e
        .accept_identity_candidate(&request(&c, None, "n"), &host("reviewer"))
        .unwrap();
    assert!(replay.duplicate);
    assert_eq!(replay.reference, accepted.reference);
    assert_eq!(
        e.accept_identity_candidate(
            &request(&c, Some(accepted.reference.revision.clone()), "n"),
            &host("reviewer")
        )
        .unwrap_err()
        .code,
        "E_REPLAY"
    );
    assert_eq!(
        e.accept_identity_candidate(&request(&c, None, "stale"), &host("reviewer"))
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    let unchanged = e
        .accept_identity_candidate(
            &request(&c, Some(accepted.reference.revision), "same"),
            &host("reviewer"),
        )
        .unwrap();
    assert!(!unchanged.changed);
    assert!(unchanged.event_id.is_none());
    assert_eq!(e.event_count().unwrap(), 4);
}
#[test]
fn independent_identities_resolve_only_authorized_members_and_split_preserves_history() {
    let (mut e, c) = setup();
    e.submit_identity_candidate(&c, &host("alice")).unwrap();
    let accepted = e
        .accept_identity_candidate(&request(&c, None, "first"), &host("reviewer"))
        .unwrap();
    let linked = e
        .resolve_identity(
            &selection(&c, &accepted.reference.revision, "operations"),
            &host("bob"),
        )
        .unwrap();
    assert_eq!(linked.graph.edges.len(), 1);
    assert!(linked
        .graph
        .nodes
        .iter()
        .any(|n| n.entity_id == "independent-A"));
    assert!(linked
        .graph
        .nodes
        .iter()
        .any(|n| n.entity_id == "independent-B"));
    assert_eq!(linked.provenance.len(), 2);
    assert!(linked
        .graph
        .nodes
        .iter()
        .all(|n| !n.properties.contains_key("local")));
    let hidden = e
        .resolve_identity(
            &selection(&c, &accepted.reference.revision, "private"),
            &host("bob"),
        )
        .unwrap();
    assert!(hidden.graph.nodes.is_empty());
    assert!(hidden.graph.edges.is_empty());
    assert!(hidden.provenance.is_empty());
    assert!(!serde_json::to_string(&hidden)
        .unwrap()
        .contains("pairwise-C"));
    assert!(!hidden
        .input_snapshots
        .iter()
        .any(|r| r.graph_id == "private"));
    let visible = e
        .resolve_identity(
            &selection(&c, &accepted.reference.revision, "private"),
            &host("alice"),
        )
        .unwrap();
    assert_eq!(visible.graph.edges.len(), 1);
    let mut at_end = selection(&c, &accepted.reference.revision, "operations");
    at_end.valid_at = 10;
    assert!(e
        .resolve_identity(&at_end, &host("bob"))
        .unwrap()
        .graph
        .edges
        .is_empty());
    let mut split = c.clone();
    split.id = "split".into();
    split.groups = vec![vec![c.groups[0][0].clone()], c.groups[0][1..].to_vec()];
    e.submit_identity_candidate(&split, &host("alice")).unwrap();
    let newer = e
        .accept_identity_candidate(
            &request(&split, Some(accepted.reference.revision.clone()), "split"),
            &host("reviewer"),
        )
        .unwrap();
    assert!(e
        .resolve_identity(
            &selection(&c, &newer.reference.revision, "operations"),
            &host("bob")
        )
        .unwrap()
        .graph
        .edges
        .is_empty());
    assert_eq!(
        e.resolve_identity(
            &selection(&c, &accepted.reference.revision, "operations"),
            &host("bob")
        )
        .unwrap()
        .graph
        .edges
        .len(),
        1
    );
}
#[test]
fn copied_mapping_values_cannot_shed_private_source_or_current_policy_restrictions() {
    let (mut e, c) = setup();
    e.submit_identity_candidate(&c, &host("alice")).unwrap();
    let accepted = e
        .accept_identity_candidate(&request(&c, None, "first"), &host("reviewer"))
        .unwrap();
    let mut data = e
        .resolve_identity(
            &selection(&c, &accepted.reference.revision, "private"),
            &host("alice"),
        )
        .unwrap()
        .graph;
    for n in &mut data.nodes {
        n.readers.clear();
    }
    for edge in &mut data.edges {
        edge.readers.clear();
    }
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
    let q: QueryPlan = serde_json::from_value(json!({"graph_id":"copy"})).unwrap();
    assert!(e.query(&q, &host("bob")).unwrap().graph.nodes.is_empty());
    assert_eq!(e.query(&q, &host("alice")).unwrap().graph.edges.len(), 1);
    e.revoke_identity_policy(&c.policy).unwrap();
    assert_eq!(
        e.resolve_identity(
            &selection(&c, &accepted.reference.revision, "private"),
            &host("alice")
        )
        .unwrap_err()
        .code,
        "E_IDENTITY_UNAVAILABLE"
    );
    assert!(e.query(&q, &host("alice")).unwrap().graph.nodes.is_empty());
    e.install_identity_policy(&policy()).unwrap();
    assert!(e.query(&q, &host("alice")).unwrap().graph.nodes.is_empty());
}
#[test]
fn raw_plans_and_capsules_cannot_poison_reserved_identity_storage() {
    let (mut e, c) = setup();
    let name = "weave:identity:poison";
    let program = Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands: vec![Command::Commit {
            graph_id: name.into(),
            branch_id: "main".into(),
            expected_head: None,
            data: GraphData::default(),
        }],
    };
    assert_eq!(
        e.execute(&program, &HostContext::new("alice", [name.into()]))
            .unwrap_err()
            .code,
        "E_RESERVED_NAMESPACE"
    );
    assert!(e.head(name, "main").unwrap().is_none());
    let program = Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands: vec![Command::CommitBatch {
            batch_id: "poison".into(),
            commits: vec![SnapshotCommit {
                graph_id: name.into(),
                branch_id: "main".into(),
                expected_head: None,
                data: GraphData::default(),
            }],
        }],
    };
    assert_eq!(
        e.execute(&program, &HostContext::new("alice", [name.into()]))
            .unwrap_err()
            .code,
        "E_RESERVED_NAMESPACE"
    );
    e.submit_identity_candidate(&c, &host("alice")).unwrap();
    let accepted = e
        .accept_identity_candidate(&request(&c, None, "first"), &host("reviewer"))
        .unwrap();
    let capsule = e
        .export_capsule(&accepted.reference, &host("alice"))
        .unwrap();
    let mut receiver = test_clock::memory().unwrap();
    assert_eq!(
        receiver
            .receive_capsule(&capsule, &host("alice"))
            .unwrap_err()
            .code,
        "E_RESERVED_NAMESPACE"
    );
    assert_eq!(receiver.event_count().unwrap(), 0);
    assert_eq!(
        e.fork_branch(
            &accepted.reference,
            "other",
            &HostContext::new("alice", [accepted.reference.graph_id.clone()])
        )
        .unwrap_err()
        .code,
        "E_RESERVED_NAMESPACE"
    );
}
#[test]
fn immutable_policy_and_unique_partition_membership_cannot_be_forged() {
    let (mut e, c) = setup();
    let mut changed = policy();
    changed.approvers = vec!["alice".into()];
    assert_eq!(
        e.install_identity_policy(&changed).unwrap_err().code,
        "E_POLICY_REVISION"
    );
    let mut fake = c.clone();
    fake.policy.revision = "uninstalled".into();
    assert_eq!(
        e.submit_identity_candidate(&fake, &host("alice"))
            .unwrap_err()
            .code,
        "E_IDENTITY_UNAVAILABLE"
    );
    let mut duplicate = c.clone();
    duplicate.groups.push(vec![c.groups[0][0].clone()]);
    assert_eq!(
        e.submit_identity_candidate(&duplicate, &host("alice"))
            .unwrap_err()
            .code,
        "E_IDENTITY_CANDIDATE"
    );
    e.submit_identity_candidate(&c, &host("alice")).unwrap();
    let mut altered = c.clone();
    altered.valid_time.end = Some(9);
    assert_eq!(
        e.submit_identity_candidate(&altered, &host("alice"))
            .unwrap_err()
            .code,
        "E_IDENTITY_CANDIDATE"
    );
    assert_eq!(e.event_count().unwrap(), 3);
}

#[test]
fn hidden_partition_members_do_not_change_visible_payload_layout_or_coverage() {
    let (mut e, with_private) = setup();
    let mut without_private = with_private.clone();
    without_private.id = "public-only".into();
    without_private.groups[0].pop();
    e.submit_identity_candidate(&without_private, &host("alice"))
        .unwrap();
    let first = e
        .accept_identity_candidate(
            &request(&without_private, None, "public"),
            &host("reviewer"),
        )
        .unwrap();
    e.submit_identity_candidate(&with_private, &host("alice"))
        .unwrap();
    let second = e
        .accept_identity_candidate(
            &request(
                &with_private,
                Some(first.reference.revision.clone()),
                "private",
            ),
            &host("reviewer"),
        )
        .unwrap();
    fn normalized(value: QueryResult, revision: &str, candidate: &str) -> serde_json::Value {
        fn walk(v: &mut serde_json::Value, revision: &str, candidate: &str) {
            match v {
                serde_json::Value::String(s) if s == revision => *s = "accepted-pin".into(),
                serde_json::Value::String(s) if s == candidate => *s = "candidate-label".into(),
                serde_json::Value::Array(values) => {
                    for v in values {
                        walk(v, revision, candidate);
                    }
                }
                serde_json::Value::Object(values) => {
                    for v in values.values_mut() {
                        walk(v, revision, candidate);
                    }
                }
                _ => {}
            }
        }
        let mut value = serde_json::to_value(value).unwrap();
        walk(&mut value, revision, candidate);
        value
    }
    for resolve in [false, true] {
        let read = |revision: &str| {
            if resolve {
                e.resolve_identity(
                    &selection(&with_private, revision, "operations"),
                    &host("bob"),
                )
                .unwrap()
            } else {
                e.query(&serde_json::from_value(json!({"graph_id":first.reference.graph_id,"revision":revision,"valid_at":5})).unwrap(), &host("bob")).unwrap()
            }
        };
        let a = read(&first.reference.revision);
        let b = read(&second.reference.revision);
        assert_eq!(a.coverage, Coverage::Partial);
        assert_eq!(a.coverage, b.coverage);
        assert_eq!(a.diagnostics, b.diagnostics);
        assert!(!serde_json::to_string(&b).unwrap().contains("pairwise-C"));
        assert!(b
            .graph
            .edges
            .iter()
            .all(|edge| !edge.assertion_properties.contains_key("partition")));
        assert_eq!(
            normalized(a, &first.reference.revision, &without_private.id),
            normalized(b, &second.reference.revision, &with_private.id)
        );
    }
}

#[test]
fn repeated_resolution_and_union_preserve_distinct_same_space_manifestations() {
    let mut e = test_clock::memory().unwrap();
    e.install_identity_policy(&policy()).unwrap();
    let a = write(&mut e, "physical", "a", "physical", &[]);
    let graph: GraphData = serde_json::from_value(json!({"nodes":[
        {"id":"b1","entity_id":"shared","space_id":"operations"},
        {"id":"b2","entity_id":"shared","space_id":"operations"}
    ]}))
    .unwrap();
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "operations".into(),
                branch_id: "main".into(),
                expected_head: None,
                data: graph,
            }],
        },
        &host("alice"),
    )
    .unwrap();
    let revision = e.head("operations", "main").unwrap().unwrap();
    let c = IdentityCandidate {
        id: "multi".into(),
        mapping_id: "equipment".into(),
        policy: policy().reference,
        groups: vec![vec![
            a,
            NodeRef {
                graph_id: "operations".into(),
                revision: revision.clone(),
                node_id: "b1".into(),
            },
            NodeRef {
                graph_id: "operations".into(),
                revision,
                node_id: "b2".into(),
            },
        ]],
        evidence: vec![],
        valid_time: Interval {
            start: 0,
            end: Some(10),
        },
        context: None,
    };
    e.submit_identity_candidate(&c, &host("alice")).unwrap();
    let accepted = e
        .accept_identity_candidate(&request(&c, None, "multi"), &host("reviewer"))
        .unwrap();
    let query = selection(&c, &accepted.reference.revision, "operations");
    let r = e.resolve_identity(&query, &host("bob")).unwrap();
    let repeated = e.resolve_identity(&query, &host("bob")).unwrap();
    assert_eq!(r, repeated);
    assert_eq!(r.graph.nodes.len(), 3);
    assert_eq!(r.graph.edges.len(), 2);
    let ctx = AlgebraContext {
        principal: "bob".into(),
        max_objects: 1000,
        max_output_bytes: 1024 * 1024,
    };
    let merged = algebra::union(r.clone(), r.clone(), &ctx).unwrap();
    let nested = algebra::union(merged.clone(), repeated, &ctx).unwrap();
    assert_eq!(merged.graph, nested.graph);
    assert_eq!(merged.graph.nodes.len(), 3);
    assert_eq!(merged.graph.edges.len(), 2);
}

#[test]
fn private_approval_evidence_protects_members_and_node_only_copies() {
    let (mut e, mut c) = setup();
    let old = e.head("private", "main").unwrap();
    let proof: GraphData = serde_json::from_value(json!({"nodes":[{"id":"proof","entity_id":"proof","space_id":"private"}],"edges":[{"id":"approval","predicate":"reviewed","from":"proof","to":"proof","valid_time":{"start":0,"end":10},"readers":["alice","reviewer"]}]})).unwrap();
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "private".into(),
                branch_id: "main".into(),
                expected_head: old,
                data: proof,
            }],
        },
        &host("alice"),
    )
    .unwrap();
    c.groups[0].pop();
    c.evidence = vec![AssertionRef {
        graph_id: "private".into(),
        revision: e.head("private", "main").unwrap().unwrap(),
        assertion_id: "approval".into(),
    }];
    e.submit_identity_candidate(&c, &host("alice")).unwrap();
    let accepted = e
        .accept_identity_candidate(&request(&c, None, "evidence"), &host("reviewer"))
        .unwrap();
    let query: QueryPlan = serde_json::from_value(
        json!({"graph_id":accepted.reference.graph_id,"revision":accepted.reference.revision}),
    )
    .unwrap();
    assert!(e
        .query(&query, &host("bob"))
        .unwrap()
        .graph
        .nodes
        .is_empty());
    assert!(e
        .resolve_identity(
            &selection(&c, &accepted.reference.revision, "operations"),
            &host("bob")
        )
        .unwrap()
        .graph
        .nodes
        .is_empty());
    let authorized = e.query(&query, &host("alice")).unwrap();
    assert_eq!(authorized.graph.nodes.len(), 2);
    assert_eq!(
        e.resolve_identity(
            &selection(&c, &accepted.reference.revision, "operations"),
            &host("alice")
        )
        .unwrap()
        .graph
        .edges
        .len(),
        1
    );
    let mut copied = authorized.graph;
    copied.edges.clear();
    for n in &mut copied.nodes {
        n.readers.clear();
    }
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "copy".into(),
                branch_id: "main".into(),
                expected_head: None,
                data: copied,
            }],
        },
        &host("alice"),
    )
    .unwrap();
    let copy: QueryPlan = serde_json::from_value(json!({"graph_id":"copy"})).unwrap();
    assert!(e.query(&copy, &host("bob")).unwrap().graph.nodes.is_empty());
    assert_eq!(e.query(&copy, &host("alice")).unwrap().graph.nodes.len(), 2);
}

#[test]
fn metadata_navigation_keeps_the_same_scoped_coverage_with_hidden_members() {
    let mut results = Vec::new();
    for include_private in [false, true] {
        let (mut e, mut c) = setup();
        if !include_private {
            c.groups[0].pop();
        }
        e.submit_identity_candidate(&c, &host("alice")).unwrap();
        let accepted = e
            .accept_identity_candidate(&request(&c, None, "accept"), &host("reviewer"))
            .unwrap();
        let carrier: GraphData = serde_json::from_value(json!({"nodes":[{"id":"carrier","entity_id":"carrier","space_id":"physical","metadata":[accepted.reference]}]})).unwrap();
        e.execute(
            &Program {
                version: VERSION.into(),
                source_revisions: vec![],
                commands: vec![Command::Commit {
                    graph_id: "copy".into(),
                    branch_id: "main".into(),
                    expected_head: None,
                    data: carrier,
                }],
            },
            &host("alice"),
        )
        .unwrap();
        let carrier_revision = e.head("copy", "main").unwrap().unwrap();
        let value = e
            .query(
                &serde_json::from_value(json!({"graph_id":"copy","include_metadata":true}))
                    .unwrap(),
                &host("bob"),
            )
            .unwrap();
        assert_eq!(value.coverage, Coverage::Partial);
        assert!(value
            .diagnostics
            .iter()
            .any(|d| d.code == "E_IDENTITY_SCOPE"));
        assert_eq!(value.metadata_graphs.len(), 3);
        assert_eq!(
            value
                .metadata_graphs
                .iter()
                .find(|g| g.reference == accepted.reference)
                .unwrap()
                .graph
                .nodes
                .len(),
            2
        );
        let normalized = serde_json::to_string(&value)
            .unwrap()
            .replace(&accepted.reference.revision, "mapping-pin")
            .replace(&carrier_revision, "carrier-pin");
        assert!(!normalized.contains("pairwise-C"));
        results.push(normalized);
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn revoked_identity_policy_invalidates_cached_views_transitions_and_dispatch_without_head_change() {
    let (mut e, mut c) = setup();
    c.groups[0].pop();
    e.submit_identity_candidate(&c, &host("alice")).unwrap();
    let accepted = e
        .accept_identity_candidate(&request(&c, None, "accept"), &host("reviewer"))
        .unwrap();
    let mut data = e
        .resolve_identity(
            &selection(&c, &accepted.reference.revision, "operations"),
            &host("bob"),
        )
        .unwrap()
        .graph;
    for n in &mut data.nodes {
        n.readers.clear();
    }
    for edge in &mut data.edges {
        edge.readers.clear();
    }
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
    let head = e.head("copy", "main").unwrap();
    let view = ViewDefinition {
        id: "accepted".into(),
        expression: GraphExpression::Query {
            query: serde_json::from_value(json!({"graph_id":"copy"})).unwrap(),
        },
        clock: ViewClock::Fixed,
    };
    let registered = e.register_view(&view, None, &host("bob")).unwrap();
    assert!(registered.current);
    assert_eq!(registered.result.graph.edges.len(), 1);
    assert!(e
        .view_changes("accepted", 0, &host("bob"))
        .unwrap()
        .is_some());
    for (id, graph) in [
        ("copy-adapter", "copy"),
        ("mapping-adapter", accepted.reference.graph_id.as_str()),
    ] {
        e.install_adapter(
            &AdapterManifest {
                id: id.into(),
                version: "1".into(),
                artifact_digest: format!("sha256:{}", "a".repeat(64)),
                config_revision: "1".into(),
                principal: "bob".into(),
                subscriptions: vec![SubscriptionScope {
                    graph_id: graph.into(),
                    branch_id: "main".into(),
                }],
                output_graphs: vec![],
                effect_destinations: vec![],
                max_attempts: 3,
                lease_ms: 100,
                max_pending_events: 100,
                projection_replay: false,
            },
            &host("bob"),
        )
        .unwrap();
        e.set_adapter_state(id, "running").unwrap();
        assert!(test_clock::at(0, || e.poll_adapter(id)).unwrap().is_some());
    }
    e.revoke_identity_policy(&c.policy).unwrap();
    assert_eq!(e.head("copy", "main").unwrap(), head);
    for freshness in [ViewFreshness::RequireCurrent, ViewFreshness::AllowStale] {
        assert_eq!(
            e.read_view("accepted", None, freshness, &host("bob"))
                .unwrap_err()
                .code,
            "E_UNAVAILABLE"
        );
    }
    assert_eq!(
        e.view_changes("accepted", 0, &host("bob"))
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
    for id in ["copy-adapter", "mapping-adapter"] {
        assert_eq!(
            test_clock::at(101, || e.poll_adapter(id)).unwrap_err().code,
            "E_UNAVAILABLE"
        );
    }
    let refreshed = e.refresh_view("accepted", None, &host("bob")).unwrap();
    assert!(refreshed.result.graph.nodes.is_empty());
    // A fresh empty result must not expose removed private member IDs through its old transition.
    assert_eq!(
        e.view_changes("accepted", 1, &host("bob"))
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
    assert!(e
        .read_view("accepted", None, ViewFreshness::AllowStale, &host("bob"))
        .unwrap()
        .result
        .graph
        .nodes
        .is_empty());
}

#[test]
fn revoked_identity_policy_blocks_signed_cached_response_retry() {
    use ed25519_dalek::SigningKey;
    use weave_policy::{
        Action, AdmissionContext, AdmissionProof, Capability, Operation, Request, RootAuthority,
        Scope,
    };
    let (mut e, mut c) = setup();
    c.groups[0].pop();
    e.submit_identity_candidate(&c, &host("alice")).unwrap();
    let accepted = e
        .accept_identity_candidate(&request(&c, None, "accept"), &host("reviewer"))
        .unwrap();
    let root = SigningKey::from_bytes(&[17; 32]);
    let user = SigningKey::from_bytes(&[18; 32]);
    let mut scopes: Vec<_> = [
        accepted.reference.graph_id.as_str(),
        "physical",
        "operations",
    ]
    .into_iter()
    .map(|id| Scope {
        graph_id: id.into(),
        branch_id: "main".into(),
        actions: [Action::Read, Action::Traverse].into(),
    })
    .collect();
    scopes.sort();
    let context = AdmissionContext {
        audience: "replica".into(),
        now_ms: 200,
        policy_epoch: "epoch".into(),
        roots: vec![RootAuthority {
            issuer: weave_policy::public_key(&root),
            audience: "replica".into(),
            policy_revision: "1".into(),
            scopes: scopes.clone(),
            not_before_ms: 0,
            expires_at_ms: 10000,
            max_delegations: 1,
        }],
        revoked_capabilities: Default::default(),
        revoked_keys: Default::default(),
        consumed_nonces: Default::default(),
    };
    e.install_admission_policy(&context).unwrap();
    let query: QueryPlan =
        serde_json::from_value(json!({"graph_id":accepted.reference.graph_id})).unwrap();
    let cap = weave_policy::sign_capability(
        Capability {
            version: weave_policy::VERSION.into(),
            issuer: weave_policy::public_key(&root),
            subject: weave_policy::public_key(&user),
            audience: "replica".into(),
            policy_revision: "1".into(),
            scopes,
            not_before_ms: 10,
            expires_at_ms: 9000,
            delegations_remaining: 0,
            parent: None,
        },
        &root,
    )
    .unwrap();
    let request = weave_policy::sign_request(
        Request {
            version: weave_policy::REQUEST_VERSION.into(),
            subject: weave_policy::public_key(&user),
            audience: "replica".into(),
            capability_id: weave_policy::capability_id(&cap).unwrap(),
            nonce: "12".repeat(32),
            issued_at_ms: 100,
            expires_at_ms: 1000,
            operation: Operation {
                action: Action::Read,
                graph_id: accepted.reference.graph_id.clone(),
                branch_id: "main".into(),
            },
            body_digest: weave_policy::body_digest(&serde_json::to_vec(&query).unwrap()),
        },
        &user,
    )
    .unwrap();
    let proof = AdmissionProof {
        chain: vec![cap],
        request,
    };
    assert_eq!(
        test_clock::at(200, || e.admit_query(&proof, &query))
            .unwrap()
            .result
            .graph
            .nodes
            .len(),
        2
    );
    assert!(
        test_clock::at(201, || e.admit_query(&proof, &query))
            .unwrap()
            .duplicate
    );
    e.revoke_identity_policy(&c.policy).unwrap();
    assert_eq!(
        test_clock::at(202, || e.admit_query(&proof, &query))
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
}

#[test]
fn program_identity_selector_is_a_pinned_read_not_acceptance_authority() {
    let (mut e, mut c) = setup();
    c.groups[0].pop();
    e.submit_identity_candidate(&c, &host("alice")).unwrap();
    let receipt = e
        .accept_identity_candidate(&request(&c, None, "accept"), &host("reviewer"))
        .unwrap();
    let selection = selection(&c, &receipt.reference.revision, "operations");
    let expected = e.resolve_identity(&selection, &host("bob")).unwrap();
    let expression = GraphExpression::ResolveIdentity { selection };
    let program = Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands: vec![Command::Evaluate {
            value: expression.clone(),
        }],
    };
    let events = e.event_count().unwrap();
    let results = e.execute(&program, &host("bob")).unwrap();
    let CommandResult::Queried { result } = &results[0] else {
        panic!("query result required")
    };
    assert_eq!(**result, expected);
    assert_eq!(e.event_count().unwrap(), events);
    let mut old = program.clone();
    old.version = "0.12.0".into();
    old.commands = vec![
        Command::Commit {
            graph_id: "copy".into(),
            branch_id: "main".into(),
            expected_head: None,
            data: GraphData::default(),
        },
        Command::Evaluate {
            value: GraphExpression::Filter {
                input: Box::new(expression),
                predicate: None,
                valid_at: None,
            },
        },
    ];
    assert_eq!(
        e.execute(&old, &host("alice")).unwrap_err().code,
        "E_VERSION"
    );
    assert!(e.head("copy", "main").unwrap().is_none());
    e.revoke_identity_policy(&c.policy).unwrap();
    assert_eq!(
        e.execute(&program, &host("bob")).unwrap_err().code,
        "E_IDENTITY_UNAVAILABLE"
    );
}

#[test]
fn root_handler_receipt_retry_rechecks_event_and_unrelated_query_authority() {
    let (mut e, mut candidate) = setup();
    candidate.groups[0].pop();
    e.submit_identity_candidate(&candidate, &host("alice"))
        .unwrap();
    let accepted = e
        .accept_identity_candidate(
            &request(&candidate, None, "receipt-accept"),
            &host("reviewer"),
        )
        .unwrap();
    let mut deliveries = vec![];
    for (id, graph, query) in [
        ("revoked-event", accepted.reference.graph_id.as_str(), false),
        ("revoked-query", "physical", true),
    ] {
        e.install_adapter(
            &AdapterManifest {
                id: id.into(),
                version: "1".into(),
                artifact_digest: format!("sha256:{}", "a".repeat(64)),
                config_revision: "1".into(),
                principal: "bob".into(),
                subscriptions: vec![SubscriptionScope {
                    graph_id: graph.into(),
                    branch_id: "main".into(),
                }],
                output_graphs: vec![],
                effect_destinations: vec![],
                max_attempts: 3,
                lease_ms: 100,
                max_pending_events: 100,
                projection_replay: false,
            },
            &host("bob"),
        )
        .unwrap();
        e.set_adapter_state(id, "running").unwrap();
        let delivery = test_clock::at(0, || e.poll_adapter(id)).unwrap().unwrap();
        let program = Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: if query {
                vec![Command::Query {query:serde_json::from_value(json!({"graph_id":accepted.reference.graph_id,"revision":accepted.reference.revision})).unwrap()}]
            } else {
                vec![]
            },
        };
        let first = e
            .complete_handler(id, &delivery.id, &delivery.lease, &program)
            .unwrap();
        assert!(!first.duplicate);
        if query {
            let CommandResult::Queried { result } = &first.results[0] else {
                panic!()
            };
            assert!(!result.graph.nodes.is_empty());
        }
        assert!(
            e.complete_handler(id, &delivery.id, &delivery.lease, &program)
                .unwrap()
                .duplicate
        );
        deliveries.push((id, delivery, program));
    }
    let count = e.event_count().unwrap();
    e.revoke_identity_policy(&candidate.policy).unwrap();
    for (id, delivery, program) in deliveries {
        assert_eq!(
            e.complete_handler(id, &delivery.id, &delivery.lease, &program)
                .unwrap_err()
                .code,
            "E_UNAVAILABLE"
        );
    }
    assert_eq!(e.event_count().unwrap(), count);
}

#[test]
fn join_endpoint_only_wrappers_recheck_relationship_policy_after_persistence() {
    let (mut e, c) = setup();
    e.submit_identity_candidate(&c, &host("alice")).unwrap();
    let accepted = e
        .accept_identity_candidate(&request(&c, None, "join-policy"), &host("reviewer"))
        .unwrap();
    let resolve = selection(&c, &accepted.reference.revision, "operations");
    let view = e.resolve_identity(&resolve, &host("alice")).unwrap();
    let left: GraphData = serde_json::from_value(json!({"nodes":[
        {"id":"outer","entity_id":"left","space_id":"s"},
        {"id":"middle","entity_id":"middle","space_id":"s"}],
        "edges":[{"id":"relation","from":"outer","to":"middle","predicate":"link","valid_time":{"start":0,"end":10},"derived_from":view.provenance}]
    })).unwrap();
    let right: GraphData = serde_json::from_value(json!({"nodes":[
        {"id":"middle","entity_id":"middle","space_id":"s"},
        {"id":"outer","entity_id":"right","space_id":"s"}],
        "edges":[{"id":"link","from":"middle","to":"outer","predicate":"link","valid_time":{"start":0,"end":10}}]
    })).unwrap();
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![
                Command::Commit {
                    graph_id: "copy".into(),
                    branch_id: "main".into(),
                    expected_head: None,
                    data: left,
                },
                Command::Commit {
                    graph_id: "operations".into(),
                    branch_id: "main".into(),
                    expected_head: e.head("operations", "main").unwrap(),
                    data: right,
                },
            ],
        },
        &host("alice"),
    )
    .unwrap();
    let expression: GraphExpression = serde_json::from_value(json!({"kind":"join","left":{"kind":"query","query":{"graph_id":"copy"}},"right":{"kind":"query","query":{"graph_id":"operations"}},"output_predicate":"path","match_on":"entity_space_to_from"})).unwrap();
    let results = e
        .execute(
            &Program {
                version: VERSION.into(),
                source_revisions: vec![],
                commands: vec![Command::Evaluate { value: expression }],
            },
            &host("alice"),
        )
        .unwrap();
    let CommandResult::Queried { result } = &results[0] else {
        panic!()
    };
    assert_eq!(result.graph.edges.len(), 1);
    assert!(result.node_origins.values().all(Vec::is_empty));
    let mut saved = result.graph.clone();
    saved.edges.clear();
    saved.attachments.clear();
    saved.schema = None;
    saved.context_typing = None;
    saved.influence = None;
    for node in &mut saved.nodes {
        node.readers.clear();
        node.type_id = None;
        assert!(!node.derived_from.is_empty());
    }
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "operations".into(),
                branch_id: "main".into(),
                expected_head: e.head("operations", "main").unwrap(),
                data: saved,
            }],
        },
        &host("alice"),
    )
    .unwrap();
    let q: QueryPlan = serde_json::from_value(json!({"graph_id":"operations"})).unwrap();
    assert_eq!(e.query(&q, &host("alice")).unwrap().graph.nodes.len(), 2);
    e.revoke_identity_policy(&c.policy).unwrap();
    let denied = e.query(&q, &host("alice")).unwrap();
    assert!(denied.graph.nodes.is_empty());
    assert_eq!(denied.coverage, Coverage::Partial);
}

#[test]
fn empty_value_influence_revocation_invalidates_cached_view_without_head_movement() {
    let (mut e, c) = setup();
    e.submit_identity_candidate(&c, &host("alice")).unwrap();
    let accepted = e
        .accept_identity_candidate(&request(&c, None, "empty-cache"), &host("reviewer"))
        .unwrap();
    let q: QueryPlan = serde_json::from_value(
        json!({"graph_id":accepted.reference.graph_id,"revision":accepted.reference.revision}),
    )
    .unwrap();
    let source = e.query(&q, &host("alice")).unwrap();
    let gate = source.node_origins.values().next().unwrap()[0].clone();
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "copy".into(),
                branch_id: "main".into(),
                expected_head: None,
                data: GraphData {
                    influence: Some(GraphInfluence {
                        assertions: vec![],
                        nodes: vec![gate],
                    }),
                    ..GraphData::default()
                },
            }],
        },
        &host("alice"),
    )
    .unwrap();
    let definition = ViewDefinition {
        id: "empty-influenced".into(),
        expression: GraphExpression::Query {
            query: serde_json::from_value(json!({"graph_id":"copy"})).unwrap(),
        },
        clock: ViewClock::Fixed,
    };
    e.register_view(&definition, None, &host("alice")).unwrap();
    let head = e.head("copy", "main").unwrap();
    e.revoke_identity_policy(&c.policy).unwrap();
    assert_eq!(e.head("copy", "main").unwrap(), head);
    for freshness in [ViewFreshness::AllowStale, ViewFreshness::RequireCurrent] {
        assert_eq!(
            e.read_view("empty-influenced", None, freshness, &host("alice"))
                .unwrap_err()
                .code,
            "E_UNAVAILABLE"
        );
    }
    assert_eq!(
        e.view_changes("empty-influenced", 0, &host("alice"))
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
}

#[cfg(feature = "recovery-testing")]
#[test]
fn identity_observer_panic_rolls_back_and_preserves_retry() {
    let (mut e, candidate) = setup();
    e.submit_identity_candidate(&candidate, &host("alice"))
        .unwrap();
    let request = request(&candidate, None, "panic");
    let count = e.event_count().unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        e.accept_identity_test_before_commit(&request, &host("reviewer"), || panic!("observer"))
    }));
    assert!(panic.is_err());
    assert!(e.identity_head(&candidate.mapping_id).unwrap().is_none());
    assert_eq!(e.event_count().unwrap(), count);
    assert!(
        !e.accept_identity_candidate(&request, &host("reviewer"))
            .unwrap()
            .duplicate
    );
}
