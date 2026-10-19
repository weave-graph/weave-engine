//! Embedding spaces, embedding facts and candidate relations (ADR-0006, core contract v1).
//!
//! Like [`crate::provenance`], these records are defined by core v1 but are
//! not yet carried by any `Program` of protocol 0.21.0. The A4 protocol bump
//! activates the reserved `GraphData.embedding_spaces` and `GraphData.embeddings`
//! fields and the reserved `nearest` and `similar_candidates` expressions.
use crate::provenance::{ClaimedProvenance, FactRef};
use crate::wire::ContractError;
use crate::{AssertionRef, GraphRef, Interval, NodeRef};
use serde::{Deserialize, Serialize};

/// Maximum vector dimensions (matches the geometry library bound).
pub const MAX_DIMENSIONS: u32 = 4096;
/// Error code for an invalid or mismatched embedding space or vector.
pub const E_SPACE: &str = "E_SPACE";
/// Reserved predicate of candidate relations. The `weave:` namespace is engine-owned.
pub const CANDIDATE_PREDICATE: &str = "weave:similar_to";

/// Distance function of a space. Scores are distances: lower is closer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingMetric {
    /// `1 - cos(a, b)`, in `[0, 2]`.
    CosineDistance,
    /// `sqrt(sum((a_i - b_i)^2))`.
    Euclidean,
}

/// Immutable descriptor of an embedding space. `(id, revision)` names it;
/// two vectors are comparable only when their spaces are equal in every field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingSpace {
    pub id: String,
    pub revision: String,
    /// Encoder (model) name, for example `text-embedding-3-small`.
    pub encoder: String,
    /// Exact encoder version or checkpoint digest.
    pub encoder_version: String,
    /// Named, versioned preprocessing (tokenization, chunking, normalization).
    pub preprocessing: String,
    pub dimensions: u32,
    pub metric: EmbeddingMetric,
}

/// Name of an embedding space.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceRef {
    pub id: String,
    pub revision: String,
}

/// What an embedding describes. IDs are graph-local, as in `GraphData`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EmbeddingSubject {
    /// The entity as a whole, independent of any manifestation.
    Entity { entity_id: String },
    /// One node (manifestation), including document-chunk nodes.
    Node { node_id: String },
    /// A legacy edge or a structural edge.
    Edge { edge_id: String },
    /// An explicit source assertion.
    Assertion { assertion_id: String },
}

/// A vector as a temporal fact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingFact {
    /// Shares the graph's edge/assertion/attachment ID namespace.
    pub id: String,
    pub space: SpaceRef,
    pub subject: EmbeddingSubject,
    /// IEEE-754 binary32 components. JSON numbers are rounded to the nearest
    /// binary32 value when decoded; the stored value is the rounded one.
    pub vector: Vec<f32>,
    pub valid_time: Interval,
    /// Empty means public. Nonempty is a disjunction of principal IDs.
    #[serde(default)]
    pub readers: Vec<String>,
    /// Conservative AND gates: an embedding computed from restricted text is
    /// restricted to readers of that text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_from: Vec<AssertionRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_nodes: Vec<NodeRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_snapshots: Vec<GraphRef>,
    /// Client-claimed provenance (ADR-0005).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<ClaimedProvenance>,
}

/// Whether a ranking was computed exhaustively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exactness {
    Exact,
    /// Reserved for approximate indexes (C5). Never produced in Phase A.
    Approximate,
}

/// A similarity-derived relation proposed by the engine. It is a result value:
/// never an asserted fact, never an identity, and never returned by ordinary
/// traversal unless explicitly requested.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateRelation {
    pub id: String,
    /// Always [`CANDIDATE_PREDICATE`].
    pub predicate: String,
    pub from: String,
    pub to: String,
    pub space: SpaceRef,
    /// Distance under the space's metric.
    pub score: f64,
    pub exactness: Exactness,
    /// Intersection of both embeddings' valid times.
    pub valid_time: Interval,
    /// The two embedding facts compared.
    pub origins: Vec<FactRef>,
}

fn label(value: &str) -> bool {
    !value.is_empty() && value.len() <= crate::provenance::MAX_LABEL_BYTES
}
fn invalid(message: &str) -> ContractError {
    ContractError {
        code: E_SPACE.into(),
        message: message.into(),
    }
}

impl EmbeddingSpace {
    pub fn reference(&self) -> SpaceRef {
        SpaceRef {
            id: self.id.clone(),
            revision: self.revision.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), ContractError> {
        let ok = [
            &self.id,
            &self.revision,
            &self.encoder,
            &self.encoder_version,
            &self.preprocessing,
        ]
        .iter()
        .all(|value| label(value))
            && (1..=MAX_DIMENSIONS).contains(&self.dimensions);
        ok.then_some(())
            .ok_or_else(|| invalid("bounded embedding space descriptor required"))
    }
    /// Exact descriptor equality. Equal dimensions alone never suffice.
    pub fn compatible(&self, other: &Self) -> Result<(), ContractError> {
        (self == other)
            .then_some(())
            .ok_or_else(|| invalid("embedding spaces differ"))
    }
}

impl EmbeddingFact {
    /// Validate this fact against its declared space.
    pub fn validate(&self, space: &EmbeddingSpace) -> Result<(), ContractError> {
        space.validate()?;
        if self.space != space.reference() {
            return Err(invalid("embedding names a different space"));
        }
        if !label(&self.id)
            || self.vector.len() != space.dimensions as usize
            || self.vector.iter().any(|v| !v.is_finite())
        {
            return Err(invalid(
                "embedding requires an ID and finite components of the declared dimension",
            ));
        }
        if !self.valid_time.valid() {
            return Err(ContractError {
                code: "E_INTERVAL".into(),
                message: "valid time end must exceed start".into(),
            });
        }
        if let Some(claim) = &self.provenance {
            claim.validate()?;
        }
        Ok(())
    }
}
