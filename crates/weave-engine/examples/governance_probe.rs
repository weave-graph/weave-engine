#[path = "../tests/support/clock.rs"]
mod test_clock;
// Fixed local test keys only. This executable is not an external authority endpoint.
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut engine = test_clock::open(&args[1]).unwrap();
    match args[2].as_str() {
        "prepare" => {
            let source = seed(&mut engine, 2);
            quorum(&engine, &proposal("a", source.clone(), None));
            quorum(&engine, &proposal("b", source, None));
            println!(
                "{}",
                json!({"events": engine.governance_event_count().unwrap()})
            );
        }
        "before" => {
            test_clock::at(20, || {
                engine.accept_governance_test_before_commit(
                    &request("a", "accept"),
                    &host(),
                    || std::process::exit(88),
                )
            })
            .unwrap();
        }
        "after" => {
            test_clock::at(20, || {
                engine.accept_governance(&request("a", "accept"), &host())
            })
            .unwrap();
            std::process::exit(89);
        }
        "retry" => println!(
            "{}",
            serde_json::to_string(
                &test_clock::at(21, || engine
                    .accept_governance(&request("a", "accept"), &host()))
                .unwrap()
            )
            .unwrap()
        ),
        "changed" => println!(
            "{}",
            json!({"error": test_clock::at(21, || engine.accept_governance(&request("b", "accept"), &host())).unwrap_err().code})
        ),
        "expired" => println!(
            "{}",
            json!({"error": test_clock::at(8001, || engine.accept_governance(&request("a", "accept"), &host())).unwrap_err().code})
        ),
        "inspect" => println!(
            "{}",
            serde_json::to_string(
                &test_clock::at(21, || engine.inspect_governance_head("team", &host())).unwrap()
            )
            .unwrap()
        ),
        _ => panic!("unknown local test operation"),
    }
}
