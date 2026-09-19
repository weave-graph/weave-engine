//! Fixed-key test host for signed publication recovery, never a deployment server.
#[path = "../tests/support/clock.rs"]
mod test_clock;
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};
use std::{collections::BTreeSet, env, fs, io::Read};
use weave_contract::SnapshotCommit;
use weave_policy::*;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        return Err("usage: admission_probe DATABASE REQUEST.json".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(&args[2])?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err("input budget".into());
    }
    let request: Value = serde_json::from_slice(&bytes)?;
    let root = SigningKey::from_bytes(&[91; 32]);
    let user = SigningKey::from_bytes(&[92; 32]);
    let scope = Scope {
        graph_id: "recovery".into(),
        branch_id: "main".into(),
        actions: [Action::Publish].into(),
    };
    let mut e = test_clock::open(&args[1])?;
    let op = request["op"].as_str().ok_or("missing op")?;
    if op == "install" {
        e.install_admission_policy(&AdmissionContext {
            audience: "recovery-test".into(),
            now_ms: 10,
            policy_epoch: "epoch:1".into(),
            roots: vec![RootAuthority {
                issuer: public_key(&root),
                audience: "recovery-test".into(),
                policy_revision: "policy:1".into(),
                scopes: vec![scope],
                not_before_ms: 0,
                expires_at_ms: 1000,
                max_delegations: 0,
            }],
            revoked_capabilities: BTreeSet::new(),
            revoked_keys: BTreeSet::new(),
            consumed_nonces: BTreeSet::new(),
        })?;
        println!(
            "{}",
            json!({"installed": true, "principal": public_key(&user)})
        );
        return Ok(());
    }
    if op != "publish" {
        return Err("unknown operation".into());
    }
    let commit: SnapshotCommit = serde_json::from_value(request["commit"].clone())?;
    let cap = sign_capability(
        Capability {
            version: VERSION.into(),
            issuer: public_key(&root),
            subject: public_key(&user),
            audience: "recovery-test".into(),
            policy_revision: "policy:1".into(),
            scopes: vec![scope],
            not_before_ms: 0,
            expires_at_ms: 1000,
            delegations_remaining: 0,
            parent: None,
        },
        &root,
    )?;
    let signed = sign_request(
        Request {
            version: REQUEST_VERSION.into(),
            subject: public_key(&user),
            audience: "recovery-test".into(),
            capability_id: capability_id(&cap)?,
            nonce: request["nonce"].as_str().ok_or("nonce")?.into(),
            issued_at_ms: 0,
            expires_at_ms: 1000,
            operation: Operation {
                action: Action::Publish,
                graph_id: commit.graph_id.clone(),
                branch_id: commit.branch_id.clone(),
            },
            body_digest: body_digest(&serde_json::to_vec(&commit)?),
        },
        &user,
    )?;
    let proof = AdmissionProof {
        chain: vec![cap],
        request: signed,
    };
    #[cfg(feature = "recovery-testing")]
    if request["crash_before_commit"].as_bool() == Some(true) {
        test_clock::at(10, || {
            e.admit_publish_test_before_commit(&proof, &commit, || std::process::exit(80))
        })?;
        return Err("crash hook not reached".into());
    }
    match test_clock::at(10, || e.admit_publish(&proof, &commit)) {
        Ok(receipt) => {
            if request["crash_after_commit"].as_bool() == Some(true) {
                std::process::exit(81);
            }
            println!("{}", serde_json::to_string(&receipt)?);
        }
        Err(error) => {
            eprintln!("{}", json!({"error": error.to_string()}));
            std::process::exit(1);
        }
    }
    Ok(())
}
