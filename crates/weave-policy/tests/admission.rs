use ed25519_dalek::SigningKey;
use std::collections::BTreeSet;
use weave_policy::*;
fn scope(actions: &[Action]) -> Scope {
    Scope {
        graph_id: "team".into(),
        branch_id: "main".into(),
        actions: actions.iter().copied().collect(),
    }
}
fn fixture() -> (AdmissionProof, AdmissionContext, Operation, [SigningKey; 3]) {
    // Fixed public test vectors only; never used as deployment identities.
    let keys = [
        SigningKey::from_bytes(&[11; 32]),
        SigningKey::from_bytes(&[22; 32]),
        SigningKey::from_bytes(&[33; 32]),
    ];
    let root = RootAuthority {
        issuer: public_key(&keys[0]),
        audience: "replica:workstation".into(),
        policy_revision: "policy:7".into(),
        scopes: vec![scope(&[Action::Read, Action::Publish, Action::Delegate])],
        not_before_ms: 0,
        expires_at_ms: 10000,
        max_delegations: 2,
    };
    let a = sign_capability(
        Capability {
            version: VERSION.into(),
            issuer: root.issuer.clone(),
            subject: public_key(&keys[1]),
            audience: root.audience.clone(),
            policy_revision: root.policy_revision.clone(),
            scopes: root.scopes.clone(),
            not_before_ms: 10,
            expires_at_ms: 9000,
            delegations_remaining: 1,
            parent: None,
        },
        &keys[0],
    )
    .unwrap();
    let b = sign_capability(
        Capability {
            version: VERSION.into(),
            issuer: public_key(&keys[1]),
            subject: public_key(&keys[2]),
            audience: root.audience.clone(),
            policy_revision: root.policy_revision.clone(),
            scopes: vec![scope(&[Action::Read])],
            not_before_ms: 20,
            expires_at_ms: 8000,
            delegations_remaining: 0,
            parent: Some(capability_id(&a).unwrap()),
        },
        &keys[1],
    )
    .unwrap();
    let operation = Operation {
        action: Action::Read,
        graph_id: "team".into(),
        branch_id: "main".into(),
    };
    let request = sign_request(
        Request {
            version: REQUEST_VERSION.into(),
            subject: public_key(&keys[2]),
            audience: root.audience.clone(),
            capability_id: capability_id(&b).unwrap(),
            nonce: "ab".repeat(32),
            issued_at_ms: 100,
            expires_at_ms: 1000,
            operation: operation.clone(),
            body_digest: body_digest(b"read pinned revision"),
        },
        &keys[2],
    )
    .unwrap();
    let ctx = AdmissionContext {
        audience: root.audience.clone(),
        now_ms: 200,
        policy_epoch: "admission:9".into(),
        roots: vec![root],
        revoked_capabilities: BTreeSet::new(),
        revoked_keys: BTreeSet::new(),
        consumed_nonces: BTreeSet::new(),
    };
    (
        AdmissionProof {
            chain: vec![a, b],
            request,
        },
        ctx,
        operation,
        keys,
    )
}
fn verify(
    p: &AdmissionProof,
    c: &AdmissionContext,
    o: &Operation,
) -> Result<VerifiedRequest, Error> {
    verify_request(p, b"read pinned revision", o, c)
}
fn resign_leaf(p: &mut AdmissionProof, keys: &[SigningKey; 3]) {
    p.chain[1] = sign_capability(p.chain[1].capability.clone(), &keys[1]).unwrap();
    p.request.request.capability_id = capability_id(&p.chain[1]).unwrap();
    p.request = sign_request(p.request.request.clone(), &keys[2]).unwrap();
}
#[test]
fn delegated_request_has_exact_scope_and_replay_identity() {
    let (p, mut ctx, op, keys) = fixture();
    let v = verify(&p, &ctx, &op).unwrap();
    assert_eq!(v.principal(), public_key(&keys[2]));
    assert_eq!(v.operation(), &op);
    assert_eq!(v.policy_epoch(), "admission:9");
    ctx.consumed_nonces.insert(v.replay_id().into());
    assert_eq!(verify(&p, &ctx, &op).unwrap_err().0, "E_REPLAY");
    // Proof of possession is bound to the body and host-decoded operation.
    assert_eq!(
        verify_request(&p, b"commit unauthorized", &op, &fixture().1)
            .unwrap_err()
            .0,
        "E_REQUEST_BINDING"
    );
    let mut wrong = op.clone();
    wrong.action = Action::Publish;
    assert_eq!(
        verify(&p, &fixture().1, &wrong).unwrap_err().0,
        "E_REQUEST_BINDING"
    );
}
#[test]
fn self_signed_authority_and_signature_tampering_fail() {
    let (mut p, ctx, op, keys) = fixture();
    p.chain[0].capability.issuer = public_key(&keys[1]);
    p.chain[0] = sign_capability(p.chain[0].capability.clone(), &keys[1]).unwrap();
    assert_eq!(verify(&p, &ctx, &op).unwrap_err().0, "E_TRUST_ROOT");
    let (mut p, ctx, op, _) = fixture();
    p.chain[1].capability.expires_at_ms = 7000;
    assert_eq!(verify(&p, &ctx, &op).unwrap_err().0, "E_SIGNATURE");
    let (mut p, ctx, op, _) = fixture();
    p.request.request.nonce = "cd".repeat(32);
    assert_eq!(verify(&p, &ctx, &op).unwrap_err().0, "E_SIGNATURE");
}
#[test]
fn delegation_cannot_expand_scope_lifetime_depth_or_change_parent() {
    for mutation in 0..5 {
        let (mut p, ctx, op, keys) = fixture();
        let child = &mut p.chain[1].capability;
        match mutation {
            0 => {
                child.scopes[0].actions.insert(Action::Traverse);
            }
            1 => child.scopes[0].graph_id = "other".into(),
            2 => child.expires_at_ms = 9500,
            3 => child.delegations_remaining = 1,
            _ => child.parent = Some("cap:wrong".into()),
        }
        resign_leaf(&mut p, &keys);
        assert_eq!(verify(&p, &ctx, &op).unwrap_err().0, "E_ATTENUATION");
    }
    let (mut p, ctx, op, keys) = fixture();
    p.chain[0].capability.scopes[0]
        .actions
        .remove(&Action::Delegate);
    p.chain[0] = sign_capability(p.chain[0].capability.clone(), &keys[0]).unwrap();
    p.chain[1].capability.parent = Some(capability_id(&p.chain[0]).unwrap());
    resign_leaf(&mut p, &keys);
    assert_eq!(verify(&p, &ctx, &op).unwrap_err().0, "E_ATTENUATION");
}
#[test]
fn revocation_current_policy_audience_and_half_open_expiry_are_enforced() {
    let (p, ctx, op, _) = fixture();
    for token in &p.chain {
        let mut c = ctx.clone();
        c.revoked_capabilities.insert(capability_id(token).unwrap());
        assert_eq!(verify(&p, &c, &op).unwrap_err().0, "E_REVOKED");
    }
    let mut c = ctx.clone();
    c.revoked_keys.insert(p.chain[0].capability.subject.clone());
    assert_eq!(verify(&p, &c, &op).unwrap_err().0, "E_REVOKED");
    let mut c = ctx.clone();
    c.roots[0].policy_revision = "policy:8".into();
    assert_eq!(verify(&p, &c, &op).unwrap_err().0, "E_TRUST_ROOT");
    let mut c = ctx.clone();
    c.audience = "replica:other".into();
    assert_eq!(verify(&p, &c, &op).unwrap_err().0, "E_AUDIENCE");
    let mut c = ctx;
    c.now_ms = 999;
    assert!(verify(&p, &c, &op).is_ok());
    c.now_ms = 1000;
    assert_eq!(verify(&p, &c, &op).unwrap_err().0, "E_EXPIRED");
    c.now_ms = 99;
    assert_eq!(verify(&p, &c, &op).unwrap_err().0, "E_EXPIRED");
}
#[test]
fn wire_limits_unknown_fields_and_weak_keys_fail_closed() {
    let (p, ctx, op, _) = fixture();
    let mut encoded = serde_json::to_value(&p).unwrap();
    encoded["authority"] = serde_json::json!({"root":true});
    assert_eq!(
        decode_proof(&serde_json::to_vec(&encoded).unwrap())
            .unwrap_err()
            .0,
        "E_PROOF_FORMAT"
    );
    assert_eq!(
        decode_proof(&vec![b' '; 65537]).unwrap_err().0,
        "E_PROOF_BUDGET"
    );
    let mut p = p;
    p.chain = vec![p.chain[0].clone(); 9];
    assert_eq!(verify(&p, &ctx, &op).unwrap_err().0, "E_CHAIN");
    let (mut p, ctx, op, _) = fixture();
    p.chain[0].capability.issuer = format!("ed25519:01{}", "00".repeat(31));
    assert_eq!(verify(&p, &ctx, &op).unwrap_err().0, "E_KEY");
}
#[test]
fn canonical_typed_encoding_round_trips_without_installing_trust() {
    let (p, ctx, op, _) = fixture();
    let compact = serde_json::to_vec(&p).unwrap();
    let pretty = serde_json::to_vec_pretty(&p).unwrap();
    assert_eq!(
        verify(&decode_proof(&compact).unwrap(), &ctx, &op).unwrap(),
        verify(&decode_proof(&pretty).unwrap(), &ctx, &op).unwrap()
    );
    let mut untrusted = ctx;
    untrusted.roots.clear();
    assert_eq!(verify(&p, &untrusted, &op).unwrap_err().0, "E_TRUST_ROOT");
}

#[test]
fn candidate_is_bound_again_at_atomic_execution_time() {
    let (p, ctx, op, _) = fixture();
    let verified = verify(&p, &ctx, &op).unwrap();
    assert_eq!(verified.body_digest(), body_digest(b"read pinned revision"));
    assert!(verified
        .check_boundary(b"read pinned revision", &op, &ctx)
        .is_ok());
    assert_eq!(
        verified
            .check_boundary(b"different read request", &op, &ctx)
            .unwrap_err()
            .0,
        "E_REQUEST_BINDING"
    );
    let mut later = ctx.clone();
    later.now_ms = verified.expires_at_ms();
    assert_eq!(
        verified
            .check_boundary(b"read pinned revision", &op, &later)
            .unwrap_err()
            .0,
        "E_EXPIRED"
    );
    later = ctx.clone();
    later.policy_epoch = "revoked-policy-epoch".into();
    assert_eq!(
        verified
            .check_boundary(b"read pinned revision", &op, &later)
            .unwrap_err()
            .0,
        "E_POLICY_CHANGED"
    );
    later = ctx.clone();
    later.audience = "another-host".into();
    assert_eq!(
        verified
            .check_boundary(b"read pinned revision", &op, &later)
            .unwrap_err()
            .0,
        "E_AUDIENCE"
    );
    later = ctx;
    later.consumed_nonces.insert(verified.replay_id().into());
    assert_eq!(
        verified
            .check_boundary(b"read pinned revision", &op, &later)
            .unwrap_err()
            .0,
        "E_REPLAY"
    );
}
