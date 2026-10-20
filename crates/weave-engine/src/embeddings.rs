//! A4 operators (protocol 0.24.0, ADR-0006): exact `nearest` and
//! `similar_candidates` over embedding facts.
//!
//! Both read one revision through [`FactSource`], select embeddings of one
//! space through the fact index (by space, subject and valid time), and
//! authorize them with [`Engine::authorize_attributed`], the engine's one
//! authorization path, together with their subjects. Only then are they
//! ranked.
//!
//! **Hidden input.** An embedding the reader cannot see (its readers, its
//! subject or one of its gates exclude the reader) is dropped before ranking,
//! so it never changes the rank, score or count of visible embeddings, nor
//! the tie-break between them. A *denied* embedding (readable, but a gate
//! denies it) makes the result partial, as in every read; an embedding whose
//! readers exclude the reader, or whose subject is hidden, is simply absent.
//!
//! **Ranking boundary.** Ranking goes through [`NeighborSearch`]. Phase A has
//! one implementation, [`ExactScan`]. An approximate index (C5) implements
//! the same trait over the already-authorized selection and must fall back to
//! the exact scan; results report which one ran (`QueryResult.exactness`).
use super::*;
use crate::fact_store::{self as store, FactFilter, Namespace, StoredFact};
use crate::facts::{self, FactSource};
use crate::traversal::restrict;
use std::collections::{BTreeSet, HashMap};
use weave_contract::embedding::{
    CandidateRelation, CandidatesPlan, EmbeddingFact, EmbeddingMetric, EmbeddingSpace,
    EmbeddingSubject, Exactness, NearestPlan, NearestQuery, Neighbor, SpaceRef,
    CANDIDATE_PREDICATE, E_NEAREST, E_SPACE, MAX_CANDIDATE_LIMIT, MAX_COMPARISONS, MAX_K,
    MAX_SCAN_COMPONENTS,
};
use weave_contract::provenance::FactRef;

/// One ranked embedding: its index in the candidate slice and its score.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ranked {
    pub(crate) index: usize,
    pub(crate) score: f64,
}

/// The ranking boundary between selection and results.
///
/// `candidates` are already selected (space, valid time, candidate set) and
/// authorized. An implementation returns the first `k` of them in rank order:
/// ascending score, then ascending subject, then ascending embedding ID.
pub(crate) trait NeighborSearch {
    fn exactness(&self) -> Exactness;
    fn rank(
        &self,
        metric: EmbeddingMetric,
        query: &[f32],
        candidates: &[&EmbeddingFact],
        k: usize,
    ) -> Vec<Ranked>;
}

/// Exhaustive ranking: every candidate is scored.
pub(crate) struct ExactScan;

/// The total rank order (ADR-0006 ranking rule 2).
fn rank_order(a: (f64, &EmbeddingFact), b: (f64, &EmbeddingFact)) -> std::cmp::Ordering {
    a.0.total_cmp(&b.0)
        .then_with(|| a.1.subject.cmp(&b.1.subject))
        .then_with(|| a.1.id.cmp(&b.1.id))
}

impl NeighborSearch for ExactScan {
    fn exactness(&self) -> Exactness {
        Exactness::Exact
    }
    fn rank(
        &self,
        metric: EmbeddingMetric,
        query: &[f32],
        candidates: &[&EmbeddingFact],
        k: usize,
    ) -> Vec<Ranked> {
        let mut scored: Vec<Ranked> = candidates
            .iter()
            .enumerate()
            .map(|(index, e)| Ranked {
                index,
                score: metric.distance(query, &e.vector),
            })
            .collect();
        let order = |a: &Ranked, b: &Ranked| {
            rank_order(
                (a.score, candidates[a.index]),
                (b.score, candidates[b.index]),
            )
        };
        if scored.len() > k && k > 0 {
            scored.select_nth_unstable_by(k - 1, order);
            scored.truncate(k);
        }
        scored.sort_by(order);
        scored.truncate(k);
        scored
    }
}

/// Subjects a candidate value admits, by origin in the operator's graph.
#[derive(Default)]
struct Subjects {
    nodes: BTreeSet<String>,
    entities: BTreeSet<String>,
    records: BTreeSet<String>,
}

impl Subjects {
    fn of(value: &QueryResult, graph: &str) -> Self {
        let mut subjects = Subjects::default();
        for node in &value.graph.nodes {
            for origin in value.node_origins.get(&node.id).into_iter().flatten() {
                if origin.graph_id == graph {
                    subjects.nodes.insert(origin.node_id.clone());
                    subjects.entities.insert(node.entity_id.clone());
                }
            }
        }
        for edge in &value.graph.edges {
            for origin in value.edge_origins.get(&edge.id).into_iter().flatten() {
                if origin.graph_id == graph {
                    subjects.records.insert(origin.assertion_id.clone());
                }
            }
        }
        subjects
    }
    fn admits(&self, subject: &EmbeddingSubject) -> bool {
        match subject {
            EmbeddingSubject::Node { node_id } => self.nodes.contains(node_id),
            EmbeddingSubject::Entity { entity_id } => self.entities.contains(entity_id),
            EmbeddingSubject::Edge { edge_id } => self.records.contains(edge_id),
            EmbeddingSubject::Assertion { assertion_id } => self.records.contains(assertion_id),
        }
    }
    fn len(&self) -> usize {
        self.nodes.len() + self.entities.len() + self.records.len()
    }
}

/// Embeddings authorized together by a streaming ranking.
const SCAN_CHUNK: usize = 256;

/// A streaming exact top-k over stored embeddings (`nearest`).
///
/// Every offered embedding counts its dimensions against
/// [`MAX_SCAN_COMPONENTS`] before it is read further, whether or not the
/// reader can see it, like the read budget. Chunks are authorized with their
/// subjects through the one authorization path; only visible, selected
/// embeddings are ranked, and only the best `k` are kept.
struct Ranking<'a> {
    engine: &'a Engine,
    source: &'a dyn FactSource,
    host: &'a HostContext,
    subjects: Option<&'a Subjects>,
    excluded: Option<&'a EmbeddingSubject>,
    metric: EmbeddingMetric,
    query: &'a [f32],
    k: usize,
    dimensions: u64,
    scanned: u64,
    max_components: u64,
    chunk: Vec<StoredFact>,
    best: Vec<EmbeddingFact>,
    denied: bool,
}

impl Ranking<'_> {
    fn offer(&mut self, stored: StoredFact) -> Result<()> {
        self.scanned = self.scanned.saturating_add(self.dimensions);
        if self.scanned > self.max_components {
            return Err(err(
                "E_BUDGET",
                "nearest scans at most 268435456 vector components",
            ));
        }
        self.chunk.push(stored);
        if self.chunk.len() == SCAN_CHUNK {
            self.flush()?;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        if self.chunk.is_empty() {
            return Ok(());
        }
        let stored = std::mem::take(&mut self.chunk);
        let (visible, denied) =
            self.engine
                .authorize_embeddings_charging(self.source, stored, self.host, false)?;
        self.denied |= denied;
        let mut pool: Vec<EmbeddingFact> = std::mem::take(&mut self.best);
        pool.extend(
            visible
                .into_values()
                .filter(|e| self.subjects.is_none_or(|s| s.admits(&e.subject)))
                .filter(|e| self.excluded != Some(&e.subject)),
        );
        let refs: Vec<&EmbeddingFact> = pool.iter().collect();
        let keep: Vec<usize> = ExactScan
            .rank(self.metric, self.query, &refs, self.k)
            .into_iter()
            .map(|r| r.index)
            .collect();
        let mut slots: Vec<Option<EmbeddingFact>> = pool.into_iter().map(Some).collect();
        self.best = keep.into_iter().filter_map(|i| slots[i].take()).collect();
        Ok(())
    }
}

fn contract(e: weave_contract::ContractError) -> Error {
    err(&e.code, &e.message)
}

fn space_ref_valid(space: &SpaceRef) -> bool {
    valid_id(&space.id) && valid_id(&space.revision)
}

/// Intersection of two half-open intervals, if nonempty.
fn intersect(a: &Interval, b: &Interval) -> Option<Interval> {
    let start = a.start.max(b.start);
    let end = match (a.end, b.end) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    };
    end.is_none_or(|end| start < end)
        .then_some(Interval { start, end })
}

/// Merge another value's coverage and inputs into a result.
fn inherit(result: &mut QueryResult, input: &QueryResult) {
    if input.coverage == Coverage::Partial {
        result.coverage = Coverage::Partial;
        for diagnostic in &input.diagnostics {
            if !result.diagnostics.iter().any(|d| d.code == diagnostic.code) {
                result.diagnostics.push(diagnostic.clone());
            }
        }
    }
    for reference in &input.input_snapshots {
        if !result.input_snapshots.contains(reference) {
            result.input_snapshots.push(reference.clone());
        }
    }
}

/// The embeddings, spaces, ranking and candidates of a value are its own
/// output: another operator takes only its graph records.
pub(crate) fn strip_embedding_output(value: &mut QueryResult) {
    value.graph.embedding_spaces.clear();
    value.graph.embeddings.clear();
    value.neighbors.clear();
    value.candidates.clear();
    value.exactness = None;
}

impl Engine {
    /// The space a plan names, read from the revision. Spaces are public
    /// descriptors, so a missing space is reported as such.
    fn space_in(&self, source: &dyn FactSource, space: &SpaceRef) -> Result<EmbeddingSpace> {
        let stored = source
            .get(Namespace::Space, &space.key())?
            .ok_or_else(|| err(E_SPACE, "embedding space is not declared in the graph"))?;
        match facts::parse(&stored.version)? {
            Fact::EmbeddingSpace(found) => Ok(found),
            _ => Err(err("E_INTEGRITY", "stored space has another kind")),
        }
    }

    /// The stored embeddings of `space` valid at `valid_at`, restricted to
    /// `subjects` when given, through the fact index.
    fn select_embeddings(
        &self,
        source: &dyn FactSource,
        space: &SpaceRef,
        valid_at: Option<i64>,
        subjects: Option<&Subjects>,
    ) -> Result<Vec<StoredFact>> {
        let limit = self.materialization_limit();
        let filter = |subject: Option<&String>, entity: Option<&String>| FactFilter {
            kind: Some(store::FactKind::Embedding),
            predicate: Some(space.key()),
            subject: subject.cloned(),
            entity: entity.cloned(),
            valid_at,
            ..FactFilter::default()
        };
        let Some(subjects) = subjects else {
            return source.select(&filter(None, None), limit);
        };
        let mut found: BTreeMap<String, StoredFact> = BTreeMap::new();
        for id in subjects.nodes.iter().chain(&subjects.records) {
            for stored in source.select(&filter(Some(id), None), limit)? {
                found.insert(stored.version.id.clone(), stored);
            }
        }
        for entity in &subjects.entities {
            for stored in source.select(&filter(None, Some(entity)), limit)? {
                found.insert(stored.version.id.clone(), stored);
            }
        }
        if found.len() > limit {
            return Err(err("E_BUDGET", "fact selection exceeds its limit"));
        }
        Ok(found.into_values().collect())
    }

    /// Authorize stored embeddings with their subjects. Returns the visible
    /// embeddings (by ID) and whether a readable one was denied.
    fn authorize_embeddings(
        &self,
        source: &dyn FactSource,
        stored: Vec<StoredFact>,
        host: &HostContext,
    ) -> Result<(BTreeMap<String, EmbeddingFact>, bool)> {
        self.authorize_embeddings_charging(source, stored, host, true)
    }

    /// [`Self::authorize_embeddings`]; with `charge` false the embedding
    /// bodies are not charged to the read budget (the scan budget counts them).
    fn authorize_embeddings_charging(
        &self,
        source: &dyn FactSource,
        stored: Vec<StoredFact>,
        host: &HostContext,
        charge: bool,
    ) -> Result<(BTreeMap<String, EmbeddingFact>, bool)> {
        if stored.is_empty() {
            return Ok((BTreeMap::new(), false));
        }
        let closure = self.closure_charging(source, stored, false, charge)?;
        let attributed = self.authorize_attributed(closure, host)?;
        let denied = attributed.header_incomplete
            || attributed
                .denied
                .keys()
                .any(|(kind, _)| *kind == FactKind::Embedding);
        let visible = attributed
            .graph
            .embeddings
            .into_iter()
            .map(|e| (e.id.clone(), e))
            .collect();
        Ok((visible, denied))
    }

    /// The graph value of an embedding operator: the visible nodes behind the
    /// given subjects, with their hosted attachments and exact origins.
    fn subject_value(
        &self,
        source: &dyn FactSource,
        graph: &str,
        branch: &str,
        revision: &str,
        subjects: &[&EmbeddingSubject],
        host: &HostContext,
    ) -> Result<QueryResult> {
        let limit = self.materialization_limit();
        let mut records: Vec<StoredFact> = Vec::new();
        for subject in subjects {
            match subject {
                EmbeddingSubject::Node { node_id } => {
                    records.extend(source.get(Namespace::Node, node_id)?)
                }
                EmbeddingSubject::Entity { entity_id } => records.extend(source.select(
                    &FactFilter {
                        kind: Some(store::FactKind::Node),
                        entity: Some(entity_id.clone()),
                        ..FactFilter::default()
                    },
                    limit,
                )?),
                EmbeddingSubject::Edge { .. } | EmbeddingSubject::Assertion { .. } => {}
            }
        }
        let nodes: BTreeMap<String, u32> =
            records.iter().map(|s| (s.version.id.clone(), 0)).collect();
        records.extend(self.whole_value_records(source)?);
        let attributed =
            self.authorize_attributed(self.closure_in(source, records, true)?, host)?;
        let incomplete = attributed.header_incomplete
            || attributed.denied.values().any(|fact| match fact {
                Fact::Node(n) => nodes.contains_key(&n.id),
                Fact::Attachment(a) => match &a.host {
                    MetadataHost::Graph => true,
                    MetadataHost::Node { id } => nodes.contains_key(id),
                    _ => false,
                },
                _ => false,
            });
        let query = QueryPlan {
            graph_id: graph.to_string(),
            revision: Some(revision.to_string()),
            branch_id: branch.to_string(),
            predicate: None,
            from: None,
            to: None,
            valid_at: None,
            include_metadata: false,
            max_depth: 8,
        };
        let mut result =
            self.query_authorized(&query, revision.to_string(), attributed.graph, false, host)?;
        restrict(&mut result, &nodes, &BTreeSet::new(), None);
        if incomplete {
            partial(
                &mut result,
                "E_DERIVATION_UNAVAILABLE",
                "some derivation dependencies unavailable",
            );
        }
        Ok(result)
    }

    /// `nearest` (A4): the `k` visible embeddings of a space closest to a query.
    pub(crate) fn nearest(
        &self,
        plan: &NearestPlan,
        values: &BTreeMap<String, QueryResult>,
        host: &HostContext,
        depth: u32,
        budget: &mut usize,
    ) -> Result<QueryResult> {
        let _read_scope = self.read_budget.enter();
        if !valid_id(&plan.graph_id)
            || !valid_id(&plan.branch_id)
            || !valid_id(&host.principal)
            || !space_ref_valid(&plan.space)
            || plan.revision.as_ref().is_some_and(|r| !valid_id(r))
            || matches!(&plan.query, NearestQuery::Embedding { embedding_id } if !valid_id(embedding_id))
        {
            return Err(err("E_ID", "identifiers require 1 to 512 UTF-8 bytes"));
        }
        if plan.k == 0 {
            return Err(err(E_NEAREST, "nearest needs k of at least 1"));
        }
        if plan.k > MAX_K {
            return Err(err("E_BUDGET", "nearest returns at most 1000 neighbors"));
        }
        let read_transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let _clock_scope = self.operation_scope()?;
        let candidate_value = plan
            .candidates
            .as_ref()
            .map(|c| self.operand(c, values, host, depth + 1, budget))
            .transpose()?;
        let revision = self
            .select_revision(
                &plan.graph_id,
                &plan.branch_id,
                &plan.revision,
                plan.as_of,
                host,
            )?
            .ok_or_else(|| err("E_UNAVAILABLE", "graph unavailable"))?;
        let source = self.fact_source(&plan.graph_id, &revision, host)?;
        let source = source.as_ref();
        let space = self.space_in(source, &plan.space)?;
        let mut incomplete = false;
        // The query vector, and the subject a query fact excludes.
        let (query, excluded) = match &plan.query {
            NearestQuery::Vector { vector } => {
                space.validate_vector(vector).map_err(contract)?;
                (vector.clone(), None)
            }
            NearestQuery::Embedding { embedding_id } => {
                let unavailable = || err("E_UNAVAILABLE", "query embedding unavailable");
                let stored = source
                    .get(Namespace::Record, embedding_id)?
                    .filter(|s| s.version.kind == store::FactKind::Embedding)
                    .ok_or_else(unavailable)?;
                let (visible, _) = self.authorize_embeddings(source, vec![stored], host)?;
                let fact = visible.into_values().next().ok_or_else(unavailable)?;
                if fact.space != plan.space {
                    return Err(err(E_SPACE, "embedding spaces differ"));
                }
                (fact.vector, Some(fact.subject))
            }
        };
        let subjects = candidate_value
            .as_ref()
            .map(|v| Subjects::of(v, &plan.graph_id));
        // Stream the selection in chunks: authorize each chunk, then keep
        // only the best k. Memory is O(k + chunk), whatever the space size.
        let mut ranking = Ranking {
            engine: self,
            source,
            host,
            subjects: subjects.as_ref(),
            excluded: excluded.as_ref(),
            metric: space.metric,
            query: &query,
            k: plan.k as usize,
            dimensions: u64::from(space.dimensions),
            scanned: 0,
            max_components: MAX_SCAN_COMPONENTS,
            chunk: Vec::new(),
            best: Vec::new(),
            denied: false,
        };
        let filter = |subject: Option<&String>, entity: Option<&String>| FactFilter {
            kind: Some(store::FactKind::Embedding),
            predicate: Some(plan.space.key()),
            subject: subject.cloned(),
            entity: entity.cloned(),
            valid_at: plan.valid_at,
            ..FactFilter::default()
        };
        match subjects.as_ref() {
            None => source.scan(&filter(None, None), &mut |stored| ranking.offer(stored))?,
            Some(subjects) => {
                let limit = self.materialization_limit();
                let mut seen: HashSet<String> = HashSet::new();
                for (subject, entity) in subjects
                    .nodes
                    .iter()
                    .chain(&subjects.records)
                    .map(|id| (Some(id), None))
                    .chain(subjects.entities.iter().map(|e| (None, Some(e))))
                {
                    for stored in source.select(&filter(subject, entity), limit)? {
                        if seen.insert(stored.version.id.clone()) {
                            ranking.offer(stored)?;
                        }
                    }
                }
            }
        }
        ranking.flush()?;
        incomplete |= ranking.denied;
        let best = std::mem::take(&mut ranking.best);
        let selected: Vec<&EmbeddingFact> = best.iter().collect();
        let search = ExactScan;
        let ranked = search.rank(space.metric, &query, &selected, plan.k as usize);
        let hits: Vec<&EmbeddingFact> = ranked.iter().map(|r| selected[r.index]).collect();
        let mut result = self.subject_value(
            source,
            &plan.graph_id,
            &plan.branch_id,
            &revision,
            &hits.iter().map(|e| &e.subject).collect::<Vec<_>>(),
            host,
        )?;
        result.neighbors = ranked
            .iter()
            .enumerate()
            .map(|(rank, r)| Neighbor {
                rank: rank as u32 + 1,
                embedding_id: selected[r.index].id.clone(),
                subject: selected[r.index].subject.clone(),
                score: r.score,
            })
            .collect();
        result.graph.embedding_spaces = vec![space];
        result.graph.embeddings = hits.into_iter().cloned().collect();
        result.exactness = Some(search.exactness());
        if let Some(value) = &candidate_value {
            inherit(&mut result, value);
        }
        if incomplete {
            partial(
                &mut result,
                "E_DERIVATION_UNAVAILABLE",
                "some embeddings unavailable",
            );
        }
        json_size(&result, MATERIALIZED_LIMIT)?;
        if let Some(transaction) = read_transaction {
            transaction.commit()?;
        }
        Ok(result)
    }

    /// `similar_candidates` (A4): similarity-derived candidate relations
    /// between visible embeddings of one space. Never written as facts.
    pub(crate) fn similar_candidates(
        &self,
        plan: &CandidatesPlan,
        values: &BTreeMap<String, QueryResult>,
        host: &HostContext,
        depth: u32,
        budget: &mut usize,
    ) -> Result<QueryResult> {
        let _read_scope = self.read_budget.enter();
        if !valid_id(&plan.graph_id)
            || !valid_id(&plan.branch_id)
            || !valid_id(&host.principal)
            || !space_ref_valid(&plan.space)
            || plan.revision.as_ref().is_some_and(|r| !valid_id(r))
        {
            return Err(err("E_ID", "identifiers require 1 to 512 UTF-8 bytes"));
        }
        if plan.k == 0 || plan.limit == 0 {
            return Err(err(
                E_NEAREST,
                "similar_candidates needs k and limit of at least 1",
            ));
        }
        if plan.max_score.is_some_and(|s| !s.is_finite() || s < 0.0) {
            return Err(err(
                E_NEAREST,
                "max_score must be a finite nonnegative distance",
            ));
        }
        if plan.k > MAX_K || plan.limit > MAX_CANDIDATE_LIMIT {
            return Err(err(
                "E_BUDGET",
                "similar_candidates allows k up to 1000 and limit up to 100000",
            ));
        }
        let read_transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let _clock_scope = self.operation_scope()?;
        let mut evaluate = |expression: &Option<Box<GraphExpression>>| {
            expression
                .as_ref()
                .map(|e| self.operand(e, values, host, depth + 1, budget))
                .transpose()
        };
        let source_value = evaluate(&plan.sources)?;
        let target_value = evaluate(&plan.targets)?;
        let revision = self
            .select_revision(
                &plan.graph_id,
                &plan.branch_id,
                &plan.revision,
                plan.as_of,
                host,
            )?
            .ok_or_else(|| err("E_UNAVAILABLE", "graph unavailable"))?;
        let source = self.fact_source(&plan.graph_id, &revision, host)?;
        let source = source.as_ref();
        let space = self.space_in(source, &plan.space)?;
        let source_subjects = source_value
            .as_ref()
            .map(|v| Subjects::of(v, &plan.graph_id));
        let target_subjects = target_value
            .as_ref()
            .map(|v| Subjects::of(v, &plan.graph_id));
        // One selection and one authorization covers both sides.
        let mut stored: BTreeMap<String, StoredFact> = BTreeMap::new();
        for subjects in [&source_subjects, &target_subjects] {
            if subjects.as_ref().is_some_and(|s| s.len() == 0) {
                continue;
            }
            for fact in
                self.select_embeddings(source, &plan.space, plan.valid_at, subjects.as_ref())?
            {
                stored.insert(fact.version.id.clone(), fact);
            }
            if subjects.is_none() {
                break;
            }
        }
        let (visible, denied) =
            self.authorize_embeddings(source, stored.into_values().collect(), host)?;
        let side = |subjects: &Option<Subjects>| -> Vec<&EmbeddingFact> {
            visible
                .values()
                .filter(|e| subjects.as_ref().is_none_or(|s| s.admits(&e.subject)))
                .collect()
        };
        let (sources, targets) = (side(&source_subjects), side(&target_subjects));
        if (sources.len() as u64).saturating_mul(targets.len() as u64) > MAX_COMPARISONS {
            return Err(err(
                "E_BUDGET",
                "similar_candidates compares at most 50000000 pairs",
            ));
        }
        // Each source's k nearest co-valid targets of another subject.
        let search = ExactScan;
        let mut pairs: BTreeMap<(String, String), (f64, &EmbeddingFact, &EmbeddingFact)> =
            BTreeMap::new();
        for s in &sources {
            let eligible: Vec<&EmbeddingFact> = targets
                .iter()
                .copied()
                .filter(|t| {
                    t.subject != s.subject && intersect(&s.valid_time, &t.valid_time).is_some()
                })
                .collect();
            for ranked in search.rank(space.metric, &s.vector, &eligible, plan.k as usize) {
                if plan.max_score.is_some_and(|max| ranked.score > max) {
                    continue;
                }
                let t = eligible[ranked.index];
                let (lo, hi) = if (&s.subject, &s.id) <= (&t.subject, &t.id) {
                    (*s, t)
                } else {
                    (t, *s)
                };
                pairs.insert((lo.id.clone(), hi.id.clone()), (ranked.score, lo, hi));
            }
        }
        let mut pairs: Vec<(f64, &EmbeddingFact, &EmbeddingFact)> = pairs.into_values().collect();
        if plan.exclude_linked {
            let linked = self.linked_nodes(
                source,
                &plan.graph_id,
                &revision,
                &pairs,
                plan.valid_at,
                host,
            )?;
            pairs.retain(|(_, lo, hi)| match (&lo.subject, &hi.subject) {
                (EmbeddingSubject::Node { node_id: a }, EmbeddingSubject::Node { node_id: b }) => {
                    !linked.contains(&(a.clone(), b.clone()))
                }
                _ => true,
            });
        }
        pairs.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then_with(|| (&a.1.subject, &a.2.subject).cmp(&(&b.1.subject, &b.2.subject)))
                .then_with(|| (&a.1.id, &a.2.id).cmp(&(&b.1.id, &b.2.id)))
        });
        if pairs.len() > plan.limit as usize {
            return Err(err("E_BUDGET", "candidate relations exceed their limit"));
        }
        let origin = |e: &EmbeddingFact| FactRef::Embedding {
            graph_id: plan.graph_id.clone(),
            revision: revision.clone(),
            embedding_id: e.id.clone(),
        };
        let mut candidates = Vec::new();
        let mut endpoints: Vec<&EmbeddingSubject> = Vec::new();
        for (score, lo, hi) in &pairs {
            let origins = vec![origin(lo), origin(hi)];
            // Stable across revisions: the graph, space and the two embedding IDs.
            let digest = Sha256::digest(serde_json::to_vec(&(
                "weave-candidate-v1",
                &plan.graph_id,
                &plan.space,
                &lo.id,
                &hi.id,
            ))?);
            candidates.push(CandidateRelation {
                id: format!("weave:candidate:{digest:x}"),
                predicate: CANDIDATE_PREDICATE.into(),
                from: lo.subject.id().to_string(),
                to: hi.subject.id().to_string(),
                space: plan.space.clone(),
                score: *score,
                exactness: search.exactness(),
                valid_time: intersect(&lo.valid_time, &hi.valid_time)
                    .ok_or_else(|| err("E_INTEGRITY", "candidate without common valid time"))?,
                origins,
            });
            endpoints.extend([&lo.subject, &hi.subject]);
        }
        let mut result = self.subject_value(
            source,
            &plan.graph_id,
            &plan.branch_id,
            &revision,
            &endpoints,
            host,
        )?;
        result.candidates = candidates;
        result.exactness = Some(search.exactness());
        for value in source_value.iter().chain(&target_value) {
            inherit(&mut result, value);
        }
        if denied {
            partial(
                &mut result,
                "E_DERIVATION_UNAVAILABLE",
                "some embeddings unavailable",
            );
        }
        json_size(&result, MATERIALIZED_LIMIT)?;
        if let Some(transaction) = read_transaction {
            transaction.commit()?;
        }
        Ok(result)
    }

    /// Node pairs (both orders) joined by a visible edge, valid at `valid_at`
    /// when set, among the node pairs of `pairs`.
    fn linked_nodes(
        &self,
        source: &dyn FactSource,
        graph: &str,
        revision: &str,
        pairs: &[(f64, &EmbeddingFact, &EmbeddingFact)],
        valid_at: Option<i64>,
        host: &HostContext,
    ) -> Result<HashSet<(String, String)>> {
        let limit = self.materialization_limit();
        let mut wanted: HashSet<(String, String)> = HashSet::new();
        for (_, lo, hi) in pairs {
            if let (EmbeddingSubject::Node { node_id: a }, EmbeddingSubject::Node { node_id: b }) =
                (&lo.subject, &hi.subject)
            {
                wanted.insert((a.clone(), b.clone()));
                wanted.insert((b.clone(), a.clone()));
            }
        }
        let mut records: HashMap<String, StoredFact> = HashMap::new();
        for (from, to) in &wanted {
            for kind in [store::FactKind::Edge, store::FactKind::StructuralEdge] {
                for stored in source.select(
                    &FactFilter {
                        kind: Some(kind),
                        subject: Some(from.clone()),
                        object: Some(to.clone()),
                        ..FactFilter::default()
                    },
                    limit,
                )? {
                    if kind == store::FactKind::StructuralEdge {
                        for assertion in source.select(
                            &FactFilter {
                                kind: Some(store::FactKind::Assertion),
                                subject: Some(stored.version.id.clone()),
                                ..FactFilter::default()
                            },
                            limit,
                        )? {
                            records.insert(assertion.version.id.clone(), assertion);
                        }
                    }
                    records.insert(stored.version.id.clone(), stored);
                }
            }
        }
        if records.is_empty() {
            return Ok(HashSet::new());
        }
        let closure = self.closure_in(source, records.into_values().collect(), false)?;
        let (authorized, _) = self.authorized(closure, host)?;
        let (graph, _) = materialize(authorized, graph, revision)?;
        Ok(graph
            .edges
            .iter()
            .filter(|e| valid_at.is_none_or(|t| e.valid_time.contains(t)))
            .map(|e| (e.from.clone(), e.to.clone()))
            .filter(|pair| wanted.contains(pair))
            .flat_map(|(a, b)| [(a.clone(), b.clone()), (b, a)])
            .collect())
    }
}

/// The planned conversion of an untyped vector property into embedding
/// facts ([`Engine::vector_property_changes`]).
#[derive(Debug, Clone, PartialEq)]
pub struct VectorImport {
    /// The branch head the changes were computed against. Apply them with
    /// `write` and this `expected_head`.
    pub head: Option<String>,
    /// The space (when not yet declared) and one embedding per converted node.
    /// Empty when the graph already holds exactly these facts.
    pub changes: Vec<FactChange>,
    /// Nodes whose property was converted.
    pub converted: usize,
    /// Components whose binary32 value differs from the stored binary64 value.
    pub rounded: usize,
}

impl Engine {
    /// The explicit migration path from the untyped science `vector` node
    /// property to embedding facts (ADR-0006 "Compatibility path").
    ///
    /// Reads the branch head as `host` and, for every visible node with an
    /// array under `property`, plans one embedding fact of `space` on that
    /// node: components rounded to the nearest binary32 value (counted in
    /// `rounded`), the node's readers, `valid_time`, and provenance method
    /// `import` with the node at the head revision as evidence. The space is
    /// declared when the graph lacks it. Nothing is written: apply the
    /// changes with `write`. The property itself is never changed or
    /// reinterpreted, and science `nearest_vectors` keeps reading it as binary64.
    ///
    /// Fails with `E_SPACE`, converting nothing, when any selected value is
    /// not an array of finite numbers of the space's dimension, overflows
    /// binary32, or is the zero vector in a cosine space.
    #[allow(clippy::too_many_arguments)]
    pub fn vector_property_changes(
        &self,
        graph: &str,
        branch: &str,
        property: &str,
        space: &EmbeddingSpace,
        valid_time: &Interval,
        importer: &str,
        importer_version: &str,
        host: &HostContext,
    ) -> Result<VectorImport> {
        if !host.writable_graphs.contains(graph) {
            return Err(err(
                "E_FORBIDDEN",
                "host has not granted graph write authority",
            ));
        }
        if !valid_id(property) || !valid_id(importer) || !valid_id(importer_version) {
            return Err(err("E_ID", "identifiers require 1 to 512 UTF-8 bytes"));
        }
        space.validate().map_err(contract)?;
        if !valid_time.valid() {
            return Err(err("E_INTERVAL", "valid time end must exceed start"));
        }
        let head = self
            .head(graph, branch)?
            .ok_or_else(|| err("E_UNAVAILABLE", "graph unavailable"))?;
        let value = self.query(
            &QueryPlan {
                graph_id: graph.into(),
                revision: Some(head.clone()),
                branch_id: branch.into(),
                predicate: None,
                from: None,
                to: None,
                valid_at: None,
                include_metadata: false,
                max_depth: 8,
            },
            host,
        )?;
        // Evidence names the revision that recorded each node's current
        // version, so a repeated conversion plans nothing.
        let recorded_in: BTreeMap<String, String> = {
            let _read_scope = self.read_budget.enter();
            let read_transaction = if self.conn.is_autocommit() {
                Some(self.conn.unchecked_transaction()?)
            } else {
                None
            };
            let _clock_scope = self.operation_scope()?;
            let source = self.fact_source(graph, &head, host)?;
            let mut out = BTreeMap::new();
            for node in &value.graph.nodes {
                if node.properties.contains_key(property) {
                    if let Some(stored) = source.get(Namespace::Node, &node.id)? {
                        out.insert(node.id.clone(), stored.recording.revision);
                    }
                }
            }
            drop(source);
            if let Some(transaction) = read_transaction {
                transaction.commit()?;
            }
            out
        };
        let reference = space.reference();
        let mut facts = vec![Fact::EmbeddingSpace(space.clone())];
        let (mut converted, mut rounded) = (0, 0);
        let mut nodes: Vec<&Node> = value.graph.nodes.iter().collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        for node in nodes {
            let Some(raw) = node.properties.get(property) else {
                continue;
            };
            let bad = || err(E_SPACE, "a vector property is not a vector of this space");
            let raw = raw.as_array().ok_or_else(bad)?;
            let mut vector = Vec::with_capacity(raw.len());
            for component in raw {
                let wide = component
                    .as_f64()
                    .filter(|x| x.is_finite())
                    .ok_or_else(bad)?;
                let narrow = wide as f32;
                if !narrow.is_finite() {
                    return Err(err(E_SPACE, "a vector component overflows binary32"));
                }
                rounded += usize::from(f64::from(narrow) != wide);
                vector.push(narrow);
            }
            space.validate_vector(&vector).map_err(contract)?;
            let id = format!("{}#{}", node.id, reference.key());
            if !valid_id(&id) {
                return Err(err("E_ID", "converted embedding ID exceeds 512 bytes"));
            }
            facts.push(Fact::Embedding(EmbeddingFact {
                id,
                space: reference.clone(),
                subject: EmbeddingSubject::Node {
                    node_id: node.id.clone(),
                },
                vector,
                valid_time: valid_time.clone(),
                readers: node.readers.clone(),
                derived_from: vec![],
                derived_nodes: vec![],
                derived_snapshots: vec![],
                provenance: Some(weave_contract::provenance::ClaimedProvenance {
                    source: None,
                    method: weave_contract::provenance::Method::Import {
                        importer: importer.into(),
                        version: importer_version.into(),
                    },
                    evidence: vec![weave_contract::provenance::EvidenceRef::Node {
                        graph_id: graph.into(),
                        revision: recorded_in
                            .get(&node.id)
                            .cloned()
                            .unwrap_or_else(|| head.clone()),

                        node_id: node.id.clone(),
                    }],
                }),
            }));
            converted += 1;
        }
        let plan = self.reconcile_facts(graph, branch, &facts, host)?;
        Ok(VectorImport {
            head: plan.head,
            changes: plan.changes,
            converted,
            rounded,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::SnapshotSource;

    #[test]
    fn the_scan_budget_fails_with_e_budget_before_reading_further() {
        let engine = Engine::memory().unwrap();
        let source = SnapshotSource::new(&GraphData::default(), "sha256:r").unwrap();
        let host = HostContext::new("alice", Vec::<String>::new());
        let mut ranking = Ranking {
            engine: &engine,
            source: &source,
            host: &host,
            subjects: None,
            excluded: None,
            metric: EmbeddingMetric::Euclidean,
            query: &[0.0; 4],
            k: 1,
            dimensions: 4,
            scanned: 0,
            max_components: 8,
            chunk: Vec::new(),
            best: Vec::new(),
            denied: false,
        };
        let stored = || StoredFact {
            position: 0,
            version: store::FactVersion {
                kind: store::FactKind::Embedding,
                id: "e".into(),
                body: "{}".into(),
                keys: store::IndexKeys::default(),
            },
            recording: store::Recording {
                writer: "w".into(),
                on_behalf_of: None,
                recorded_at_ms: 0,
                revision: "sha256:r".into(),
            },
        };
        ranking.offer(stored()).unwrap();
        ranking.offer(stored()).unwrap();
        assert_eq!(ranking.offer(stored()).unwrap_err().code, "E_BUDGET");
        assert_eq!(
            ranking.chunk.len(),
            2,
            "the over-budget embedding is never read"
        );
        assert_eq!(MAX_SCAN_COMPONENTS, 268_435_456);
        const { assert!(100_000 * 1_536 < MAX_SCAN_COMPONENTS) };
    }
}
