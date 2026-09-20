//! Explicit host approval of a currently authenticated isolated proposal.
use super::*;
use serde::{Deserialize, Serialize};
use weave_policy::{Action, AdmissionProof, Operation};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IntegrationRequest {
    pub proposal_id: String,
    pub branch_id: String,
    pub expected_head: Option<String>,
    pub nonce: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IntegrationReceipt {
    pub proposal_id: String,
    pub source_subject: String,
    pub reference: GraphRef,
    pub branch_id: String,
    pub event_id: Option<String>,
    pub duplicate: bool,
}
impl Engine {
    pub(crate) fn initialize_integration(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS integration_receipts(actor TEXT NOT NULL,nonce TEXT NOT NULL,body_hash TEXT NOT NULL,receipt TEXT NOT NULL,PRIMARY KEY(actor,nonce));")?;
        Ok(())
    }
    /// A current Propose signature authenticates bytes; independent host authority approves publication.
    pub fn integrate_proposal(
        &mut self,
        request: &IntegrationRequest,
        proof: &AdmissionProof,
        host: &HostContext,
    ) -> Result<IntegrationReceipt> {
        self.integrate_boundary(request, proof, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn integrate_proposal_test_before_commit(
        &mut self,
        request: &IntegrationRequest,
        proof: &AdmissionProof,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<IntegrationReceipt> {
        self.integrate_boundary(request, proof, host, before_commit)
    }
    fn integrate_boundary(
        &mut self,
        request: &IntegrationRequest,
        proof: &AdmissionProof,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<IntegrationReceipt> {
        let _scope = self.read_budget.enter();
        if [
            &request.proposal_id,
            &request.branch_id,
            &request.nonce,
            &host.principal,
        ]
        .iter()
        .any(|s| !valid_id(s))
            || request.expected_head.as_ref().is_some_and(|s| !valid_id(s))
        {
            return Err(err("E_ID", "invalid integration request"));
        }
        self.conn.execute_batch("SAVEPOINT proposal_integration")?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _clock_scope = self.operation_write_scope()?;
            self.read_budget.request()?;
            let limit = self.read_budget.remaining().min(16 * 1024 * 1024) as i64;
            let row:Option<(String,String,Option<String>)>=self.conn.query_row("SELECT substr(subject,1,513),substr(body_digest,1,129),CASE WHEN length(CAST(capsule AS BLOB))<=?2 THEN capsule END FROM isolated_proposals WHERE id=?1",params![request.proposal_id,limit],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            let (subject, body_digest, body) =
                row.ok_or_else(|| err("E_UNAVAILABLE", "proposal unavailable"))?;
            let body = body.ok_or_else(|| err("E_BUDGET", "proposal read exceeds budget"))?;
            self.read_budget.charge(body.len())?;
            if weave_policy::body_digest(body.as_bytes()) != body_digest {
                return Err(err("E_INTEGRITY", "isolated proposal digest differs"));
            }
            let capsule: Capsule = serde_json::from_str(&body)?;
            let root = capsule
                .revisions
                .iter()
                .find(|r| {
                    r.graph_id == capsule.root.graph_id && r.revision == capsule.root.revision
                })
                .ok_or_else(|| err("E_INTEGRITY", "proposal root missing"))?;
            let operation = Operation {
                action: Action::Propose,
                graph_id: root.graph_id.clone(),
                branch_id: root.branch_id.clone(),
            };
            let verified = self.verify_admission(proof, body.as_bytes(), &operation)?;
            if verified.principal() != subject {
                return Err(err("E_FORBIDDEN", "proposal subject differs"));
            }
            let scopes = &proof
                .chain
                .last()
                .ok_or_else(|| err("E_FORBIDDEN", "proposal chain missing"))?
                .capability
                .scopes;
            for record in &capsule.revisions {
                admission::require_scope(
                    scopes,
                    &record.graph_id,
                    &record.branch_id,
                    &[Action::Propose],
                )?;
                identity_acceptance::require_external_graph(&record.graph_id)?;
                identity_acceptance::require_external_schema(&record.data)?;
                if !host.writable_graphs.contains(&record.graph_id) {
                    return Err(err(
                        "E_FORBIDDEN",
                        "host must authorize every imported graph",
                    ));
                }
            }
            let hash = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&(request, &subject, &body_digest))?)
            );
            let previous:Option<(String,Option<String>)>=self.conn.query_row("SELECT substr(body_hash,1,65),CASE WHEN length(CAST(receipt AS BLOB))<=8192 THEN receipt END FROM integration_receipts WHERE actor=?1 AND nonce=?2",params![host.principal,request.nonce],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if let Some((old, receipt)) = previous {
                if old != hash {
                    return Err(err("E_REPLAY", "integration nonce binds another decision"));
                }
                for record in &capsule.revisions {
                    self.require_integration_visibility(record, host)?;
                }
                let mut receipt: IntegrationReceipt = serde_json::from_str(
                    &receipt
                        .ok_or_else(|| err("E_BUDGET", "integration receipt exceeds budget"))?,
                )?;
                receipt.duplicate = true;
                return Ok(receipt);
            }
            let count: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM integration_receipts WHERE actor=?1",
                [&host.principal],
                |r| r.get(0),
            )?;
            if count >= 10000 {
                return Err(err(
                    "E_BACKPRESSURE",
                    "integration receipt capacity reached",
                ));
            }
            if self.head(&capsule.root.graph_id, &request.branch_id)? != request.expected_head {
                return Err(err("E_CONFLICT", "integration expected head differs"));
            }
            self.receive_capsule(&capsule, host)?;
            for record in &capsule.revisions {
                self.require_integration_visibility(record, host)?;
            }
            let changed = request.expected_head.as_ref() != Some(&capsule.root.revision);
            self.accept_revision(
                &capsule.root,
                &request.branch_id,
                request.expected_head.as_deref(),
                host,
            )?;
            let event_id = if changed {
                Some(self.conn.query_row("SELECT event_id FROM events WHERE graph_id=?1 AND branch_id=?2 AND revision=?3 ORDER BY sequence DESC LIMIT 1",params![capsule.root.graph_id,request.branch_id,capsule.root.revision],|r|r.get(0))?)
            } else {
                None
            };
            let receipt = IntegrationReceipt {
                proposal_id: request.proposal_id.clone(),
                source_subject: subject,
                reference: capsule.root.clone(),
                branch_id: request.branch_id.clone(),
                event_id,
                duplicate: false,
            };
            self.conn.execute(
                "INSERT INTO integration_receipts VALUES (?1,?2,?3,?4)",
                params![
                    host.principal,
                    request.nonce,
                    hash,
                    serde_json::to_string(&receipt)?
                ],
            )?;
            before_commit();
            Ok(receipt)
        }));
        let result = operation_clock::rollback_unwind(
            outcome,
            &self.conn,
            "ROLLBACK TO proposal_integration; RELEASE proposal_integration",
        );
        match result {
            Ok(value) => {
                self.conn.execute_batch("RELEASE proposal_integration")?;
                Ok(value)
            }
            Err(error) => {
                self.conn.execute_batch(
                    "ROLLBACK TO proposal_integration; RELEASE proposal_integration",
                )?;
                Err(error)
            }
        }
    }
    fn require_integration_visibility(
        &self,
        record: &CapsuleRevision,
        host: &HostContext,
    ) -> Result<()> {
        if !self.identity_reference_allowed(&record.graph_id, &record.revision, host)? {
            return Err(err("E_UNAVAILABLE", "proposal content unavailable"));
        }
        let stored = self
            .load(&record.graph_id, &record.revision)?
            .ok_or_else(|| err("E_UNAVAILABLE", "integrated source unavailable"))?;
        if stored != record.data {
            return Err(err(
                "E_INTEGRITY",
                "integrated source differs from signed proposal",
            ));
        }
        let (visible, incomplete) = self.authorized(stored, host)?;
        if incomplete || visible != record.data {
            return Err(err("E_UNAVAILABLE", "proposal content unavailable"));
        }
        self.validate_required_metadata(&record.data, host)
    }
}
