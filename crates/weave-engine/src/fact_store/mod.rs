//! Storage backend abstraction for the fact log, its indexes and checkpoints
//! ([ADR-0004](../../../docs/architecture/ADR-0004-fact-log-and-storage-backend.md)).
//!
//! [`FactBackend`] is the only path to fact-log storage. It is deliberately
//! semantics-free: fact bodies are opaque canonical JSON produced by the
//! engine, and the engine extracts the [`IndexKeys`]. Validation,
//! authorization, provenance merging and revision IDs stay in the engine.
//! The engine is also the only source of `writer` and `recorded_at_ms`; a
//! backend stores them verbatim.
//!
//! [`sqlite::SqliteFactStore`] is the production backend.
//! [`memory::MemoryFactStore`] is the non-durable reference backend. Both pass
//! the conformance suite in `tests/fact_store_conformance.rs`, which checks
//! every guarantee documented on [`FactBackend`].
use crate::{err, Result};

pub mod memory;
pub mod sqlite;

/// Largest accepted fact body or line header, in bytes.
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
/// Largest accepted checkpoint body, in bytes.
pub const MAX_CHECKPOINT_BYTES: usize = 256 * 1024 * 1024;
/// Largest accepted identifier, writer or index key, in UTF-8 bytes.
pub const MAX_KEY_BYTES: usize = 512;

/// Identity namespace of a fact within one graph (core v1 `identity.assertion`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Namespace {
    /// Node manifestation IDs.
    Node,
    /// Edges, structural edges, assertions, attachments and embeddings.
    Record,
    /// Embedding spaces, keyed `id@revision` (store 31).
    Space,
}

/// Kind of a stored fact. The declaration order is the materialization order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FactKind {
    Node,
    Edge,
    StructuralEdge,
    Assertion,
    Attachment,
    /// An embedding space descriptor (store 31).
    EmbeddingSpace,
    /// An embedding fact (store 31).
    Embedding,
}

impl FactKind {
    pub const ALL: [FactKind; 7] = [
        FactKind::Node,
        FactKind::Edge,
        FactKind::StructuralEdge,
        FactKind::Assertion,
        FactKind::Attachment,
        FactKind::EmbeddingSpace,
        FactKind::Embedding,
    ];
    pub fn namespace(self) -> Namespace {
        match self {
            FactKind::Node => Namespace::Node,
            FactKind::EmbeddingSpace => Namespace::Space,
            _ => Namespace::Record,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            FactKind::Node => "node",
            FactKind::Edge => "edge",
            FactKind::StructuralEdge => "structural_edge",
            FactKind::Assertion => "assertion",
            FactKind::Attachment => "attachment",
            FactKind::EmbeddingSpace => "embedding_space",
            FactKind::Embedding => "embedding",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
    /// True for the kinds an ordinary graph read materializes: every kind
    /// except embedding spaces and embeddings.
    pub fn is_graph(self) -> bool {
        !matches!(self, FactKind::EmbeddingSpace | FactKind::Embedding)
    }
}

impl Namespace {
    pub fn as_str(self) -> &'static str {
        match self {
            Namespace::Node => "node",
            Namespace::Record => "record",
            Namespace::Space => "space",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "node" => Some(Namespace::Node),
            "record" => Some(Namespace::Record),
            "space" => Some(Namespace::Space),
            _ => None,
        }
    }
}

/// Half-open valid interval `[start, end)`; `end: None` is unbounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidTime {
    pub start: i64,
    pub end: Option<i64>,
}

impl ValidTime {
    pub fn contains(&self, t: i64) -> bool {
        t >= self.start && self.end.is_none_or(|end| t < end)
    }
}

/// Index keys extracted from a fact body by the engine (ADR-0004 section 5).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexKeys {
    pub entity: Option<String>,
    pub subject: Option<String>,
    pub object: Option<String>,
    pub predicate: Option<String>,
    /// `None` means valid at every time.
    pub valid: Option<ValidTime>,
    /// The record carries whole-value influence (declared snapshot gates or
    /// attachment influence) that a filtered read must still account for.
    pub carrier: bool,
}

/// One version of a fact, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactVersion {
    pub kind: FactKind,
    pub id: String,
    /// Canonical JSON of the record, produced by the engine.
    pub body: String,
    pub keys: IndexKeys,
}

/// Engine-recorded part of a version's provenance (ADR-0005).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    /// The writing `HostContext.principal`, never a key or display name.
    pub writer: String,
    /// Reserved for B1 delegation (ADR-0007 section 13): the principal an
    /// agent writer acted for. Engine-recorded, outside content; `None` until B1.
    pub on_behalf_of: Option<String>,
    pub recorded_at_ms: i64,
    /// The revision that first recorded this version. Base copies keep the original.
    pub revision: String,
}

/// One operation of a fact revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactOp {
    /// Add a version. It supersedes a live fact with the same key and kind.
    Assert(FactVersion),
    /// End the live version of a key.
    Retract { kind: FactKind, id: String },
}

impl FactOp {
    pub fn kind(&self) -> FactKind {
        match self {
            FactOp::Assert(version) => version.kind,
            FactOp::Retract { kind, .. } => *kind,
        }
    }
    pub fn id(&self) -> &str {
        match self {
            FactOp::Assert(version) => &version.id,
            FactOp::Retract { id, .. } => id,
        }
    }
}

/// A fact revision to append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRevision {
    pub revision: String,
    pub graph_id: String,
    pub branch_id: String,
    pub parent: Option<String>,
    pub writer: String,
    /// Reserved for B1 delegation (ADR-0007 section 13): the principal an
    /// agent writer acted for. Engine-recorded, outside content; `None` until B1.
    pub on_behalf_of: Option<String>,
    pub recorded_at_ms: i64,
    pub ops: Vec<FactOp>,
}

/// State a new line starts from: the graph header and every live fact of the
/// parent, each with its original recording.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BaseState {
    pub header: String,
    pub facts: Vec<(FactVersion, Recording)>,
}

/// A stored fact revision and its place in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionInfo {
    pub revision: String,
    pub graph_id: String,
    pub branch_id: String,
    pub parent: Option<String>,
    pub line: i64,
    pub seq: i64,
    /// The revision's own operations occupy `[first_position, first_position + op_count)`.
    pub first_position: i64,
    pub op_count: i64,
    pub writer: String,
    /// Reserved for B1 delegation (ADR-0007 section 13): the principal an
    /// agent writer acted for. Engine-recorded, outside content; `None` until B1.
    pub on_behalf_of: Option<String>,
    pub recorded_at_ms: i64,
}

/// A version live at some revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFact {
    pub position: i64,
    pub version: FactVersion,
    pub recording: Recording,
}

/// One logged operation of a revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedOp {
    pub position: i64,
    pub op: FactOp,
}

/// Conjunctive selection over the facts live at a revision.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FactFilter {
    pub kind: Option<FactKind>,
    pub entity: Option<String>,
    pub subject: Option<String>,
    pub object: Option<String>,
    pub predicate: Option<String>,
    /// Facts whose valid time contains this instant (facts without one always match).
    pub valid_at: Option<i64>,
    /// Facts whose live version was recorded in `[start, end)` milliseconds.
    pub recorded: Option<(i64, i64)>,
    /// Only influence carriers.
    pub carrier: bool,
    /// Only facts with neither a subject nor an entity key (graph-hosted attachments).
    pub unanchored: bool,
    /// Only graph facts: no embedding spaces and no embeddings (store 31).
    pub graph_only: bool,
}

impl FactFilter {
    /// True when `fact` satisfies every set criterion. Backends may use this
    /// after an index scan; it defines the filter's meaning.
    pub fn matches(&self, fact: &StoredFact) -> bool {
        let keys = &fact.version.keys;
        let same = |want: &Option<String>, have: &Option<String>| {
            want.as_ref().is_none_or(|w| have.as_ref() == Some(w))
        };
        self.kind.is_none_or(|k| k == fact.version.kind)
            && same(&self.entity, &keys.entity)
            && same(&self.subject, &keys.subject)
            && same(&self.object, &keys.object)
            && same(&self.predicate, &keys.predicate)
            && self
                .valid_at
                .is_none_or(|t| keys.valid.is_none_or(|v| v.contains(t)))
            && (!self.carrier || keys.carrier)
            && (!self.unanchored || (keys.subject.is_none() && keys.entity.is_none()))
            && (!self.graph_only || fact.version.kind.is_graph())
            && self.recorded.is_none_or(|(start, end)| {
                fact.recording.recorded_at_ms >= start && fact.recording.recorded_at_ms < end
            })
    }
}

/// Storage for fact revisions, their log, indexes and checkpoints.
///
/// Guarantees every implementation MUST meet (ADR-0004 section 6):
///
/// - **Atomicity.** Writes between [`begin`](Self::begin) and
///   [`commit`](Self::commit) become visible together; after
///   [`rollback`](Self::rollback) none of them is visible. A failed `append`
///   changes nothing.
/// - **Durability.** When [`durable`](Self::durable) is true, committed data
///   survives process exit.
/// - **Isolation.** Reads see committed state plus the caller's own
///   uncommitted writes.
/// - **Immutability.** Logged operations never change; only derived index
///   metadata and checkpoints change.
/// - **Positions** strictly increase in commit order, are contiguous within one
///   revision and are never reassigned.
/// - **Recorded time.** `writer` and `recorded_at_ms` are stored verbatim and
///   never generated. Along a line, `recorded_at_ms` never decreases
///   (`E_HISTORY_CLOCK`).
/// - **Lines.** A revision whose parent is the tip of a line continues it and
///   takes no base state. Any other revision starts a line and requires one
///   (`E_INTEGRITY` otherwise). A revision ID is never reused (`E_CONFLICT`).
/// - **Operations.** A revision has at least one operation, and its keys are
///   distinct. A retract needs a live key; a retract or superseding assert must
///   match the live fact's kind (`E_CONFLICT`).
/// - **Reads.** [`get`](Self::get) and [`select`](Self::select) return exactly
///   the versions live at the revision, in `(kind, id)` order. `select` fails
///   with `E_BUDGET` instead of truncating. Reads of an unknown revision fail
///   with `E_UNAVAILABLE`.
pub trait FactBackend {
    /// Whether committed data survives process exit.
    fn durable(&self) -> bool;
    fn begin(&mut self) -> Result<()>;
    fn commit(&mut self) -> Result<()>;
    fn rollback(&mut self) -> Result<()>;
    /// The stored fact revision, or `None` when `revision` is not a fact revision.
    fn revision(&self, revision: &str) -> Result<Option<RevisionInfo>>;
    /// True when `revision` is a fact revision with no successor on its line.
    fn is_tip(&self, revision: &str) -> Result<bool>;
    /// Append a revision. `base` is required exactly when it starts a line.
    fn append(&mut self, revision: NewRevision, base: Option<BaseState>) -> Result<RevisionInfo>;
    /// The graph header of the revision's line.
    fn header(&self, revision: &str) -> Result<String>;
    /// The version of a key live at a revision.
    fn get(&self, revision: &str, namespace: Namespace, id: &str) -> Result<Option<StoredFact>>;
    /// Every version live at a revision that matches `filter`, at most `limit`.
    fn select(&self, revision: &str, filter: &FactFilter, limit: usize) -> Result<Vec<StoredFact>>;
    /// Visit every version live at a revision that matches `filter`, in no
    /// particular order, without materializing them (store 31, A4). A visitor
    /// error stops the scan and is returned. The default materializes; a
    /// production backend streams.
    fn scan(
        &self,
        revision: &str,
        filter: &FactFilter,
        visit: &mut dyn FnMut(StoredFact) -> Result<()>,
    ) -> Result<()> {
        for fact in self.select(revision, filter, usize::MAX - 1)? {
            visit(fact)?;
        }
        Ok(())
    }
    /// The revision's own operations, in position order.
    fn ops(&self, revision: &str) -> Result<Vec<LoggedOp>>;
    /// Every key asserted or retracted between two states of `to`'s line, in
    /// `(namespace, id)` order, at most `limit` (`E_BUDGET` beyond). `from`
    /// is another revision of that line (either order), or the line's base
    /// parent (`None` for a line started from an empty graph). Any other
    /// `from` gives `Ok(None)`: the two states share no line. A key whose live
    /// version is the same in both states may still be listed; a key whose
    /// live version differs always is.
    fn changed_keys(
        &self,
        from: Option<&str>,
        to: &str,
        limit: usize,
    ) -> Result<Option<Vec<(Namespace, String)>>>;
    /// Cache a materialized checkpoint body for a revision, replacing any prior one.
    fn put_checkpoint(&mut self, revision: &str, body: &str) -> Result<()>;
    fn checkpoint(&self, revision: &str) -> Result<Option<String>>;
    /// Remove a cached checkpoint; true when one existed.
    fn drop_checkpoint(&mut self, revision: &str) -> Result<bool>;
}

pub(crate) fn bounded(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_KEY_BYTES
}

/// Shared input validation, so every backend rejects the same malformed input.
pub(crate) fn validate_new(revision: &NewRevision, base: Option<&BaseState>) -> Result<()> {
    let ids_ok = bounded(&revision.revision)
        && bounded(&revision.graph_id)
        && bounded(&revision.branch_id)
        && bounded(&revision.writer)
        && revision.on_behalf_of.as_deref().is_none_or(bounded)
        && revision.parent.as_deref().is_none_or(bounded)
        && revision.recorded_at_ms >= 0;
    if !ids_ok {
        return Err(err(
            "E_ID",
            "fact revision identifiers require 1 to 512 bytes and a nonnegative time",
        ));
    }
    if revision.ops.is_empty() {
        return Err(err("E_CONFLICT", "a fact revision needs an operation"));
    }
    let mut keys = std::collections::HashSet::new();
    for op in &revision.ops {
        if !keys.insert((op.kind().namespace(), op.id())) {
            return Err(err("E_CONFLICT", "a fact key occurs twice in one revision"));
        }
        if let FactOp::Assert(version) = op {
            validate_version(version)?;
        } else if !bounded(op.id()) {
            return Err(err("E_ID", "fact identifiers require 1 to 512 bytes"));
        }
    }
    if let Some(base) = base {
        if base.header.len() > MAX_BODY_BYTES {
            return Err(err("E_BUDGET", "line header exceeds 4 MiB"));
        }
        let mut keys = std::collections::HashSet::new();
        for (version, recording) in &base.facts {
            validate_version(version)?;
            if !bounded(&recording.writer)
                || recording
                    .on_behalf_of
                    .as_deref()
                    .is_some_and(|p| !bounded(p))
                || !bounded(&recording.revision)
                || recording.recorded_at_ms < 0
            {
                return Err(err("E_ID", "base recording requires bounded identifiers"));
            }
            if !keys.insert((version.kind.namespace(), version.id.as_str())) {
                return Err(err("E_CONFLICT", "a fact key occurs twice in a base state"));
            }
        }
    }
    Ok(())
}

fn validate_version(version: &FactVersion) -> Result<()> {
    let keys = &version.keys;
    let optional = |v: &Option<String>| v.as_deref().is_none_or(bounded);
    if !bounded(&version.id)
        || !optional(&keys.entity)
        || !optional(&keys.subject)
        || !optional(&keys.object)
        || !optional(&keys.predicate)
    {
        return Err(err("E_ID", "fact keys require 1 to 512 bytes"));
    }
    if keys
        .valid
        .is_some_and(|v| v.end.is_some_and(|end| end <= v.start))
    {
        return Err(err("E_INTERVAL", "valid time end must exceed start"));
    }
    if version.body.len() > MAX_BODY_BYTES {
        return Err(err("E_BUDGET", "fact body exceeds 4 MiB"));
    }
    Ok(())
}

/// Sort key that defines the `(kind, id)` read order.
pub(crate) fn order(facts: &mut [StoredFact]) {
    facts.sort_by(|a, b| (a.version.kind, &a.version.id).cmp(&(b.version.kind, &b.version.id)));
}
