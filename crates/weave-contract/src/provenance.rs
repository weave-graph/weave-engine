//! Per-fact provenance records (ADR-0005, core contract v1).
//!
//! These records are defined by core v1 but are not yet carried by any
//! `Program` of protocol 0.21.0: the A2 protocol bump activates the reserved
//! `provenance` field on fact records and the result's `fact_provenance` list.
//! Until then they are validated only as standalone records (golden fixtures).
//!
//! The split is deliberate. [`ClaimedProvenance`] is what a client may send;
//! it has no writer or recorded-time field, so a client-supplied writer fails
//! to decode with `E_UNKNOWN_FIELD`. [`FactProvenance`] is what the engine
//! returns: its `writer` and `recorded_at_ms` are set by the engine only.
use crate::wire::ContractError;
use serde::{Deserialize, Serialize};

/// Maximum evidence references on one fact.
pub const MAX_EVIDENCE: usize = 64;
/// Maximum UTF-8 bytes of any provenance identifier or label.
pub const MAX_LABEL_BYTES: usize = 512;
/// Error code for a structurally valid but unacceptable provenance record.
pub const E_PROVENANCE: &str = "E_PROVENANCE";

/// How a fact was produced. A claim by the writer, not an attestation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Method {
    /// Entered or confirmed by a person.
    Human,
    /// Loaded by a named importer.
    Import { importer: String, version: String },
    /// Derived by a named, revisioned rule set.
    Rule { rule_id: String, revision: String },
    /// Produced by a model (for example an extraction or embedding model).
    Model { model: String, version: String },
    /// Not stated. Used for facts migrated from stores without per-fact
    /// provenance and for writes that omit a claim. Unknown, never "human".
    Unspecified,
}

/// A reference to evidence a writer cites for a fact.
///
/// Evidence is descriptive: it neither grants nor restricts authority.
/// Restriction-carrying dependencies stay in `derived_from`, `derived_nodes`
/// and `derived_snapshots`. Readers never see evidence they cannot read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceRef {
    /// An edge, explicit assertion or metadata attachment at an exact revision.
    Assertion {
        graph_id: String,
        revision: String,
        assertion_id: String,
    },
    /// A node manifestation at an exact revision.
    Node {
        graph_id: String,
        revision: String,
        node_id: String,
    },
    /// A whole graph revision.
    Graph { graph_id: String, revision: String },
    /// An external artifact (document, log file, series export) by URI and digest.
    Artifact { uri: String, digest: String },
    /// An embedding fact at an exact revision (0.24), for example an origin
    /// of a persisted candidate relation.
    Embedding {
        graph_id: String,
        revision: String,
        embedding_id: String,
    },
}

/// Provenance a client may claim when writing a fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimedProvenance {
    /// Claimed source label (system, document, sensor), not an authenticated identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub method: Method,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceRef>,
}

/// Most links in an agent ownership chain, matching the 8-token bound of
/// `weave-policy` capability chains (ADR-0007 section 3).
pub const MAX_DELEGATION_CHAIN: usize = 8;

/// Most reviewers named in one [`ReviewProvenance`], matching the bound on a
/// review policy's threshold.
pub const MAX_REVIEWERS: usize = 32;

/// How a fact reached a reviewed graph (1.3.0, B5): the proposal it was
/// written in, the decision that published it and the reviewers whose
/// approvals made up the quorum. Engine-recorded and outside fact content,
/// like `writer`, which stays the proposer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewProvenance {
    /// The proposal the fact was written in.
    pub proposal_id: String,
    /// The decision that accepted the proposal.
    pub decision_id: String,
    /// The reviewers whose approvals of the accepted version formed the
    /// quorum, sorted. Never the proposer.
    pub reviewers: Vec<String>,
    /// Engine-recorded time at which the proposal was first submitted.
    pub proposed_at_ms: i64,
    /// Engine-recorded time of the decision. It equals the fact's
    /// `recorded_at_ms`: a proposed fact is recorded only when accepted.
    pub decided_at_ms: i64,
}

impl ReviewProvenance {
    pub fn validate(&self) -> Result<(), ContractError> {
        if !label(&self.proposal_id)
            || !label(&self.decision_id)
            || self.reviewers.is_empty()
            || self.reviewers.len() > MAX_REVIEWERS
            || !self.reviewers.iter().all(|r| label(r))
            || self.reviewers.windows(2).any(|w| w[0] >= w[1])
            || self.proposed_at_ms < 0
            || self.decided_at_ms < self.proposed_at_ms
        {
            return Err(invalid(
                "review provenance needs bounded identifiers, 1 to 32 sorted distinct reviewers and ordered times",
            ));
        }
        Ok(())
    }
}

/// Provenance of one stored fact as returned by the engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactProvenance {
    /// Principal that wrote the fact, recorded by the engine from host authority.
    pub writer: String,
    /// Delegation chain of an agent writer (1.1.0, ADR-0007 section 8): the
    /// agent's owner first, ending at the root user. Engine-recorded and
    /// outside content. Omitted when the writer acted for no one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_behalf_of: Vec<String>,
    /// Engine-recorded time (Unix milliseconds) at which this replica recorded the fact.
    pub recorded_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub method: Method,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceRef>,
    /// The review that published the fact (1.3.0, B5), for a fact written
    /// through an accepted proposal. Omitted for facts written directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewProvenance>,
}

/// Identity of one stored fact, used to key provenance in results.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FactRef {
    /// Edge, explicit assertion or attachment (one disjoint ID namespace per revision).
    Assertion {
        graph_id: String,
        revision: String,
        assertion_id: String,
    },
    Node {
        graph_id: String,
        revision: String,
        node_id: String,
    },
    /// An embedding fact (ADR-0006).
    Embedding {
        graph_id: String,
        revision: String,
        embedding_id: String,
    },
}

/// One entry of a result's reserved `fact_provenance` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceEntry {
    pub fact: FactRef,
    pub provenance: FactProvenance,
}

fn label(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_LABEL_BYTES
}
fn invalid(message: &str) -> ContractError {
    ContractError {
        code: E_PROVENANCE.into(),
        message: message.into(),
    }
}

impl Method {
    pub fn validate(&self) -> Result<(), ContractError> {
        let ok = match self {
            Self::Human | Self::Unspecified => true,
            Self::Import { importer, version } => label(importer) && label(version),
            Self::Rule { rule_id, revision } => label(rule_id) && label(revision),
            Self::Model { model, version } => label(model) && label(version),
        };
        ok.then_some(())
            .ok_or_else(|| invalid("method labels require 1 to 512 bytes"))
    }
}
impl EvidenceRef {
    pub fn validate(&self) -> Result<(), ContractError> {
        let ok = match self {
            Self::Assertion {
                graph_id,
                revision,
                assertion_id: id,
            }
            | Self::Node {
                graph_id,
                revision,
                node_id: id,
            }
            | Self::Embedding {
                graph_id,
                revision,
                embedding_id: id,
            } => label(graph_id) && label(revision) && label(id),
            Self::Graph { graph_id, revision } => label(graph_id) && label(revision),
            Self::Artifact { uri, digest } => label(uri) && label(digest),
        };
        ok.then_some(())
            .ok_or_else(|| invalid("evidence identifiers require 1 to 512 bytes"))
    }
}
fn validate_claim(
    source: &Option<String>,
    method: &Method,
    evidence: &[EvidenceRef],
) -> Result<(), ContractError> {
    if source.as_deref().is_some_and(|s| !label(s)) {
        return Err(invalid("source label requires 1 to 512 bytes"));
    }
    method.validate()?;
    if evidence.len() > MAX_EVIDENCE {
        return Err(ContractError {
            code: crate::wire::E_BUDGET.into(),
            message: "at most 64 evidence references".into(),
        });
    }
    evidence.iter().try_for_each(EvidenceRef::validate)
}
impl ClaimedProvenance {
    pub fn validate(&self) -> Result<(), ContractError> {
        validate_claim(&self.source, &self.method, &self.evidence)
    }
}
impl FactProvenance {
    pub fn validate(&self) -> Result<(), ContractError> {
        if !label(&self.writer) || self.recorded_at_ms < 0 {
            return Err(invalid(
                "engine-recorded writer and nonnegative recorded time required",
            ));
        }
        if self.on_behalf_of.len() > MAX_DELEGATION_CHAIN
            || !self.on_behalf_of.iter().all(|p| label(p))
        {
            return Err(invalid(
                "on_behalf_of holds at most 8 bounded principal identifiers",
            ));
        }
        if let Some(review) = &self.review {
            review.validate()?;
            if review.decided_at_ms != self.recorded_at_ms
                || review.reviewers.contains(&self.writer)
            {
                return Err(invalid(
                    "a reviewed fact is recorded at its decision and never reviewed by its writer",
                ));
            }
        }
        validate_claim(&self.source, &self.method, &self.evidence)
    }
    /// Combine an engine-recorded writer and time with a client claim.
    /// This is the only constructor a writer path should use.
    pub fn record(writer: String, recorded_at_ms: i64, claim: Option<ClaimedProvenance>) -> Self {
        let claim = claim.unwrap_or(ClaimedProvenance {
            source: None,
            method: Method::Unspecified,
            evidence: Vec::new(),
        });
        Self {
            writer,
            on_behalf_of: Vec::new(),
            recorded_at_ms,
            source: claim.source,
            method: claim.method,
            evidence: claim.evidence,
            review: None,
        }
    }
}
