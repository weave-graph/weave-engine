//! Signed per-operation admission, separate from trusted local administration.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use weave_policy::{
    Action, AdmissionContext, AdmissionProof, Operation, RootAuthority, Scope, VerifiedRequest,
};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Admitted<T> {
    pub duplicate: bool,
    pub result: T,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProposalReceipt {
    pub id: String,
    pub root: GraphRef,
    pub revision_count: usize,
}
#[derive(Serialize, Deserialize)]
struct ReadReceipt {
    result: QueryResult,
    dependencies: Vec<GraphRef>,
}
#[derive(Serialize, Deserialize)]
struct StoredRoot {
    issuer: String,
    audience: String,
    policy_revision: String,
    scopes: Vec<Scope>,
    not_before_ms: i64,
    expires_at_ms: i64,
    max_delegations: u8,
}
#[derive(Serialize, Deserialize)]
struct StoredPolicy {
    audience: String,
    epoch: String,
    roots: Vec<StoredRoot>,
    revoked_capabilities: BTreeSet<String>,
    revoked_keys: BTreeSet<String>,
    consumed_nonces: BTreeSet<String>,
}
fn policy_error(error: weave_policy::Error) -> Error {
    err(error.0, "signed admission rejected")
}
impl Engine {
    pub(crate) fn initialize_admission(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS admission_policy(id INTEGER PRIMARY KEY CHECK(id=1),epoch TEXT NOT NULL,policy TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS admission_epochs(epoch TEXT PRIMARY KEY,policy TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS admission_receipts(nonce TEXT PRIMARY KEY,subject TEXT NOT NULL,epoch TEXT NOT NULL,body_digest TEXT NOT NULL,operation TEXT NOT NULL,response TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS isolated_proposals(id TEXT PRIMARY KEY,subject TEXT NOT NULL,body_digest TEXT NOT NULL,capsule TEXT NOT NULL);")?;
        Ok(())
    }
    /// Out-of-band trusted administrator only. No signed request or graph can install a root.
    pub fn install_admission_policy(&self, ctx: &AdmissionContext) -> Result<()> {
        if !valid_id(&ctx.audience) || !valid_id(&ctx.policy_epoch) {
            return Err(err("E_POLICY", "invalid installed policy identity"));
        }
        let stored = StoredPolicy {
            audience: ctx.audience.clone(),
            epoch: ctx.policy_epoch.clone(),
            roots: ctx
                .roots
                .iter()
                .map(|r| StoredRoot {
                    issuer: r.issuer.clone(),
                    audience: r.audience.clone(),
                    policy_revision: r.policy_revision.clone(),
                    scopes: r.scopes.clone(),
                    not_before_ms: r.not_before_ms,
                    expires_at_ms: r.expires_at_ms,
                    max_delegations: r.max_delegations,
                })
                .collect(),
            revoked_capabilities: ctx.revoked_capabilities.clone(),
            revoked_keys: ctx.revoked_keys.clone(),
            consumed_nonces: ctx.consumed_nonces.clone(),
        };
        json_size(&stored, 1024 * 1024)?;
        let encoded = serde_json::to_string(&stored)?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let old: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT epoch,policy FROM admission_policy WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let prior_epoch: Option<String> = self
            .conn
            .query_row(
                "SELECT policy FROM admission_epochs WHERE epoch=?1",
                [&ctx.policy_epoch],
                |r| r.get(0),
            )
            .optional()?;
        if prior_epoch.is_some()
            && !old
                .as_ref()
                .is_some_and(|(epoch, policy)| epoch == &ctx.policy_epoch && policy == &encoded)
        {
            return Err(err(
                "E_POLICY_EPOCH",
                "policy epochs cannot be changed or reactivated",
            ));
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO admission_epochs VALUES (?1,?2)",
            params![ctx.policy_epoch, encoded],
        )?;
        self.conn.execute("INSERT INTO admission_policy VALUES (1,?1,?2) ON CONFLICT(id) DO UPDATE SET epoch=excluded.epoch,policy=excluded.policy",params![ctx.policy_epoch,encoded])?;
        tx.commit()?;
        Ok(())
    }
    fn admission_context(&self, now_ms: i64) -> Result<AdmissionContext> {
        let policy: Option<String> = self
            .conn
            .query_row("SELECT policy FROM admission_policy WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        let stored: StoredPolicy = serde_json::from_str(
            &policy.ok_or_else(|| err("E_POLICY", "no admission policy installed"))?,
        )?;
        Ok(AdmissionContext {
            audience: stored.audience,
            now_ms,
            policy_epoch: stored.epoch,
            roots: stored
                .roots
                .into_iter()
                .map(|r| RootAuthority {
                    issuer: r.issuer,
                    audience: r.audience,
                    policy_revision: r.policy_revision,
                    scopes: r.scopes,
                    not_before_ms: r.not_before_ms,
                    expires_at_ms: r.expires_at_ms,
                    max_delegations: r.max_delegations,
                })
                .collect(),
            revoked_capabilities: stored.revoked_capabilities,
            revoked_keys: stored.revoked_keys,
            consumed_nonces: stored.consumed_nonces,
        })
    }
    fn verify_admission(
        &self,
        proof: &AdmissionProof,
        body: &[u8],
        operation: &Operation,
        now_ms: i64,
    ) -> Result<VerifiedRequest> {
        let context = self.admission_context(now_ms)?;
        let verified =
            weave_policy::verify_request(proof, body, operation, &context).map_err(policy_error)?;
        verified
            .check_boundary(body, operation, &context)
            .map_err(policy_error)?;
        Ok(verified)
    }
    fn prior_admission<T: serde::de::DeserializeOwned>(
        &self,
        verified: &VerifiedRequest,
    ) -> Result<Option<T>> {
        let row: Option<(String, String, String)> = self
            .conn
            .query_row(
                "SELECT epoch,body_digest,response FROM admission_receipts WHERE nonce=?1",
                [verified.replay_id()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((epoch, body, response)) = row {
            if epoch != verified.policy_epoch() {
                return Err(err("E_POLICY_CHANGED", "retry crosses an admission epoch"));
            }
            if body != verified.body_digest() {
                return Err(err("E_REPLAY", "nonce already binds another request body"));
            }
            let operation: String = self.conn.query_row(
                "SELECT operation FROM admission_receipts WHERE nonce=?1",
                [verified.replay_id()],
                |r| r.get(0),
            )?;
            if operation != serde_json::to_string(verified.operation())? {
                return Err(err("E_REPLAY", "nonce already binds another operation"));
            }
            return Ok(Some(serde_json::from_str(&response)?));
        }
        Ok(None)
    }
    fn record_admission(
        &self,
        verified: &VerifiedRequest,
        response: &impl Serialize,
    ) -> Result<()> {
        json_size(response, MATERIALIZED_LIMIT)?;
        let encoded = serde_json::to_string(response)?;
        let (count, used): (i64,i64) = self.conn.query_row("SELECT COUNT(*), COALESCE(SUM(length(CAST(response AS BLOB))),0) FROM admission_receipts WHERE subject=?1", [verified.principal()], |r|Ok((r.get(0)?,r.get(1)?)))?;
        if count >= 10000 || used as usize + encoded.len() > 64 * 1024 * 1024 {
            return Err(err("E_BACKPRESSURE", "admission receipt quota exceeded"));
        }
        self.conn.execute(
            "INSERT INTO admission_receipts VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                verified.replay_id(),
                verified.principal(),
                verified.policy_epoch(),
                verified.body_digest(),
                serde_json::to_string(verified.operation())?,
                encoded
            ],
        )?;
        Ok(())
    }
    /// The canonical typed query JSON is the signed body. Retry returns the original pinned response.
    pub fn admit_query(
        &mut self,
        proof: &AdmissionProof,
        query: &QueryPlan,
        now_ms: i64,
    ) -> Result<Admitted<QueryResult>> {
        let _read_scope = self.read_budget.enter();
        json_size(query, 1024 * 1024)?;
        let body = serde_json::to_vec(query)?;
        let operation = Operation {
            action: Action::Read,
            graph_id: query.graph_id.clone(),
            branch_id: query.branch_id.clone(),
        };
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let verified = self.verify_admission(proof, &body, &operation, now_ms)?;
        let scopes = &proof
            .chain
            .last()
            .expect("verified chain")
            .capability
            .scopes;
        require_scope(
            scopes,
            &query.graph_id,
            &query.branch_id,
            &[Action::Read, Action::Traverse],
        )?;
        let host = HostContext::new(verified.principal(), []);
        let prior = self.prior_admission::<ReadReceipt>(&verified)?;
        let receipt = if let Some(prior) = &prior {
            // A narrower retry capability cannot retrieve an older broader response.
            self.check_dependency_closure(
                prior.dependencies.clone(),
                scopes,
                verified.principal(),
            )?;
            self.require_current_result_authority(&prior.result, &host)?;
            ReadReceipt {
                result: prior.result.clone(),
                dependencies: prior.dependencies.clone(),
            }
        } else {
            let revision = query
                .revision
                .clone()
                .or(self.head(&query.graph_id, &query.branch_id)?)
                .ok_or_else(|| err("E_UNAVAILABLE", "query unavailable"))?;
            if !self.reachable_revision(&query.graph_id, &query.branch_id, &revision)? {
                return Err(err(
                    "E_SCOPE",
                    "query revision is outside authorized accepted history",
                ));
            }
            let dependencies = self.check_dependency_closure(
                vec![GraphRef {
                    graph_id: query.graph_id.clone(),
                    revision,
                }],
                scopes,
                verified.principal(),
            )?;
            ReadReceipt {
                result: self.query(query, &host)?,
                dependencies,
            }
        };
        if prior.is_none() {
            self.record_admission(&verified, &receipt)?;
        }
        tx.commit()?;
        Ok(Admitted {
            duplicate: prior.is_some(),
            result: receipt.result,
        })
    }
    /// One graph/branch publication, never an unrestricted program. Egress remains subject-scoped.
    pub fn admit_publish(
        &mut self,
        proof: &AdmissionProof,
        commit: &SnapshotCommit,
        now_ms: i64,
    ) -> Result<Admitted<CommitReceipt>> {
        self.admit_publish_boundary(proof, commit, now_ms, || {})
    }
    /// Test-only process termination boundary, absent from ordinary builds.
    #[cfg(feature = "recovery-testing")]
    pub fn admit_publish_test_before_commit(
        &mut self,
        proof: &AdmissionProof,
        commit: &SnapshotCommit,
        now_ms: i64,
        before_commit: impl FnOnce(),
    ) -> Result<Admitted<CommitReceipt>> {
        self.admit_publish_boundary(proof, commit, now_ms, before_commit)
    }
    fn admit_publish_boundary(
        &mut self,
        proof: &AdmissionProof,
        commit: &SnapshotCommit,
        now_ms: i64,
        before_commit: impl FnOnce(),
    ) -> Result<Admitted<CommitReceipt>> {
        let _read_scope = self.read_budget.enter();
        json_size(commit, 16 * 1024 * 1024)?;
        let body = serde_json::to_vec(commit)?;
        let operation = Operation {
            action: Action::Publish,
            graph_id: commit.graph_id.clone(),
            branch_id: commit.branch_id.clone(),
        };
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let verified = self.verify_admission(proof, &body, &operation, now_ms)?;
        if let Some(result) = self.prior_admission(&verified)? {
            tx.commit()?;
            return Ok(Admitted {
                duplicate: true,
                result,
            });
        }
        let scopes = &proof
            .chain
            .last()
            .expect("verified chain")
            .capability
            .scopes;
        require_private_egress(&commit.data, verified.principal())?;
        self.require_installed_schema(&commit.data)?;
        let host = HostContext::new(verified.principal(), [commit.graph_id.clone()]);
        // Remote publication cannot install mutable handles without an explicit pin context.
        if commit
            .data
            .attachments
            .iter()
            .any(|a| matches!(a.value, MetadataValue::LiveGraph { .. }))
        {
            return Err(err(
                "E_UNSUPPORTED",
                "remote live metadata publication requires a pinned export context",
            ));
        }
        self.check_dependency_closure(
            all_dependencies(&commit.data),
            scopes,
            verified.principal(),
        )?;
        let (revision, event_id) = self.commit_inner(
            &commit.graph_id,
            &commit.branch_id,
            commit.expected_head.as_deref(),
            &commit.data,
            &host,
        )?;
        let result = CommitReceipt {
            graph_id: commit.graph_id.clone(),
            branch_id: commit.branch_id.clone(),
            revision,
            event_id,
        };
        self.record_admission(&verified, &result)?;
        before_commit();
        tx.commit()?;
        Ok(Admitted {
            duplicate: false,
            result,
        })
    }
    /// Proposals occupy an isolated raw store and cannot preempt accepted identity/schema registries.
    pub fn admit_proposal(
        &mut self,
        proof: &AdmissionProof,
        capsule: &Capsule,
        now_ms: i64,
    ) -> Result<Admitted<ProposalReceipt>> {
        let _read_scope = self.read_budget.enter();
        json_size(capsule, 16 * 1024 * 1024)?;
        let root = capsule
            .revisions
            .iter()
            .find(|r| r.graph_id == capsule.root.graph_id && r.revision == capsule.root.revision)
            .ok_or_else(|| err("E_INTEGRITY", "proposal root absent"))?;
        let operation = Operation {
            action: Action::Propose,
            graph_id: root.graph_id.clone(),
            branch_id: root.branch_id.clone(),
        };
        let body = serde_json::to_vec(capsule)?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let verified = self.verify_admission(proof, &body, &operation, now_ms)?;
        if let Some(result) = self.prior_admission(&verified)? {
            tx.commit()?;
            return Ok(Admitted {
                duplicate: true,
                result,
            });
        }
        let scopes = &proof
            .chain
            .last()
            .expect("verified chain")
            .capability
            .scopes;
        for revision in &capsule.revisions {
            require_scope(
                scopes,
                &revision.graph_id,
                &revision.branch_id,
                &[Action::Propose],
            )?;
        }
        let mut temporary = Engine::memory()?;
        temporary.receive_capsule(
            capsule,
            &HostContext::new(
                verified.principal(),
                capsule.revisions.iter().map(|r| r.graph_id.clone()),
            ),
        )?;
        let id = format!(
            "proposal:{:x}",
            Sha256::digest(serde_json::to_vec(&(
                verified.principal(),
                verified.body_digest()
            ))?)
        );
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM isolated_proposals WHERE id=?1)",
            [&id],
            |r| r.get(0),
        )?;
        if !exists {
            let used: i64 = self.conn.query_row(
                "SELECT COALESCE(SUM(length(CAST(capsule AS BLOB))),0) FROM isolated_proposals WHERE subject=?1",
                [verified.principal()],
                |r| r.get(0),
            )?;
            if used as usize + body.len() > 64 * 1024 * 1024 {
                return Err(err("E_BACKPRESSURE", "proposal storage quota exceeded"));
            }
            self.conn.execute(
                "INSERT INTO isolated_proposals VALUES (?1,?2,?3,?4)",
                params![
                    id,
                    verified.principal(),
                    verified.body_digest(),
                    String::from_utf8(body).map_err(|_| err("E_ENCODING", "proposal encoding"))?
                ],
            )?;
        }
        let result = ProposalReceipt {
            id,
            root: capsule.root.clone(),
            revision_count: capsule.revisions.len(),
        };
        self.record_admission(&verified, &result)?;
        tx.commit()?;
        Ok(Admitted {
            duplicate: false,
            result,
        })
    }
    fn require_installed_schema(&self, data: &GraphData) -> Result<()> {
        if let Some(schema) = &data.schema {
            let stored: Option<String> = self
                .conn
                .query_row(
                    "SELECT descriptor FROM schema_registry WHERE id=?1 AND revision=?2",
                    params![schema.id, schema.revision],
                    |r| r.get(0),
                )
                .optional()?;
            if stored
                .and_then(|s| serde_json::from_str::<GraphSchema>(&s).ok())
                .as_ref()
                != Some(schema)
            {
                return Err(err(
                    "E_SCHEMA_AUTHORITY",
                    "remote publication requires an exact host-installed schema",
                ));
            }
        }
        Ok(())
    }
    fn require_reference_scope(
        &self,
        reference: &GraphRef,
        scopes: &[Scope],
        steps: &mut usize,
    ) -> Result<()> {
        for scope in scopes {
            if scope.graph_id == reference.graph_id
                && scope.actions.contains(&Action::Read)
                && scope.actions.contains(&Action::Traverse)
                && self.reachable_revision_bounded(
                    &reference.graph_id,
                    &scope.branch_id,
                    &reference.revision,
                    steps,
                )?
            {
                return Ok(());
            }
        }
        Err(err(
            "E_SCOPE",
            "dependency outside authorized accepted history",
        ))
    }
    fn reachable_revision(&self, graph: &str, branch: &str, revision: &str) -> Result<bool> {
        self.reachable_revision_bounded(graph, branch, revision, &mut 0)
    }
    fn reachable_revision_bounded(
        &self,
        graph: &str,
        branch: &str,
        revision: &str,
        steps: &mut usize,
    ) -> Result<bool> {
        let mut cursor = self.head(graph, branch)?;
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let Some(current) = cursor else {
                return Ok(false);
            };
            *steps += 1;
            if *steps > 10000 {
                return Err(err("E_BUDGET", "admission ancestry work budget exceeded"));
            }
            if current == revision {
                return Ok(true);
            }
            if !seen.insert(current.clone()) {
                return Err(err("E_INTEGRITY", "accepted ancestry cycle"));
            }
            cursor = self
                .conn
                .query_row(
                    "SELECT parent FROM revisions WHERE graph_id=?1 AND revision=?2",
                    params![graph, current],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
        }
        Err(err(
            "E_BUDGET",
            "accepted ancestry exceeds admission budget",
        ))
    }
    fn check_dependency_closure(
        &self,
        roots: Vec<GraphRef>,
        scopes: &[Scope],
        principal: &str,
    ) -> Result<Vec<GraphRef>> {
        let mut queue = std::collections::VecDeque::from(roots);
        let mut seen = HashSet::new();
        let mut bytes = 0usize;
        let mut ancestry_steps = 0;
        let mut dependencies = Vec::new();
        while let Some(reference) = queue.pop_front() {
            if !seen.insert((reference.graph_id.clone(), reference.revision.clone())) {
                continue;
            }
            if seen.len() > 1000 {
                return Err(err("E_BUDGET", "admission dependency budget exceeded"));
            }
            self.require_reference_scope(&reference, scopes, &mut ancestry_steps)?;
            if !self.identity_reference_allowed(
                &reference.graph_id,
                &reference.revision,
                &HostContext::new(principal, []),
            )? {
                return Err(err(
                    "E_UNAVAILABLE",
                    "dependency unavailable under current authority",
                ));
            }
            dependencies.push(reference.clone());
            let data = self
                .load(&reference.graph_id, &reference.revision)?
                .ok_or_else(|| err("E_UNAVAILABLE", "dependency unavailable"))?;
            // Primitive reader/endpoint filtering does not follow any dependency.
            // Hidden objects cannot influence traversal queues or materialization budgets.
            let data = visible(data, principal);
            bytes += json_size(&data, MATERIALIZED_LIMIT.saturating_sub(bytes))?;
            // Visible candidate conclusions still require their full provenance scope before release.
            for attachment in &data.attachments {
                if let MetadataValue::LiveGraph {
                    graph_id,
                    branch_id,
                } = &attachment.value
                {
                    require_scope(
                        scopes,
                        graph_id,
                        branch_id,
                        &[Action::Read, Action::Traverse],
                    )?;
                    let revision = self
                        .head(graph_id, branch_id)?
                        .ok_or_else(|| err("E_UNAVAILABLE", "dependency unavailable"))?;
                    queue.push_back(GraphRef {
                        graph_id: graph_id.clone(),
                        revision,
                    });
                }
            }
            queue.extend(all_dependencies(&data));
        }
        Ok(dependencies)
    }
}
fn require_scope(scopes: &[Scope], graph: &str, branch: &str, actions: &[Action]) -> Result<()> {
    if !scopes.iter().any(|s| {
        s.graph_id == graph
            && s.branch_id == branch
            && actions.iter().all(|a| s.actions.contains(a))
    }) {
        return Err(err("E_SCOPE", "operation outside capability scope"));
    }
    Ok(())
}
fn require_private_egress(data: &GraphData, principal: &str) -> Result<()> {
    let expected = [principal.to_owned()];
    if data.nodes.iter().any(|n| n.readers != expected)
        || data.edges.iter().any(|e| e.readers != expected)
        || data.structural_edges.iter().any(|e| e.readers != expected)
        || data.assertions.iter().any(|a| a.readers != expected)
        || data.attachments.iter().any(|a| a.readers != expected)
    {
        return Err(err(
            "E_EGRESS",
            "remote output must retain subject restriction",
        ));
    }
    Ok(())
}
fn all_dependencies(data: &GraphData) -> Vec<GraphRef> {
    let mut references = refs(data);
    references.extend(
        data.edges
            .iter()
            .flat_map(|e| &e.derived_from)
            .chain(data.assertions.iter().flat_map(|a| &a.derived_from))
            .chain(data.attachments.iter().filter_map(|a| a.origin.as_ref()))
            .map(|r| GraphRef {
                graph_id: r.graph_id.clone(),
                revision: r.revision.clone(),
            }),
    );
    references.extend(
        data.edges
            .iter()
            .filter_map(|e| e.structural_ref.as_ref())
            .map(|r| GraphRef {
                graph_id: r.graph_id.clone(),
                revision: r.revision.clone(),
            }),
    );
    references.extend(
        data.edges
            .iter()
            .filter_map(|e| e.assertion_context.clone()),
    );
    for derivation in data
        .edges
        .iter()
        .flat_map(|e| &e.derivations)
        .chain(data.assertions.iter().flat_map(|a| &a.derivations))
    {
        references.extend(derivation.input_snapshots.clone());
        references.extend(derivation.premises.iter().map(|p| GraphRef {
            graph_id: p.graph_id.clone(),
            revision: p.revision.clone(),
        }));
    }
    references
}
