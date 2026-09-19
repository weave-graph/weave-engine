#[path = "../tests/support/clock.rs"]
mod test_clock;
use ed25519_dalek::SigningKey;
use serde_json::json;
use weave_contract::{GraphRef, QueryPlan, VERSION};
use weave_engine::*;
use weave_policy::{
    Action, AdmissionContext, AdmissionProof, Capability, Operation, Request, RootAuthority, Scope,
};
fn host() -> HostContext {
    HostContext::new("reviewer", ["source".into()])
}
struct Peer {
    root: SigningKey,
    user: SigningKey,
    context: AdmissionContext,
}
impl Peer {
    fn new() -> Self {
        let root = SigningKey::from_bytes(&[21; 32]);
        let user = SigningKey::from_bytes(&[22; 32]);
        let scopes = vec![Scope {
            graph_id: "source".into(),
            branch_id: "main".into(),
            actions: [Action::Propose].into(),
        }];
        let context = AdmissionContext {
            audience: "receiver".into(),
            now_ms: 200,
            policy_epoch: "epoch1".into(),
            roots: vec![RootAuthority {
                issuer: weave_policy::public_key(&root),
                audience: "receiver".into(),
                policy_revision: "1".into(),
                scopes,
                not_before_ms: 0,
                expires_at_ms: 10000,
                max_delegations: 1,
            }],
            revoked_capabilities: Default::default(),
            revoked_keys: Default::default(),
            consumed_nonces: Default::default(),
        };
        Self {
            root,
            user,
            context,
        }
    }
    fn proof(&self, capsule: &Capsule) -> AdmissionProof {
        let cap = weave_policy::sign_capability(
            Capability {
                version: weave_policy::VERSION.into(),
                issuer: weave_policy::public_key(&self.root),
                subject: weave_policy::public_key(&self.user),
                audience: "receiver".into(),
                policy_revision: "1".into(),
                scopes: self.context.roots[0].scopes.clone(),
                not_before_ms: 10,
                expires_at_ms: 9000,
                delegations_remaining: 0,
                parent: None,
            },
            &self.root,
        )
        .unwrap();
        let request = weave_policy::sign_request(
            Request {
                version: weave_policy::REQUEST_VERSION.into(),
                subject: weave_policy::public_key(&self.user),
                audience: "receiver".into(),
                capability_id: weave_policy::capability_id(&cap).unwrap(),
                nonce: "ab".repeat(32),
                issued_at_ms: 100,
                expires_at_ms: 1000,
                operation: Operation {
                    action: Action::Propose,
                    graph_id: "source".into(),
                    branch_id: "main".into(),
                },
                body_digest: weave_policy::body_digest(&serde_json::to_vec(capsule).unwrap()),
            },
            &self.user,
        )
        .unwrap();
        AdmissionProof {
            chain: vec![cap],
            request,
        }
    }
}
fn source(readers: serde_json::Value) -> (Engine, Capsule) {
    let mut e = test_clock::memory().unwrap();
    e.execute(&serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"source","data":{"nodes":[{"id":"a","entity_id":"a","space_id":"s","readers":readers},{"id":"b","entity_id":"b","space_id":"s","readers":readers}],"edges":[{"id":"link","from":"a","to":"b","predicate":"connected","valid_time":{"start":0},"readers":readers}]}}]})).unwrap(),&host()).unwrap();
    let root = GraphRef {
        graph_id: "source".into(),
        revision: e.head("source", "main").unwrap().unwrap(),
    };
    let capsule = e.export_capsule(&root, &host()).unwrap();
    (e, capsule)
}
fn decision(id: String) -> IntegrationRequest {
    IntegrationRequest {
        proposal_id: id,
        branch_id: "offline".into(),
        expected_head: None,
        nonce: "approval".into(),
    }
}
#[allow(dead_code)]
fn query() -> QueryPlan {
    serde_json::from_value(json!({"graph_id":"source","branch_id":"offline"})).unwrap()
}

// Deterministic test keys and a trusted local host only; never a credential fixture for deployment.
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let operation = &args[2];
    let (_, capsule) = source(json!([]));
    let peer = Peer::new();
    let proof = peer.proof(&capsule);
    let mut receiver = test_clock::open(&args[1]).unwrap();
    if operation == "prepare" {
        receiver.install_admission_policy(&peer.context).unwrap();
    }
    let proposed = test_clock::at(200, || receiver.admit_proposal(&proof, &capsule)).unwrap();
    let request = decision(proposed.result.id);
    let output = match operation.as_str() {
        "prepare" => json!({"isolated":true,"events":receiver.event_count().unwrap()}),
        "before" => {
            test_clock::at(201, || {
                receiver.integrate_proposal_test_before_commit(&request, &proof, &host(), || {
                    std::process::exit(86)
                })
            })
            .unwrap();
            unreachable!()
        }
        "after" => {
            test_clock::at(201, || {
                receiver.integrate_proposal(&request, &proof, &host())
            })
            .unwrap();
            std::process::exit(87)
        }
        "retry" => {
            json!({"receipt":test_clock::at(202, || receiver.integrate_proposal(&request,&proof,&host())).unwrap(),"events":receiver.event_count().unwrap(),"edges":receiver.query(&query(),&host()).unwrap().graph.edges.len()})
        }
        "changed" => {
            let mut changed = request.clone();
            changed.branch_id = "other".into();
            json!({"error":test_clock::at(202, || receiver.integrate_proposal(&changed,&proof,&host())).unwrap_err().code})
        }
        "revoke" => {
            let mut revoked = peer.context.clone();
            revoked.policy_epoch = "epoch2".into();
            revoked
                .revoked_keys
                .insert(weave_policy::public_key(&peer.user));
            receiver.install_admission_policy(&revoked).unwrap();
            json!({"error":test_clock::at(202, || receiver.integrate_proposal(&request,&proof,&host())).unwrap_err().code})
        }
        _ => panic!("unknown test operation"),
    };
    println!("{}", serde_json::to_string(&output).unwrap());
}
