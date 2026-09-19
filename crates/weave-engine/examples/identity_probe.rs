//! Actual-process recovery probe; termination hook exists only with recovery-testing.
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
fn request() -> IdentityDecisionRequest {
    IdentityDecisionRequest {
        candidate_id: "candidate".into(),
        expected_head: None,
        nonce: "decision-one".into(),
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: identity_probe DATABASE seed|crash-before|crash-after|retry|changed|revoke|resolve".into());
    }
    let mut e = Engine::open(&args[1])?;
    let actor = HostContext::new("reviewer", []);
    match args[2].as_str() {
        "seed" => {
            e.install_identity_policy(&policy())?;
            let mut members = Vec::new();
            for (graph, entity) in [
                ("physical", "independent-A"),
                ("operations", "independent-B"),
            ] {
                let p: Program = serde_json::from_value(
                    json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"data":{"nodes":[{"id":"node","entity_id":entity,"space_id":graph}],"edges":[]}}]}),
                )?;
                e.execute(&p, &HostContext::new("alice", [graph.into()]))?;
                members.push(NodeRef {
                    graph_id: graph.into(),
                    revision: e.head(graph, "main")?.unwrap(),
                    node_id: "node".into(),
                });
            }
            e.submit_identity_candidate(
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
            println!(
                "{}",
                json!({"events":e.event_count()?,"head":e.identity_head("equipment")?})
            );
        }
        #[cfg(feature = "recovery-testing")]
        "crash-before" => {
            e.accept_identity_test_before_commit(&request(), &actor, || std::process::exit(83))?;
            return Err("pre-commit termination hook not reached".into());
        }
        "crash-after" => {
            e.accept_identity_candidate(&request(), &actor)?;
            std::process::exit(84);
        }
        "retry" | "changed" => {
            let mut r = request();
            if args[2] == "changed" {
                r.expected_head = e.identity_head("equipment")?;
            }
            match e.accept_identity_candidate(&r, &actor) {
                Ok(receipt) => println!("{}", json!({"receipt":receipt,"events":e.event_count()?})),
                Err(error) => println!("{}", json!({"error":error.code,"events":e.event_count()?})),
            }
        }
        "revoke" => {
            e.revoke_identity_policy(&policy().reference)?;
            println!("{}", json!({"events":e.event_count()?}));
        }
        "resolve" => {
            let r = IdentityResolve {
                mapping_id: "equipment".into(),
                revision: e.identity_head("equipment")?.ok_or("mapping missing")?,
                policy: policy().reference,
                source: NodeRef {
                    graph_id: "physical".into(),
                    revision: e.head("physical", "main")?.unwrap(),
                    node_id: "node".into(),
                },
                target_space: "operations".into(),
                valid_at: 5,
                context: ContextSelection::Default,
            };
            match e.resolve_identity(&r, &HostContext::new("bob", [])) {
                Ok(value) => println!(
                    "{}",
                    json!({"nodes":value.graph.nodes.len(),"edges":value.graph.edges.len()})
                ),
                Err(error) => println!("{}", json!({"error":error.code})),
            }
        }
        _ => return Err("unknown operation".into()),
    }
    Ok(())
}
