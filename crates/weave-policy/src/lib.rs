//! Signed capability evidence for a trusted admission host. No graph or token can
//! install its own trust root. Verification alone does not consume a replay nonce.
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{self, Write};

pub const VERSION: &str = "weave-capability-0.1";
pub const REQUEST_VERSION: &str = "weave-request-0.1";
const MAX_BYTES: usize = 64 * 1024;
const MAX_CHAIN: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub &'static str);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Discover,
    Read,
    Traverse,
    Propose,
    Publish,
    Delegate,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub graph_id: String,
    pub branch_id: String,
    pub actions: BTreeSet<Action>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub version: String,
    pub issuer: String,
    pub subject: String,
    pub audience: String,
    pub policy_revision: String,
    pub scopes: Vec<Scope>,
    pub not_before_ms: i64,
    pub expires_at_ms: i64,
    pub delegations_remaining: u8,
    pub parent: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedCapability {
    pub capability: Capability,
    pub signature: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub action: Action,
    pub graph_id: String,
    pub branch_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: String,
    pub subject: String,
    pub audience: String,
    pub capability_id: String,
    pub nonce: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub operation: Operation,
    pub body_digest: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedRequest {
    pub request: Request,
    pub signature: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionProof {
    pub chain: Vec<SignedCapability>,
    pub request: SignedRequest,
}

/// Installed out of band by the host; deliberately not deserializable from a proof.
#[derive(Debug, Clone)]
pub struct RootAuthority {
    pub issuer: String,
    pub audience: String,
    pub policy_revision: String,
    pub scopes: Vec<Scope>,
    pub not_before_ms: i64,
    pub expires_at_ms: i64,
    pub max_delegations: u8,
}
/// All clocks, current policy, revocations and nonce history come from the host.
#[derive(Debug, Clone)]
pub struct AdmissionContext {
    pub audience: String,
    pub now_ms: i64,
    pub policy_epoch: String,
    pub roots: Vec<RootAuthority>,
    pub revoked_capabilities: BTreeSet<String>,
    pub revoked_keys: BTreeSet<String>,
    pub consumed_nonces: BTreeSet<String>,
}
/// A candidate admission, not permission to execute arbitrary plan commands.
/// The host must reserve replay_id atomically with its authorized mutation and
/// recheck policy_epoch at that same boundary. Fields cannot be forged by JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRequest {
    principal: String,
    operation: Operation,
    body_digest: String,
    audience: String,
    capability_id: String,
    replay_id: String,
    policy_epoch: String,
    issued_at_ms: i64,
    expires_at_ms: i64,
}
impl VerifiedRequest {
    pub fn principal(&self) -> &str {
        &self.principal
    }
    pub fn operation(&self) -> &Operation {
        &self.operation
    }
    pub fn body_digest(&self) -> &str {
        &self.body_digest
    }
    /// Run inside the host's atomic operation/nonce transaction. Every trust or
    /// revocation change must advance the host policy epoch before admission.
    pub fn check_boundary(
        &self,
        body: &[u8],
        operation: &Operation,
        ctx: &AdmissionContext,
    ) -> Result<()> {
        if self.body_digest != body_digest(body) || &self.operation != operation {
            return Err(Error("E_REQUEST_BINDING"));
        }
        if self.audience != ctx.audience {
            return Err(Error("E_AUDIENCE"));
        }
        if self.policy_epoch != ctx.policy_epoch {
            return Err(Error("E_POLICY_CHANGED"));
        }
        if ctx.now_ms < self.issued_at_ms || ctx.now_ms >= self.expires_at_ms {
            return Err(Error("E_EXPIRED"));
        }
        if ctx.consumed_nonces.contains(&self.replay_id) {
            return Err(Error("E_REPLAY"));
        }
        Ok(())
    }
    pub fn capability_id(&self) -> &str {
        &self.capability_id
    }
    pub fn replay_id(&self) -> &str {
        &self.replay_id
    }
    pub fn policy_epoch(&self) -> &str {
        &self.policy_epoch
    }
    pub fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms
    }
}
struct Counter(usize);
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| io::Error::other("proof budget"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn bounded(value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(Counter(MAX_BYTES), value).map_err(|_| Error("E_PROOF_BUDGET"))
}
fn bytes(domain: &str, value: &impl Serialize) -> Result<Vec<u8>> {
    bounded(value)?;
    serde_json::to_vec(&(domain, value)).map_err(|_| Error("E_ENCODING"))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn decode<const N: usize>(value: &str) -> Result<[u8; N]> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error("E_ENCODING"));
    }
    let mut result = [0; N];
    for (i, chunk) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let digit = |b: u8| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
        result[i] = digit(chunk[0]) * 16 + digit(chunk[1]);
    }
    Ok(result)
}
pub fn public_key(key: &SigningKey) -> String {
    format!("ed25519:{}", hex(&key.verifying_key().to_bytes()))
}
fn verifying_key(value: &str) -> Result<VerifyingKey> {
    let raw = decode::<32>(value.strip_prefix("ed25519:").ok_or(Error("E_KEY"))?)?;
    let key = VerifyingKey::from_bytes(&raw).map_err(|_| Error("E_KEY"))?;
    if key.is_weak() {
        return Err(Error("E_KEY"));
    }
    Ok(key)
}
fn verify_signature(
    key: &str,
    signature: &str,
    domain: &str,
    value: &impl Serialize,
) -> Result<()> {
    let signature = Signature::from_bytes(&decode::<64>(signature)?);
    verifying_key(key)?
        .verify_strict(&bytes(domain, value)?, &signature)
        .map_err(|_| Error("E_SIGNATURE"))
}
pub fn body_digest(body: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(body)))
}
pub fn capability_id(token: &SignedCapability) -> Result<String> {
    Ok(format!(
        "cap:{}",
        hex(&Sha256::digest(bytes("weave-capability-id-v0.1", token)?))
    ))
}
/// Signing does not establish authority; only a host-installed root can do that.
pub fn sign_capability(capability: Capability, key: &SigningKey) -> Result<SignedCapability> {
    if capability.issuer != public_key(key) {
        return Err(Error("E_KEY"));
    }
    let signature = hex(&key
        .sign(&bytes("weave-capability-signature-v0.1", &capability)?)
        .to_bytes());
    Ok(SignedCapability {
        capability,
        signature,
    })
}
pub fn sign_request(request: Request, key: &SigningKey) -> Result<SignedRequest> {
    if request.subject != public_key(key) {
        return Err(Error("E_KEY"));
    }
    let signature = hex(&key
        .sign(&bytes("weave-request-signature-v0.1", &request)?)
        .to_bytes());
    Ok(SignedRequest { request, signature })
}
fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
fn valid_scopes(scopes: &[Scope]) -> bool {
    !scopes.is_empty()
        && scopes.len() <= 64
        && scopes
            .iter()
            .all(|s| valid_id(&s.graph_id) && valid_id(&s.branch_id) && !s.actions.is_empty())
        && scopes
            .windows(2)
            .all(|s| (&s[0].graph_id, &s[0].branch_id) < (&s[1].graph_id, &s[1].branch_id))
}
fn attenuates(child: &[Scope], parent: &[Scope], delegating: bool) -> bool {
    child.iter().all(|c| {
        parent.iter().any(|p| {
            p.graph_id == c.graph_id
                && p.branch_id == c.branch_id
                && c.actions.is_subset(&p.actions)
                && (!delegating || p.actions.contains(&Action::Delegate))
        })
    })
}
fn active(start: i64, end: i64, now: i64) -> bool {
    start >= 0 && start < end && start <= now && now < end
}

/// Parse only bounded wire proofs, rejecting unknown fields and malformed JSON.
pub fn decode_proof(wire: &[u8]) -> Result<AdmissionProof> {
    if wire.len() > MAX_BYTES {
        return Err(Error("E_PROOF_BUDGET"));
    }
    serde_json::from_slice(wire).map_err(|_| Error("E_PROOF_FORMAT"))
}
/// expected_operation is chosen by the host after decoding the actual operation,
/// not copied blindly from the proof. Multi-operation plans need per-operation checks.
pub fn verify_request(
    proof: &AdmissionProof,
    body: &[u8],
    expected_operation: &Operation,
    ctx: &AdmissionContext,
) -> Result<VerifiedRequest> {
    bounded(proof)?;
    if proof.chain.is_empty() || proof.chain.len() > MAX_CHAIN {
        return Err(Error("E_CHAIN"));
    }
    if ctx.now_ms < 0 || !valid_id(&ctx.audience) || !valid_id(&ctx.policy_epoch) {
        return Err(Error("E_HOST_CONTEXT"));
    }
    let mut previous: Option<(&Capability, String)> = None;
    let mut ids = BTreeSet::new();
    for token in &proof.chain {
        let c = &token.capability;
        if c.version != VERSION
            || !valid_scopes(&c.scopes)
            || !valid_id(&c.policy_revision)
            || c.delegations_remaining >= MAX_CHAIN as u8
        {
            return Err(Error("E_CAPABILITY"));
        }
        if c.audience != ctx.audience {
            return Err(Error("E_AUDIENCE"));
        }
        if !active(c.not_before_ms, c.expires_at_ms, ctx.now_ms) {
            return Err(Error("E_EXPIRED"));
        }
        verifying_key(&c.subject)?;
        if ctx.revoked_keys.contains(&c.issuer) || ctx.revoked_keys.contains(&c.subject) {
            return Err(Error("E_REVOKED"));
        }
        verify_signature(
            &c.issuer,
            &token.signature,
            "weave-capability-signature-v0.1",
            c,
        )?;
        let id = capability_id(token)?;
        if !ids.insert(id.clone()) {
            return Err(Error("E_CHAIN"));
        }
        if ctx.revoked_capabilities.contains(&id) {
            return Err(Error("E_REVOKED"));
        }
        if let Some((parent, parent_id)) = &previous {
            if c.parent.as_ref() != Some(parent_id)
                || c.issuer != parent.subject
                || c.policy_revision != parent.policy_revision
                || c.not_before_ms < parent.not_before_ms
                || c.expires_at_ms > parent.expires_at_ms
                || c.delegations_remaining >= parent.delegations_remaining
                || !attenuates(&c.scopes, &parent.scopes, true)
            {
                return Err(Error("E_ATTENUATION"));
            }
        } else {
            if c.parent.is_some() {
                return Err(Error("E_CHAIN"));
            }
            let authorized = ctx.roots.iter().any(|r| {
                r.issuer == c.issuer
                    && r.audience == c.audience
                    && r.policy_revision == c.policy_revision
                    && valid_scopes(&r.scopes)
                    && active(r.not_before_ms, r.expires_at_ms, ctx.now_ms)
                    && c.not_before_ms >= r.not_before_ms
                    && c.expires_at_ms <= r.expires_at_ms
                    && c.delegations_remaining <= r.max_delegations
                    && attenuates(&c.scopes, &r.scopes, false)
            });
            if !authorized {
                return Err(Error("E_TRUST_ROOT"));
            }
        }
        previous = Some((c, id));
    }
    let (leaf, leaf_id) = previous.ok_or(Error("E_CHAIN"))?;
    let request = &proof.request.request;
    if request.version != REQUEST_VERSION
        || request.subject != leaf.subject
        || request.capability_id != leaf_id
    {
        return Err(Error("E_REQUEST"));
    }
    if request.audience != ctx.audience {
        return Err(Error("E_AUDIENCE"));
    }
    if !active(request.issued_at_ms, request.expires_at_ms, ctx.now_ms)
        || request.issued_at_ms < leaf.not_before_ms
        || request.expires_at_ms > leaf.expires_at_ms
        || request.expires_at_ms - request.issued_at_ms > 300_000
    {
        return Err(Error("E_EXPIRED"));
    }
    decode::<32>(&request.nonce)?;
    if &request.operation != expected_operation || request.body_digest != body_digest(body) {
        return Err(Error("E_REQUEST_BINDING"));
    }
    if !leaf.scopes.iter().any(|s| {
        s.graph_id == request.operation.graph_id
            && s.branch_id == request.operation.branch_id
            && s.actions.contains(&request.operation.action)
    }) {
        return Err(Error("E_SCOPE"));
    }
    verify_signature(
        &request.subject,
        &proof.request.signature,
        "weave-request-signature-v0.1",
        request,
    )?;
    // Nonce identity is independent of body, capability chain and action. Reusing
    // a nonce under the same subject/audience is rejected across those changes.
    let replay_id = format!(
        "nonce:{}",
        hex(&Sha256::digest(bytes(
            "weave-request-nonce-v0.1",
            &(&request.subject, &request.audience, &request.nonce)
        )?))
    );
    if ctx.consumed_nonces.contains(&replay_id) {
        return Err(Error("E_REPLAY"));
    }
    Ok(VerifiedRequest {
        principal: leaf.subject.clone(),
        operation: request.operation.clone(),
        body_digest: request.body_digest.clone(),
        audience: request.audience.clone(),
        capability_id: leaf_id,
        replay_id,
        policy_epoch: ctx.policy_epoch.clone(),
        issued_at_ms: request.issued_at_ms,
        expires_at_ms: request.expires_at_ms,
    })
}
