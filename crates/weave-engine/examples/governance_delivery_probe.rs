#[path = "../tests/support/clock.rs"]
mod test_clock;
// Fixed local test keys only; no remote authority endpoint.
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

fn adapter(id: &str, principal: &str) -> AdapterManifest {
    AdapterManifest {
        id: id.into(),
        version: "1".into(),
        artifact_digest: format!("sha256:{}", "0".repeat(64)),
        config_revision: "1".into(),
        principal: principal.into(),
        subscriptions: vec![SubscriptionScope {
            graph_id: "source".into(),
            branch_id: "main".into(),
        }],
        output_graphs: vec![],
        effect_destinations: vec![],
        max_attempts: 2,
        lease_ms: 100,
        max_pending_events: 100,
        projection_replay: false,
    }
}
fn install(e: &Engine, id: &str, principal: &str) -> HostContext {
    let h = HostContext::new(principal, []);
    e.install_adapter(&adapter(id, principal), &h).unwrap();
    e.set_adapter_state(id, "running").unwrap();
    test_clock::at(20, || e.subscribe_governance(id, "team", &h)).unwrap();
    h
}
fn accepted(e: &Engine, id: &str, source: GraphRef, head: Option<String>) -> GovernanceReceipt {
    quorum(e, &proposal(id, source, head));
    test_clock::at(20, || e.accept_governance(&request(id, id), &host())).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut e = test_clock::open(&args[1]).unwrap();
    let reader = HostContext::new("reader", []);
    match args[2].as_str() {
        "prepare" => {
            let source = seed(&mut e, 2);
            accepted(&e, "a", source, None);
            install(&e, "reader", "reader");
            println!(
                "{}",
                serde_json::to_string(
                    &test_clock::at(20, || e.poll_governance("reader", "team", &reader))
                        .unwrap()
                        .unwrap()
                )
                .unwrap()
            );
        }
        "before" => {
            test_clock::at(21, || {
                e.acknowledge_governance_test_before_commit(
                    "reader",
                    "team",
                    &args[3],
                    &args[4],
                    &reader,
                    || std::process::exit(90),
                )
            })
            .unwrap();
        }
        "after" => {
            test_clock::at(21, || {
                e.acknowledge_governance("reader", "team", &args[3], &args[4], &reader)
            })
            .unwrap();
            std::process::exit(91);
        }
        "retry" => println!(
            "{}",
            serde_json::to_string(
                &test_clock::at(22, || e
                    .acknowledge_governance("reader", "team", &args[3], &args[4], &reader))
                .unwrap()
            )
            .unwrap()
        ),
        "changed" => println!(
            "{}",
            json!({"error":test_clock::at(22, || e.acknowledge_governance("reader","team",&args[3],&"f".repeat(48),&reader)).unwrap_err().code})
        ),
        "revoke" => {
            let head = test_clock::at(20, || e.inspect_governance_head("team", &host())).unwrap();
            let mut next = policy(2);
            next.reference.revision = "2".into();
            next.readers = vec!["collector".into()];
            let q = GovernanceProposal {
                id: "revoke".into(),
                view_id: "team".into(),
                policy: policy(2).reference,
                expected_head: head.decision_id,
                expires_at_ms: 9000,
                action: GovernanceAction::ReplacePolicy { policy: next },
            };
            quorum(&e, &q);
            test_clock::at(20, || {
                e.accept_governance(&request("revoke", "revoke"), &host())
            })
            .unwrap();
            println!(
                "{}",
                json!({"error":test_clock::at(22, || e.acknowledge_governance("reader","team",&args[3],&args[4],&reader)).unwrap_err().code})
            );
        }
        _ => panic!("unknown local test operation"),
    }
}
