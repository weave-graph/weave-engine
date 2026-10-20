//! Embedding spaces, embedding facts and candidate relations (ADR-0006, core contract v1).
//!
//! Protocol 0.24.0 (A4) activates them on the wire: `GraphData.embedding_spaces`
//! and `GraphData.embeddings`, the `embedding_space` and `embedding` fact kinds
//! of `write`, the `nearest` and `similar_candidates` expressions, and the
//! result fields `QueryResult.neighbors`, `QueryResult.candidates` and
//! `QueryResult.exactness`.
use crate::provenance::{ClaimedProvenance, FactRef};
use crate::wire::ContractError;
use crate::{main_branch, AssertionRef, GraphExpression, GraphRef, Interval, NodeRef};
use serde::{Deserialize, Serialize};

/// Maximum vector dimensions (matches the geometry library bound).
pub const MAX_DIMENSIONS: u32 = 4096;
/// Error code for an invalid or mismatched embedding space or vector.
pub const E_SPACE: &str = "E_SPACE";
/// Error code for a malformed `nearest` or `similar_candidates` expression.
pub const E_NEAREST: &str = "E_NEAREST";
/// Reserved predicate of candidate relations. The `weave:` namespace is engine-owned.
pub const CANDIDATE_PREDICATE: &str = "weave:similar_to";
/// Most neighbors one `nearest` may return, and most candidates per source.
pub const MAX_K: u32 = 1_000;
/// Default `limit` of `similar_candidates` (candidates returned).
pub const DEFAULT_CANDIDATE_LIMIT: u32 = 1_000;
/// Largest `limit` of `similar_candidates`.
pub const MAX_CANDIDATE_LIMIT: u32 = 100_000;
/// Scan budget of one `nearest`: the most vector components (stored
/// embeddings scanned times the space's dimensions) it may read, 2^28. That
/// is about 174,000 vectors of 1,536 dimensions; more fails with `E_BUDGET`.
/// It replaces the read budget for the vectors a ranking streams through.
pub const MAX_SCAN_COMPONENTS: u64 = 1 << 28;
/// Most source-target comparisons one `similar_candidates` may make.
pub const MAX_COMPARISONS: u64 = 50_000_000;
/// Separator of a space's fact key (`id@revision`). A space revision never contains it.
pub const SPACE_KEY_SEPARATOR: char = '@';

/// Node type of a document chunk (A4 convention). A chunk is an ordinary
/// node; the engine gives the type no special meaning.
pub const CHUNK_TYPE: &str = "weave.DocumentChunk";
/// Chunk property: URI of the text artifact the chunk was cut from.
pub const CHUNK_ARTIFACT_URI: &str = "artifact_uri";
/// Chunk property: digest of that artifact.
pub const CHUNK_ARTIFACT_DIGEST: &str = "artifact_digest";
/// Chunk property: first UTF-8 byte offset of the chunk in the artifact.
pub const CHUNK_START: &str = "start";
/// Chunk property: end (exclusive) UTF-8 byte offset of the chunk.
pub const CHUNK_END: &str = "end";
/// Predicate of an edge from a chunk to a node of an entity it mentions.
pub const MENTIONS_PREDICATE: &str = "weave:mentions";

/// Distance function of a space. Scores are distances: lower is closer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingMetric {
    /// `1 - cos(a, b)`, in `[0, 2]`.
    CosineDistance,
    /// `sqrt(sum((a_i - b_i)^2))`.
    Euclidean,
}

impl EmbeddingMetric {
    /// The normative distance between two vectors of equal length.
    ///
    /// Components are widened to binary64 exactly, and every sum is taken in
    /// component order, so the result is bit-for-bit reproducible:
    /// - `euclidean`: `sqrt(sum((a_i - b_i)^2))`;
    /// - `cosine_distance`: `1 - dot(a, b) / (sqrt(dot(a, a)) * sqrt(dot(b, b)))`,
    ///   clamped to `[0, 2]`.
    pub fn distance(self, a: &[f32], b: &[f32]) -> f64 {
        debug_assert_eq!(a.len(), b.len());
        match self {
            Self::Euclidean => {
                let mut sum = 0.0f64;
                for (x, y) in a.iter().zip(b) {
                    let d = f64::from(*x) - f64::from(*y);
                    sum += d * d;
                }
                sum.sqrt()
            }
            Self::CosineDistance => {
                let (mut dot, mut na, mut nb) = (0.0f64, 0.0f64, 0.0f64);
                for (x, y) in a.iter().zip(b) {
                    let (x, y) = (f64::from(*x), f64::from(*y));
                    dot += x * y;
                    na += x * x;
                    nb += y * y;
                }
                (1.0 - dot / (na.sqrt() * nb.sqrt())).clamp(0.0, 2.0)
            }
        }
    }
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

impl SpaceRef {
    /// The space's fact key within a graph: `id@revision`. It is unambiguous
    /// because a valid space revision never contains `@`.
    pub fn key(&self) -> String {
        format!("{}{SPACE_KEY_SEPARATOR}{}", self.id, self.revision)
    }
}

/// What an embedding describes. IDs are graph-local, as in `GraphData`.
///
/// The derived order (kind, then ID) is the tie-break order of rankings.
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

impl EmbeddingSubject {
    /// The subject's graph-local ID (entity, node, edge or assertion ID).
    pub fn id(&self) -> &str {
        match self {
            Self::Entity { entity_id } => entity_id,
            Self::Node { node_id } => node_id,
            Self::Edge { edge_id } => edge_id,
            Self::Assertion { assertion_id } => assertion_id,
        }
    }
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
    /// Group references (1.1.0, ADR-0007 section 2): a disjunction alongside
    /// `readers`, resolved against current group membership when a request
    /// runs. Omitted when empty, so records without groups keep their bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reader_groups: Vec<String>,
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

/// The query of a `nearest` expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NearestQuery {
    /// A caller-supplied vector in the plan's space (rounded to binary32 on decode).
    Vector { vector: Vec<f32> },
    /// An embedding fact of the same graph revision, which must be visible.
    /// Its own subject is excluded from the result.
    Embedding { embedding_id: String },
}

/// Exact k-nearest-neighbor search over the embeddings of one space
/// (`kind: "nearest"`, 0.24).
///
/// Selection comes before ranking: only embeddings of `space` that are valid
/// at `valid_at` (when set), whose subject is in `candidates` (when set), and
/// that the evaluating principal may read together with their subject are
/// ranked. Hidden embeddings never change the rank, score or count of
/// visible ones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NearestPlan {
    pub graph_id: String,
    #[serde(default = "main_branch")]
    pub branch_id: String,
    /// An exact revision. Absent means the branch head, or `as_of` when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Read the branch as this replica knew it at this recorded time (Unix ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<i64>,
    pub space: SpaceRef,
    pub query: NearestQuery,
    /// Neighbors to return (1 to 1,000).
    pub k: u32,
    /// Rank only embeddings whose valid time contains this instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_at: Option<i64>,
    /// Rank only embeddings whose subject is a record of this value: its
    /// nodes, their entities, and its edges, by origin in `graph_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Box<GraphExpression>>,
}

/// Candidate similarity relations between the embeddings of one space
/// (`kind: "similar_candidates"`, 0.24).
///
/// For every visible source embedding, its `k` nearest visible target
/// embeddings of another subject become candidates when their distance is at
/// most `max_score`. A pair is reported once, from the lower subject to the
/// higher. Candidates are result values, never facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidatesPlan {
    pub graph_id: String,
    #[serde(default = "main_branch")]
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<i64>,
    pub space: SpaceRef,
    /// Candidates per source embedding (1 to 1,000).
    pub k: u32,
    /// Keep only pairs at most this distance apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_at: Option<i64>,
    /// Subjects to find candidates for; absent means every embedding of the space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Box<GraphExpression>>,
    /// Subjects candidates may point to; absent means every embedding of the space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Box<GraphExpression>>,
    /// Drop pairs of nodes already joined by a visible edge (either direction,
    /// valid at `valid_at` when set), so only relations absent from the graph remain.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exclude_linked: bool,
    /// Most candidates the result may hold; more fails with `E_BUDGET`.
    #[serde(default = "default_candidate_limit")]
    pub limit: u32,
}

fn default_candidate_limit() -> u32 {
    DEFAULT_CANDIDATE_LIMIT
}

/// One ranked hit of a `nearest` expression (`QueryResult.neighbors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Neighbor {
    /// 1-based rank among the visible, selected embeddings.
    pub rank: u32,
    pub embedding_id: String,
    pub subject: EmbeddingSubject,
    /// Distance under the space's metric (lower is closer).
    pub score: f64,
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
            && !self.revision.contains(SPACE_KEY_SEPARATOR)
            && self.reference().key().len() <= crate::provenance::MAX_LABEL_BYTES
            && (1..=MAX_DIMENSIONS).contains(&self.dimensions);
        ok.then_some(()).ok_or_else(|| {
            invalid("bounded embedding space descriptor required (a revision has no '@')")
        })
    }
    /// Exact descriptor equality. Equal dimensions alone never suffice.
    pub fn compatible(&self, other: &Self) -> Result<(), ContractError> {
        (self == other)
            .then_some(())
            .ok_or_else(|| invalid("embedding spaces differ"))
    }
    /// Validate a vector (a query or a fact's) against this space.
    pub fn validate_vector(&self, vector: &[f32]) -> Result<(), ContractError> {
        if vector.len() != self.dimensions as usize || vector.iter().any(|v| !v.is_finite()) {
            return Err(invalid(
                "a vector needs finite components of the declared dimension",
            ));
        }
        if self.metric == EmbeddingMetric::CosineDistance && vector.iter().all(|v| *v == 0.0) {
            return Err(invalid("cosine distance requires a nonzero vector"));
        }
        Ok(())
    }
}

impl EmbeddingFact {
    /// Validate this fact against its declared space.
    pub fn validate(&self, space: &EmbeddingSpace) -> Result<(), ContractError> {
        space.validate()?;
        if self.space != space.reference() {
            return Err(invalid("embedding names a different space"));
        }
        if !label(&self.id) || !label(self.subject.id()) {
            return Err(invalid("embedding requires bounded ID and subject"));
        }
        space.validate_vector(&self.vector)?;
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
