//! Trusted administrator test fixture for source-language integration, not a remote API.
//! Policy installation/acceptance here cannot be expressed in a graph Program.
use serde_json::json;
use weave_contract::{ContextSelection, Interval, NodeRef, Program, VERSION};
use weave_engine::{
    Engine, HostContext, IdentityCandidate, IdentityDecisionRequest, IdentityPolicy,
    IdentityPolicyRef, IdentityResolve,
};
fn policy() -> IdentityPolicy {
    IdentityPolicy {
        reference: IdentityPolicyRef {
            id: "review".into(),
            revision: "1".into(),
        },
        proposers: vec!["alice".into()],
        approvers: vec!["reviewer".into()],
        readers: vec![],
        allowed_spaces: vec!["physical".into(), "operations".into()],
        max_members: 8,
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: native_services_fixture DATABASE seed|revoke|events".into());
    }
    let mut engine = Engine::open(&args[1])?;
    match args[2].as_str() {
        "seed" => {
            engine.install_identity_policy(&policy())?;
            let mut members = Vec::new();
            for (graph, entity) in [
                ("physical", "independent-A"),
                ("operations", "independent-B"),
            ] {
                let program: Program = serde_json::from_value(
                    json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"data":{"nodes":[{"id":"node","entity_id":entity,"space_id":graph,"properties":{"local_state":graph}}],"edges":[]}}]}),
                )?;
                engine.execute(&program, &HostContext::new("alice", [graph.into()]))?;
                members.push(NodeRef {
                    graph_id: graph.into(),
                    revision: engine.head(graph, "main")?.ok_or("missing source head")?,
                    node_id: "node".into(),
                });
            }
            let source = members[0].clone();
            engine.submit_identity_candidate(
                &IdentityCandidate {
                    id: "candidate".into(),
                    mapping_id: "equipment".into(),
                    policy: policy().reference,
                    groups: vec![members],
                    evidence: vec![],
                    valid_time: Interval {
                        start: 0,
                        end: Some(10),
                    },
                    context: None,
                },
                &HostContext::new("alice", []),
            )?;
            let accepted = engine.accept_identity_candidate(
                &IdentityDecisionRequest {
                    candidate_id: "candidate".into(),
                    expected_head: None,
                    nonce: "fixture-acceptance".into(),
                },
                &HostContext::new("reviewer", []),
            )?;
            let selection = IdentityResolve {
                mapping_id: "equipment".into(),
                revision: accepted.reference.revision,
                policy: policy().reference,
                source,
                target_space: "operations".into(),
                valid_at: 5,
                context: ContextSelection::Default,
            };
            let direct = engine.resolve_identity(&selection, &HostContext::new("reader", []))?;
            println!(
                "{}",
                json!({"selection":selection,"direct":direct,"events":engine.event_count()?})
            );
        }
        "revoke" => {
            engine.revoke_identity_policy(&policy().reference)?;
            println!("{}", json!({"events":engine.event_count()?}));
        }
        "events" => println!("{}", json!({"events":engine.event_count()?})),
        _ => return Err("unknown fixture operation".into()),
    }
    Ok(())
}
