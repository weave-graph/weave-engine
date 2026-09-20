//! Explicit trusted test host for the three-process paper trace. Never a remote server.
//! Test-only keys, host-selected clock, artifact execution and export are deliberately local.
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;
use weave_policy::{
    Action, AdmissionContext, AdmissionProof, Capability, Operation, Request, RootAuthority, Scope,
};
type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const GRAPHS: [&str; 7] = [
    "Installation",
    "Evidence",
    "Annotations",
    "Warning",
    "ClustersPhone",
    "ClustersWork",
    "Saved",
];
fn owner_key() -> SigningKey {
    SigningKey::from_bytes(&[201; 32])
}
fn owner() -> String {
    weave_policy::public_key(&owner_key())
}
fn host() -> HostContext {
    HostContext::new(owner(), GRAPHS.map(String::from))
}
fn field<'a>(v: &'a Value, k: &str) -> TestResult<&'a str> {
    v[k].as_str().ok_or_else(|| format!("missing {k}").into())
}
fn root_key(peer: &str) -> TestResult<SigningKey> {
    Ok(SigningKey::from_bytes(
        &[match peer {
            "P" => 202,
            "W" => 203,
            "T" => 204,
            _ => return Err("unknown test peer".into()),
        }; 32],
    ))
}
fn scopes() -> Vec<Scope> {
    let mut scopes: Vec<Scope> = GRAPHS
        .into_iter()
        .flat_map(|g| {
            ["main", "phone", "phone-import", "work"].map(|b| Scope {
                graph_id: g.into(),
                branch_id: b.into(),
                actions: [Action::Propose].into(),
            })
        })
        .collect();
    scopes.sort();
    scopes
}
fn context(peer: &str) -> TestResult<AdmissionContext> {
    Ok(AdmissionContext {
        audience: format!("fixture-{peer}"),
        now_ms: 20,
        policy_epoch: "epoch1".into(),
        roots: vec![RootAuthority {
            issuer: weave_policy::public_key(&root_key(peer)?),
            audience: format!("fixture-{peer}"),
            policy_revision: "1".into(),
            scopes: scopes(),
            not_before_ms: 0,
            expires_at_ms: 10000,
            max_delegations: 1,
        }],
        revoked_capabilities: Default::default(),
        revoked_keys: Default::default(),
        consumed_nonces: Default::default(),
    })
}
fn proof(peer: &str, capsule: &Capsule, nonce: &str) -> TestResult<AdmissionProof> {
    let cap = weave_policy::sign_capability(
        Capability {
            version: weave_policy::VERSION.into(),
            issuer: weave_policy::public_key(&root_key(peer)?),
            subject: owner(),
            audience: format!("fixture-{peer}"),
            policy_revision: "1".into(),
            scopes: scopes(),
            not_before_ms: 0,
            expires_at_ms: 9000,
            delegations_remaining: 0,
            parent: None,
        },
        &root_key(peer)?,
    )?;
    let root = capsule
        .revisions
        .iter()
        .find(|r| r.graph_id == capsule.root.graph_id && r.revision == capsule.root.revision)
        .ok_or("capsule root")?;
    let request = weave_policy::sign_request(
        Request {
            version: weave_policy::REQUEST_VERSION.into(),
            subject: owner(),
            audience: format!("fixture-{peer}"),
            capability_id: weave_policy::capability_id(&cap)?,
            nonce: format!("{:x}", Sha256::digest(nonce.as_bytes())),
            issued_at_ms: 0,
            expires_at_ms: 9000,
            operation: Operation {
                action: Action::Propose,
                graph_id: root.graph_id.clone(),
                branch_id: root.branch_id.clone(),
            },
            body_digest: weave_policy::body_digest(&serde_json::to_vec(capsule)?),
        },
        &owner_key(),
    )?;
    Ok(AdmissionProof {
        chain: vec![cap],
        request,
    })
}
fn accept(e: &Engine, v: &Value) -> TestResult<Value> {
    let source: GraphRef = serde_json::from_value(v["source"].clone())?;
    let view = field(v, "view")?;
    let id = field(v, "id")?;
    let branch = field(v, "branch")?;
    let key = SigningKey::from_bytes(&[205; 32]);
    let reference = GovernancePolicyRef {
        id: format!("policy-{view}"),
        revision: "1".into(),
    };
    e.install_governance_root(&GovernancePolicy {
        view_id: view.into(),
        reference: reference.clone(),
        members: vec![weave_policy::public_key(&key)],
        threshold: 1,
        proposers: vec![owner()],
        readers: vec![owner()],
        allowed_sources: vec![GovernanceSourceScope {
            graph_id: source.graph_id.clone(),
            branch_id: branch.into(),
        }],
        not_before_ms: 0,
        expires_at_ms: 10000,
    })?;
    let head = e.inspect_governance_head(view, &host())?.decision_id;
    let proposal = GovernanceProposal {
        id: id.into(),
        view_id: view.into(),
        policy: reference.clone(),
        expected_head: head.clone(),
        expires_at_ms: 9000,
        action: GovernanceAction::Publish {
            source,
            branch_id: branch.into(),
        },
    };
    let proposed = e.propose_governance(&proposal, &host())?;
    let approval = sign_governance_approval(
        GovernanceApproval {
            proposal_id: id.into(),
            proposal_digest: proposed.digest,
            view_id: view.into(),
            policy: reference,
            expected_head: head,
            member: weave_policy::public_key(&key),
            issued_at_ms: 0,
            expires_at_ms: 9000,
            nonce: format!("approval-{id}"),
        },
        &key,
    )?;
    e.record_governance_approval(&approval, &host())?;
    Ok(serde_json::to_value(e.accept_governance(
        &GovernanceDecisionRequest {
            proposal_id: id.into(),
            nonce: format!("decision-{id}"),
        },
        &host(),
    )?)?)
}
fn run(e: &mut Engine, peer: &str, v: &Value) -> TestResult<Value> {
    Ok(match field(v, "op")? {
        "init" => {
            e.install_admission_policy(&context(peer)?)?;
            json!({"owner":owner(),"protocol":VERSION,"peer":peer})
        }
        "execute" => serde_json::to_value(
            e.execute(&serde_json::from_value(v["program"].clone())?, &host())?,
        )?,
        "query" => serde_json::to_value(e.query(
            &serde_json::from_value(v["query"].clone())?,
            &if v["outsider"] == true {
                HostContext::new("independent-reviewer", Vec::<String>::new())
            } else {
                host()
            },
        )?)?,
        "head" => json!({"revision":e.head(field(v,"graph")?,field(v,"branch")?)?}),
        "counts" => json!({"events":e.event_count()?,"trusted_inspection":true}),
        "fork" => {
            e.fork_branch(
                &serde_json::from_value(v["reference"].clone())?,
                field(v, "branch")?,
                &host(),
            )?;
            json!({"forked":true})
        }
        "export" => {
            json!({"trusted_export":true,"capsule":e.export_capsule(&serde_json::from_value(v["reference"].clone())?,&host())?})
        }
        "propose" => {
            let capsule: Capsule = serde_json::from_value(v["capsule"].clone())?;
            let proof: AdmissionProof = if v.get("proof").is_some() {
                serde_json::from_value(v["proof"].clone())?
            } else {
                proof(peer, &capsule, field(v, "nonce")?)?
            };
            json!({"admitted":e.admit_proposal(&proof,&capsule)?,"proof":proof})
        }
        "integrate" => serde_json::to_value(e.integrate_proposal(
            &serde_json::from_value(v["request"].clone())?,
            &serde_json::from_value(v["proof"].clone())?,
            &host(),
        )?)?,
        "accept_revision" => {
            e.accept_revision(
                &serde_json::from_value(v["reference"].clone())?,
                field(v, "branch")?,
                v["expected"].as_str(),
                &host(),
            )?;
            json!({"accepted":true})
        }
        "install_adapter" => {
            let m = AdapterManifest {
                id: field(v, "id")?.into(),
                version: "1".into(),
                artifact_digest: format!("sha256:{}", "a".repeat(64)),
                config_revision: "1".into(),
                principal: owner(),
                subscriptions: serde_json::from_value(v["subscriptions"].clone())?,
                output_graphs: serde_json::from_value(v["outputs"].clone())?,
                effect_destinations: vec!["fake-maintenance".into()],
                max_attempts: 3,
                lease_ms: 1000,
                max_pending_events: 1000,
                projection_replay: false,
            };
            e.install_adapter(&m, &host())?;
            e.set_adapter_state(&m.id, "running")?;
            json!({"installed":true,"trusted_host_recipe":true})
        }
        "poll" => serde_json::to_value(e.poll_adapter(field(v, "id")?)?)?,
        "complete" => {
            let program: Program = serde_json::from_value(v["program"].clone())?;
            #[cfg(feature = "recovery-testing")]
            if v["kill_before_commit"] == true {
                e.complete_handler_test_before_commit(
                    field(v, "id")?,
                    field(v, "event")?,
                    field(v, "lease")?,
                    &program,
                    || std::process::exit(94),
                )?;
                return Err("kill hook not reached".into());
            }
            let receipt = e.complete_handler(
                field(v, "id")?,
                field(v, "event")?,
                field(v, "lease")?,
                &program,
            )?;
            if v["kill_after_commit"] == true {
                std::process::exit(92);
            }
            serde_json::to_value(receipt)?
        }
        "cluster" => serde_json::to_value(
            e.cluster_navigation(&serde_json::from_value(v["selection"].clone())?, &host())?,
        )?,
        "govern" => accept(e, v)?,
        "accepted" => serde_json::to_value(e.query_accepted_view(
            &AcceptedViewSelection {
                view_id: field(&v["selection"], "view_id")?.into(),
                decision_id: Some(field(&v["selection"], "decision_id")?.into()),
            },
            &if v["outsider"] == true {
                HostContext::new("independent-reviewer", Vec::<String>::new())
            } else {
                host()
            },
        )?)?,
        "effect_request" => serde_json::to_value(e.request_effect(
            field(v, "id")?,
            field(v, "event")?,
            field(v, "lease")?,
            "fake-maintenance",
            field(v, "key")?,
            v["payload"].clone(),
        )?)?,
        "effect_send_and_lose_response" => {
            let intent = e.begin_effect_dispatch(field(v, "intent")?)?;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(field(v, "destination_file")?)?;
            writeln!(
                file,
                "{}",
                json!({"intent":intent.id,"key":intent.idempotency_key,"payload":intent.payload})
            )?;
            file.sync_all()?;
            std::process::exit(93);
        }
        "effect_begin" => serde_json::to_value(e.begin_effect_dispatch(field(v, "intent")?)?)?,
        "effect_status" => serde_json::to_value(e.effect_intent(field(v, "intent")?)?)?,
        "effect_reconcile" => {
            e.reconcile_effect(field(v, "intent")?, "confirmed", v["evidence"].clone())?;
            json!({"reconciled":true})
        }
        _ => return Err("unknown trusted test operation".into()),
    })
}
fn main() {
    let result = (|| -> TestResult<Value> {
        let a: Vec<_> = std::env::args().collect();
        if a.len() != 4 {
            return Err("usage: three_peer_trace DATABASE P|W|T REQUEST.json".into());
        }
        root_key(&a[2])?;
        let mut bytes = Vec::new();
        std::fs::File::open(&a[3])?
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("fixture request budget".into());
        }
        let v: Value = serde_json::from_slice(&bytes)?;
        let mut e = Engine::open_with_clock(
            &a[1],
            Arc::new(ManualClock::new(
                v["fixture_clock_ms"].as_i64().unwrap_or(20),
            )),
        )?;
        run(&mut e, &a[2], &v)
    })();
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => {
            let code = error
                .downcast_ref::<weave_engine::Error>()
                .map_or("E_FIXTURE", |e| e.code.as_str());
            eprintln!("{}", json!({"code":code,"message":error.to_string()}));
            std::process::exit(1);
        }
    }
}
