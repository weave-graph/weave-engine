//! Fact-level writes (protocol 0.22.0, ADR-0004).
//!
//! A `write` command asserts, corrects or retracts individual facts of one
//! graph branch in one atomic revision. Its revision ID is content-derived
//! from the graph, branch, parent and the effective operations; the writer
//! and recorded time are engine-recorded (ADR-0005) and never accepted here.
//!
//! From 0.24.0 (A4) a write also carries embedding spaces and embedding facts
//! (ADR-0006). A space is immutable: it is asserted once and never corrected
//! or retracted.
use crate::embedding::{EmbeddingFact, EmbeddingSpace};
use crate::{Assertion, Edge, MetadataAttachment, Node, StructuralEdge};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;

/// Most changes one `write` command may carry.
pub const MAX_WRITE_CHANGES: usize = 100_000;

/// One fact record, tagged by `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fact {
    Node(Node),
    Edge(Edge),
    StructuralEdge(StructuralEdge),
    Assertion(Assertion),
    Attachment(MetadataAttachment),
    /// An embedding space descriptor (0.24, ADR-0006). Immutable once written.
    EmbeddingSpace(EmbeddingSpace),
    /// An embedding fact (0.24, ADR-0006).
    Embedding(EmbeddingFact),
}

/// The kind of a fact record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    Node,
    Edge,
    StructuralEdge,
    Assertion,
    Attachment,
    EmbeddingSpace,
    Embedding,
}

impl Fact {
    pub fn kind(&self) -> FactKind {
        match self {
            Fact::Node(_) => FactKind::Node,
            Fact::Edge(_) => FactKind::Edge,
            Fact::StructuralEdge(_) => FactKind::StructuralEdge,
            Fact::Assertion(_) => FactKind::Assertion,
            Fact::Attachment(_) => FactKind::Attachment,
            Fact::EmbeddingSpace(_) => FactKind::EmbeddingSpace,
            Fact::Embedding(_) => FactKind::Embedding,
        }
    }
    /// The record's `id` field. For an embedding space this is the space ID
    /// alone; its fact key is [`Fact::key_id`].
    pub fn id(&self) -> &str {
        match self {
            Fact::Node(n) => &n.id,
            Fact::Edge(e) => &e.id,
            Fact::StructuralEdge(e) => &e.id,
            Fact::Assertion(a) => &a.id,
            Fact::Attachment(a) => &a.id,
            Fact::EmbeddingSpace(s) => &s.id,
            Fact::Embedding(e) => &e.id,
        }
    }
    /// The ID that names this fact in `write` targets and change sets: the
    /// record's `id`, or `id@revision` for an embedding space.
    pub fn key_id(&self) -> Cow<'_, str> {
        match self {
            Fact::EmbeddingSpace(s) => Cow::Owned(s.reference().key()),
            other => Cow::Borrowed(other.id()),
        }
    }
}

/// Names one fact of the graph being written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactTarget {
    pub kind: FactKind,
    pub id: String,
}

/// Optional per-fact optimistic concurrency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FactExpectation {
    /// The fact must not be live at the branch head.
    Absent,
    /// The fact's live version at the head must be the version live at `revision`.
    UnchangedSince { revision: String },
}

/// One change of a `write` command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case", deny_unknown_fields)]
pub enum FactChange {
    /// Add a fact that is not live. Re-asserting the identical live version is a no-op.
    Assert { fact: Fact },
    /// Replace the live version of a fact with the same kind and ID.
    Correct {
        fact: Fact,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected: Option<FactExpectation>,
    },
    /// End the live version of a fact. History keeps it.
    Retract {
        target: FactTarget,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected: Option<FactExpectation>,
    },
}

/// `expected_head` of a `write`: absent means no head check, `null` requires
/// a new branch, and a string must equal the current head.
pub mod expected_head {
    use super::*;
    pub fn serialize<S: Serializer>(
        value: &Option<Option<String>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(inner) => inner.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Option<String>>, D::Error> {
        Option::<String>::deserialize(deserializer).map(Some)
    }
}
