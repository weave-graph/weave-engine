//! B5 (#12): optional review of shared knowledge.
//!
//! Each graph of a registry store has a review mode. `direct` (the default)
//! publishes writes immediately. `reviewed` refuses every direct write: a
//! principal with Write *proposes* fact changes, and reviewers named by the
//! graph's policy accept or reject them. An accepted proposal is written as
//! its proposer, in one fact revision, and its facts record the proposal,
//! the decision and the approving reviewers as engine-recorded provenance.
//!
//! The rules are the existing governance machinery's (`governance.rs`) on
//! registry authority:
//!
//! - **Immutable approval subjects.** A vote names the digest of the exact
//!   proposal version it approves. An amended proposal is a new version
//!   with a new digest; earlier approvals do not count for it, and a vote
//!   naming an old digest fails with `E_REVIEW_CHANGED`.
//! - **M of N.** A proposal is accepted when `threshold` distinct reviewers,
//!   none of them the proposer, approve its current version under the
//!   current policy revision, each still eligible when the quorum is
//!   counted.
//! - **Policy transitions.** Tightening a direct graph is immediate for an
//!   Administer holder. Every change of a reviewed graph's policy is itself
//!   a proposal, decided by the quorum of the policy it replaces; the new
//!   policy never authorizes itself.
//! - **Atomic decision.** The vote, the decision and the published revision
//!   commit in one savepoint, against the head the write sees.
//!
//! Authority is the registry's: graph ACLs, delegations and group
//! membership resolved per request. Nothing here consults
//! `HostContext::writable_graphs`. Review state lives in its own tables,
//! outside every graph, so pending and rejected proposals never reach a
//! graph read; rows are never updated or deleted.
use super::*;
use crate::registry::{Permission, PrincipalKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use weave_contract::provenance::ReviewProvenance;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS review_policies(graph_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),body TEXT NOT NULL,writer TEXT NOT NULL,decision_id TEXT,recorded_at_ms INTEGER NOT NULL,PRIMARY KEY(graph_id,revision));
CREATE TABLE IF NOT EXISTS review_proposals(sequence INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT UNIQUE NOT NULL,graph_id TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('facts','policy')),proposer TEXT NOT NULL,on_behalf_of TEXT,proposed_at_ms INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS review_proposals_graph ON review_proposals(graph_id,sequence);
CREATE INDEX IF NOT EXISTS review_proposals_proposer ON review_proposals(graph_id,proposer);
CREATE TABLE IF NOT EXISTS review_versions(proposal_id TEXT NOT NULL REFERENCES review_proposals(id),version INTEGER NOT NULL CHECK(version>0),digest TEXT NOT NULL,body TEXT NOT NULL,recorded_at_ms INTEGER NOT NULL,PRIMARY KEY(proposal_id,version));
CREATE TABLE IF NOT EXISTS review_votes(proposal_id TEXT NOT NULL REFERENCES review_proposals(id),version INTEGER NOT NULL,policy_revision INTEGER NOT NULL,reviewer TEXT NOT NULL,verdict TEXT NOT NULL CHECK(verdict IN ('approve','reject')),note TEXT,recorded_at_ms INTEGER NOT NULL,PRIMARY KEY(proposal_id,version,policy_revision,reviewer));
CREATE TABLE IF NOT EXISTS review_decisions(proposal_id TEXT PRIMARY KEY REFERENCES review_proposals(id),id TEXT UNIQUE NOT NULL,status TEXT NOT NULL CHECK(status IN ('accepted','rejected','withdrawn')),version INTEGER NOT NULL,revision TEXT,reviewers TEXT NOT NULL,note TEXT,policy_revision INTEGER NOT NULL,decided_at_ms INTEGER NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS review_decisions_revision ON review_decisions(revision) WHERE revision IS NOT NULL;
CREATE TRIGGER IF NOT EXISTS review_policies_immutable BEFORE UPDATE ON review_policies BEGIN SELECT RAISE(ABORT,'review records are immutable'); END;
CREATE TRIGGER IF NOT EXISTS review_policies_kept BEFORE DELETE ON review_policies BEGIN SELECT RAISE(ABORT,'review records are immutable'); END;
CREATE TRIGGER IF NOT EXISTS review_proposals_immutable BEFORE UPDATE ON review_proposals BEGIN SELECT RAISE(ABORT,'review records are immutable'); END;
CREATE TRIGGER IF NOT EXISTS review_proposals_kept BEFORE DELETE ON review_proposals BEGIN SELECT RAISE(ABORT,'review records are immutable'); END;
CREATE TRIGGER IF NOT EXISTS review_versions_immutable BEFORE UPDATE ON review_versions BEGIN SELECT RAISE(ABORT,'review records are immutable'); END;
CREATE TRIGGER IF NOT EXISTS review_versions_kept BEFORE DELETE ON review_versions BEGIN SELECT RAISE(ABORT,'review records are immutable'); END;
CREATE TRIGGER IF NOT EXISTS review_votes_immutable BEFORE UPDATE ON review_votes BEGIN SELECT RAISE(ABORT,'review records are immutable'); END;
CREATE TRIGGER IF NOT EXISTS review_votes_kept BEFORE DELETE ON review_votes BEGIN SELECT RAISE(ABORT,'review records are immutable'); END;
CREATE TRIGGER IF NOT EXISTS review_decisions_immutable BEFORE UPDATE ON review_decisions BEGIN SELECT RAISE(ABORT,'review records are immutable'); END;
CREATE TRIGGER IF NOT EXISTS review_decisions_kept BEFORE DELETE ON review_decisions BEGIN SELECT RAISE(ABORT,'review records are immutable'); END;
";

/// Most changes in one proposal version.
pub const MAX_PROPOSAL_CHANGES: usize = 1000;
/// Most proposals one listing returns.
pub const MAX_REVIEW_LIST: usize = 100;
/// Most reviewers a policy names, and its highest threshold.
const MAX_REVIEWERS: usize = weave_contract::provenance::MAX_REVIEWERS;
/// Most undecided proposals one proposer holds in one graph.
const MAX_PENDING_PER_PROPOSER: i64 = 1000;
/// Most versions (the first submission and its amendments) of one proposal.
const MAX_VERSIONS: i64 = 64;
/// Largest stored proposal version.
const MAX_PROPOSAL_BYTES: usize = 4 * 1024 * 1024;
/// Largest vote note.
const MAX_NOTE_BYTES: usize = 2048;
/// Most proposals one listing inspects before giving up.
const MAX_LIST_SCAN: usize = 100_000;
const DIGEST_DOMAIN: &str = "weave-review-proposal-v1";

/// A graph's review mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewMode {
    /// Writes publish immediately (personal and trusted graphs). The
    /// default for every graph without a policy.
    Direct,
    /// Writes are proposals; `threshold` distinct reviewers accept them.
    Reviewed {
        /// Users or groups whose active users may review. Agents never
        /// review: approving shared knowledge is a human decision.
        reviewers: Vec<String>,
        threshold: u32,
    },
}

/// A graph's current review policy. Revision 0 is the implicit direct
/// policy of a graph that never had one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPolicy {
    pub graph_id: String,
    pub revision: u64,
    pub mode: ReviewMode,
}

/// What a policy change did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewPolicyChange {
    /// A direct graph's policy changed at once.
    Applied { policy: ReviewPolicy },
    /// A reviewed graph's policy change became a proposal for the current
    /// reviewers.
    Proposed { proposal: ReviewProposalReceipt },
}

/// What a proposal asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewContent {
    /// Fact changes to one branch, exactly as a `write` command carries them.
    Facts {
        branch_id: String,
        changes: Vec<FactChange>,
    },
    /// A replacement policy, decided by the quorum of the revision it
    /// replaces.
    Policy {
        mode: ReviewMode,
        expected_revision: u64,
    },
}

/// The receipt of a submitted or amended proposal. Reviewers approve
/// exactly `digest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewProposalReceipt {
    pub proposal_id: String,
    pub version: u32,
    pub digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Approve,
    Reject,
}

/// One reviewer's vote on one exact proposal version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewVote {
    pub proposal_id: String,
    /// The digest of the version the reviewer read.
    pub digest: String,
    pub verdict: ReviewVerdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Pending,
    Accepted,
    Rejected,
    Withdrawn,
}

impl ReviewStatus {
    fn as_str(self) -> &'static str {
        match self {
            ReviewStatus::Pending => "pending",
            ReviewStatus::Accepted => "accepted",
            ReviewStatus::Rejected => "rejected",
            ReviewStatus::Withdrawn => "withdrawn",
        }
    }
    fn parse(text: &str) -> Result<Self> {
        Ok(match text {
            "accepted" => ReviewStatus::Accepted,
            "rejected" => ReviewStatus::Rejected,
            "withdrawn" => ReviewStatus::Withdrawn,
            _ => return Err(corrupt()),
        })
    }
}

/// The final decision on a proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewDecision {
    pub decision_id: String,
    pub status: ReviewStatus,
    /// The version decided.
    pub version: u32,
    /// For an acceptance, the approving reviewers of the quorum; for a
    /// rejection, the rejecting reviewer; for a withdrawal, whoever withdrew.
    pub reviewers: Vec<String>,
    /// The fact revision an accepted fact proposal published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub decided_at_ms: i64,
}

/// A proposal as one principal may see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewProposal {
    pub id: String,
    pub graph_id: String,
    pub proposer: String,
    /// The proposer's delegation chain when it is an agent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_behalf_of: Vec<String>,
    pub proposed_at_ms: i64,
    /// The current version and its digest.
    pub version: u32,
    pub digest: String,
    pub content: ReviewContent,
    pub status: ReviewStatus,
    /// Reviewers who approved the current version under the current policy
    /// revision (pending proposals).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approvals: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<ReviewDecision>,
}

/// Which proposals of one graph to list, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewProposalFilter {
    pub graph_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ReviewStatus>,
    /// Continue after this proposal (the last of the previous page).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// At most [`MAX_REVIEW_LIST`]; the default is 50.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// One review operation, for hosts that pass requests as JSON (the
/// `weave-engine review` command).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewRequest {
    Policy {
        graph_id: String,
    },
    SetPolicy {
        graph_id: String,
        mode: ReviewMode,
        expected_revision: u64,
    },
    Propose {
        graph_id: String,
        branch_id: String,
        changes: Vec<FactChange>,
    },
    Amend {
        proposal_id: String,
        changes: Vec<FactChange>,
    },
    Withdraw {
        proposal_id: String,
    },
    Vote(ReviewVote),
    Show {
        proposal_id: String,
    },
    List(ReviewProposalFilter),
}

/// The stored body of one proposal version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredVersion {
    content: ReviewContent,
    /// Reader clauses a reviewer must satisfy (`Engine::proposal_scope`).
    scope: Vec<(Vec<String>, Vec<String>)>,
}

/// One proposal's stored rows.
struct Row {
    sequence: i64,
    id: String,
    graph_id: String,
    kind: String,
    proposer: String,
    on_behalf_of: Vec<String>,
    proposed_at_ms: i64,
    version: u32,
    digest: String,
    body: StoredVersion,
    decision: Option<ReviewDecision>,
}

/// How a principal relates to a proposal it may see.
struct Standing {
    /// The proposer, or a principal in its ownership chain.
    proposer_side: bool,
    /// The principal's identity as an eligible reviewer within the
    /// proposal's read scope.
    reviewer: bool,
}

fn unavailable() -> Error {
    err("E_UNAVAILABLE", "proposal unavailable")
}
fn corrupt() -> Error {
    err("E_INTEGRITY", "stored review record is malformed")
}
fn stale() -> Error {
    err(
        "E_REVIEW_STALE",
        "the proposal no longer applies to the graph",
    )
}
fn not_reviewer() -> Error {
    err("E_FORBIDDEN", "the principal may not review this proposal")
}
fn random_id(conn: &rusqlite::Connection, prefix: &str) -> Result<String> {
    Ok(
        conn.query_row("SELECT ?1 || lower(hex(randomblob(16)))", [prefix], |r| {
            r.get(0)
        })?,
    )
}
fn digest_of(
    id: &str,
    version: u32,
    graph: &str,
    proposer: &str,
    content: &ReviewContent,
) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            DIGEST_DOMAIN,
            id,
            version,
            graph,
            proposer,
            content
        ))?)
    ))
}
fn validate_note(note: &Option<String>) -> Result<()> {
    if note
        .as_ref()
        .is_some_and(|n| n.is_empty() || n.len() > MAX_NOTE_BYTES)
    {
        return Err(err("E_REVIEW", "a note requires 1 to 2048 bytes"));
    }
    Ok(())
}
fn validate_changes(changes: &[FactChange]) -> Result<()> {
    if changes.is_empty() {
        return Err(err("E_FACT_WRITE", "a proposal needs at least one change"));
    }
    if changes.len() > MAX_PROPOSAL_CHANGES {
        return Err(err("E_BUDGET", "a proposal carries at most 1000 changes"));
    }
    Ok(())
}
/// Write-path errors that a stale proposal produces at acceptance. Every
/// one is reported as one `E_REVIEW_STALE`, so a reviewer learns only that
/// the proposal no longer applies, never why (the cause may be a record
/// the reviewer cannot read).
fn kept_at_acceptance(error: &Error) -> bool {
    ["E_BUDGET", "E_INTEGRITY", "E_STORAGE", "E_FORMAT"].contains(&error.code.as_str())
        || error.code.starts_with("E_CLOCK")
}

impl Engine {
    pub(crate) fn initialize_review(&self) -> Result<()> {
        self.conn.execute_batch(SCHEMA)?;
        Ok(())
    }

    /// True when `graph` has a reviewed policy. Policies exist only in
    /// registry stores.
    pub(crate) fn review_required(&self, graph: &str) -> Result<bool> {
        Ok(matches!(
            self.current_review_policy(graph)?.mode,
            ReviewMode::Reviewed { .. }
        ))
    }

    fn current_review_policy(&self, graph: &str) -> Result<ReviewPolicy> {
        let row: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT revision,CASE WHEN length(CAST(body AS BLOB))<=65536 THEN body ELSE '' END FROM review_policies WHERE graph_id=?1 ORDER BY revision DESC LIMIT 1",
                [graph],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            None => ReviewPolicy {
                graph_id: graph.into(),
                revision: 0,
                mode: ReviewMode::Direct,
            },
            Some((revision, body)) => ReviewPolicy {
                graph_id: graph.into(),
                revision: u64::try_from(revision).map_err(|_| corrupt())?,
                mode: serde_json::from_str(&body).map_err(|_| corrupt())?,
            },
        })
    }

    /// Engine-recorded review provenance of the facts a fact revision
    /// recorded, when an accepted proposal published that revision.
    pub(crate) fn review_provenance(&self, revision: &str) -> Result<Option<ReviewProvenance>> {
        let row: Option<(String, String, String, i64, i64)> = self
            .conn
            .query_row(
                "SELECT d.proposal_id,d.id,d.reviewers,p.proposed_at_ms,d.decided_at_ms FROM review_decisions d JOIN review_proposals p ON p.id=d.proposal_id WHERE d.revision=?1 AND d.status='accepted'",
                [revision],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((proposal_id, decision_id, reviewers, proposed_at_ms, decided_at_ms)) = row else {
            return Ok(None);
        };
        Ok(Some(ReviewProvenance {
            proposal_id,
            decision_id,
            reviewers: serde_json::from_str(&reviewers).map_err(|_| corrupt())?,
            proposed_at_ms,
            decided_at_ms,
        }))
    }

    /// Review operations need the registry: reviewers, proposers and
    /// eligibility are registry principals.
    fn require_review_registry(&self, host: &HostContext) -> Result<()> {
        if !valid_id(&host.principal) {
            return Err(err("E_ID", "identifiers require 1 to 512 bytes"));
        }
        if self.registry_head()?.is_none() {
            return Err(err("E_REGISTRY_DISABLED", "the store has no registry"));
        }
        Ok(())
    }

    /// One review mutation: a savepoint with the operation clock.
    fn review_atomic<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let _scope = self.read_budget.enter();
        self.conn.execute_batch("SAVEPOINT weave_review")?;
        let result = (|| {
            let _clock = self.operation_write_scope()?;
            f()
        })();
        match result {
            Ok(value) => {
                self.conn.execute_batch("RELEASE weave_review")?;
                Ok(value)
            }
            Err(error) => {
                self.conn
                    .execute_batch("ROLLBACK TO weave_review; RELEASE weave_review")?;
                Err(error)
            }
        }
    }

    /// One review read: a read snapshot with the operation clock.
    fn review_read<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let _scope = self.read_budget.enter();
        let transaction = self.optional_read_transaction()?;
        let value = {
            let _clock = self.operation_scope()?;
            f()?
        };
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        Ok(value)
    }

    fn validate_review_mode(&self, mode: &ReviewMode) -> Result<()> {
        let ReviewMode::Reviewed {
            reviewers,
            threshold,
        } = mode
        else {
            return Ok(());
        };
        if reviewers.is_empty()
            || reviewers.len() > MAX_REVIEWERS
            || reviewers.iter().collect::<BTreeSet<_>>().len() != reviewers.len()
            || *threshold == 0
            || *threshold as usize > MAX_REVIEWERS
        {
            return Err(err(
                "E_REVIEW_POLICY",
                "a reviewed policy names 1 to 32 distinct reviewers and a threshold of 1 to 32",
            ));
        }
        for reviewer in reviewers {
            if !valid_id(reviewer) {
                return Err(err("E_ID", "reviewer identifiers require 1 to 512 bytes"));
            }
            match self.active_principal_kind(reviewer)? {
                // Naming a principal is a graph administrator's view, as for
                // a graph grant.
                None => return Err(err("E_UNAVAILABLE", "principal unavailable")),
                Some(PrincipalKind::Agent) => {
                    return Err(err(
                        "E_REVIEW_POLICY",
                        "reviewers are users or groups; agents never review",
                    ))
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// The current review policy of `graph`. Needs Read on the graph.
    pub fn review_policy(&self, graph: &str, host: &HostContext) -> Result<ReviewPolicy> {
        self.require_review_registry(host)?;
        self.review_read(|| {
            self.require_graph(host, graph, Permission::Read)?;
            self.current_review_policy(graph)
        })
    }

    /// Change the review mode of `graph`. Needs Administer on the graph and
    /// the current policy `expected_revision` (`E_CONFLICT` otherwise).
    ///
    /// A direct graph changes at once. A reviewed graph's change becomes a
    /// policy proposal, decided by the current policy's reviewers; the
    /// proposed policy has no say in its own acceptance.
    pub fn set_review_policy(
        &self,
        graph: &str,
        mode: &ReviewMode,
        expected_revision: u64,
        host: &HostContext,
    ) -> Result<ReviewPolicyChange> {
        self.require_review_registry(host)?;
        if !valid_id(graph) {
            return Err(err("E_ID", "identifiers require 1 to 512 bytes"));
        }
        identity_acceptance::require_external_graph(graph)?;
        if graph.starts_with("weave:") {
            return Err(err(
                "E_RESERVED_NAMESPACE",
                "engine-reserved weave: graphs have no review policy",
            ));
        }
        self.review_atomic(|| {
            self.require_graph(host, graph, Permission::Administer)?;
            self.validate_review_mode(mode)?;
            let current = self.current_review_policy(graph)?;
            if current.revision != expected_revision {
                return Err(err("E_CONFLICT", "the review policy changed"));
            }
            if &current.mode == mode {
                return Ok(ReviewPolicyChange::Applied { policy: current });
            }
            match current.mode {
                ReviewMode::Direct => {
                    let policy = self.install_review_policy(graph, mode, host, None)?;
                    Ok(ReviewPolicyChange::Applied { policy })
                }
                ReviewMode::Reviewed { .. } => {
                    let content = ReviewContent::Policy {
                        mode: mode.clone(),
                        expected_revision,
                    };
                    let proposal = self.store_proposal(graph, "policy", content, vec![], host)?;
                    Ok(ReviewPolicyChange::Proposed { proposal })
                }
            }
        })
    }

    fn install_review_policy(
        &self,
        graph: &str,
        mode: &ReviewMode,
        writer: &HostContext,
        decision: Option<&str>,
    ) -> Result<ReviewPolicy> {
        let revision = self.current_review_policy(graph)?.revision + 1;
        self.conn.execute(
            "INSERT INTO review_policies VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                graph,
                i64::try_from(revision)
                    .map_err(|_| err("E_BUDGET", "policy revisions exhausted"))?,
                serde_json::to_string(mode)?,
                writer.principal,
                decision,
                self.operation_time()?
            ],
        )?;
        Ok(ReviewPolicy {
            graph_id: graph.into(),
            revision,
            mode: mode.clone(),
        })
    }

    fn store_proposal(
        &self,
        graph: &str,
        kind: &str,
        content: ReviewContent,
        scope: Vec<(Vec<String>, Vec<String>)>,
        host: &HostContext,
    ) -> Result<ReviewProposalReceipt> {
        let pending: i64 = self.conn.query_row(
            "SELECT count(*) FROM review_proposals p WHERE p.graph_id=?1 AND p.proposer=?2 AND NOT EXISTS(SELECT 1 FROM review_decisions d WHERE d.proposal_id=p.id)",
            params![graph, host.principal],
            |r| r.get(0),
        )?;
        if pending >= MAX_PENDING_PER_PROPOSER {
            return Err(err(
                "E_BUDGET",
                "a proposer holds at most 1000 undecided proposals per graph",
            ));
        }
        let id = random_id(&self.conn, "proposal:")?;
        let now = self.operation_time()?;
        let chain = self.delegation_chain(host)?;
        self.conn.execute(
            "INSERT INTO review_proposals(id,graph_id,kind,proposer,on_behalf_of,proposed_at_ms) VALUES (?1,?2,?3,?4,?5,?6)",
            params![id, graph, kind, host.principal, facts::encode_delegation(&chain)?, now],
        )?;
        self.store_version(&id, 1, graph, &host.principal, content, scope)
    }

    fn store_version(
        &self,
        id: &str,
        version: u32,
        graph: &str,
        proposer: &str,
        content: ReviewContent,
        scope: Vec<(Vec<String>, Vec<String>)>,
    ) -> Result<ReviewProposalReceipt> {
        let digest = digest_of(id, version, graph, proposer, &content)?;
        let body = StoredVersion { content, scope };
        json_size(&body, MAX_PROPOSAL_BYTES)?;
        self.conn.execute(
            "INSERT INTO review_versions VALUES (?1,?2,?3,?4,?5)",
            params![
                id,
                version,
                digest,
                serde_json::to_string(&body)?,
                self.operation_time()?
            ],
        )?;
        Ok(ReviewProposalReceipt {
            proposal_id: id.into(),
            version,
            digest,
        })
    }

    /// Validate fact changes as the proposer would write them now, and
    /// compute their review scope. Nothing is kept.
    fn check_proposed_changes(
        &self,
        graph: &str,
        branch: &str,
        changes: &[FactChange],
        host: &HostContext,
    ) -> Result<Vec<(Vec<String>, Vec<String>)>> {
        validate_changes(changes)?;
        let scope = self.proposal_scope(graph, branch, changes)?;
        self.conn.execute_batch("SAVEPOINT weave_review_check")?;
        let outcome = self.write_proposed(graph, branch, changes, host);
        self.conn
            .execute_batch("ROLLBACK TO weave_review_check; RELEASE weave_review_check")?;
        match outcome? {
            (_, Some(_)) => Ok(scope),
            (_, None) => Err(err("E_FACT_WRITE", "the proposal changes nothing")),
        }
    }

    /// Propose fact changes to a reviewed graph. Needs write authority on
    /// the branch (graph Write, a branch entry, or an agent's delegated
    /// Write), and the changes must be a valid write by the proposer now:
    /// every `write` error applies. Nothing reaches the graph until the
    /// proposal is accepted. A direct graph fails with
    /// `E_REVIEW_NOT_REQUIRED`: write the change instead.
    pub fn propose_review(
        &self,
        graph: &str,
        branch: &str,
        changes: &[FactChange],
        host: &HostContext,
    ) -> Result<ReviewProposalReceipt> {
        self.require_review_registry(host)?;
        if !valid_id(graph) || !valid_id(branch) {
            return Err(err("E_ID", "identifiers require 1 to 512 bytes"));
        }
        self.review_atomic(|| {
            self.require_graph(host, graph, Permission::Discover)?;
            if !self.branch_write_authority(host, graph, branch)? {
                return Err(err(
                    "E_FORBIDDEN",
                    "host has not granted graph write authority",
                ));
            }
            if !self.review_required(graph)? {
                return Err(err(
                    "E_REVIEW_NOT_REQUIRED",
                    "the graph publishes directly; write the change",
                ));
            }
            let scope = self.check_proposed_changes(graph, branch, changes, host)?;
            let content = ReviewContent::Facts {
                branch_id: branch.into(),
                changes: changes.to_vec(),
            };
            self.store_proposal(graph, "facts", content, scope, host)
        })
    }

    /// Replace the changes of the proposer's own pending fact proposal. The
    /// amendment is a new version with a new digest: approvals of earlier
    /// versions do not count for it.
    pub fn amend_review(
        &self,
        proposal_id: &str,
        changes: &[FactChange],
        host: &HostContext,
    ) -> Result<ReviewProposalReceipt> {
        self.require_review_registry(host)?;
        self.review_atomic(|| {
            let row = self.load_proposal(proposal_id)?.ok_or_else(unavailable)?;
            self.standing(&row, host)?.ok_or_else(unavailable)?;
            if row.proposer != host.principal {
                return Err(err("E_FORBIDDEN", "only the proposer amends a proposal"));
            }
            if row.decision.is_some() {
                return Err(err("E_REVIEW_DECIDED", "the proposal is already decided"));
            }
            let ReviewContent::Facts { branch_id, .. } = &row.body.content else {
                return Err(err("E_REVIEW", "a policy proposal cannot be amended"));
            };
            if !self.review_required(&row.graph_id)? {
                return Err(err(
                    "E_REVIEW_NOT_REQUIRED",
                    "the graph publishes directly; write the change",
                ));
            }
            if i64::from(row.version) >= MAX_VERSIONS {
                return Err(err("E_BUDGET", "a proposal has at most 64 versions"));
            }
            let branch = branch_id.clone();
            if !self.branch_write_authority(host, &row.graph_id, &branch)? {
                return Err(err(
                    "E_FORBIDDEN",
                    "host has not granted graph write authority",
                ));
            }
            let scope = self.check_proposed_changes(&row.graph_id, &branch, changes, host)?;
            let content = ReviewContent::Facts {
                branch_id: branch,
                changes: changes.to_vec(),
            };
            self.store_version(
                &row.id,
                row.version + 1,
                &row.graph_id,
                &row.proposer,
                content,
                scope,
            )
        })
    }

    /// Withdraw a pending proposal: by its proposer, or by a principal in an
    /// agent proposer's ownership chain. It stays in history as withdrawn.
    pub fn withdraw_review(&self, proposal_id: &str, host: &HostContext) -> Result<ReviewProposal> {
        self.require_review_registry(host)?;
        self.review_atomic(|| {
            let row = self.load_proposal(proposal_id)?.ok_or_else(unavailable)?;
            let standing = self.standing(&row, host)?.ok_or_else(unavailable)?;
            if !standing.proposer_side {
                return Err(err("E_FORBIDDEN", "only the proposer withdraws a proposal"));
            }
            if row.decision.is_some() {
                return Err(err("E_REVIEW_DECIDED", "the proposal is already decided"));
            }
            let policy = self.current_review_policy(&row.graph_id)?;
            self.record_decision(
                &row,
                ReviewStatus::Withdrawn,
                std::slice::from_ref(&host.principal),
                None,
                &None,
                policy.revision,
            )?;
            self.proposal_view(proposal_id, host)
        })
    }

    /// Approve or reject one exact version of a proposal.
    ///
    /// The voter must be an eligible reviewer of the graph's current policy
    /// (an active user named directly or through a group, with Read on the
    /// graph), not the proposer, and must satisfy every reader clause of the
    /// proposal's records. A proposal outside that scope fails exactly as a
    /// missing one. A rejection decides the proposal. An approval decides it
    /// when `threshold` distinct, still-eligible reviewers have approved the
    /// current version under the current policy revision: the changes are
    /// then written as the proposer in the same savepoint. If they no longer
    /// apply (a conflicting write, the proposer lost Write), the vote fails
    /// with `E_REVIEW_STALE` and nothing is recorded.
    pub fn vote_review(&self, vote: &ReviewVote, host: &HostContext) -> Result<ReviewProposal> {
        self.require_review_registry(host)?;
        validate_note(&vote.note)?;
        self.review_atomic(|| {
            let row = self
                .load_proposal(&vote.proposal_id)?
                .ok_or_else(unavailable)?;
            let standing = self.standing(&row, host)?.ok_or_else(unavailable)?;
            if row.decision.is_some() {
                return Err(err("E_REVIEW_DECIDED", "the proposal is already decided"));
            }
            let policy = self.current_review_policy(&row.graph_id)?;
            let ReviewMode::Reviewed { threshold, .. } = &policy.mode else {
                return Err(err(
                    "E_REVIEW_NOT_REQUIRED",
                    "the graph publishes directly; the proposal can only be withdrawn",
                ));
            };
            if !standing.reviewer || row.proposer == host.principal {
                return Err(not_reviewer());
            }
            if vote.digest != row.digest {
                return Err(err(
                    "E_REVIEW_CHANGED",
                    "the proposal differs from the version reviewed",
                ));
            }
            let verdict = match vote.verdict {
                ReviewVerdict::Approve => "approve",
                ReviewVerdict::Reject => "reject",
            };
            let prior: Option<String> = self
                .conn
                .query_row(
                    "SELECT verdict FROM review_votes WHERE proposal_id=?1 AND version=?2 AND policy_revision=?3 AND reviewer=?4",
                    params![row.id, row.version, policy.revision as i64, host.principal],
                    |r| r.get(0),
                )
                .optional()?;
            match prior {
                Some(prior) if prior == verdict => {
                    return self.proposal_view(&row.id, host);
                }
                Some(_) => {
                    return Err(err(
                        "E_CONFLICT",
                        "the reviewer already voted on this version",
                    ))
                }
                None => {}
            }
            self.conn.execute(
                "INSERT INTO review_votes VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    row.id,
                    row.version,
                    policy.revision as i64,
                    host.principal,
                    verdict,
                    vote.note,
                    self.operation_time()?
                ],
            )?;
            // The voter sees the outcome of its own vote, even when the
            // accepted policy no longer names it.
            let outcome = |engine: &Engine| -> Result<ReviewProposal> {
                engine.render(engine.load_proposal(&row.id)?.ok_or_else(corrupt)?)
            };
            match vote.verdict {
                ReviewVerdict::Reject => {
                    self.record_decision(
                        &row,
                        ReviewStatus::Rejected,
                        std::slice::from_ref(&host.principal),
                        None,
                        &vote.note,
                        policy.revision,
                    )?;
                }
                ReviewVerdict::Approve => {
                    let quorum = self.counted_approvals(&row, &policy)?;
                    if quorum.len() >= *threshold as usize {
                        self.accept(&row, &policy, &quorum)?;
                    }
                }
            }
            outcome(self)
        })
    }

    /// The approvals of the current version under the current policy
    /// revision whose reviewers are eligible now.
    fn counted_approvals(&self, row: &Row, policy: &ReviewPolicy) -> Result<Vec<String>> {
        let ReviewMode::Reviewed { reviewers, .. } = &policy.mode else {
            return Ok(vec![]);
        };
        let mut statement = self.conn.prepare(
            "SELECT reviewer FROM review_votes WHERE proposal_id=?1 AND version=?2 AND policy_revision=?3 AND verdict='approve' ORDER BY reviewer",
        )?;
        let voters = statement
            .query_map(params![row.id, row.version, policy.revision as i64], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut counted = Vec::new();
        for voter in voters {
            if voter == row.proposer {
                continue;
            }
            let Some(access) = self.reviewer_access(&voter, &row.graph_id, reviewers)? else {
                continue;
            };
            if in_scope(&row.body.scope, &access) {
                counted.push(voter);
            }
        }
        Ok(counted)
    }

    /// Publish an approved proposal and record the decision.
    fn accept(&self, row: &Row, policy: &ReviewPolicy, quorum: &[String]) -> Result<()> {
        match &row.body.content {
            ReviewContent::Facts { branch_id, changes } => {
                let proposer = HostContext::new(row.proposer.clone(), Vec::<String>::new());
                let revision =
                    match self.write_proposed(&row.graph_id, branch_id, changes, &proposer) {
                        Ok((revision, Some(_))) => revision,
                        Ok((_, None)) => return Err(stale()),
                        Err(error) if kept_at_acceptance(&error) => return Err(error),
                        Err(_) => return Err(stale()),
                    };
                self.record_decision(
                    row,
                    ReviewStatus::Accepted,
                    quorum,
                    Some(&revision),
                    &None,
                    policy.revision,
                )
                .map(|_| ())
            }
            ReviewContent::Policy {
                mode,
                expected_revision,
            } => {
                if *expected_revision != policy.revision {
                    return Err(stale());
                }
                let decision_id = self.record_decision(
                    row,
                    ReviewStatus::Accepted,
                    quorum,
                    None,
                    &None,
                    policy.revision,
                )?;
                let writer = HostContext::new(row.proposer.clone(), Vec::<String>::new());
                self.install_review_policy(&row.graph_id, mode, &writer, Some(&decision_id))?;
                Ok(())
            }
        }
    }

    fn record_decision(
        &self,
        row: &Row,
        status: ReviewStatus,
        reviewers: &[String],
        revision: Option<&str>,
        note: &Option<String>,
        policy_revision: u64,
    ) -> Result<String> {
        let mut reviewers = reviewers.to_vec();
        reviewers.sort();
        reviewers.dedup();
        let id = random_id(&self.conn, "decision:")?;
        self.conn.execute(
            "INSERT INTO review_decisions VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                row.id,
                id,
                status.as_str(),
                row.version,
                revision,
                serde_json::to_string(&reviewers)?,
                note,
                policy_revision as i64,
                self.operation_time()?
            ],
        )?;
        Ok(id)
    }

    /// One proposal as `host` may see it; `E_UNAVAILABLE` exactly as for a
    /// missing proposal otherwise.
    pub fn review_proposal(&self, proposal_id: &str, host: &HostContext) -> Result<ReviewProposal> {
        self.require_review_registry(host)?;
        self.review_read(|| self.proposal_view(proposal_id, host))
    }

    fn proposal_view(&self, proposal_id: &str, host: &HostContext) -> Result<ReviewProposal> {
        let row = self.load_proposal(proposal_id)?.ok_or_else(unavailable)?;
        self.standing(&row, host)?.ok_or_else(unavailable)?;
        self.render(row)
    }

    fn render(&self, row: Row) -> Result<ReviewProposal> {
        let (status, approvals) = match &row.decision {
            Some(decision) => (decision.status, vec![]),
            None => {
                let policy = self.current_review_policy(&row.graph_id)?;
                let mut statement = self.conn.prepare(
                    "SELECT reviewer FROM review_votes WHERE proposal_id=?1 AND version=?2 AND policy_revision=?3 AND verdict='approve' ORDER BY reviewer",
                )?;
                let approvals = statement
                    .query_map(params![row.id, row.version, policy.revision as i64], |r| {
                        r.get::<_, String>(0)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                (ReviewStatus::Pending, approvals)
            }
        };
        Ok(ReviewProposal {
            id: row.id,
            graph_id: row.graph_id,
            proposer: row.proposer,
            on_behalf_of: row.on_behalf_of,
            proposed_at_ms: row.proposed_at_ms,
            version: row.version,
            digest: row.digest,
            content: row.body.content,
            status,
            approvals,
            decision: row.decision,
        })
    }

    /// The proposals of one graph that `host` may see, oldest first: its
    /// own (and, for an owner, its agents'), and, for an eligible reviewer,
    /// those whose every record it may read. Needs Read on the graph. A
    /// listing reports no total, and `after` must name a proposal `host` may
    /// see.
    pub fn list_review_proposals(
        &self,
        filter: &ReviewProposalFilter,
        host: &HostContext,
    ) -> Result<Vec<ReviewProposal>> {
        self.require_review_registry(host)?;
        let limit = filter.limit.unwrap_or(50);
        if limit == 0 || limit > MAX_REVIEW_LIST {
            return Err(err("E_BUDGET", "a listing returns 1 to 100 proposals"));
        }
        if !valid_id(&filter.graph_id) {
            return Err(err("E_ID", "identifiers require 1 to 512 bytes"));
        }
        self.review_read(|| {
            self.require_graph(host, &filter.graph_id, Permission::Read)?;
            let after = match &filter.after {
                None => 0,
                Some(id) => {
                    let row = self.load_proposal(id)?.ok_or_else(unavailable)?;
                    if row.graph_id != filter.graph_id || self.standing(&row, host)?.is_none() {
                        return Err(unavailable());
                    }
                    row.sequence
                }
            };
            let mut statement = self.conn.prepare(
                "SELECT id FROM review_proposals WHERE graph_id=?1 AND sequence>?2 ORDER BY sequence",
            )?;
            let ids = statement
                .query_map(params![filter.graph_id, after], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for (scanned, id) in ids.enumerate() {
                if scanned >= MAX_LIST_SCAN {
                    return Err(err("E_BUDGET", "a listing inspects at most 100000 proposals"));
                }
                let row = self.load_proposal(&id?)?.ok_or_else(corrupt)?;
                let status = row.decision.as_ref().map_or(ReviewStatus::Pending, |d| d.status);
                if filter.status.is_some_and(|s| s != status) {
                    continue;
                }
                if self.standing(&row, host)?.is_none() {
                    continue;
                }
                out.push(self.render(row)?);
                if out.len() == limit {
                    break;
                }
            }
            json_size(&out, MATERIALIZED_LIMIT)?;
            Ok(out)
        })
    }

    /// Run one JSON review request (the `weave-engine review` command).
    pub fn review_request(
        &self,
        request: &ReviewRequest,
        host: &HostContext,
    ) -> Result<serde_json::Value> {
        Ok(match request {
            ReviewRequest::Policy { graph_id } => {
                serde_json::to_value(self.review_policy(graph_id, host)?)?
            }
            ReviewRequest::SetPolicy {
                graph_id,
                mode,
                expected_revision,
            } => serde_json::to_value(self.set_review_policy(
                graph_id,
                mode,
                *expected_revision,
                host,
            )?)?,
            ReviewRequest::Propose {
                graph_id,
                branch_id,
                changes,
            } => serde_json::to_value(self.propose_review(graph_id, branch_id, changes, host)?)?,
            ReviewRequest::Amend {
                proposal_id,
                changes,
            } => serde_json::to_value(self.amend_review(proposal_id, changes, host)?)?,
            ReviewRequest::Withdraw { proposal_id } => {
                serde_json::to_value(self.withdraw_review(proposal_id, host)?)?
            }
            ReviewRequest::Vote(vote) => serde_json::to_value(self.vote_review(vote, host)?)?,
            ReviewRequest::Show { proposal_id } => {
                serde_json::to_value(self.review_proposal(proposal_id, host)?)?
            }
            ReviewRequest::List(filter) => {
                serde_json::to_value(self.list_review_proposals(filter, host)?)?
            }
        })
    }

    /// How `host` stands towards a proposal, or `None` when it may not see
    /// it: no Read on the graph, or neither on the proposer's side nor an
    /// eligible reviewer within the proposal's read scope. A policy
    /// proposal is also visible to the graph's Administer holders.
    fn standing(&self, row: &Row, host: &HostContext) -> Result<Option<Standing>> {
        let Some(permission) = self.graph_permission(host, &row.graph_id)? else {
            return Ok(None);
        };
        if permission < Permission::Read {
            return Ok(None);
        }
        let proposer_side =
            row.proposer == host.principal || row.on_behalf_of.contains(&host.principal);
        let policy = self.current_review_policy(&row.graph_id)?;
        let reviewer = match &policy.mode {
            ReviewMode::Reviewed { reviewers, .. } => self
                .reviewer_access(&host.principal, &row.graph_id, reviewers)?
                .is_some_and(|access| in_scope(&row.body.scope, &access)),
            ReviewMode::Direct => false,
        };
        let administrator = row.kind == "policy" && permission == Permission::Administer;
        Ok(
            (proposer_side || reviewer || administrator).then_some(Standing {
                proposer_side,
                reviewer,
            }),
        )
    }

    fn load_proposal(&self, id: &str) -> Result<Option<Row>> {
        if !valid_id(id) {
            return Err(unavailable());
        }
        type Head = (i64, String, String, String, Option<String>, i64);
        let head: Option<Head> = self
            .conn
            .query_row(
                "SELECT sequence,graph_id,kind,proposer,on_behalf_of,proposed_at_ms FROM review_proposals WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?;
        let Some((sequence, graph_id, kind, proposer, on_behalf_of, proposed_at_ms)) = head else {
            return Ok(None);
        };
        let (version, digest, body): (u32, String, String) = self.conn.query_row(
            "SELECT version,digest,body FROM review_versions WHERE proposal_id=?1 ORDER BY version DESC LIMIT 1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        self.read_budget.charge(body.len())?;
        let body: StoredVersion = serde_json::from_str(&body).map_err(|_| corrupt())?;
        if digest != digest_of(id, version, &graph_id, &proposer, &body.content)? {
            return Err(corrupt());
        }
        type Decided = (
            String,
            String,
            u32,
            Option<String>,
            String,
            Option<String>,
            i64,
        );
        let decision: Option<Decided> = self
            .conn
            .query_row(
                "SELECT id,status,version,revision,reviewers,note,decided_at_ms FROM review_decisions WHERE proposal_id=?1",
                [id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()?;
        let decision = match decision {
            None => None,
            Some((decision_id, status, version, revision, reviewers, note, decided_at_ms)) => {
                Some(ReviewDecision {
                    decision_id,
                    status: ReviewStatus::parse(&status)?,
                    version,
                    reviewers: serde_json::from_str(&reviewers).map_err(|_| corrupt())?,
                    revision,
                    note,
                    decided_at_ms,
                })
            }
        };
        Ok(Some(Row {
            sequence,
            id: id.into(),
            graph_id,
            kind,
            proposer,
            on_behalf_of: facts::decode_delegation(on_behalf_of.as_deref())?,
            proposed_at_ms,
            version,
            digest,
            body,
            decision,
        }))
    }
}

/// True when `access` satisfies every reader clause of a proposal.
fn in_scope(scope: &[(Vec<String>, Vec<String>)], access: &visibility::Access) -> bool {
    scope
        .iter()
        .all(|(readers, groups)| visibility::record_admits(readers, groups, access))
}
