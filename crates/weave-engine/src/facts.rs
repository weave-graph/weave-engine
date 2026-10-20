//! Fact-level writes, fact-revision reads and per-fact provenance
//! (ADR-0004, ADR-0005). Storage goes through [`crate::fact_store`] only.
use super::*;
use crate::fact_store::{
    self as store, sqlite::SqliteFactStore, BaseState, FactBackend, FactFilter, FactOp,
    FactVersion, IndexKeys, NewRevision, Recording, ValidTime,
};
use std::collections::{BTreeSet, HashMap};
use weave_contract::embedding::{EmbeddingSpace, EmbeddingSubject};
use weave_contract::facts::MAX_WRITE_CHANGES;
use weave_contract::provenance::{
    ClaimedProvenance, EvidenceRef, FactProvenance, FactRef, Method, ProvenanceEntry,
};

/// Prefix of every fact revision ID.
pub(crate) const FACT_PREFIX: &str = "fact:sha256:";
/// Writer recorded for base copies of snapshot facts whose revision has no commit event.
const UNRECORDED: &str = "weave:unrecorded";

/// The `reader_groups` of a fact record (spaces have none).
pub(crate) fn fact_reader_groups(fact: &Fact) -> &[String] {
    match fact {
        Fact::Node(n) => &n.reader_groups,
        Fact::Edge(e) => &e.reader_groups,
        Fact::StructuralEdge(e) => &e.reader_groups,
        Fact::Assertion(a) => &a.reader_groups,
        Fact::Attachment(a) => &a.reader_groups,
        Fact::EmbeddingSpace(_) => &[],
        Fact::Embedding(e) => &e.reader_groups,
    }
}

/// The `readers` of a fact record (spaces have none).
fn fact_readers(fact: &Fact) -> &[String] {
    match fact {
        Fact::Node(n) => &n.readers,
        Fact::Edge(e) => &e.readers,
        Fact::StructuralEdge(e) => &e.readers,
        Fact::Assertion(a) => &a.readers,
        Fact::Attachment(a) => &a.readers,
        Fact::EmbeddingSpace(_) => &[],
        Fact::Embedding(e) => &e.readers,
    }
}

/// The stored `on_behalf_of` column: a JSON array of principal IDs, the
/// agent writer's owner first (ADR-0007 section 8). Absent for every
/// non-delegated write.
pub(crate) fn encode_delegation(chain: &[String]) -> Result<Option<String>> {
    if chain.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::to_string(chain)?))
}
pub(crate) fn decode_delegation(stored: Option<&str>) -> Result<Vec<String>> {
    match stored {
        None => Ok(Vec::new()),
        Some(text) => {
            let chain: Vec<String> = serde_json::from_str(text)
                .map_err(|_| err("E_INTEGRITY", "stored delegation chain is malformed"))?;
            if chain.is_empty()
                || chain.len() > weave_contract::provenance::MAX_DELEGATION_CHAIN
                || !chain.iter().all(|p| valid_id(p))
            {
                return Err(err("E_INTEGRITY", "stored delegation chain is malformed"));
            }
            Ok(chain)
        }
    }
}

/// Every `reader_groups` entry of a graph value's records.
pub(crate) fn data_reader_groups(data: &GraphData) -> impl Iterator<Item = &String> {
    data.nodes
        .iter()
        .flat_map(|n| &n.reader_groups)
        .chain(data.edges.iter().flat_map(|e| &e.reader_groups))
        .chain(data.structural_edges.iter().flat_map(|e| &e.reader_groups))
        .chain(data.assertions.iter().flat_map(|a| &a.reader_groups))
        .chain(data.attachments.iter().flat_map(|a| &a.reader_groups))
        .chain(data.embeddings.iter().flat_map(|e| &e.reader_groups))
}

pub(crate) fn is_fact_revision(revision: &str) -> bool {
    revision.starts_with(FACT_PREFIX)
}

pub(crate) fn store_kind(kind: FactKind) -> store::FactKind {
    match kind {
        FactKind::Node => store::FactKind::Node,
        FactKind::Edge => store::FactKind::Edge,
        FactKind::StructuralEdge => store::FactKind::StructuralEdge,
        FactKind::Assertion => store::FactKind::Assertion,
        FactKind::Attachment => store::FactKind::Attachment,
        FactKind::EmbeddingSpace => store::FactKind::EmbeddingSpace,
        FactKind::Embedding => store::FactKind::Embedding,
    }
}
pub(crate) fn contract_kind(kind: store::FactKind) -> FactKind {
    match kind {
        store::FactKind::Node => FactKind::Node,
        store::FactKind::Edge => FactKind::Edge,
        store::FactKind::StructuralEdge => FactKind::StructuralEdge,
        store::FactKind::Assertion => FactKind::Assertion,
        store::FactKind::Attachment => FactKind::Attachment,
        store::FactKind::EmbeddingSpace => FactKind::EmbeddingSpace,
        store::FactKind::Embedding => FactKind::Embedding,
    }
}
pub(crate) fn namespace(kind: FactKind) -> store::Namespace {
    store_kind(kind).namespace()
}
pub(crate) type Key = (store::Namespace, String);
pub(crate) fn key(fact: &Fact) -> Key {
    (namespace(fact.kind()), fact.key_id().into_owned())
}

fn valid(interval: &Interval) -> Option<ValidTime> {
    Some(ValidTime {
        start: interval.start,
        end: interval.end,
    })
}

/// Index keys of a record (ADR-0004 section 5).
fn index_keys(fact: &Fact) -> IndexKeys {
    IndexKeys {
        carrier: is_carrier(fact),
        ..anchor_keys(fact)
    }
}

/// Records whose influence survives filtering (`collect_declared_snapshots`
/// and `input_influence` read them from the whole authorized graph value).
fn is_carrier(fact: &Fact) -> bool {
    match fact {
        Fact::Node(n) => !n.derived_snapshots.is_empty(),
        Fact::Edge(e) => !e.derived_snapshots.is_empty(),
        Fact::StructuralEdge(_) => false,
        Fact::Assertion(a) => !a.derived_snapshots.is_empty(),
        Fact::Attachment(a) => {
            !a.derived_snapshots.is_empty()
                || !a.derived_from.is_empty()
                || !a.derived_nodes.is_empty()
        }
        // Embedding gates restrict only the embedding itself.
        Fact::EmbeddingSpace(_) | Fact::Embedding(_) => false,
    }
}

fn anchor_keys(fact: &Fact) -> IndexKeys {
    match fact {
        Fact::Node(n) => IndexKeys {
            entity: Some(n.entity_id.clone()),
            subject: Some(n.id.clone()),
            valid: n.valid_time.as_ref().and_then(valid),
            ..IndexKeys::default()
        },
        Fact::Edge(e) => IndexKeys {
            subject: Some(e.from.clone()),
            object: Some(e.to.clone()),
            predicate: Some(e.predicate.clone()),
            valid: valid(&e.valid_time),
            ..IndexKeys::default()
        },
        Fact::StructuralEdge(e) => IndexKeys {
            subject: Some(e.from.clone()),
            object: Some(e.to.clone()),
            predicate: Some(e.predicate.clone()),
            ..IndexKeys::default()
        },
        Fact::Assertion(a) => IndexKeys {
            subject: Some(a.edge_id.clone()),
            valid: valid(&a.valid_time),
            ..IndexKeys::default()
        },
        Fact::Attachment(a) => {
            let (entity, subject) = match &a.host {
                MetadataHost::Entity { id } => (Some(id.clone()), None),
                MetadataHost::Node { id }
                | MetadataHost::Edge { id }
                | MetadataHost::Assertion { id } => (None, Some(id.clone())),
                MetadataHost::Graph => (None, None),
            };
            IndexKeys {
                entity,
                subject,
                predicate: Some(a.key.clone()),
                valid: valid(&a.valid_time),
                ..IndexKeys::default()
            }
        }
        Fact::EmbeddingSpace(_) => IndexKeys::default(),
        // Embeddings are selected by space (`predicate`) and subject.
        Fact::Embedding(e) => {
            let (entity, subject) = match &e.subject {
                EmbeddingSubject::Entity { entity_id } => (Some(entity_id.clone()), None),
                other => (None, Some(other.id().to_string())),
            };
            IndexKeys {
                entity,
                subject,
                predicate: Some(e.space.key()),
                valid: valid(&e.valid_time),
                ..IndexKeys::default()
            }
        }
    }
}

pub(crate) fn version(fact: &Fact) -> Result<FactVersion> {
    Ok(FactVersion {
        kind: store_kind(fact.kind()),
        id: fact.key_id().into_owned(),
        body: serde_json::to_string(fact)?,
        keys: index_keys(fact),
    })
}

pub(crate) fn parse(version: &FactVersion) -> Result<Fact> {
    let fact: Fact = serde_json::from_str(&version.body)
        .map_err(|_| err("E_INTEGRITY", "stored fact is malformed"))?;
    if fact.kind() != contract_kind(version.kind) || fact.key_id() != version.id {
        return Err(err("E_INTEGRITY", "stored fact does not match its key"));
    }
    Ok(fact)
}

/// The graph-level fields of a graph value, with every fact list empty.
pub(crate) fn header_of(data: &GraphData) -> GraphData {
    GraphData {
        nodes: vec![],
        edges: vec![],
        structural_edges: vec![],
        assertions: vec![],
        attachments: vec![],
        embedding_spaces: vec![],
        embeddings: vec![],
        ..data.clone()
    }
}

pub(crate) fn facts_of(data: &GraphData) -> Vec<Fact> {
    data.nodes
        .iter()
        .cloned()
        .map(Fact::Node)
        .chain(data.edges.iter().cloned().map(Fact::Edge))
        .chain(
            data.structural_edges
                .iter()
                .cloned()
                .map(Fact::StructuralEdge),
        )
        .chain(data.assertions.iter().cloned().map(Fact::Assertion))
        .chain(data.attachments.iter().cloned().map(Fact::Attachment))
        .chain(
            data.embedding_spaces
                .iter()
                .cloned()
                .map(Fact::EmbeddingSpace),
        )
        .chain(data.embeddings.iter().cloned().map(Fact::Embedding))
        .collect()
}

pub(crate) fn push(data: &mut GraphData, fact: Fact) {
    match fact {
        Fact::Node(n) => data.nodes.push(n),
        Fact::Edge(e) => data.edges.push(e),
        Fact::StructuralEdge(e) => data.structural_edges.push(e),
        Fact::Assertion(a) => data.assertions.push(a),
        Fact::Attachment(a) => data.attachments.push(a),
        Fact::EmbeddingSpace(s) => data.embedding_spaces.push(s),
        Fact::Embedding(e) => data.embeddings.push(e),
    }
}

/// The asserted and corrected records of a write, as a graph value (for
/// checks that inspect written records, such as handler egress).
pub(crate) fn asserted_data(changes: &[FactChange]) -> GraphData {
    let mut data = GraphData::default();
    for change in changes {
        if let FactChange::Assert { fact } | FactChange::Correct { fact, .. } = change {
            push(&mut data, fact.clone());
        }
    }
    data
}

/// True when a graph value uses a 0.22 field (node valid time or claimed provenance).
pub(crate) fn requires_v022(data: &GraphData) -> bool {
    data.nodes
        .iter()
        .any(|n| n.valid_time.is_some() || n.provenance.is_some())
        || data.edges.iter().any(|e| e.provenance.is_some())
        || data.structural_edges.iter().any(|e| e.provenance.is_some())
        || data.assertions.iter().any(|a| a.provenance.is_some())
        || data.attachments.iter().any(|a| a.provenance.is_some())
}

/// True when a graph value uses a 0.24 field (embedding spaces or embeddings).
pub(crate) fn requires_v024(data: &GraphData) -> bool {
    !data.embedding_spaces.is_empty() || !data.embeddings.is_empty()
}

/// True when a claim cites an embedding as evidence (0.24).
pub(crate) fn cites_embedding(claim: &Option<ClaimedProvenance>) -> bool {
    claim.as_ref().is_some_and(|c| {
        c.evidence
            .iter()
            .any(|e| matches!(e, EvidenceRef::Embedding { .. }))
    })
}

/// True when any record of a graph value cites an embedding as evidence (0.24).
pub(crate) fn data_cites_embedding(data: &GraphData) -> bool {
    facts_of(data)
        .iter()
        .any(|fact| cites_embedding(&claim_field(fact)))
}

/// The inline claim of a record, if it has one.
pub(crate) fn claim_field(fact: &Fact) -> Option<ClaimedProvenance> {
    match fact {
        Fact::Node(n) => n.provenance.clone(),
        Fact::Edge(e) => e.provenance.clone(),
        Fact::StructuralEdge(e) => e.provenance.clone(),
        Fact::Assertion(a) => a.provenance.clone(),
        Fact::Attachment(a) => a.provenance.clone(),
        Fact::EmbeddingSpace(_) => None,
        Fact::Embedding(e) => e.provenance.clone(),
    }
}

/// The claim recorded for a record: its inline `provenance`, else its legacy
/// claimed source label with method `unspecified` (ADR-0005 migration rule).
fn claim_of(fact: &Fact) -> Option<ClaimedProvenance> {
    let (claim, legacy) = match fact {
        Fact::Node(n) => (&n.provenance, None),
        Fact::Edge(e) => (&e.provenance, e.assertion_source.clone()),
        Fact::StructuralEdge(e) => (&e.provenance, None),
        Fact::Assertion(a) => (&a.provenance, Some(a.source.clone())),
        Fact::Attachment(a) => (&a.provenance, None),
        Fact::EmbeddingSpace(_) => return None,
        Fact::Embedding(e) => (&e.provenance, None),
    };
    claim.clone().or_else(|| {
        legacy.map(|source| ClaimedProvenance {
            source: Some(source),
            method: Method::Unspecified,
            evidence: vec![],
        })
    })
}

/// Validate node valid time and claimed provenance on every record.
pub(crate) fn validate_claims(data: &GraphData) -> Result<()> {
    if data
        .nodes
        .iter()
        .any(|n| n.valid_time.as_ref().is_some_and(|v| !v.valid()))
    {
        return Err(err("E_INTERVAL", "valid time end must exceed start"));
    }
    for fact in facts_of(data) {
        let (claim, legacy) = match &fact {
            Fact::Node(n) => (&n.provenance, None),
            Fact::Edge(e) => (&e.provenance, e.assertion_source.as_ref()),
            Fact::StructuralEdge(e) => (&e.provenance, None),
            Fact::Assertion(a) => (&a.provenance, Some(&a.source)),
            Fact::Attachment(a) => (&a.provenance, None),
            Fact::EmbeddingSpace(_) => continue,
            Fact::Embedding(e) => (&e.provenance, None),
        };
        if let Some(claim) = claim {
            claim.validate().map_err(|e| err(&e.code, &e.message))?;
            if let (Some(source), Some(legacy)) = (&claim.source, legacy) {
                if source != legacy {
                    return Err(err(
                        "E_PROVENANCE",
                        "claimed provenance source differs from the record's source label",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn conflict(message: &str) -> Error {
    err("E_CONFLICT", message)
}

/// Structural identity a correction cannot change (ADR-0004 section 2).
fn same_identity(old: &Fact, new: &Fact) -> bool {
    match (old, new) {
        (Fact::Node(a), Fact::Node(b)) => a.entity_id == b.entity_id && a.space_id == b.space_id,
        (Fact::Edge(a), Fact::Edge(b)) => {
            (&a.from, &a.to, &a.predicate) == (&b.from, &b.to, &b.predicate)
        }
        (Fact::StructuralEdge(a), Fact::StructuralEdge(b)) => {
            (&a.from, &a.to, &a.predicate) == (&b.from, &b.to, &b.predicate)
        }
        (Fact::Assertion(a), Fact::Assertion(b)) => a.edge_id == b.edge_id,
        (Fact::Attachment(a), Fact::Attachment(b)) => a.host == b.host && a.key == b.key,
        // A space is immutable: only the identical descriptor has its identity.
        (Fact::EmbeddingSpace(a), Fact::EmbeddingSpace(b)) => a == b,
        (Fact::Embedding(a), Fact::Embedding(b)) => a.space == b.space && a.subject == b.subject,
        _ => false,
    }
}

/// True when `dependent` needs `target` to be live.
fn depends_on(dependent: &Fact, target: &Fact) -> bool {
    match (dependent, target) {
        (Fact::Edge(e), Fact::Node(n)) => e.from == n.id || e.to == n.id,
        (Fact::StructuralEdge(e), Fact::Node(n)) => e.from == n.id || e.to == n.id,
        (Fact::Assertion(a), Fact::StructuralEdge(e)) => a.edge_id == e.id,
        (Fact::Attachment(a), _) => match (&a.host, target) {
            (MetadataHost::Node { id }, Fact::Node(n)) => id == &n.id,
            (MetadataHost::Edge { id }, Fact::Edge(e)) => id == &e.id,
            (MetadataHost::Edge { id }, Fact::StructuralEdge(e)) => id == &e.id,
            (MetadataHost::Assertion { id }, Fact::Assertion(a)) => id == &a.id,
            _ => false,
        },
        (Fact::Embedding(e), target) => match (&e.subject, target) {
            (EmbeddingSubject::Node { node_id }, Fact::Node(n)) => node_id == &n.id,
            (EmbeddingSubject::Edge { edge_id }, Fact::Edge(t)) => edge_id == &t.id,
            (EmbeddingSubject::Edge { edge_id }, Fact::StructuralEdge(t)) => edge_id == &t.id,
            (EmbeddingSubject::Assertion { assertion_id }, Fact::Assertion(a)) => {
                assertion_id == &a.id
            }
            _ => false,
        },
        _ => false,
    }
}

/// The state a write builds on.
enum Parent {
    Empty,
    Snapshot {
        revision: String,
        data: Box<GraphData>,
    },
    Fact {
        revision: String,
    },
}

/// The result of [`Engine::reconcile_facts`].
#[derive(Debug, Clone, PartialEq)]
pub struct FactReconciliation {
    /// The branch head the changes were computed against.
    pub head: Option<String>,
    /// Asserts and corrections, in input order. Identical facts are omitted.
    pub changes: Vec<FactChange>,
}

/// One version of a fact as seen at some revision.
struct Seen {
    fact: Fact,
    /// The revision that recorded this version, when it was recorded by a fact write.
    recorded_in: Option<String>,
}

/// Read access to the facts live at one revision: a fact revision through the
/// backend's indexes, or a snapshot revision through an in-memory index of its
/// stored payload. Both give the same records, so one closure and traversal
/// path serves both kinds of revision.
pub(crate) trait FactSource {
    /// The graph-level fields, with every fact list empty.
    fn header(&self) -> Result<GraphData>;
    fn get(&self, namespace: store::Namespace, id: &str) -> Result<Option<store::StoredFact>>;
    /// Every live fact matching `filter`, in `(kind, id)` order; `E_BUDGET` beyond `limit`.
    fn select(&self, filter: &FactFilter, limit: usize) -> Result<Vec<store::StoredFact>>;
    /// Visit every live fact matching `filter`, in no particular order,
    /// without materializing them.
    fn scan(
        &self,
        filter: &FactFilter,
        visit: &mut dyn FnMut(store::StoredFact) -> Result<()>,
    ) -> Result<()>;
    /// True when bodies read here are not yet charged to the read budget
    /// (a snapshot payload was charged once, when it was loaded).
    fn charged(&self) -> bool;
}

/// A fact revision, read through the SQLite backend.
pub(crate) struct LineSource<'c> {
    store: SqliteFactStore<'c>,
    revision: String,
}

impl<'c> LineSource<'c> {
    pub(crate) fn new(store: SqliteFactStore<'c>, revision: &str) -> Self {
        Self {
            store,
            revision: revision.into(),
        }
    }
}

impl FactSource for LineSource<'_> {
    fn header(&self) -> Result<GraphData> {
        serde_json::from_str(&self.store.header(&self.revision)?)
            .map_err(|_| err("E_INTEGRITY", "stored line header is malformed"))
    }
    fn get(&self, namespace: store::Namespace, id: &str) -> Result<Option<store::StoredFact>> {
        self.store.get(&self.revision, namespace, id)
    }
    fn select(&self, filter: &FactFilter, limit: usize) -> Result<Vec<store::StoredFact>> {
        self.store.select(&self.revision, filter, limit)
    }
    fn scan(
        &self,
        filter: &FactFilter,
        visit: &mut dyn FnMut(store::StoredFact) -> Result<()>,
    ) -> Result<()> {
        self.store.scan(&self.revision, filter, visit)
    }
    fn charged(&self) -> bool {
        true
    }
}

/// A snapshot revision's payload, indexed by the keys the fact index has.
pub(crate) struct SnapshotSource {
    header: GraphData,
    facts: Vec<store::StoredFact>,
    by_key: HashMap<Key, usize>,
    by_subject: HashMap<String, Vec<usize>>,
    by_object: HashMap<String, Vec<usize>>,
    by_entity: HashMap<String, Vec<usize>>,
    carriers: Vec<usize>,
    unanchored: Vec<usize>,
}

impl SnapshotSource {
    pub(crate) fn new(data: &GraphData, revision: &str) -> Result<Self> {
        let mut source = Self {
            header: header_of(data),
            facts: Vec::new(),
            by_key: HashMap::new(),
            by_subject: HashMap::new(),
            by_object: HashMap::new(),
            by_entity: HashMap::new(),
            carriers: Vec::new(),
            unanchored: Vec::new(),
        };
        for fact in facts_of(data) {
            let index = source.facts.len();
            let stored = store::StoredFact {
                position: index as i64,
                version: version(&fact)?,
                recording: Recording {
                    writer: String::new(),
                    on_behalf_of: None,
                    recorded_at_ms: 0,
                    revision: revision.into(),
                },
            };
            if source.by_key.insert(key(&fact), index).is_some() {
                return Err(err("E_INTEGRITY", "a fact key occurs twice in a snapshot"));
            }
            let keys = &stored.version.keys;
            for (map, value) in [
                (&mut source.by_subject, &keys.subject),
                (&mut source.by_object, &keys.object),
                (&mut source.by_entity, &keys.entity),
            ] {
                if let Some(value) = value {
                    map.entry(value.clone()).or_default().push(index);
                }
            }
            if keys.carrier {
                source.carriers.push(index);
            }
            if keys.subject.is_none() && keys.entity.is_none() {
                source.unanchored.push(index);
            }
            source.facts.push(stored);
        }
        Ok(source)
    }
}

impl FactSource for SnapshotSource {
    fn header(&self) -> Result<GraphData> {
        Ok(self.header.clone())
    }
    fn get(&self, namespace: store::Namespace, id: &str) -> Result<Option<store::StoredFact>> {
        Ok(self
            .by_key
            .get(&(namespace, id.to_string()))
            .map(|index| self.facts[*index].clone()))
    }
    fn select(&self, filter: &FactFilter, limit: usize) -> Result<Vec<store::StoredFact>> {
        let all: Vec<usize>;
        let candidates: &[usize] = if let Some(subject) = &filter.subject {
            self.by_subject.get(subject).map_or(&[], Vec::as_slice)
        } else if let Some(object) = &filter.object {
            self.by_object.get(object).map_or(&[], Vec::as_slice)
        } else if let Some(entity) = &filter.entity {
            self.by_entity.get(entity).map_or(&[], Vec::as_slice)
        } else if filter.carrier {
            &self.carriers
        } else if filter.unanchored {
            &self.unanchored
        } else {
            all = (0..self.facts.len()).collect();
            &all
        };
        let mut facts = Vec::new();
        for index in candidates {
            let fact = &self.facts[*index];
            if filter.matches(fact) {
                if facts.len() == limit {
                    return Err(err("E_BUDGET", "fact selection exceeds its limit"));
                }
                facts.push(fact.clone());
            }
        }
        store::order(&mut facts);
        Ok(facts)
    }
    fn scan(
        &self,
        filter: &FactFilter,
        visit: &mut dyn FnMut(store::StoredFact) -> Result<()>,
    ) -> Result<()> {
        for fact in self.select(filter, usize::MAX - 1)? {
            visit(fact)?;
        }
        Ok(())
    }
    fn charged(&self) -> bool {
        false
    }
}

impl Engine {
    /// Store 30 (ADR-0004 section 10): create the empty fact store. Existing
    /// rows are never rewritten. A store below 30 must not have fact tables,
    /// and a store at 30 must have all of them.
    pub(crate) fn initialize_fact_store(&self, version: i64) -> Result<()> {
        let present = store::sqlite::installed(&self.conn)?;
        if (version < 30 && present != 0)
            || (version >= 30 && present != store::sqlite::TABLES.len())
        {
            return Err(err(
                "E_INTEGRITY",
                "fact store schema does not match the store version",
            ));
        }
        if !store::sqlite::current_layout(&self.conn)? {
            return Err(err(
                "E_STORAGE_VERSION",
                "fact store from a pre-release store-30 build; recreate the store",
            ));
        }
        store::sqlite::install(&self.conn)
    }

    pub(crate) fn fact_store(&self) -> SqliteFactStore<'_> {
        SqliteFactStore::new(&self.conn)
    }

    fn parent_header(&self, parent: &Parent) -> Result<GraphData> {
        match parent {
            Parent::Empty => Ok(GraphData::default()),
            Parent::Snapshot { data, .. } => Ok(header_of(data)),
            Parent::Fact { revision } => serde_json::from_str(&self.fact_store().header(revision)?)
                .map_err(|_| err("E_INTEGRITY", "stored line header is malformed")),
        }
    }

    fn parent_get(&self, parent: &Parent, key: &Key) -> Result<Option<Seen>> {
        match parent {
            Parent::Empty => Ok(None),
            Parent::Snapshot { data, .. } => Ok(facts_of(data)
                .into_iter()
                .find(|f| namespace(f.kind()) == key.0 && f.key_id() == key.1)
                .map(|fact| Seen {
                    fact,
                    recorded_in: None,
                })),
            Parent::Fact { revision } => self
                .fact_store()
                .get(revision, key.0, &key.1)?
                .map(|stored| {
                    Ok(Seen {
                        fact: parse(&stored.version)?,
                        recorded_in: Some(stored.recording.revision),
                    })
                })
                .transpose(),
        }
    }

    /// Live parent facts matching a filter (index scan, or a snapshot scan).
    fn parent_select(&self, parent: &Parent, filter: &FactFilter) -> Result<Vec<Fact>> {
        match parent {
            Parent::Empty => Ok(vec![]),
            Parent::Snapshot { data, .. } => Ok(facts_of(data)
                .into_iter()
                .filter(|fact| {
                    let keys = index_keys(fact);
                    let same = |want: &Option<String>, have: &Option<String>| {
                        want.as_ref().is_none_or(|w| have.as_ref() == Some(w))
                    };
                    filter.kind.is_none_or(|k| k == store_kind(fact.kind()))
                        && same(&filter.entity, &keys.entity)
                        && same(&filter.subject, &keys.subject)
                        && same(&filter.object, &keys.object)
                })
                .collect()),
            Parent::Fact { revision } => self
                .fact_store()
                .select(revision, filter, MAX_WRITE_CHANGES * 10)?
                .iter()
                .map(|stored| parse(&stored.version))
                .collect(),
        }
    }

    /// The version of a fact live at any revision of `graph`.
    fn seen_at(&self, graph: &str, revision: &str, key: &Key) -> Result<Option<Seen>> {
        if is_fact_revision(revision) {
            let info = self.fact_store().revision(revision)?;
            if info.is_none_or(|i| i.graph_id != graph) {
                return Err(conflict("expected revision unavailable"));
            }
            return self.parent_get(
                &Parent::Fact {
                    revision: revision.into(),
                },
                key,
            );
        }
        let data = self
            .load(graph, revision)?
            .ok_or_else(|| conflict("expected revision unavailable"))?;
        self.parent_get(
            &Parent::Snapshot {
                revision: revision.into(),
                data: Box::new(data),
            },
            key,
        )
    }

    /// True when `access` admits a live parent fact: its own `readers` and
    /// `reader_groups`, and for an edge both live endpoint nodes'.
    fn writer_reads(
        &self,
        parent: &Parent,
        fact: &Fact,
        access: &visibility::Access,
    ) -> Result<bool> {
        let admits =
            |f: &Fact| visibility::record_admits(fact_readers(f), fact_reader_groups(f), access);
        if !admits(fact) {
            return Ok(false);
        }
        let endpoints: [&str; 2] = match fact {
            Fact::Edge(e) => [&e.from, &e.to],
            Fact::StructuralEdge(e) => [&e.from, &e.to],
            _ => return Ok(true),
        };
        for node in endpoints {
            if let Some(seen) = self.parent_get(parent, &(store::Namespace::Node, node.into()))? {
                if !admits(&seen.fact) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn check_expectation(
        &self,
        graph: &str,
        key: &Key,
        live: &Option<Seen>,
        expected: &Option<FactExpectation>,
    ) -> Result<()> {
        match expected {
            None => Ok(()),
            Some(FactExpectation::Absent) => match live {
                None => Ok(()),
                Some(_) => Err(conflict("expected fact to be absent")),
            },
            Some(FactExpectation::UnchangedSince { revision }) => {
                if !valid_id(revision) {
                    return Err(err("E_ID", "expected revision requires 1 to 512 bytes"));
                }
                let then = self.seen_at(graph, revision, key)?;
                let unchanged = match (live, &then) {
                    (Some(now), Some(then)) => {
                        now.fact == then.fact
                            && match (&now.recorded_in, &then.recorded_in) {
                                (Some(a), Some(b)) => a == b,
                                _ => true,
                            }
                    }
                    _ => false,
                };
                if unchanged {
                    Ok(())
                } else {
                    Err(conflict("fact changed since the expected revision"))
                }
            }
        }
    }

    /// Execute one `write` command (ADR-0004). Returns the new revision and
    /// event, or the unchanged head when every change is a no-op.
    pub(crate) fn write_facts(
        &self,
        graph: &str,
        branch: &str,
        expected_head: &Option<Option<String>>,
        changes: &[FactChange],
        host: &HostContext,
    ) -> Result<(String, Option<String>)> {
        identity_acceptance::require_external_graph(graph)?;
        // The `weave:` namespace is engine-owned (ADR-0007 section 13, item 5).
        if graph.starts_with("weave:") {
            return Err(err(
                "E_RESERVED_NAMESPACE",
                "fact writes to engine-reserved weave: graphs are rejected",
            ));
        }
        self.require_branch_write(host, graph, branch)?;
        // Named groups must exist and be usable by the writer (ADR-0007 section 2).
        self.validate_reader_groups(
            changes.iter().flat_map(|change| match change {
                FactChange::Assert { fact } | FactChange::Correct { fact, .. } => {
                    fact_reader_groups(fact)
                }
                FactChange::Retract { .. } => &[],
            }),
            host,
        )?;
        self.append_facts(graph, branch, expected_head, changes, host)
    }

    /// The write itself, after authorization: also the registry's write path.
    pub(crate) fn append_facts(
        &self,
        graph: &str,
        branch: &str,
        expected_head: &Option<Option<String>>,
        changes: &[FactChange],
        host: &HostContext,
    ) -> Result<(String, Option<String>)> {
        if !valid_id(graph) || !valid_id(branch) || !valid_id(&host.principal) {
            return Err(err("E_ID", "identifiers must not be empty"));
        }
        if changes.is_empty() {
            return Err(err("E_FACT_WRITE", "a write needs at least one change"));
        }
        if changes.len() > MAX_WRITE_CHANGES {
            return Err(err("E_BUDGET", "a write carries at most 100000 changes"));
        }
        let head = self.head(graph, branch)?;
        if let Some(expected) = expected_head {
            if &head != expected {
                return Err(conflict("expected head differs from current branch head"));
            }
        }
        let parent = match &head {
            None => Parent::Empty,
            Some(revision) if is_fact_revision(revision) => Parent::Fact {
                revision: revision.clone(),
            },
            Some(revision) => Parent::Snapshot {
                revision: revision.clone(),
                data: Box::new(
                    self.load(graph, revision)?
                        .ok_or_else(|| err("E_UNAVAILABLE", "graph unavailable"))?,
                ),
            },
        };
        let header = self.parent_header(&parent)?;
        // In a registry store, correcting or retracting a fact needs Read on
        // it (ADR-0007 section 4). A live fact the writer cannot read is
        // treated as absent, so the write fails exactly as for a missing
        // fact. The registry's own writes are exempt.
        let reader = if self.registry_head()?.is_some() && !registry::reserved(graph) {
            Some(self.access(host)?)
        } else {
            None
        };
        // Effective operations: key -> Some(new version) or None (retract).
        let mut ops: BTreeMap<Key, Option<Fact>> = BTreeMap::new();
        let mut live_before: BTreeMap<Key, Option<Fact>> = BTreeMap::new();
        for change in changes {
            let (k, kind) = match change {
                FactChange::Assert { fact } | FactChange::Correct { fact, .. } => {
                    (key(fact), fact.kind())
                }
                FactChange::Retract { target, .. } => {
                    if !valid_id(&target.id) {
                        return Err(err("E_ID", "fact identifiers require 1 to 512 bytes"));
                    }
                    ((namespace(target.kind), target.id.clone()), target.kind)
                }
            };
            if live_before.contains_key(&k) {
                return Err(conflict("a fact occurs twice in one write"));
            }
            let mut live = self.parent_get(&parent, &k)?;
            if let (Some(access), Some(seen)) = (&reader, &live) {
                if !matches!(change, FactChange::Assert { .. })
                    && !self.writer_reads(&parent, &seen.fact, access)?
                {
                    live = None;
                }
            }
            if live.as_ref().is_some_and(|l| l.fact.kind() != kind) {
                return Err(conflict("a live fact with this ID has another kind"));
            }
            match change {
                FactChange::Assert { fact } => match &live {
                    Some(l) if &l.fact == fact => {}
                    Some(_) => return Err(conflict("fact is already live; use correct")),
                    None => {
                        ops.insert(k.clone(), Some(fact.clone()));
                    }
                },
                FactChange::Correct { fact, expected } => {
                    self.check_expectation(graph, &k, &live, expected)?;
                    let l = live
                        .as_ref()
                        .ok_or_else(|| conflict("corrected fact is not live"))?;
                    if !same_identity(&l.fact, fact) {
                        return Err(conflict(
                            "a correction cannot change structural identity; write a new fact",
                        ));
                    }
                    if &l.fact != fact {
                        ops.insert(k.clone(), Some(fact.clone()));
                    }
                }
                FactChange::Retract { expected, .. } => {
                    if kind == FactKind::EmbeddingSpace {
                        return Err(conflict("embedding spaces are immutable"));
                    }
                    self.check_expectation(graph, &k, &live, expected)?;
                    if live.is_none() {
                        return Err(conflict("retracted fact is not live"));
                    }
                    ops.insert(k.clone(), None);
                }
            }
            live_before.insert(k, live.map(|l| l.fact));
        }
        if ops.is_empty() {
            let revision = head.ok_or_else(|| err("E_INTEGRITY", "no-op write without head"))?;
            return Ok((revision, None));
        }
        // Post-state lookup: this write's changes over the parent state.
        let post = |k: &Key| -> Result<Option<Fact>> {
            match ops.get(k) {
                Some(change) => Ok(change.clone()),
                None => Ok(self.parent_get(&parent, k)?.map(|s| s.fact)),
            }
        };
        // Records that survive in the post state and depend on a retracted fact.
        for (k, change) in &ops {
            if change.is_some() {
                continue;
            }
            let Some(Some(target)) = live_before.get(k) else {
                continue;
            };
            let mut candidates = Vec::new();
            let mut filters = vec![FactFilter {
                subject: Some(k.1.clone()),
                ..FactFilter::default()
            }];
            if let Fact::Node(node) = target {
                filters.push(FactFilter {
                    object: Some(k.1.clone()),
                    ..FactFilter::default()
                });
                // Entity-hosted attachments need some live node of the entity.
                let entity = node.entity_id.clone();
                let others = self
                    .parent_select(
                        &parent,
                        &FactFilter {
                            kind: Some(store::FactKind::Node),
                            entity: Some(entity.clone()),
                            ..FactFilter::default()
                        },
                    )?
                    .into_iter()
                    .filter(|f| post(&key(f)).is_ok_and(|p| p.is_some()))
                    .count()
                    + ops
                        .values()
                        .flatten()
                        .filter(|f| matches!(f, Fact::Node(n) if n.entity_id == entity && !live_before.get(&key(f)).is_some_and(|l| l.is_some())))
                        .count();
                if others == 0 {
                    for kind in [store::FactKind::Attachment, store::FactKind::Embedding] {
                        filters.push(FactFilter {
                            kind: Some(kind),
                            entity: Some(entity.clone()),
                            ..FactFilter::default()
                        });
                    }
                }
            }
            for filter in &filters {
                candidates.extend(self.parent_select(&parent, filter)?);
            }
            for candidate in candidates {
                let ck = key(&candidate);
                if &ck == k {
                    continue;
                }
                let entity_host = match (&candidate, target) {
                    (Fact::Attachment(a), Fact::Node(n)) => {
                        a.host
                            == MetadataHost::Entity {
                                id: n.entity_id.clone(),
                            }
                    }
                    (Fact::Embedding(e), Fact::Node(n)) => {
                        e.subject
                            == EmbeddingSubject::Entity {
                                entity_id: n.entity_id.clone(),
                            }
                    }
                    _ => false,
                };
                if let Some(after) = post(&ck)? {
                    if entity_host || depends_on(&after, target) {
                        return Err(err(
                            "E_DEPENDENT_FACT",
                            "a retracted fact still has live dependent facts",
                        ));
                    }
                }
            }
        }
        // Validate written records together with the records they reference.
        let mut neighborhood = header.clone();
        let mut included: BTreeSet<Key> = BTreeSet::new();
        let mut include = |fact: Fact, data: &mut GraphData| {
            if included.insert(key(&fact)) {
                push(data, fact);
            }
        };
        let node_key = |id: &str| (store::Namespace::Node, id.to_string());
        let record_key = |id: &str| (store::Namespace::Record, id.to_string());
        for fact in ops.values().flatten() {
            let mut needed: Vec<Key> = vec![];
            match fact {
                Fact::Node(_) => {}
                Fact::Edge(e) => needed.extend([node_key(&e.from), node_key(&e.to)]),
                Fact::StructuralEdge(e) => needed.extend([node_key(&e.from), node_key(&e.to)]),
                Fact::Assertion(a) => needed.push(record_key(&a.edge_id)),
                Fact::Attachment(a) => match &a.host {
                    MetadataHost::Node { id } => needed.push(node_key(id)),
                    MetadataHost::Edge { id } | MetadataHost::Assertion { id } => {
                        needed.push(record_key(id))
                    }
                    MetadataHost::Entity { id } => {
                        let mut found = ops
                            .values()
                            .flatten()
                            .filter(|f| matches!(f, Fact::Node(n) if &n.entity_id == id))
                            .map(key)
                            .collect::<Vec<_>>();
                        for node in self.parent_select(
                            &parent,
                            &FactFilter {
                                kind: Some(store::FactKind::Node),
                                entity: Some(id.clone()),
                                ..FactFilter::default()
                            },
                        )? {
                            found.push(key(&node));
                        }
                        needed.extend(found);
                    }
                    MetadataHost::Graph => {}
                },
                Fact::EmbeddingSpace(_) => {}
                Fact::Embedding(e) => {
                    needed.push((store::Namespace::Space, e.space.key()));
                    match &e.subject {
                        EmbeddingSubject::Node { node_id } => needed.push(node_key(node_id)),
                        EmbeddingSubject::Edge { edge_id } => needed.push(record_key(edge_id)),
                        EmbeddingSubject::Assertion { assertion_id } => {
                            needed.push(record_key(assertion_id))
                        }
                        EmbeddingSubject::Entity { entity_id } => {
                            needed.extend(
                                ops.values()
                                    .flatten()
                                    .filter(
                                        |f| matches!(f, Fact::Node(n) if &n.entity_id == entity_id),
                                    )
                                    .map(key),
                            );
                            for node in self.parent_select(
                                &parent,
                                &FactFilter {
                                    kind: Some(store::FactKind::Node),
                                    entity: Some(entity_id.clone()),
                                    ..FactFilter::default()
                                },
                            )? {
                                needed.push(key(&node));
                            }
                        }
                    }
                }
            }
            include(fact.clone(), &mut neighborhood);
            let mut queue = needed;
            while let Some(k) = queue.pop() {
                let Some(dependency) = post(&k)? else {
                    continue;
                };
                match &dependency {
                    Fact::Edge(e) => queue.extend([node_key(&e.from), node_key(&e.to)]),
                    Fact::StructuralEdge(e) => queue.extend([node_key(&e.from), node_key(&e.to)]),
                    Fact::Assertion(a) => queue.push(record_key(&a.edge_id)),
                    _ => {}
                }
                include(dependency, &mut neighborhood);
            }
        }
        validate_graph(&neighborhood)?;
        validate_embeddings(&neighborhood)?;
        self.validate_structures(graph, &neighborhood)?;
        // Canonical operations and the content-derived revision ID (ADR-0004 section 3).
        let retracted_kind = |k: &Key| -> Result<FactKind> {
            live_before
                .get(k)
                .cloned()
                .flatten()
                .map(|f| f.kind())
                .ok_or_else(|| err("E_INTEGRITY", "retracted fact kind unknown"))
        };
        let mut canonical = Vec::new();
        let mut logged = Vec::new();
        for (k, change) in &ops {
            match change {
                Some(fact) => {
                    canonical.push(serde_json::json!({ "assert": fact }));
                    logged.push(FactOp::Assert(version(fact)?));
                }
                None => {
                    let kind = retracted_kind(k)?;
                    canonical.push(serde_json::json!({ "retract": { "kind": kind, "id": k.1 } }));
                    logged.push(FactOp::Retract {
                        kind: store_kind(kind),
                        id: k.1.clone(),
                    });
                }
            }
        }
        let revision = fact_revision_id(graph, branch, &head, &canonical)?;
        let base = match &parent {
            Parent::Fact { revision } if self.fact_store().is_tip(revision)? => None,
            Parent::Fact { revision } => Some(BaseState {
                header: serde_json::to_string(&header)?,
                facts: self
                    .fact_store()
                    .select(revision, &FactFilter::default(), usize::MAX - 1)?
                    .into_iter()
                    .map(|stored| (stored.version, stored.recording))
                    .collect(),
            }),
            Parent::Snapshot { revision, data } => {
                let writer = self
                    .revision_writer(revision)?
                    .unwrap_or_else(|| UNRECORDED.into());
                let recording = Recording {
                    on_behalf_of: None,
                    writer,
                    recorded_at_ms: self.recorded_at(revision)?,
                    revision: revision.clone(),
                };
                let mut facts = Vec::new();
                for fact in facts_of(data) {
                    facts.push((version(&fact)?, recording.clone()));
                }
                Some(BaseState {
                    header: serde_json::to_string(&header)?,
                    facts,
                })
            }
            Parent::Empty => Some(BaseState {
                header: serde_json::to_string(&header)?,
                facts: vec![],
            }),
        };
        let time = self.operation_time()?;
        let event_id = format!("commit:{revision}");
        self.conn.execute(
            "INSERT INTO revisions VALUES (?1,?2,?3,?4,?5,'')",
            params![revision, graph, branch, head, time],
        )?;
        self.fact_store().append(
            NewRevision {
                on_behalf_of: encode_delegation(&self.delegation_chain(host)?)?,
                revision: revision.clone(),
                graph_id: graph.into(),
                branch_id: branch.into(),
                parent: head.clone(),
                writer: host.principal.clone(),
                recorded_at_ms: time,
                ops: logged,
            },
            base,
        )?;
        self.advance_head(
            &GraphRef {
                graph_id: graph.into(),
                revision: revision.clone(),
            },
            branch,
            ObservationKind::Committed,
        )?;
        self.conn.execute(
            "INSERT INTO events(event_id,graph_id,branch_id,revision,actor) VALUES (?1,?2,?3,?4,?5)",
            params![event_id, graph, branch, revision, host.principal],
        )?;
        self.record_local_causal_root(&event_id)?;
        self.validate_required_metadata(&neighborhood, host)?;
        self.record_structures(graph, &neighborhood)?;
        Ok((revision, Some(event_id)))
    }

    /// The changes that make the live facts of a branch equal `facts`, for
    /// importers that re-send whole tables (A5): `assert` for a fact that is
    /// not live, `correct` for a live fact of the same kind and ID that
    /// differs, and nothing for an identical live fact. An empty result means
    /// a write of these facts would change nothing.
    ///
    /// It needs write authority on the graph and discloses no more than a
    /// `write` of the same facts would: whether each key is absent, identical
    /// or different, never a stored version. Apply the changes with `write`
    /// and `expected_head` set to the returned head, so a concurrent writer
    /// makes the write fail with `E_CONFLICT` instead of being overwritten.
    pub fn reconcile_facts(
        &self,
        graph: &str,
        branch: &str,
        facts: &[Fact],
        host: &HostContext,
    ) -> Result<FactReconciliation> {
        identity_acceptance::require_external_graph(graph)?;
        if graph.starts_with("weave:") {
            return Err(err(
                "E_RESERVED_NAMESPACE",
                "fact writes to engine-reserved weave: graphs are rejected",
            ));
        }
        self.require_branch_write(host, graph, branch)?;
        if !valid_id(graph) || !valid_id(branch) || !valid_id(&host.principal) {
            return Err(err("E_ID", "identifiers must not be empty"));
        }
        if facts.len() > MAX_WRITE_CHANGES {
            return Err(err(
                "E_BUDGET",
                "a reconciliation carries at most 100000 facts",
            ));
        }
        let head = self.head(graph, branch)?;
        let parent = match &head {
            None => Parent::Empty,
            Some(revision) if is_fact_revision(revision) => Parent::Fact {
                revision: revision.clone(),
            },
            Some(revision) => Parent::Snapshot {
                revision: revision.clone(),
                data: Box::new(
                    self.load(graph, revision)?
                        .ok_or_else(|| err("E_UNAVAILABLE", "graph unavailable"))?,
                ),
            },
        };
        let mut seen: BTreeSet<Key> = BTreeSet::new();
        let mut changes = Vec::new();
        for fact in facts {
            if !valid_id(&fact.key_id()) {
                return Err(err("E_ID", "fact identifiers require 1 to 512 bytes"));
            }
            let k = key(fact);
            if !seen.insert(k.clone()) {
                return Err(conflict("a fact occurs twice in one reconciliation"));
            }
            match self.parent_get(&parent, &k)? {
                None => changes.push(FactChange::Assert { fact: fact.clone() }),
                Some(live) if live.fact.kind() != fact.kind() => {
                    return Err(conflict("a live fact with this ID has another kind"));
                }
                Some(live) if &live.fact == fact => {}
                Some(live) => {
                    if !same_identity(&live.fact, fact) {
                        return Err(conflict(
                            "a correction cannot change structural identity; write a new fact",
                        ));
                    }
                    changes.push(FactChange::Correct {
                        fact: fact.clone(),
                        expected: None,
                    });
                }
            }
        }
        Ok(FactReconciliation { head, changes })
    }

    /// The principal of the first commit event of a revision, if any.
    pub(crate) fn revision_writer(&self, revision: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT actor FROM events WHERE revision=?1 ORDER BY sequence LIMIT 1",
                [revision],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Materialize a fact revision (ADR-0004 section 7). `None` when it is not
    /// a revision of `graph`.
    /// The stored fact revision of `graph`, after checking its revision row
    /// and recomputing its content-derived ID from the logged operations.
    pub(crate) fn verified_fact_revision(
        &self,
        graph: &str,
        revision: &str,
    ) -> Result<Option<store::RevisionInfo>> {
        let store = self.fact_store();
        let Some(info) = store.revision(revision)? else {
            let row: Option<String> = self
                .conn
                .query_row(
                    "SELECT graph_id FROM revisions WHERE revision=?1",
                    [revision],
                    |r| r.get(0),
                )
                .optional()?;
            return match row {
                Some(_) => Err(err("E_INTEGRITY", "fact revision lacks its log entry")),
                None => Ok(None),
            };
        };
        if info.graph_id != graph {
            return Ok(None);
        }
        let row: Option<(String, String, Option<String>, i64)> = self
            .conn
            .query_row(
                "SELECT graph_id,branch_id,parent,recorded_at FROM revisions WHERE revision=?1",
                [revision],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        if row
            != Some((
                info.graph_id.clone(),
                info.branch_id.clone(),
                info.parent.clone(),
                info.recorded_at_ms,
            ))
        {
            return Err(err(
                "E_INTEGRITY",
                "fact revision disagrees with its revision row",
            ));
        }
        // Recompute the content-derived ID from the logged operations.
        let mut canonical = Vec::new();
        for op in store.ops(revision)? {
            canonical.push(match &op.op {
                FactOp::Assert(version) => serde_json::json!({ "assert": parse(version)? }),
                FactOp::Retract { kind, id } => {
                    serde_json::json!({ "retract": { "kind": contract_kind(*kind), "id": id } })
                }
            });
        }
        if fact_revision_id(graph, &info.branch_id, &info.parent, &canonical)? != revision {
            return Err(err("E_INTEGRITY", "fact revision digest mismatch"));
        }
        Ok(Some(info))
    }

    /// Rows a whole materialization may read before it must fail with
    /// `E_BUDGET`: a stored fact body is never shorter than 16 bytes.
    pub(crate) fn materialization_limit(&self) -> usize {
        self.read_budget.remaining() / 16
    }

    pub(crate) fn load_fact_revision(
        &self,
        graph: &str,
        revision: &str,
    ) -> Result<Option<GraphData>> {
        if self.verified_fact_revision(graph, revision)?.is_none() {
            return Ok(None);
        }
        let store = self.fact_store();
        if let Some(body) = store.checkpoint(revision)? {
            self.read_budget.charge(body.len())?;
            let data: GraphData = serde_json::from_str(&body)
                .map_err(|_| err("E_INTEGRITY", "stored checkpoint is malformed"))?;
            return Ok(Some(data));
        }
        let mut data: GraphData = serde_json::from_str(&store.header(revision)?)
            .map_err(|_| err("E_INTEGRITY", "stored line header is malformed"))?;
        // Ordinary reads never materialize embeddings or spaces (ADR-0006):
        // only `nearest`, `similar_candidates` and `changes` read them.
        let facts = store
            .select(
                revision,
                &FactFilter {
                    graph_only: true,
                    ..FactFilter::default()
                },
                self.materialization_limit(),
            )
            .map_err(|e| match e.code.as_str() {
                "E_BUDGET" => err("E_BUDGET", "cumulative graph-read byte budget exceeded"),
                _ => e,
            })?;
        for stored in &facts {
            self.read_budget.charge(stored.version.body.len())?;
        }
        for stored in &facts {
            push(&mut data, parse(&stored.version)?);
        }
        Ok(Some(data))
    }

    /// The part of a fact revision that a filtered `query` reads (ADR-0004
    /// section 7): the edges, structural edges and assertions the filter can
    /// select, every influence carrier, and the closure each of them needs to
    /// be authorized exactly as in the whole revision (endpoints, hosts,
    /// attachments on them, entity nodes and graph-hosted attachments).
    /// Running the ordinary query pipeline on this subset gives the same
    /// result as on the whole revision.
    pub(crate) fn load_query_subset(
        &self,
        graph: &str,
        revision: &str,
        query: &QueryPlan,
    ) -> Result<Option<GraphData>> {
        self.read_budget.request()?;
        if self.verified_fact_revision(graph, revision)?.is_none() {
            return Ok(None);
        }
        let store = self.fact_store();
        let limit = self.materialization_limit();
        let edge_filter = |kind| FactFilter {
            kind: Some(kind),
            predicate: query.predicate.clone(),
            subject: query.from.clone(),
            object: query.to.clone(),
            ..FactFilter::default()
        };
        let mut seeds = Vec::new();
        seeds.extend(store.select(
            revision,
            &FactFilter {
                valid_at: query.valid_at,
                ..edge_filter(store::FactKind::Edge)
            },
            limit,
        )?);
        let structures = store.select(
            revision,
            &edge_filter(store::FactKind::StructuralEdge),
            limit,
        )?;
        for structure in &structures {
            seeds.extend(store.select(
                revision,
                &FactFilter {
                    kind: Some(store::FactKind::Assertion),
                    subject: Some(structure.version.id.clone()),
                    ..FactFilter::default()
                },
                limit,
            )?);
        }
        seeds.extend(structures);
        seeds.extend(store.select(
            revision,
            &FactFilter {
                carrier: true,
                ..FactFilter::default()
            },
            limit,
        )?);
        seeds.extend(store.select(
            revision,
            &FactFilter {
                kind: Some(store::FactKind::Attachment),
                unanchored: true,
                ..FactFilter::default()
            },
            limit,
        )?);
        self.fact_closure(revision, seeds, true).map(Some)
    }

    /// The part of a fact revision that a dependency check on one referenced
    /// fact reads: that fact and the closure it needs to be authorized.
    pub(crate) fn load_reference_subset(
        &self,
        graph: &str,
        revision: &str,
        namespace: store::Namespace,
        id: &str,
    ) -> Result<Option<GraphData>> {
        self.read_budget.request()?;
        if self.verified_fact_revision(graph, revision)?.is_none() {
            return Ok(None);
        }
        let seeds = self
            .fact_store()
            .get(revision, namespace, id)?
            .into_iter()
            .collect();
        self.fact_closure(revision, seeds, false).map(Some)
    }

    /// A revision as a dependency check on one of its facts needs it: the
    /// whole snapshot, or for a fact revision only that fact's closure.
    pub(crate) fn load_for_reference(
        &self,
        graph: &str,
        revision: &str,
        namespace: store::Namespace,
        id: &str,
    ) -> Result<Option<GraphData>> {
        if is_fact_revision(revision) {
            self.load_reference_subset(graph, revision, namespace, id)
        } else {
            self.load(graph, revision)
        }
    }

    /// Header plus `seeds` and everything they depend on. With `hosted`, the
    /// attachments hosted on included records are added too (a query result
    /// keeps them; a dependency check does not read them).
    fn fact_closure(
        &self,
        revision: &str,
        seeds: Vec<store::StoredFact>,
        hosted: bool,
    ) -> Result<GraphData> {
        self.closure_in(&LineSource::new(self.fact_store(), revision), seeds, hosted)
    }

    /// [`Self::fact_closure`] over any [`FactSource`].
    pub(crate) fn closure_in(
        &self,
        store: &dyn FactSource,
        seeds: Vec<store::StoredFact>,
        hosted: bool,
    ) -> Result<GraphData> {
        self.closure_charging(store, seeds, hosted, true)
    }

    /// [`Self::closure_in`]; with `charge_embeddings` false, embedding bodies
    /// are not charged to the read budget because the caller charged them to
    /// its own scan budget (`nearest`).
    pub(crate) fn closure_charging(
        &self,
        store: &dyn FactSource,
        seeds: Vec<store::StoredFact>,
        hosted: bool,
        charge_embeddings: bool,
    ) -> Result<GraphData> {
        let limit = self.materialization_limit();
        let mut facts: BTreeMap<(store::FactKind, String), Fact> = BTreeMap::new();
        let mut queue: Vec<store::StoredFact> = seeds;
        let mut hosted_done: BTreeSet<String> = BTreeSet::new();
        while let Some(stored) = queue.pop() {
            let slot = (stored.version.kind, stored.version.id.clone());
            if facts.contains_key(&slot) {
                continue;
            }
            if store.charged()
                && (charge_embeddings || stored.version.kind != store::FactKind::Embedding)
            {
                self.read_budget.charge(stored.version.body.len())?;
            }
            let fact = parse(&stored.version)?;

            let get = |namespace, id: &str| store.get(namespace, id);
            let mut needed = Vec::new();
            match &fact {
                Fact::Node(_) => {}
                Fact::Edge(e) => {
                    needed.extend(get(store::Namespace::Node, &e.from)?);
                    needed.extend(get(store::Namespace::Node, &e.to)?);
                }
                Fact::StructuralEdge(e) => {
                    needed.extend(get(store::Namespace::Node, &e.from)?);
                    needed.extend(get(store::Namespace::Node, &e.to)?);
                }
                Fact::Assertion(a) => needed.extend(get(store::Namespace::Record, &a.edge_id)?),
                Fact::Attachment(a) => match &a.host {
                    MetadataHost::Node { id } => needed.extend(get(store::Namespace::Node, id)?),
                    MetadataHost::Edge { id } | MetadataHost::Assertion { id } => {
                        needed.extend(get(store::Namespace::Record, id)?)
                    }
                    MetadataHost::Entity { id } => {
                        // Entity-hosted attachments survive while any node of the entity does.
                        needed.extend(store.select(
                            &FactFilter {
                                kind: Some(store::FactKind::Node),
                                entity: Some(id.clone()),
                                ..FactFilter::default()
                            },
                            limit,
                        )?);
                    }
                    MetadataHost::Graph => {}
                },
                Fact::EmbeddingSpace(_) => {}
                // An embedding is visible only with its subject.
                Fact::Embedding(e) => match &e.subject {
                    EmbeddingSubject::Node { node_id } => {
                        needed.extend(get(store::Namespace::Node, node_id)?)
                    }
                    EmbeddingSubject::Edge { edge_id } => {
                        needed.extend(get(store::Namespace::Record, edge_id)?)
                    }
                    EmbeddingSubject::Assertion { assertion_id } => {
                        needed.extend(get(store::Namespace::Record, assertion_id)?)
                    }
                    EmbeddingSubject::Entity { entity_id } => {
                        needed.extend(store.select(
                            &FactFilter {
                                kind: Some(store::FactKind::Node),
                                entity: Some(entity_id.clone()),
                                ..FactFilter::default()
                            },
                            limit,
                        )?);
                    }
                },
            }
            if hosted
                && !matches!(
                    fact,
                    Fact::Attachment(_) | Fact::EmbeddingSpace(_) | Fact::Embedding(_)
                )
                && hosted_done.insert(fact.id().to_string())
            {
                needed.extend(store.select(
                    &FactFilter {
                        kind: Some(store::FactKind::Attachment),
                        subject: Some(fact.id().to_string()),
                        ..FactFilter::default()
                    },
                    limit,
                )?);
            }
            if let Fact::Node(n) = &fact {
                if hosted && hosted_done.insert(format!("entity:{}", n.entity_id)) {
                    needed.extend(store.select(
                        &FactFilter {
                            kind: Some(store::FactKind::Attachment),
                            entity: Some(n.entity_id.clone()),
                            ..FactFilter::default()
                        },
                        limit,
                    )?);
                }
            }
            facts.insert(slot, fact);
            queue.extend(needed);
        }
        let mut data = store.header()?;
        for fact in facts.into_values() {
            push(&mut data, fact);
        }
        Ok(data)
    }

    /// Cache the materialized state of a fact revision as a checkpoint. The
    /// caller needs write authority on the graph. Returns false for a revision
    /// that is not a fact revision of `graph`.
    pub fn checkpoint_fact_revision(
        &self,
        graph: &str,
        revision: &str,
        host: &HostContext,
    ) -> Result<bool> {
        if !self.graph_writable(host, graph, None)? {
            return Err(err(
                "E_FORBIDDEN",
                "host has not granted graph write authority",
            ));
        }
        let _scope = self.read_budget.enter();
        if !is_fact_revision(revision) {
            return Ok(false);
        }
        let Some(data) = self.load_fact_revision(graph, revision)? else {
            return Ok(false);
        };
        self.fact_store()
            .put_checkpoint(revision, &serde_json::to_string(&data)?)?;
        Ok(true)
    }

    /// Drop a cached checkpoint. Results never change.
    pub fn drop_fact_checkpoint(&self, revision: &str, host: &HostContext) -> Result<bool> {
        let Some(info) = self.fact_store().revision(revision)? else {
            return Ok(false);
        };
        if !self.graph_writable(host, &info.graph_id, None)? {
            return Err(err(
                "E_FORBIDDEN",
                "host has not granted graph write authority",
            ));
        }
        let mut store = self.fact_store();
        store.drop_checkpoint(revision)
    }

    fn provenance_of(
        &self,
        reference: &FactRef,
        snapshots: &mut BTreeMap<(String, String), Option<GraphData>>,
    ) -> Result<Option<FactProvenance>> {
        let (graph, revision, k) = match reference {
            FactRef::Node {
                graph_id,
                revision,
                node_id,
            } => (
                graph_id,
                revision,
                (store::Namespace::Node, node_id.clone()),
            ),
            FactRef::Assertion {
                graph_id,
                revision,
                assertion_id,
            } => (
                graph_id,
                revision,
                (store::Namespace::Record, assertion_id.clone()),
            ),
            FactRef::Embedding {
                graph_id,
                revision,
                embedding_id,
            } => (
                graph_id,
                revision,
                (store::Namespace::Record, embedding_id.clone()),
            ),
        };
        if is_fact_revision(revision) {
            let store = self.fact_store();
            if store
                .revision(revision)?
                .is_none_or(|i| &i.graph_id != graph)
            {
                return Ok(None);
            }
            return store
                .get(revision, k.0, &k.1)?
                .map(|stored| {
                    let fact = parse(&stored.version)?;
                    let mut provenance = FactProvenance::record(
                        stored.recording.writer,
                        stored.recording.recorded_at_ms,
                        claim_of(&fact),
                    );
                    provenance.on_behalf_of =
                        decode_delegation(stored.recording.on_behalf_of.as_deref())?;
                    Ok(provenance)
                })
                .transpose();
        }
        let Some(writer) = self.revision_writer(revision)? else {
            return Ok(None);
        };
        let slot = snapshots
            .entry((graph.clone(), revision.clone()))
            .or_insert(None);
        if slot.is_none() {
            *slot = self.load(graph, revision)?;
        }
        let Some(data) = slot.as_ref() else {
            return Ok(None);
        };
        let Some(fact) = facts_of(data)
            .into_iter()
            .find(|f| namespace(f.kind()) == k.0 && f.key_id() == k.1)
        else {
            return Ok(None);
        };
        Ok(Some(FactProvenance::record(
            writer,
            self.recorded_at(revision)?,
            claim_of(&fact),
        )))
    }

    fn evidence_visible(&self, evidence: &EvidenceRef, host: &HostContext) -> Result<bool> {
        match evidence {
            EvidenceRef::Assertion {
                graph_id,
                revision,
                assertion_id,
            } => self.premises_visible(
                &[AssertionRef {
                    graph_id: graph_id.clone(),
                    revision: revision.clone(),
                    assertion_id: assertion_id.clone(),
                }],
                host,
                &mut HashSet::new(),
                &mut 1000,
                0,
            ),
            EvidenceRef::Node {
                graph_id,
                revision,
                node_id,
            } => self.node_refs_visible(
                &[NodeRef {
                    graph_id: graph_id.clone(),
                    revision: revision.clone(),
                    node_id: node_id.clone(),
                }],
                host,
                &mut HashSet::new(),
                &mut 1000,
                0,
            ),
            EvidenceRef::Graph { graph_id, revision } => self.snapshot_refs_visible(
                &[GraphRef {
                    graph_id: graph_id.clone(),
                    revision: revision.clone(),
                }],
                host,
            ),
            EvidenceRef::Artifact { .. } => Ok(true),
            EvidenceRef::Embedding {
                graph_id,
                revision,
                embedding_id,
            } => self.embedding_ref_visible(graph_id, revision, embedding_id, host),
        }
    }

    /// True when an embedding fact at an exact revision is visible to the host.
    pub(crate) fn embedding_ref_visible(
        &self,
        graph: &str,
        revision: &str,
        id: &str,
        host: &HostContext,
    ) -> Result<bool> {
        if !valid_id(graph) || !valid_id(revision) || !valid_id(id) {
            return Ok(false);
        }
        if !is_fact_revision(revision)
            || !self.protected_reference_allowed(graph, revision, host)?
        {
            return Ok(false);
        }
        let Some(data) =
            self.load_reference_subset(graph, revision, store::Namespace::Record, id)?
        else {
            return Ok(false);
        };
        let (data, _) = self.authorized(data, host)?;
        Ok(data.embeddings.iter().any(|e| e.id == id))
    }

    /// Fill `fact_provenance` for every stored source fact a result cites
    /// (ADR-0005 "Reads"). Only facts already in the authorized result are
    /// listed; hidden evidence is removed and makes coverage partial.
    pub(crate) fn attach_fact_provenance(
        &self,
        result: &mut QueryResult,
        host: &HostContext,
    ) -> Result<()> {
        let mut references = BTreeSet::new();
        for origin in result.node_origins.values().flatten() {
            references.insert(FactRef::Node {
                graph_id: origin.graph_id.clone(),
                revision: origin.revision.clone(),
                node_id: origin.node_id.clone(),
            });
        }
        for origin in result
            .edge_origins
            .values()
            .chain(result.attachment_origins.values())
            .flatten()
        {
            references.insert(FactRef::Assertion {
                graph_id: origin.graph_id.clone(),
                revision: origin.revision.clone(),
                assertion_id: origin.assertion_id.clone(),
            });
        }
        // Embeddings in the value (`nearest` hits) and candidate origins.
        let revision_of = |graph: &str| -> Option<String> {
            result
                .input_snapshots
                .iter()
                .find(|r| r.graph_id == graph)
                .map(|r| r.revision.clone())
        };
        if !result.graph.embeddings.is_empty() {
            let graph_id = result
                .snapshots
                .keys()
                .next()
                .cloned()
                .ok_or_else(|| err("E_INTEGRITY", "embedding value without an input"))?;
            let revision = revision_of(&graph_id)
                .ok_or_else(|| err("E_INTEGRITY", "embedding value without an input"))?;
            for embedding in &result.graph.embeddings {
                references.insert(FactRef::Embedding {
                    graph_id: graph_id.clone(),
                    revision: revision.clone(),
                    embedding_id: embedding.id.clone(),
                });
            }
        }
        for candidate in &result.candidates {
            references.extend(candidate.origins.iter().cloned());
        }
        if let Some(changes) = &result.changes {
            for entry in &changes.entries {
                for (fact, revision) in [
                    (&entry.before, &changes.from_revision),
                    (&entry.after, &changes.to_revision),
                ] {
                    let (Some(fact), Some(revision)) = (fact, revision) else {
                        continue;
                    };
                    // Spaces are descriptors without claimed provenance.
                    if matches!(fact, Fact::EmbeddingSpace(_)) {
                        continue;
                    }
                    let graph_id = result
                        .input_snapshots
                        .iter()
                        .find(|r| &r.revision == revision)
                        .map(|r| r.graph_id.clone())
                        .ok_or_else(|| err("E_INTEGRITY", "change set revision not an input"))?;
                    references.insert(match fact {
                        Fact::Node(n) => FactRef::Node {
                            graph_id,
                            revision: revision.clone(),
                            node_id: n.id.clone(),
                        },
                        Fact::Embedding(e) => FactRef::Embedding {
                            graph_id,
                            revision: revision.clone(),
                            embedding_id: e.id.clone(),
                        },

                        other => FactRef::Assertion {
                            graph_id,
                            revision: revision.clone(),
                            assertion_id: other.id().to_string(),
                        },
                    });
                }
            }
        }
        let mut snapshots = BTreeMap::new();
        let mut entries = Vec::new();
        let (mut unavailable, mut hidden) = (false, false);
        for fact in references {
            let Some(mut provenance) = self.provenance_of(&fact, &mut snapshots)? else {
                unavailable = true;
                continue;
            };
            let before = provenance.evidence.len();
            let mut kept = Vec::new();
            for evidence in provenance.evidence {
                if self.evidence_visible(&evidence, host)? {
                    kept.push(evidence);
                }
            }
            hidden |= kept.len() != before;
            provenance.evidence = kept;
            entries.push(ProvenanceEntry { fact, provenance });
        }
        json_size(&entries, MATERIALIZED_LIMIT)?;
        result.fact_provenance = entries;
        if unavailable {
            partial(
                result,
                "E_PROVENANCE_UNAVAILABLE",
                "provenance unavailable for some facts",
            );
        }
        if hidden {
            partial(
                result,
                "E_EVIDENCE_UNAVAILABLE",
                "some evidence unavailable",
            );
        }
        Ok(())
    }
}

/// `fact:sha256:` + SHA-256 of the canonical write content (ADR-0004 section 3).
pub(crate) fn fact_revision_id(
    graph: &str,
    branch: &str,
    parent: &Option<String>,
    ops: &[serde_json::Value],
) -> Result<String> {
    let bytes = serde_json::to_vec(&("weave-fact-revision-v1", graph, branch, parent, ops))?;
    Ok(format!("{FACT_PREFIX}{:x}", Sha256::digest(bytes)))
}

/// Node valid time (ADR-0004 section 8). Removes nodes whose explicit valid
/// time excludes `t` and the edges that need them. Returns the IDs of nodes
/// with an explicit valid time that contains `t`.
pub(crate) fn retain_valid_nodes(graph: &mut GraphData, t: i64) -> HashSet<String> {
    graph
        .nodes
        .retain(|n| n.valid_time.as_ref().is_none_or(|v| v.contains(t)));
    let ids: HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    graph
        .edges
        .retain(|e| ids.contains(e.from.as_str()) && ids.contains(e.to.as_str()));
    graph
        .structural_edges
        .retain(|e| ids.contains(e.from.as_str()) && ids.contains(e.to.as_str()));
    graph
        .nodes
        .iter()
        .filter(|n| n.valid_time.is_some())
        .map(|n| n.id.clone())
        .collect()
}

/// Validate the embedding spaces and embeddings of a write's neighborhood
/// (ADR-0006): every space is a bounded descriptor, and every embedding
/// matches its space exactly and names a live subject of the right kind.
pub(crate) fn validate_embeddings(data: &GraphData) -> Result<()> {
    let contract = |e: weave_contract::ContractError| err(&e.code, &e.message);
    let mut spaces: HashMap<String, &EmbeddingSpace> = HashMap::new();
    for space in &data.embedding_spaces {
        space.validate().map_err(contract)?;
        spaces.insert(space.reference().key(), space);
    }
    let nodes: HashSet<&str> = data.nodes.iter().map(|n| n.id.as_str()).collect();
    let entities: HashSet<&str> = data.nodes.iter().map(|n| n.entity_id.as_str()).collect();
    let edges: HashSet<&str> = data
        .edges
        .iter()
        .map(|e| e.id.as_str())
        .chain(data.structural_edges.iter().map(|e| e.id.as_str()))
        .collect();
    let assertions: HashSet<&str> = data.assertions.iter().map(|a| a.id.as_str()).collect();
    for embedding in &data.embeddings {
        let space = spaces.get(&embedding.space.key()).ok_or_else(|| {
            err(
                "E_SPACE",
                "an embedding names a space that is not declared in the graph",
            )
        })?;
        embedding.validate(space).map_err(contract)?;
        let gates = embedding
            .derived_from
            .len()
            .saturating_add(embedding.derived_nodes.len())
            .saturating_add(embedding.derived_snapshots.len());
        if gates > 1000 {
            return Err(err("E_BUDGET", "an embedding carries at most 1000 gates"));
        }
        let bounded =
            embedding.derived_from.iter().all(|r| {
                valid_id(&r.graph_id) && valid_id(&r.revision) && valid_id(&r.assertion_id)
            }) && embedding
                .derived_nodes
                .iter()
                .all(|r| valid_id(&r.graph_id) && valid_id(&r.revision) && valid_id(&r.node_id))
                && embedding
                    .derived_snapshots
                    .iter()
                    .all(|r| valid_id(&r.graph_id) && valid_id(&r.revision))
                && embedding.readers.iter().all(|r| valid_id(r));
        if !bounded {
            return Err(err(
                "E_REFERENCE",
                "embedding readers and gates need bounded identifiers",
            ));
        }
        let present = match &embedding.subject {
            EmbeddingSubject::Node { node_id } => nodes.contains(node_id.as_str()),
            EmbeddingSubject::Entity { entity_id } => entities.contains(entity_id.as_str()),
            EmbeddingSubject::Edge { edge_id } => edges.contains(edge_id.as_str()),
            EmbeddingSubject::Assertion { assertion_id } => {
                assertions.contains(assertion_id.as_str())
            }
        };
        if !present {
            return Err(err(
                "E_ENDPOINT",
                "an embedding subject must be a live record of its kind",
            ));
        }
    }
    Ok(())
}
