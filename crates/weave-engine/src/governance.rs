const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS governance_policies(view_id TEXT NOT NULL,id TEXT NOT NULL,revision TEXT NOT NULL,body TEXT NOT NULL,PRIMARY KEY(view_id,id,revision));
CREATE TABLE IF NOT EXISTS governance_views(view_id TEXT PRIMARY KEY,policy_id TEXT NOT NULL,policy_revision TEXT NOT NULL,decision_id TEXT,source TEXT);
CREATE TABLE IF NOT EXISTS governance_proposals(id TEXT PRIMARY KEY,view_id TEXT NOT NULL,digest TEXT NOT NULL,body TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS governance_approvals(proposal_id TEXT NOT NULL,member TEXT NOT NULL,body TEXT NOT NULL,PRIMARY KEY(proposal_id,member));
CREATE TABLE IF NOT EXISTS governance_approval_nonces(member TEXT NOT NULL,nonce TEXT NOT NULL,digest TEXT NOT NULL,view_id TEXT NOT NULL,PRIMARY KEY(member,nonce));
CREATE TABLE IF NOT EXISTS governance_decisions(id TEXT PRIMARY KEY,view_id TEXT NOT NULL,proposal_id TEXT UNIQUE NOT NULL,policy_id TEXT NOT NULL,policy_revision TEXT NOT NULL,parent TEXT,accepted_at_ms INTEGER NOT NULL,body TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS governance_receipts(actor TEXT NOT NULL,nonce TEXT NOT NULL,digest TEXT NOT NULL,body TEXT NOT NULL,view_id TEXT NOT NULL,PRIMARY KEY(actor,nonce));
CREATE TABLE IF NOT EXISTS governance_events(sequence INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT UNIQUE NOT NULL,view_id TEXT NOT NULL,decision_id TEXT NOT NULL,event_type TEXT NOT NULL,recorded_at_ms INTEGER NOT NULL);
";
const QUOTA_BYTES: &str = "SELECT COALESCE(SUM(length(CAST(body AS BLOB))),0) FROM (
                SELECT body FROM governance_policies WHERE view_id=?1 UNION ALL
                SELECT body FROM governance_proposals WHERE view_id=?1 UNION ALL
                SELECT a.body FROM governance_approvals a JOIN governance_proposals p ON p.id=a.proposal_id WHERE p.view_id=?1 UNION ALL
                SELECT body FROM governance_decisions WHERE view_id=?1 UNION ALL
                SELECT body FROM governance_receipts WHERE view_id=?1)";
const QUOTA_RECORDS: &str = "SELECT (SELECT count(*) FROM governance_approval_nonces WHERE view_id=?1)+(SELECT count(*) FROM governance_events WHERE view_id=?1)";
const LOAD_HEAD: &str = "SELECT substr(policy_id,1,513),substr(policy_revision,1,513),substr(decision_id,1,513),CASE WHEN source IS NULL THEN NULL WHEN length(CAST(source AS BLOB))<=65536 THEN source ELSE '' END FROM governance_views WHERE view_id=?1";
const LOAD_POLICY: &str = "SELECT CASE WHEN length(CAST(body AS BLOB))<=65536 THEN body ELSE NULL END FROM governance_policies WHERE view_id=?1 AND id=?2 AND revision=?3";
const LOAD_PROPOSAL: &str = "SELECT CASE WHEN length(CAST(body AS BLOB))<=65536 THEN body ELSE NULL END,substr(digest,1,65) FROM governance_proposals WHERE id=?1";
const LOAD_RECEIPT: &str = "SELECT substr(digest,1,65),CASE WHEN length(CAST(body AS BLOB))<=65536 THEN body ELSE NULL END FROM governance_receipts WHERE actor=?1 AND nonce=?2";
const LOAD_APPROVALS: &str = "SELECT CASE WHEN length(CAST(body AS BLOB))<=65536 THEN body ELSE NULL END FROM governance_approvals WHERE proposal_id=?1 ORDER BY member LIMIT 33";
const ADVANCE_HEAD: &str = "UPDATE governance_views SET policy_id=?1,policy_revision=?2,decision_id=?3,source=?4 WHERE view_id=?5 AND decision_id IS ?6";
// Native policy-authorized acceptance. No accepted GraphData is exposed by this profile.
use super::*;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
const LIMIT: usize = 64 * 1024;
const DOMAIN: &str = "weave-governance-approval-v1";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernancePolicyRef {
    pub id: String,
    pub revision: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceSourceScope {
    pub graph_id: String,
    pub branch_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernancePolicy {
    pub view_id: String,
    pub reference: GovernancePolicyRef,
    pub members: Vec<String>,
    pub threshold: usize,
    pub proposers: Vec<String>,
    pub readers: Vec<String>,
    pub allowed_sources: Vec<GovernanceSourceScope>,
    pub not_before_ms: i64,
    pub expires_at_ms: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GovernanceAction {
    Publish { source: GraphRef, branch_id: String },
    ReplacePolicy { policy: GovernancePolicy },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceProposal {
    pub id: String,
    pub view_id: String,
    pub policy: GovernancePolicyRef,
    pub expected_head: Option<String>,
    pub expires_at_ms: i64,
    pub action: GovernanceAction,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceProposalReceipt {
    pub proposal_id: String,
    pub digest: String,
    pub duplicate: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceApproval {
    pub proposal_id: String,
    pub proposal_digest: String,
    pub view_id: String,
    pub policy: GovernancePolicyRef,
    pub expected_head: Option<String>,
    pub member: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub nonce: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedGovernanceApproval {
    pub approval: GovernanceApproval,
    pub signature: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceDecisionRequest {
    pub proposal_id: String,
    pub nonce: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceReceipt {
    pub view_id: String,
    pub decision_id: String,
    pub proposal_id: String,
    pub policy: GovernancePolicyRef,
    pub event_id: String,
    pub duplicate: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceHead {
    pub view_id: String,
    pub policy: GovernancePolicyRef,
    pub decision_id: Option<String>,
    pub source: Option<GraphRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredProposal {
    proposal: GovernanceProposal,
    proposer: String,
}
fn failure(code: &str) -> Error {
    err(code, "governance operation unavailable or invalid")
}
fn digest(value: &impl Serialize) -> Result<String> {
    json_size(value, LIMIT)?;
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn decode<const N: usize>(s: &str) -> Result<[u8; N]> {
    if s.len() != N * 2
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(failure("E_GOV_SIGNATURE"));
    }
    let mut out = [0; N];
    for (i, pair) in s.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        out[i] = u8::from_str_radix(
            std::str::from_utf8(pair).map_err(|_| failure("E_GOV_SIGNATURE"))?,
            16,
        )
        .map_err(|_| failure("E_GOV_SIGNATURE"))?;
    }
    Ok(out)
}
fn signature_bytes(approval: &GovernanceApproval) -> Result<Vec<u8>> {
    json_size(approval, LIMIT)?;
    Ok(serde_json::to_vec(&(DOMAIN, approval))?)
}
pub fn sign_governance_approval(
    approval: GovernanceApproval,
    key: &SigningKey,
) -> Result<SignedGovernanceApproval> {
    if approval.member != weave_policy::public_key(key) {
        return Err(failure("E_GOV_SIGNATURE"));
    }
    let signature = key
        .sign(&signature_bytes(&approval)?)
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(SignedGovernanceApproval {
        approval,
        signature,
    })
}
fn verify_signature(signed: &SignedGovernanceApproval) -> Result<()> {
    let key = VerifyingKey::from_bytes(&decode(
        signed
            .approval
            .member
            .strip_prefix("ed25519:")
            .ok_or_else(|| failure("E_GOV_SIGNATURE"))?,
    )?)
    .map_err(|_| failure("E_GOV_SIGNATURE"))?;
    let signature = Signature::from_bytes(&decode(&signed.signature)?);
    key.verify_strict(&signature_bytes(&signed.approval)?, &signature)
        .map_err(|_| failure("E_GOV_SIGNATURE"))
}
fn strings(values: &[String], nonempty: bool) -> bool {
    (!nonempty || !values.is_empty())
        && values.len() <= 32
        && values.iter().all(|v| valid_id(v))
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
fn validate_policy(p: &GovernancePolicy) -> Result<()> {
    if !valid_id(&p.view_id)
        || !valid_id(&p.reference.id)
        || !valid_id(&p.reference.revision)
        || !strings(&p.members, true)
        || !strings(&p.proposers, true)
        || !strings(&p.readers, false)
        || p.threshold == 0
        || p.threshold > p.members.len()
        || p.allowed_sources.is_empty()
        || p.allowed_sources.len() > 32
        || p.not_before_ms >= p.expires_at_ms
    {
        return Err(failure("E_GOV_POLICY"));
    }
    for member in &p.members {
        let key = VerifyingKey::from_bytes(&decode(
            member
                .strip_prefix("ed25519:")
                .ok_or_else(|| failure("E_GOV_SIGNATURE"))?,
        )?)
        .map_err(|_| failure("E_GOV_POLICY"))?;
        if key.is_weak() {
            return Err(failure("E_GOV_POLICY"));
        }
    }
    let mut scopes = BTreeSet::new();
    for scope in &p.allowed_sources {
        if !valid_id(&scope.graph_id)
            || !valid_id(&scope.branch_id)
            || !scopes.insert((&scope.graph_id, &scope.branch_id))
        {
            return Err(failure("E_GOV_POLICY"));
        }
    }
    json_size(p, LIMIT)?;
    Ok(())
}
impl Engine {
    pub(crate) fn initialize_governance(&self) -> Result<()> {
        self.conn.execute_batch(SCHEMA)?;
        Ok(())
    }
    fn gov_atomic<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.conn.execute_batch("SAVEPOINT governance")?;
        match f() {
            Ok(v) => {
                if let Err(error) = self.conn.execute_batch("RELEASE governance") {
                    self.conn
                        .execute_batch("ROLLBACK TO governance; RELEASE governance")?;
                    return Err(error.into());
                }
                Ok(v)
            }
            Err(e) => {
                self.conn.execute_batch(
                    "ROLLBACK TO governance;
 RELEASE governance",
                )?;
                Err(e)
            }
        }
    }
    fn gov_quota(&self, view: &str) -> Result<()> {
        let bytes: i64 = self.conn.query_row(QUOTA_BYTES, [view], |r| r.get(0))?;
        let records: i64 = self.conn.query_row(QUOTA_RECORDS, [view], |r| r.get(0))?;
        if bytes < 0
            || records < 0
            || bytes.saturating_add(records.saturating_mul(2048)) > 64 * 1024 * 1024
        {
            return Err(failure("E_BUDGET"));
        }
        Ok(())
    }
    fn gov_text<T: serde::de::DeserializeOwned>(&self, text: Option<String>) -> Result<T> {
        let text = text.ok_or_else(|| failure("E_GOV_UNAVAILABLE"))?;
        self.read_budget.charge(text.len())?;
        if text.len() > LIMIT {
            return Err(failure("E_BUDGET"));
        }
        Ok(serde_json::from_str(&text)?)
    }
    fn gov_head(&self, view: &str) -> Result<GovernanceHead> {
        let row: Option<(String, String, Option<String>, Option<String>)> = self
            .conn
            .query_row(LOAD_HEAD, [view], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        let (id, revision, decision_id, source) =
            row.ok_or_else(|| failure("E_GOV_UNAVAILABLE"))?;
        if !valid_id(view)
            || !valid_id(&id)
            || !valid_id(&revision)
            || decision_id.as_ref().is_some_and(|d| !valid_id(d))
        {
            return Err(failure("E_INTEGRITY"));
        }
        Ok(GovernanceHead {
            view_id: view.into(),
            policy: GovernancePolicyRef { id, revision },
            decision_id,
            source: source.map(|s| self.gov_text(Some(s))).transpose()?,
        })
    }
    fn gov_policy(
        &self,
        view: &str,
        reference: &GovernancePolicyRef,
        now: i64,
    ) -> Result<GovernancePolicy> {
        let body: Option<Option<String>> = self
            .conn
            .query_row(
                LOAD_POLICY,
                params![view, reference.id, reference.revision],
                |r| r.get(0),
            )
            .optional()?;
        let p: GovernancePolicy = self.gov_text(body.flatten())?;
        validate_policy(&p)?;
        if p.view_id != view
            || &p.reference != reference
            || now < p.not_before_ms
            || now >= p.expires_at_ms
        {
            return Err(failure("E_GOV_POLICY"));
        }
        Ok(p)
    }
    fn gov_proposal(&self, id: &str) -> Result<(StoredProposal, String)> {
        let row: Option<(Option<String>, String)> = self
            .conn
            .query_row(LOAD_PROPOSAL, [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let (body, expected) = row.ok_or_else(|| failure("E_GOV_UNAVAILABLE"))?;
        let record: StoredProposal = self.gov_text(body)?;
        if record.proposal.id != id
            || digest(&("weave-governance-proposal-v1", &record))? != expected
        {
            return Err(failure("E_INTEGRITY"));
        }
        Ok((record, expected))
    }
    fn gov_source(
        &self,
        source: &GraphRef,
        branch: &str,
        p: &GovernancePolicy,
        host: &HostContext,
    ) -> Result<()> {
        if !p
            .allowed_sources
            .iter()
            .any(|s| s.graph_id == source.graph_id && s.branch_id == branch)
            || !self.reachable_revision(&source.graph_id, branch, &source.revision)?
        {
            return Err(failure("E_GOV_SOURCE"));
        }
        if !self.identity_reference_allowed(&source.graph_id, &source.revision, host)? {
            return Err(failure("E_GOV_UNAVAILABLE"));
        }
        let raw = self
            .load(&source.graph_id, &source.revision)?
            .ok_or_else(|| failure("E_GOV_UNAVAILABLE"))?;
        let (data, incomplete) = self.authorized(raw.clone(), host)?;
        if incomplete || data != raw {
            return Err(failure("E_GOV_UNAVAILABLE"));
        }
        self.validate_required_metadata(&data, host)?;
        Ok(())
    }
    fn gov_existing_source(
        &self,
        source: &GraphRef,
        policy: &GovernancePolicy,
        host: &HostContext,
    ) -> Result<()> {
        for scope in &policy.allowed_sources {
            if scope.graph_id == source.graph_id
                && self
                    .gov_source(source, &scope.branch_id, policy, host)
                    .is_ok()
            {
                return Ok(());
            }
        }
        Err(failure("E_GOV_UNAVAILABLE"))
    }
    fn gov_check_proposal(
        &self,
        record: &StoredProposal,
        p: &GovernancePolicy,
        now: i64,
        host: &HostContext,
    ) -> Result<()> {
        let q = &record.proposal;
        if !valid_id(&host.principal)
            || !p.proposers.contains(&host.principal)
            || !p.proposers.contains(&record.proposer)
            || q.policy != p.reference
            || q.view_id != p.view_id
            || now >= q.expires_at_ms
            || q.expires_at_ms > p.expires_at_ms
        {
            return Err(failure("E_GOV_POLICY"));
        }
        match &q.action {
            GovernanceAction::Publish { source, branch_id } => {
                self.gov_source(source, branch_id, p, host)
            }
            GovernanceAction::ReplacePolicy { policy } => {
                // Changing authority must not bypass the predecessor's current source visibility.
                if let Some(source) = self.gov_head(&p.view_id)?.source {
                    self.gov_existing_source(&source, p, host)?;
                }
                validate_policy(policy)?;
                if policy.view_id != p.view_id
                    || policy.reference.id != p.reference.id
                    || policy.reference.revision == p.reference.revision
                    || policy.not_before_ms > now
                    || policy.expires_at_ms <= now
                {
                    return Err(failure("E_GOV_POLICY"));
                }
                Ok(())
            }
        }
    }
    /// Bootstrap only. Existing roots can change solely through preceding-policy approval.
    pub fn install_governance_root(&self, policy: &GovernancePolicy) -> Result<bool> {
        let _scope = self.read_budget.enter();
        validate_policy(policy)?;
        self.gov_atomic(|| {
            let exists: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM governance_views WHERE view_id=?1)",
                [&policy.view_id],
                |r| r.get(0),
            )?;
            if exists {
                let head = self.gov_head(&policy.view_id)?;
                let stored =
                    self.gov_policy(&policy.view_id, &head.policy, policy.not_before_ms)?;
                if head.decision_id.is_none() && stored == *policy {
                    return Ok(false);
                }
                return Err(failure("E_GOV_BOOTSTRAP"));
            }
            self.conn.execute(
                "INSERT INTO governance_policies VALUES (?1,?2,?3,?4)",
                params![
                    policy.view_id,
                    policy.reference.id,
                    policy.reference.revision,
                    serde_json::to_string(policy)?
                ],
            )?;
            self.conn.execute(
                "INSERT INTO governance_views VALUES (?1,?2,?3,NULL,NULL)",
                params![
                    policy.view_id,
                    policy.reference.id,
                    policy.reference.revision
                ],
            )?;
            self.gov_quota(&policy.view_id)?;
            Ok(true)
        })
    }
    pub fn propose_governance(
        &self,
        proposal: &GovernanceProposal,
        now: i64,
        host: &HostContext,
    ) -> Result<GovernanceProposalReceipt> {
        let _scope = self.read_budget.enter();
        json_size(proposal, LIMIT)?;
        if !valid_id(&host.principal)
            || !valid_id(&proposal.id)
            || !valid_id(&proposal.view_id)
            || proposal
                .expected_head
                .as_ref()
                .is_some_and(|h| !valid_id(h))
        {
            return Err(failure("E_GOV_PROPOSAL"));
        }
        self.gov_atomic(|| {
            let head = self.gov_head(&proposal.view_id)?;
            let policy = self.gov_policy(&proposal.view_id, &head.policy, now)?;
            let record = StoredProposal {
                proposal: proposal.clone(),
                proposer: host.principal.clone(),
            };
            self.gov_check_proposal(&record, &policy, now, host)?;
            let hash = digest(&("weave-governance-proposal-v1", &record))?;
            let prior: Option<String> = self
                .conn
                .query_row(
                    "SELECT substr(digest,1,65) FROM governance_proposals WHERE id=?1",
                    [&proposal.id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(prior) = prior {
                if prior != hash {
                    return Err(failure("E_GOV_REPLAY"));
                }
                return Ok(GovernanceProposalReceipt {
                    proposal_id: proposal.id.clone(),
                    digest: hash,
                    duplicate: true,
                });
            }
            if head.decision_id != proposal.expected_head {
                return Err(failure("E_CAS"));
            }
            let count: i64 = self.conn.query_row(
                "SELECT count(*) FROM governance_proposals WHERE view_id=?1",
                [&proposal.view_id],
                |r| r.get(0),
            )?;
            if count >= 1000 {
                return Err(failure("E_BUDGET"));
            }
            self.conn.execute(
                "INSERT INTO governance_proposals VALUES (?1,?2,?3,?4)",
                params![
                    proposal.id,
                    proposal.view_id,
                    hash,
                    serde_json::to_string(&record)?
                ],
            )?;
            self.gov_quota(&proposal.view_id)?;
            Ok(GovernanceProposalReceipt {
                proposal_id: proposal.id.clone(),
                digest: hash,
                duplicate: false,
            })
        })
    }
    fn gov_verify_approval(
        &self,
        signed: &SignedGovernanceApproval,
        record: &StoredProposal,
        hash: &str,
        p: &GovernancePolicy,
        now: i64,
    ) -> Result<()> {
        let a = &signed.approval;
        let q = &record.proposal;
        if a.proposal_id != q.id
            || a.proposal_digest != hash
            || a.view_id != q.view_id
            || a.policy != q.policy
            || a.expected_head != q.expected_head
            || !p.members.contains(&a.member)
            || !valid_id(&a.nonce)
            || a.issued_at_ms > now
            || a.issued_at_ms < p.not_before_ms
            || now >= a.expires_at_ms
            || a.expires_at_ms > q.expires_at_ms
            || a.issued_at_ms >= a.expires_at_ms
        {
            return Err(failure("E_GOV_APPROVAL"));
        }
        verify_signature(signed)
    }
    pub fn record_governance_approval(
        &self,
        signed: &SignedGovernanceApproval,
        now: i64,
        host: &HostContext,
    ) -> Result<bool> {
        let _scope = self.read_budget.enter();
        json_size(signed, LIMIT)?;
        if !valid_id(&host.principal) {
            return Err(failure("E_GOV_POLICY"));
        }
        self.gov_atomic(|| {
            let (record, hash) = self.gov_proposal(&signed.approval.proposal_id)?;
            let head = self.gov_head(&record.proposal.view_id)?;
            let policy = self.gov_policy(&head.view_id, &head.policy, now)?;
            self.gov_check_proposal(&record, &policy, now, host)?;
            self.gov_verify_approval(signed, &record, &hash, &policy, now)?;
            let encoded = serde_json::to_string(signed)?;
            let approval_hash = digest(signed)?;
            let prior: Option<String> = self.conn.query_row(
                "SELECT substr(digest,1,65) FROM governance_approval_nonces WHERE member=?1 AND nonce=?2",
                params![signed.approval.member, signed.approval.nonce],
                |row| row.get(0),
            ).optional()?;
            if let Some(prior) = prior {
                if prior == approval_hash {
                    return Ok(false);
                }
                return Err(failure("E_GOV_REPLAY"));
            }
            if head.decision_id != record.proposal.expected_head {
                return Err(failure("E_CAS"));
            }
            let occupied: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM governance_approvals WHERE proposal_id=?1 AND member=?2)",
                params![record.proposal.id, signed.approval.member],
                |row| row.get(0),
            )?;
            if occupied {
                return Err(failure("E_GOV_APPROVAL"));
            }
            self.conn.execute(
                "INSERT INTO governance_approvals VALUES (?1,?2,?3)",
                params![record.proposal.id, signed.approval.member, encoded],
            )?;
            self.conn.execute(
                "INSERT INTO governance_approval_nonces VALUES (?1,?2,?3,?4)",
                params![signed.approval.member, signed.approval.nonce, approval_hash, signed.approval.view_id],
            )?;
            self.gov_quota(&signed.approval.view_id)?;
            Ok(true)
        })
    }

    pub fn accept_governance(
        &self,
        request: &GovernanceDecisionRequest,
        now: i64,
        host: &HostContext,
    ) -> Result<GovernanceReceipt> {
        self.accept_governance_observed(request, now, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn accept_governance_test_before_commit(
        &self,
        request: &GovernanceDecisionRequest,
        now: i64,
        host: &HostContext,
        hook: impl FnOnce(),
    ) -> Result<GovernanceReceipt> {
        self.accept_governance_observed(request, now, host, hook)
    }
    fn accept_governance_observed(
        &self,
        request: &GovernanceDecisionRequest,
        now: i64,
        host: &HostContext,
        hook: impl FnOnce(),
    ) -> Result<GovernanceReceipt> {
        let _scope = self.read_budget.enter();
        if !valid_id(&host.principal)
            || !valid_id(&request.nonce)
            || !valid_id(&request.proposal_id)
        {
            return Err(failure("E_GOV_PROPOSAL"));
        }
        self.gov_atomic(|| {
            let (record, proposal_hash) = self.gov_proposal(&request.proposal_id)?;
            let proposal = &record.proposal;
            let head = self.gov_head(&proposal.view_id)?;
            let policy = self.gov_policy(&proposal.view_id, &head.policy, now)?;
            self.gov_check_proposal(&record, &policy, now, host)?;
            let request_hash = digest(&(request, &proposal_hash))?;
            let prior: Option<(String, Option<String>)> = self.conn.query_row(
                LOAD_RECEIPT,
                params![host.principal, request.nonce],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional()?;
            // Response-loss retry must meet current policy, time and source authority.
            let mut statement = self.conn.prepare(LOAD_APPROVALS)?;
            let rows = statement.query_map([&proposal.id], |row| row.get::<_, Option<String>>(0))?;
            let mut members = BTreeSet::new();
            for row in rows {
                let signed: SignedGovernanceApproval = self.gov_text(row?)?;
                if signed.approval.expires_at_ms <= now {
                    verify_signature(&signed)?;
                    continue;
                }
                self.gov_verify_approval(&signed, &record, &proposal_hash, &policy, now)?;
                if !members.insert(signed.approval.member) || members.len() > 32 {
                    return Err(failure("E_GOV_APPROVAL"));
                }
            }
            if members.len() < policy.threshold {
                return Err(failure("E_GOV_QUORUM"));
            }
            if let Some((hash, body)) = prior {
                if hash != request_hash {
                    return Err(failure("E_GOV_REPLAY"));
                }
                let mut receipt: GovernanceReceipt = self.gov_text(body)?;
                receipt.duplicate = true;
                return Ok(receipt);
            }
            if head.decision_id != proposal.expected_head {
                return Err(failure("E_CAS"));
            }
            let count: i64 = self.conn.query_row(
                "SELECT count(*) FROM governance_receipts WHERE actor=?1",
                [&host.principal], |row| row.get(0),
            )?;
            if count >= 10000 {
                return Err(failure("E_BUDGET"));
            }
            let decision = digest(&(
                "weave-governance-decision-v1", &proposal.view_id,
                &proposal_hash, &proposal.expected_head,
            ))?;
            let event_id = digest(&("weave-governance-event-v1", &decision))?;
            let (next_policy, source, event_type) = match &proposal.action {
                GovernanceAction::Publish { source, .. } => (
                    policy.reference.clone(), Some(source.clone()), "view.accepted",
                ),
                GovernanceAction::ReplacePolicy { policy: next } => {
                    let exists: bool = self.conn.query_row(
                        "SELECT EXISTS(SELECT 1 FROM governance_policies WHERE view_id=?1 AND id=?2 AND revision=?3)",
                        params![next.view_id, next.reference.id, next.reference.revision],
                        |row| row.get(0),
                    )?;
                    if exists {
                        return Err(failure("E_GOV_POLICY"));
                    }
                    self.conn.execute(
                        "INSERT INTO governance_policies VALUES (?1,?2,?3,?4)",
                        params![next.view_id, next.reference.id, next.reference.revision, serde_json::to_string(next)?],
                    )?;
                    (next.reference.clone(), head.source.clone(), "policy.changed")
                }
            };
            let receipt = GovernanceReceipt {
                view_id: proposal.view_id.clone(), decision_id: decision.clone(),
                proposal_id: proposal.id.clone(), policy: next_policy.clone(),
                event_id: event_id.clone(), duplicate: false,
            };
            self.conn.execute(
                "INSERT INTO governance_decisions VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![decision, proposal.view_id, proposal.id, proposal.policy.id,
                    proposal.policy.revision, proposal.expected_head, now, serde_json::to_string(&receipt)?],
            )?;
            let changed = self.conn.execute(
                ADVANCE_HEAD,
                params![next_policy.id, next_policy.revision, decision,
                    source.as_ref().map(serde_json::to_string).transpose()?, proposal.view_id, proposal.expected_head],
            )?;
            if changed != 1 {
                return Err(failure("E_CAS"));
            }
            self.conn.execute(
                "INSERT INTO governance_receipts VALUES (?1,?2,?3,?4,?5)",
                params![host.principal, request.nonce, request_hash, serde_json::to_string(&receipt)?, proposal.view_id],
            )?;
            self.conn.execute(
                "INSERT INTO governance_events(id,view_id,decision_id,event_type,recorded_at_ms) VALUES (?1,?2,?3,?4,?5)",
                params![event_id, proposal.view_id, decision, event_type, now],
            )?;
            self.gov_quota(&proposal.view_id)?;
            hook();
            Ok(receipt)
        })
    }

    /// Trusted embedding inspection only;
    /// This is not a reusable accepted graph result.
    pub fn inspect_governance_head(
        &self,
        view: &str,
        now: i64,
        host: &HostContext,
    ) -> Result<GovernanceHead> {
        let _scope = self.read_budget.enter();
        if !valid_id(&host.principal) || !valid_id(view) {
            return Err(failure("E_GOV_UNAVAILABLE"));
        }
        let read_transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let head = self.gov_head(view)?;
        let policy = self.gov_policy(view, &head.policy, now)?;
        if !policy.readers.is_empty()
            && !policy.readers.contains(&host.principal)
            && !policy.proposers.contains(&host.principal)
        {
            return Err(failure("E_GOV_UNAVAILABLE"));
        }
        if let Some(source) = &head.source {
            self.gov_existing_source(source, &policy, host)?;
        }
        if let Some(transaction) = read_transaction {
            transaction.commit()?;
        }
        Ok(head)
    }
    /// Trusted host diagnostic;
    /// Unscoped counts are not an externally authorized stream.
    pub fn governance_event_count(&self) -> Result<u64> {
        let count: i64 =
            self.conn
                .query_row("SELECT count(*) FROM governance_events", [], |r| r.get(0))?;
        u64::try_from(count).map_err(|_| failure("E_INTEGRITY"))
    }
}
