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
    /// True when the per-subject index lookups select this stored
    /// embedding: its subject key names a candidate node or record, or its
    /// entity key a candidate entity.
    #[cfg_attr(not(test), allow(dead_code))]
    fn selects(&self, stored: &StoredFact) -> bool {
        let keys = &stored.version.keys;
        keys.subject
            .as_ref()
            .is_some_and(|s| self.nodes.contains(s) || self.records.contains(s))
            || keys
                .entity
                .as_ref()
                .is_some_and(|e| self.entities.contains(e))
    }
}

#[cfg(test)]
thread_local! {
    /// Makes a restricted `nearest` select by a full scan, as the reference
    /// for the differential test of the index lookups.
    static SCAN_RESTRICTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
fn scan_restricted_for_tests() -> bool {
    SCAN_RESTRICTED.with(|s| s.get())
}
#[cfg(not(test))]
fn scan_restricted_for_tests() -> bool {
    false
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
        self.require_graph_snapshot(host, &plan.graph_id)?;
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
        // A restricted ranking looks up each visible candidate subject's
        // embeddings through the subject and entity index, so its cost
        // follows the candidates, not the space. The choice depends only on
        // whether the plan has candidates, never on what the reader can see:
        // `subjects` holds only the authorized candidate value's subjects.
        match subjects.as_ref() {
            None => source.scan(&filter(None, None), &mut |stored| ranking.offer(stored))?,
            // Test-only reference path: the same selection by a full scan.
            Some(subjects) if scan_restricted_for_tests() => {
                source.scan(&filter(None, None), &mut |stored| {
                    if subjects.selects(&stored) {
                        ranking.offer(stored)
                    } else {
                        Ok(())
                    }
                })?
            }
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
        self.require_graph_snapshot(host, &plan.graph_id)?;
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
        self.require_branch_write(host, graph, branch)?;
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
                reader_groups: node.reader_groups.clone(),
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

    struct Rng(u64);
    impl Rng {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
        fn pick<'a>(&mut self, items: &'a [&'a str]) -> &'a str {
            items[self.next(items.len() as u64) as usize]
        }
    }

    fn run(
        engine: &mut Engine,
        principal: &str,
        writes: bool,
        commands: serde_json::Value,
    ) -> String {
        let program: weave_contract::Program = serde_json::from_value(
            serde_json::json!({"version": weave_contract::VERSION, "commands": commands}),
        )
        .unwrap();
        let grants: Vec<String> = if writes {
            vec!["g".into(), "h".into()]
        } else {
            vec![]
        };
        match engine.execute(&program, &HostContext::new(principal, grants)) {
            Ok(out) => serde_json::to_string(&out).unwrap(),
            Err(e) => format!("error {} {}", e.code, e.message),
        }
    }

    /// A random store: nodes sharing entities, edges, and embeddings on
    /// nodes, entities and edges, with ties, valid times, reader
    /// restrictions and derivation gates on a hidden node, over three
    /// recorded writes.
    fn random_store(engine: &mut Engine, rng: &mut Rng) {
        use serde_json::json;
        let hidden = json!(["admin"]);
        let readers = |rng: &mut Rng| {
            if rng.next(4) == 0 {
                hidden.clone()
            } else {
                json!([])
            }
        };
        let w = |engine: &mut Engine, graph: &str, changes: Vec<serde_json::Value>| {
            let out = run(
                engine,
                "writer",
                true,
                json!([{"op": "write", "graph_id": graph, "changes": changes}]),
            );
            assert!(!out.starts_with("error"), "{out}");
            out
        };
        let gate = w(
            engine,
            "h",
            vec![
                json!({"change": "assert", "fact": {"kind": "node", "id": "text",
            "entity_id": "T", "space_id": "s", "readers": hidden}}),
            ],
        );
        let gate: serde_json::Value = serde_json::from_str(&gate).unwrap();
        let gate = gate[0]["revision"].as_str().unwrap().to_string();
        let mut changes = vec![
            json!({"change": "assert", "fact": {"kind": "embedding_space", "id": "sp",
            "revision": "1", "encoder": "t", "encoder_version": "1", "preprocessing": "none",
            "dimensions": 3, "metric": if rng.next(2) == 0 { "cosine_distance" } else { "euclidean" }}}),
        ];
        for i in 0..30 {
            changes.push(
                json!({"change": "assert", "fact": {"kind": "node", "id": format!("n{i}"),
                "entity_id": format!("E{}", i % 10), "space_id": "s", "readers": readers(rng)}}),
            );
        }
        for i in 0..40 {
            changes.push(
                json!({"change": "assert", "fact": {"kind": "edge", "id": format!("e{i}"),
                "predicate": rng.pick(&["p", "q"]), "from": format!("n{}", rng.next(30)),
                "to": format!("n{}", rng.next(30)), "valid_time": {"start": rng.next(3) as i64},
                "readers": readers(rng)}}),
            );
        }
        w(engine, "g", changes);
        for round in 0..2 {
            let mut changes = vec![];
            for j in 0..40 {
                let subject = match rng.next(5) {
                    0 => json!({"kind": "entity", "entity_id": format!("E{}", rng.next(10))}),
                    1 => json!({"kind": "edge", "edge_id": format!("e{}", rng.next(40))}),
                    _ => json!({"kind": "node", "node_id": format!("n{}", rng.next(30))}),
                };
                let vector: Vec<f32> = (0..3).map(|_| 1.0 + rng.next(3) as f32).collect();
                let start = rng.next(4) as i64;
                let end = if rng.next(3) == 0 {
                    json!(start + 1 + rng.next(4) as i64)
                } else {
                    json!(null)
                };
                let mut fact = json!({"kind": "embedding", "id": format!("v{round}-{j}"),
                    "space": {"id": "sp", "revision": "1"}, "subject": subject, "vector": vector,
                    "valid_time": {"start": start, "end": end}, "readers": readers(rng)});
                if rng.next(10) == 0 {
                    fact["derived_nodes"] =
                        json!([{"graph_id": "h", "revision": gate, "node_id": "text"}]);
                }
                changes.push(json!({"change": "assert", "fact": fact}));
            }
            w(engine, "g", changes);
        }
    }

    #[test]
    fn restricted_nearest_index_lookups_equal_a_full_scan_on_random_stores() {
        use serde_json::json;
        let (mut nonempty, mut partial, mut errors, mut plans) = (0, 0, 0, 0);
        for seed in 0..12u64 {
            let dir = tempfile::tempdir().unwrap();
            let mut engines = vec![Engine::memory().unwrap()];
            engines.push(Engine::open(dir.path().join("s.db")).unwrap());
            for engine in &mut engines {
                let mut rng = Rng(seed * 7919 + 13);
                random_store(engine, &mut rng);
                let mut rng = Rng(seed * 104_729 + 1);
                for _ in 0..25 {
                    let seeds: Vec<String> = (0..1 + rng.next(3))
                        .map(|_| format!("n{}", rng.next(31)))
                        .collect();
                    let candidates = match rng.next(3) {
                        0 => json!({"kind": "query", "query": {"graph_id": "g", "from": seeds[0]}}),
                        _ => {
                            json!({"kind": "expand", "graph_id": "g", "seeds": seeds, "hops": rng.next(3),
                            "direction": rng.pick(&["out", "in", "both"])})
                        }
                    };
                    let k = [1u32, 3, 10, 50][rng.next(4) as usize];
                    let mut plan = json!({"kind": "nearest", "graph_id": "g", "space": {"id": "sp", "revision": "1"},
                        "k": k, "candidates": candidates});
                    plan["query"] = if rng.next(3) == 0 {
                        json!({"kind": "embedding", "embedding_id": format!("v{}-{}", rng.next(2), rng.next(40))})
                    } else {
                        json!({"kind": "vector", "vector": (0..3).map(|_| 1.0 + rng.next(3) as f32).collect::<Vec<_>>()})
                    };
                    if rng.next(2) == 0 {
                        plan["valid_at"] = json!(rng.next(6));
                    }
                    for principal in ["admin", "analyst"] {
                        let commands = json!([{"op": "evaluate", "value": plan}]);
                        let lookup = run(engine, principal, false, commands.clone());
                        SCAN_RESTRICTED.with(|s| s.set(true));
                        let scan = run(engine, principal, false, commands);
                        SCAN_RESTRICTED.with(|s| s.set(false));
                        assert_eq!(lookup, scan, "seed {seed} {principal} {plan}");
                        plans += 1;
                        errors += usize::from(lookup.starts_with("error"));
                        nonempty += usize::from(lookup.contains("\"neighbors\""));
                        partial += usize::from(lookup.contains("\"partial\""));
                    }
                }
            }
        }
        assert!(nonempty > plans / 3, "{nonempty} of {plans} have hits");
        assert!(partial > 0, "derivation gates make some results partial");
        assert!(
            errors > 0 && errors < plans,
            "errors are compared too: {errors}"
        );
    }
}
