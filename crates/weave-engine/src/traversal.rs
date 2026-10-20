//! A3 operators (protocol 0.23.0): k-hop `expand`, `changes` change sets and
//! `as_of` selection.
//!
//! Both operators read through [`FactSource`]: a fact revision through the
//! fact-store indexes, a snapshot revision through an in-memory index of its
//! payload. Every record they return is authorized by [`Engine::authorized`],
//! the engine's one authorization path, on the record together with the
//! closure it depends on (endpoints, structures, hosts), which gives the same
//! decision as authorizing the whole revision (ADR-0004 section 7).
//!
//! **Hidden input.** Records the reader cannot see are dropped before they
//! influence anything: traversal continues only from visible nodes along
//! visible edges, the result-size limit counts visible output, and coverage
//! becomes partial only for denials on records in the output's own region.
//! A record is *denied* when its readers admit the principal but a derivation
//! gate or dependency does not; a record whose readers exclude the principal
//! is simply absent and never makes coverage partial.
use super::*;
use crate::fact_store::{self as store, FactBackend, FactFilter, Namespace, StoredFact};
use crate::facts::{self, FactSource, Key, LineSource, SnapshotSource};
use std::collections::{BTreeSet, HashMap};
use weave_contract::embedding::EmbeddingSubject;
use weave_contract::traversal::{
    ChangeBound, ChangeEntry, ChangeKind, ChangeSet, ChangesPlan, Direction, ExpandPlan, MAX_HOPS,
    MAX_LIMIT, MAX_SCOPE, MAX_SEEDS,
};

/// A record's identity within one revision: its kind and ID.
pub(crate) type Slot = (FactKind, String);

fn slot(fact: &Fact) -> Slot {
    (fact.kind(), fact.key_id().into_owned())
}

fn derivation_count(fact: &Fact) -> usize {
    match fact {
        Fact::Node(n) => n.derivations.len(),
        Fact::Edge(e) => e.derivations.len(),
        Fact::StructuralEdge(_) => 0,
        Fact::Assertion(a) => a.derivations.len(),
        Fact::Attachment(a) => a.derivations.len(),
        Fact::EmbeddingSpace(_) | Fact::Embedding(_) => 0,
    }
}

fn influence_count(data: &GraphData) -> usize {
    data.influence.as_ref().map_or(0, |i| i.derivations.len())
}

/// An authorized graph value with its denials attributed to records.
pub(crate) struct Attributed {
    /// The authorized value.
    pub(crate) graph: GraphData,
    /// What the reader's `readers` admit, before derivation gates.
    pub(crate) readable: GraphData,
    /// Readable records that authorization removed or narrowed.
    pub(crate) denied: BTreeMap<Slot, Fact>,
    /// The graph-level gates denied the value, or narrowed its alternatives.
    pub(crate) header_incomplete: bool,
}

/// True when an expression uses `expand` or `changes`.
pub(crate) fn uses_traversal(expression: &GraphExpression) -> bool {
    let mut pending = vec![expression];
    while let Some(expression) = pending.pop() {
        match expression {
            GraphExpression::Expand(_)
            | GraphExpression::Changes(_)
            | GraphExpression::AsOf { .. }
            | GraphExpression::Nearest(_)
            | GraphExpression::SimilarCandidates(_) => return true,

            GraphExpression::Window { input, .. }
            | GraphExpression::TypedContext { input, .. }
            | GraphExpression::Counterparts { input, .. }
            | GraphExpression::Explain { input }
            | GraphExpression::Context { input, .. }
            | GraphExpression::Reason { input, .. }
            | GraphExpression::Project { input, .. }
            | GraphExpression::Support { input, .. }
            | GraphExpression::Metadata { input, .. }
            | GraphExpression::Filter { input, .. } => pending.push(input),
            GraphExpression::Sequence { left, right, .. }
            | GraphExpression::Union { left, right }
            | GraphExpression::Join { left, right, .. }
            | GraphExpression::Diff {
                before: left,
                after: right,
            } => pending.extend([left.as_ref(), right.as_ref()]),
            GraphExpression::Geometry { operation, .. } => pending.extend(operation.inputs()),
            GraphExpression::AcceptedHistory { .. }
            | GraphExpression::AcceptedGraph { .. }
            | GraphExpression::CurrentView { .. }
            | GraphExpression::ResolveIdentity { .. }
            | GraphExpression::Cluster { .. }
            | GraphExpression::Query { .. }
            | GraphExpression::RecordedQuery { .. }
            | GraphExpression::Reference { .. } => {}
        }
    }
    false
}

fn bounded_ids(ids: &[String], max: usize) -> Result<()> {
    if ids.len() > max {
        return Err(err("E_BUDGET", "too many identifiers in one expression"));
    }
    if ids.iter().any(|id| !valid_id(id)) {
        return Err(err("E_ID", "identifiers require 1 to 512 UTF-8 bytes"));
    }
    Ok(())
}

fn limit(value: u32) -> Result<usize> {
    if value == 0 || value > MAX_LIMIT {
        return Err(err("E_BUDGET", "limit must be between 1 and 200000"));
    }
    Ok(value as usize)
}

/// Keep only the listed nodes and edges of a query value, and the
/// attachments (valid at `valid_at`) whose host survives, with their origins.
pub(crate) fn restrict(
    value: &mut QueryResult,
    nodes: &BTreeMap<String, u32>,
    edges: &BTreeSet<String>,
    valid_at: Option<i64>,
) {
    value.graph.nodes.retain(|n| nodes.contains_key(&n.id));
    value.graph.edges.retain(|e| edges.contains(&e.id));
    if let Some(t) = valid_at {
        value.graph.attachments.retain(|a| a.valid_time.contains(t));
    }
    prune_attachments(&mut value.graph);
    let kept_attachments: HashSet<&str> = value
        .graph
        .attachments
        .iter()
        .map(|a| a.id.as_str())
        .collect();
    value.node_origins.retain(|id, _| nodes.contains_key(id));
    value
        .attachment_origins
        .retain(|id, _| kept_attachments.contains(id.as_str()));
    value.edge_origins.retain(|id, _| edges.contains(id));
    value.provenance = Vec::new();
    for edge in &value.graph.edges {
        for reference in value.edge_origins.get(&edge.id).into_iter().flatten() {
            if !value.provenance.contains(reference) {
                value.provenance.push(reference.clone());
            }
        }
    }
}

impl Engine {
    /// Authorize `data` and attribute every denial to the record it removed.
    pub(crate) fn authorize_attributed(
        &self,
        data: GraphData,
        host: &HostContext,
    ) -> Result<Attributed> {
        let readable = visible(data.clone(), &host.principal);
        let (graph, _) = self.authorized(data, host)?;
        let kept: HashMap<Slot, usize> = facts::facts_of(&graph)
            .iter()
            .map(|f| (slot(f), derivation_count(f)))
            .collect();
        let denied = facts::facts_of(&readable)
            .into_iter()
            .filter(|f| kept.get(&slot(f)) != Some(&derivation_count(f)))
            .map(|f| (slot(&f), f))
            .collect();
        let empty = GraphData::default();
        let header_incomplete = influence_count(&graph) != influence_count(&readable)
            || (facts::header_of(&graph) == empty && facts::header_of(&readable) != empty);
        Ok(Attributed {
            graph,
            readable,
            denied,
            header_incomplete,
        })
    }

    /// The revision an operator reads: exact, as of a recorded time, or the head.
    pub(crate) fn select_revision(
        &self,
        graph: &str,
        branch: &str,
        revision: &Option<String>,
        as_of: Option<i64>,
        host: &HostContext,
    ) -> Result<Option<String>> {
        match (revision, as_of) {
            (Some(_), Some(_)) => Err(err(
                "E_HISTORY_SELECTOR",
                "choose a revision or an as_of time",
            )),
            (Some(revision), None) => Ok(Some(revision.clone())),
            (None, Some(t)) => self.revision_as_of(graph, branch, t, host),
            (None, None) => self
                .head(graph, branch)?
                .map(Some)
                .ok_or_else(|| err("E_UNAVAILABLE", "graph unavailable")),
        }
    }

    /// The facts of one revision of `graph`, after the same access checks as a pinned read.
    pub(crate) fn fact_source(
        &self,
        graph: &str,
        revision: &str,
        host: &HostContext,
    ) -> Result<Box<dyn FactSource + '_>> {
        if !valid_id(revision) {
            return Err(err("E_ID", "identifiers require 1 to 512 UTF-8 bytes"));
        }
        if !self.protected_reference_allowed(graph, revision, host)? {
            return Err(err("E_UNAVAILABLE", "graph unavailable"));
        }
        if facts::is_fact_revision(revision) {
            self.read_budget.request()?;
            if self.verified_fact_revision(graph, revision)?.is_none() {
                return Err(err("E_UNAVAILABLE", "graph unavailable"));
            }
            return Ok(Box::new(LineSource::new(self.fact_store(), revision)));
        }
        let data = self
            .load(graph, revision)?
            .ok_or_else(|| err("E_UNAVAILABLE", "graph unavailable"))?;
        Ok(Box::new(SnapshotSource::new(&data, revision)?))
    }

    /// Every influence carrier and graph-hosted attachment: the records a
    /// filtered read always includes so whole-value influence is exact.
    pub(crate) fn whole_value_records(&self, source: &dyn FactSource) -> Result<Vec<StoredFact>> {
        let limit = self.materialization_limit();
        let mut records = source.select(
            &FactFilter {
                carrier: true,
                ..FactFilter::default()
            },
            limit,
        )?;
        records.extend(source.select(
            &FactFilter {
                kind: Some(store::FactKind::Attachment),
                unanchored: true,
                ..FactFilter::default()
            },
            limit,
        )?);
        Ok(records)
    }

    /// `as_of` (A3): `query` read from the revision the branch had at a
    /// recorded time. The selected revision is authorized record by record,
    /// so the result is exactly a read pinned to that revision: a reader who
    /// cannot see every record gets the visible subset, and nothing about the
    /// hidden ones.
    pub(crate) fn as_of(
        &self,
        query: &QueryPlan,
        unix_millis: i64,
        host: &HostContext,
    ) -> Result<QueryResult> {
        if query.revision.is_some() {
            return Err(err(
                "E_HISTORY_SELECTOR",
                "choose a revision or an as_of time",
            ));
        }
        if !valid_id(&query.graph_id) || !valid_id(&query.branch_id) || !valid_id(&host.principal) {
            return Err(err("E_ID", "identifiers require 1 to 512 UTF-8 bytes"));
        }
        let _read_scope = self.read_budget.enter();
        let read_transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let _clock_scope = self.operation_scope()?;
        let revision = self
            .revision_as_of(&query.graph_id, &query.branch_id, unix_millis, host)?
            .ok_or_else(|| err("E_UNAVAILABLE", "graph unavailable"))?;
        let mut pinned = query.clone();
        pinned.revision = Some(revision);
        let result = self.query(&pinned, host)?;
        if let Some(transaction) = read_transaction {
            transaction.commit()?;
        }
        Ok(result)
    }

    /// `expand` (A3): the exact k-hop neighborhood of the visible seeds.
    pub(crate) fn expand(
        &self,
        plan: &ExpandPlan,
        values: &BTreeMap<String, QueryResult>,
        host: &HostContext,
        depth: u32,
        budget: &mut usize,
    ) -> Result<QueryResult> {
        let _read_scope = self.read_budget.enter();
        if !valid_id(&plan.graph_id)
            || !valid_id(&plan.branch_id)
            || !valid_id(&host.principal)
            || plan.revision.as_ref().is_some_and(|r| !valid_id(r))
        {
            return Err(err("E_ID", "identifiers require 1 to 512 UTF-8 bytes"));
        }
        if plan.seeds.is_empty() && plan.seeds_from.is_none() {
            return Err(err("E_EXPAND", "expand needs at least one seed"));
        }
        bounded_ids(&plan.seeds, MAX_SEEDS)?;
        bounded_ids(&plan.predicates, MAX_SCOPE)?;
        if plan.hops == 0 {
            return Err(err("E_EXPAND", "expand needs at least one hop"));
        }
        if plan.hops > MAX_HOPS {
            return Err(err("E_BUDGET", "expand traverses at most 16 hops"));
        }
        let limit = limit(plan.limit)?;
        let read_transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let _clock_scope = self.operation_scope()?;
        // Seeds from another value (0.24): its nodes, by origin in this graph.
        let seed_value = plan
            .seeds_from
            .as_ref()
            .map(|input| self.operand(input, values, host, depth + 1, budget))
            .transpose()?;
        let mut seeded = plan.clone();
        if let Some(value) = &seed_value {
            for node in &value.graph.nodes {
                for origin in value.node_origins.get(&node.id).into_iter().flatten() {
                    if origin.graph_id == plan.graph_id && !seeded.seeds.contains(&origin.node_id) {
                        seeded.seeds.push(origin.node_id.clone());
                    }
                }
            }
            if seeded.seeds.len() > MAX_SEEDS {
                return Err(err("E_BUDGET", "too many identifiers in one expression"));
            }
        }
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
        let mut result = self.expand_in(&seeded, &revision, source.as_ref(), limit, host)?;
        if let Some(value) = &seed_value {
            if value.coverage == Coverage::Partial {
                result.coverage = Coverage::Partial;
                for diagnostic in &value.diagnostics {
                    if !result.diagnostics.iter().any(|d| d.code == diagnostic.code) {
                        result.diagnostics.push(diagnostic.clone());
                    }
                }
            }
            for reference in &value.input_snapshots {
                if !result.input_snapshots.contains(reference) {
                    result.input_snapshots.push(reference.clone());
                }
            }
        }

        if let Some(transaction) = read_transaction {
            transaction.commit()?;
        }
        Ok(result)
    }

    fn expand_in(
        &self,
        plan: &ExpandPlan,
        revision: &str,
        source: &dyn FactSource,
        limit: usize,
        host: &HostContext,
    ) -> Result<QueryResult> {
        let read_limit = self.materialization_limit();
        let predicates: BTreeSet<&str> = plan.predicates.iter().map(String::as_str).collect();
        let follows = |p: &str| predicates.is_empty() || predicates.contains(p);
        let single = (predicates.len() == 1).then(|| plan.predicates[0].clone());
        // Seeds: visible nodes, valid at `valid_at` when set.
        let mut seeds = Vec::new();
        let mut named = BTreeSet::new();
        for id in &plan.seeds {
            if named.insert(id.as_str()) {
                seeds.extend(source.get(Namespace::Node, id)?);
            }
        }
        let seed_records = seeds.clone();
        let authorized_seeds =
            self.authorize_attributed(self.closure_in(source, seeds, false)?, host)?;
        let mut seed_graph = authorized_seeds.graph;
        let present: HashSet<String> = seed_graph.nodes.iter().map(|n| n.id.clone()).collect();
        let seeds_unavailable = named.iter().any(|id| !present.contains(*id));
        if let Some(t) = plan.valid_at {
            facts::retain_valid_nodes(&mut seed_graph, t);
        }
        let mut nodes: BTreeMap<String, u32> =
            seed_graph.nodes.iter().map(|n| (n.id.clone(), 0)).collect();
        let mut frontier: Vec<String> = nodes.keys().cloned().collect();
        let mut edges: BTreeSet<String> = BTreeSet::new();
        // The stored record behind every traversed edge (an edge, or an explicit assertion).
        let mut traversed: Vec<StoredFact> = Vec::new();
        let mut incomplete = authorized_seeds.header_incomplete;
        if nodes.len() > limit {
            return Err(err("E_BUDGET", "expansion exceeds its result limit"));
        }
        for hop in 0..plan.hops {
            if frontier.is_empty() {
                break;
            }
            let mut candidates: BTreeMap<Key, StoredFact> = BTreeMap::new();
            let mut structures = Vec::new();
            for node in &frontier {
                let mut filters = Vec::new();
                for (outgoing, wanted) in [
                    (true, plan.direction != Direction::In),
                    (false, plan.direction != Direction::Out),
                ] {
                    if !wanted {
                        continue;
                    }
                    for kind in [store::FactKind::Edge, store::FactKind::StructuralEdge] {
                        filters.push(FactFilter {
                            kind: Some(kind),
                            subject: outgoing.then(|| node.clone()),
                            object: (!outgoing).then(|| node.clone()),
                            predicate: single.clone(),
                            valid_at: (kind == store::FactKind::Edge)
                                .then_some(plan.valid_at)
                                .flatten(),
                            ..FactFilter::default()
                        });
                    }
                }
                for filter in filters {
                    for stored in source.select(&filter, read_limit)? {
                        if !stored
                            .version
                            .keys
                            .predicate
                            .as_deref()
                            .is_some_and(follows)
                        {
                            continue;
                        }
                        let key = (Namespace::Record, stored.version.id.clone());
                        if stored.version.kind == store::FactKind::StructuralEdge
                            && !candidates.contains_key(&key)
                        {
                            structures.push(stored.version.id.clone());
                        }
                        candidates.insert(key, stored);
                    }
                }
            }
            for structure in structures {
                for stored in source.select(
                    &FactFilter {
                        kind: Some(store::FactKind::Assertion),
                        subject: Some(structure),
                        valid_at: plan.valid_at,
                        ..FactFilter::default()
                    },
                    read_limit,
                )? {
                    candidates.insert((Namespace::Record, stored.version.id.clone()), stored);
                }
            }
            if candidates.is_empty() {
                break;
            }
            let candidate_slots: BTreeSet<Slot> = candidates
                .values()
                .map(|s| (facts::contract_kind(s.version.kind), s.version.id.clone()))
                .collect();
            let closure = self.closure_in(source, candidates.values().cloned().collect(), false)?;
            let attributed = self.authorize_attributed(closure, host)?;
            incomplete |= attributed.header_incomplete
                || attributed
                    .denied
                    .keys()
                    .any(|s| candidate_slots.contains(s));
            let (mut graph, _) = materialize(attributed.graph, &plan.graph_id, revision)?;
            if let Some(t) = plan.valid_at {
                facts::retain_valid_nodes(&mut graph, t);
            }
            let from_frontier: HashSet<&str> = frontier.iter().map(String::as_str).collect();
            let mut next = Vec::new();
            for edge in &graph.edges {
                if !follows(&edge.predicate)
                    || plan.valid_at.is_some_and(|t| !edge.valid_time.contains(t))
                {
                    continue;
                }
                let mut reached = Vec::new();
                if plan.direction != Direction::In && from_frontier.contains(edge.from.as_str()) {
                    reached.push(&edge.to);
                }
                if plan.direction != Direction::Out && from_frontier.contains(edge.to.as_str()) {
                    reached.push(&edge.from);
                }
                if reached.is_empty() {
                    continue;
                }
                if edges.insert(edge.id.clone()) {
                    if let Some(stored) = candidates.get(&(Namespace::Record, edge.id.clone())) {
                        traversed.push(stored.clone());
                    }
                }
                for node in reached {
                    if !nodes.contains_key(node) {
                        nodes.insert(node.clone(), hop + 1);
                        next.push(node.clone());
                    }
                }
            }
            if nodes.len() + edges.len() > limit {
                return Err(err("E_BUDGET", "expansion exceeds its result limit"));
            }
            frontier = next;
        }
        // Assemble the value from the reached records, their hosted
        // attachments and the whole-value carriers, through the query pipeline.
        let mut records: Vec<StoredFact> = seed_records
            .into_iter()
            .filter(|s| nodes.contains_key(&s.version.id))
            .collect();
        records.extend(traversed.iter().cloned());
        records.extend(self.whole_value_records(source)?);
        let attributed =
            self.authorize_attributed(self.closure_in(source, records, true)?, host)?;
        let entities: HashSet<&str> = attributed
            .readable
            .nodes
            .iter()
            .filter(|n| nodes.contains_key(&n.id))
            .map(|n| n.entity_id.as_str())
            .collect();
        let structures: HashSet<String> = traversed
            .iter()
            .filter_map(|s| match facts::parse(&s.version) {
                Ok(Fact::Assertion(a)) => Some(a.edge_id),
                _ => None,
            })
            .collect();
        incomplete |= attributed.header_incomplete
            || attributed.denied.values().any(|fact| match fact {
                Fact::Node(n) => nodes.contains_key(&n.id),
                Fact::Edge(e) => edges.contains(&e.id),
                Fact::Assertion(a) => edges.contains(&a.id),
                Fact::StructuralEdge(e) => structures.contains(&e.id),
                Fact::Attachment(a) => match &a.host {
                    MetadataHost::Graph => true,
                    MetadataHost::Node { id } => nodes.contains_key(id),
                    MetadataHost::Entity { id } => entities.contains(id.as_str()),
                    MetadataHost::Edge { id } | MetadataHost::Assertion { id } => {
                        edges.contains(id) || structures.contains(id)
                    }
                },
                Fact::EmbeddingSpace(_) | Fact::Embedding(_) => false,
            });
        let query = QueryPlan {
            graph_id: plan.graph_id.clone(),
            revision: Some(revision.to_string()),
            branch_id: plan.branch_id.clone(),
            predicate: None,
            from: None,
            to: None,
            valid_at: None,
            include_metadata: false,
            max_depth: 8,
        };
        let mut result =
            self.query_authorized(&query, revision.to_string(), attributed.graph, false, host)?;
        restrict(&mut result, &nodes, &edges, plan.valid_at);
        if seeds_unavailable {
            partial(
                &mut result,
                "E_SEED_UNAVAILABLE",
                "some seeds are unavailable",
            );
        }
        if incomplete {
            partial(
                &mut result,
                "E_DERIVATION_UNAVAILABLE",
                "some derivation dependencies unavailable",
            );
        }
        json_size(&result, MATERIALIZED_LIMIT)?;
        Ok(result)
    }

    /// The revision one end of a change set names; `None` is the empty state.
    fn bound_revision(
        &self,
        graph: &str,
        branch: &str,
        bound: &ChangeBound,
        host: &HostContext,
    ) -> Result<Option<String>> {
        match bound {
            ChangeBound::AsOf { unix_millis } => {
                self.revision_as_of(graph, branch, *unix_millis, host)
            }
            ChangeBound::Cursor { revision } => {
                if !valid_id(revision) {
                    return Err(err("E_ID", "identifiers require 1 to 512 UTF-8 bytes"));
                }
                let owner: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT graph_id FROM revisions WHERE revision=?1",
                        [revision],
                        |r| r.get(0),
                    )
                    .optional()?;
                if owner.as_deref() != Some(graph) {
                    return Err(err("E_UNAVAILABLE", "graph unavailable"));
                }
                Ok(Some(revision.clone()))
            }
            ChangeBound::Head => self
                .head(graph, branch)?
                .map(Some)
                .ok_or_else(|| err("E_UNAVAILABLE", "graph unavailable")),
        }
    }

    /// Every fact of a source, charged to the read budget.
    fn all_facts(&self, source: &dyn FactSource) -> Result<Vec<StoredFact>> {
        let facts = source.select(&FactFilter::default(), self.materialization_limit())?;
        if source.charged() {
            for stored in &facts {
                self.read_budget.charge(stored.version.body.len())?;
            }
        }
        Ok(facts)
    }

    /// The records whose authorized value can depend on `fact`: edges and
    /// structures at a node, assertions on a structure and attachments on a host.
    fn dependents(&self, source: &dyn FactSource, fact: &Fact) -> Result<Vec<StoredFact>> {
        let limit = self.materialization_limit();
        let select = |kind, subject: Option<&str>, object: Option<&str>, entity: Option<&str>| {
            source.select(
                &FactFilter {
                    kind: Some(kind),
                    subject: subject.map(str::to_string),
                    object: object.map(str::to_string),
                    entity: entity.map(str::to_string),
                    ..FactFilter::default()
                },
                limit,
            )
        };
        let id = fact.id();
        if matches!(fact, Fact::EmbeddingSpace(_) | Fact::Embedding(_)) {
            return Ok(vec![]);
        }
        let mut out = select(store::FactKind::Attachment, Some(id), None, None)?;
        out.extend(select(store::FactKind::Embedding, Some(id), None, None)?);
        match fact {
            Fact::Node(n) => {
                for kind in [store::FactKind::Edge, store::FactKind::StructuralEdge] {
                    out.extend(select(kind, Some(id), None, None)?);
                    out.extend(select(kind, None, Some(id), None)?);
                }
                for kind in [store::FactKind::Attachment, store::FactKind::Embedding] {
                    out.extend(select(kind, None, None, Some(&n.entity_id))?);
                }
            }
            Fact::StructuralEdge(_) => {
                out.extend(select(store::FactKind::Assertion, Some(id), None, None)?)
            }
            Fact::Edge(_)
            | Fact::Assertion(_)
            | Fact::Attachment(_)
            | Fact::EmbeddingSpace(_)
            | Fact::Embedding(_) => {}
        }
        Ok(out)
    }

    /// `changes` (A3): the fact-by-fact difference of two authorized states.
    pub(crate) fn changes(&self, plan: &ChangesPlan, host: &HostContext) -> Result<QueryResult> {
        let _read_scope = self.read_budget.enter();
        if !valid_id(&plan.graph_id) || !valid_id(&plan.branch_id) || !valid_id(&host.principal) {
            return Err(err("E_ID", "identifiers require 1 to 512 UTF-8 bytes"));
        }
        bounded_ids(&plan.entities, MAX_SCOPE)?;
        bounded_ids(&plan.predicates, MAX_SCOPE)?;
        let limit = limit(plan.limit)?;
        let read_transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let _clock_scope = self.operation_scope()?;
        let graph = plan.graph_id.as_str();
        let from = self.bound_revision(graph, &plan.branch_id, &plan.from, host)?;
        let to = self.bound_revision(graph, &plan.branch_id, &plan.to, host)?;
        let from_source = from
            .as_deref()
            .map(|r| self.fact_source(graph, r, host))
            .transpose()?;
        let to_source = to
            .as_deref()
            .map(|r| self.fact_source(graph, r, host))
            .transpose()?;
        let result = self.changes_in(
            plan,
            (&from, from_source.as_deref()),
            (&to, to_source.as_deref()),
            limit,
            host,
        )?;
        if let Some(transaction) = read_transaction {
            transaction.commit()?;
        }
        Ok(result)
    }

    /// Keys whose stored version may differ between the two states, through
    /// the log when both lie on one fact line, else by comparing both states.
    fn changed_keys(
        &self,
        from: (&Option<String>, Option<&dyn FactSource>),
        to: (&Option<String>, Option<&dyn FactSource>),
    ) -> Result<BTreeSet<Key>> {
        if from.0 == to.0 {
            return Ok(BTreeSet::new());
        }
        let limit = self.materialization_limit();
        let store = self.fact_store();
        // The log answers when one state is on the other's line, or is the
        // revision that line started from; try both directions.
        let mut logged = None;
        for (other, revision) in [(from.0, to.0), (to.0, from.0)] {
            if let Some(revision) = revision.as_deref().filter(|r| facts::is_fact_revision(r)) {
                logged = store.changed_keys(other.as_deref(), revision, limit)?;
                if logged.is_some() {
                    break;
                }
            }
        }
        if let Some(keys) = logged {
            return Ok(keys.into_iter().collect());
        }
        let mut versions: BTreeMap<Key, [Option<String>; 2]> = BTreeMap::new();
        for (side, source) in [from.1, to.1].into_iter().enumerate() {
            let Some(source) = source else { continue };
            for stored in self.all_facts(source)? {
                versions
                    .entry((stored.version.kind.namespace(), stored.version.id.clone()))
                    .or_default()[side] = Some(stored.version.body);
            }
        }
        Ok(versions
            .into_iter()
            .filter(|(_, [a, b])| a != b)
            .map(|(key, _)| key)
            .collect())
    }

    fn changes_in(
        &self,
        plan: &ChangesPlan,
        from: (&Option<String>, Option<&dyn FactSource>),
        to: (&Option<String>, Option<&dyn FactSource>),
        limit: usize,
        host: &HostContext,
    ) -> Result<QueryResult> {
        let sides = [from.1, to.1];
        let read_limit = self.materialization_limit();
        // Keys whose stored content differs between the two states.
        let mut changed: BTreeSet<Key> = BTreeSet::new();
        let mut changed_slots: BTreeSet<Slot> = BTreeSet::new();
        for key in self.changed_keys(from, to)? {
            let mut versions = [None, None];
            for (side, source) in sides.iter().enumerate() {
                if let Some(source) = source {
                    if let Some(stored) = source.get(key.0, &key.1)? {
                        versions[side] = Some(facts::parse(&stored.version)?);
                    }
                }
            }
            if versions[0] != versions[1] {
                changed_slots.extend(versions.iter().flatten().map(slot));
                changed.insert(key);
            }
        }
        // Records whose authorized value can change with them.
        let mut region: BTreeSet<Key> = changed.clone();
        let mut pending: Vec<Key> = changed.iter().cloned().collect();
        while let Some(key) = pending.pop() {
            for source in sides.iter().flatten() {
                let Some(stored) = source.get(key.0, &key.1)? else {
                    continue;
                };
                for dependent in self.dependents(*source, &facts::parse(&stored.version)?)? {
                    let dependent = (dependent.version.kind.namespace(), dependent.version.id);
                    if region.insert(dependent.clone()) {
                        if region.len() > read_limit {
                            return Err(err("E_BUDGET", "change region exceeds the read budget"));
                        }
                        pending.push(dependent);
                    }
                }
            }
        }
        // Authorize the region in both states.
        let mut states: [Option<Attributed>; 2] = [None, None];
        for (side, source) in sides.iter().enumerate() {
            let Some(source) = source else { continue };
            let mut records = Vec::new();
            for key in &region {
                records.extend(source.get(key.0, &key.1)?);
            }
            let closure = self.closure_in(*source, records, false)?;
            states[side] = Some(self.authorize_attributed(closure, host)?);
        }
        let values = |side: usize| -> BTreeMap<Slot, Fact> {
            states[side]
                .as_ref()
                .map(|state| {
                    facts::facts_of(&state.graph)
                        .into_iter()
                        .filter(|f| region.contains(&facts::key(f)))
                        .map(|f| (slot(&f), f))
                        .collect()
                })
                .unwrap_or_default()
        };
        let (before, after) = (values(0), values(1));
        let entities: BTreeSet<&str> = plan.entities.iter().map(String::as_str).collect();
        let predicates: BTreeSet<&str> = plan.predicates.iter().map(String::as_str).collect();
        let scopes: Vec<Option<Scope>> = states
            .iter()
            .map(|state| {
                state
                    .as_ref()
                    .map(|state| Scope::new(&state.readable, &entities, &predicates))
            })
            .collect();
        let in_scope = |fact: &Fact, side: usize| -> bool {
            scopes[side]
                .as_ref()
                .is_some_and(|scope| scope.admits(fact))
        };
        let mut entries = Vec::new();
        let slots: BTreeSet<&Slot> = before.keys().chain(after.keys()).collect();
        for slot in slots {
            let (old, new) = (before.get(slot), after.get(slot));
            let change = match (old, new) {
                (Some(_), None) => ChangeKind::Retracted,
                (None, Some(_)) => ChangeKind::Asserted,
                (Some(a), Some(b)) if a != b => ChangeKind::Corrected,
                _ => continue,
            };
            if !(old.is_some_and(|f| in_scope(f, 0)) || new.is_some_and(|f| in_scope(f, 1))) {
                continue;
            }
            if entries.len() == limit {
                return Err(err("E_BUDGET", "change set exceeds its limit"));
            }
            entries.push(ChangeEntry {
                change,
                kind: slot.0,
                id: slot.1.clone(),
                before: old.cloned(),
                after: new.cloned(),
            });
        }
        let incomplete = states.iter().enumerate().any(|(side, state)| {
            state.as_ref().is_some_and(|state| {
                state.header_incomplete
                    || state
                        .denied
                        .iter()
                        .any(|(slot, fact)| changed_slots.contains(slot) && in_scope(fact, side))
            })
        });
        let mut input_snapshots = Vec::new();
        for revision in [from.0, to.0].into_iter().flatten() {
            let reference = GraphRef {
                graph_id: plan.graph_id.clone(),
                revision: revision.clone(),
            };
            if !input_snapshots.contains(&reference) {
                input_snapshots.push(reference);
            }
        }
        let mut result = QueryResult {
            recorded_observations: vec![],
            accepted_observations: vec![],
            selected_context: None,
            source_revisions: vec![],
            version: VERSION.into(),
            graph: GraphData::default(),
            snapshots: to
                .0
                .iter()
                .map(|r| (plan.graph_id.clone(), r.clone()))
                .collect(),
            input_snapshots,
            coverage: Coverage::Complete,
            diagnostics: vec![],
            provenance: vec![],
            edge_origins: BTreeMap::new(),
            node_origins: BTreeMap::new(),
            attachment_origins: BTreeMap::new(),
            metadata_graphs: vec![],
            fact_provenance: vec![],
            changes: Some(ChangeSet {
                from_revision: from.0.clone(),
                to_revision: to.0.clone(),
                entries,
            }),
            neighbors: vec![],
            candidates: vec![],
            exactness: None,
        };
        if incomplete {
            partial(
                &mut result,
                "E_DERIVATION_UNAVAILABLE",
                "some changes are unavailable",
            );
        }
        json_size(&result, MATERIALIZED_LIMIT)?;
        Ok(result)
    }
}

/// Resolves the endpoints and hosts a change-set scope needs.
struct Scope<'a> {
    entity_of: HashMap<&'a str, &'a str>,
    edges: HashMap<&'a str, (&'a str, &'a str)>,
    structures: HashMap<&'a str, (&'a str, &'a str, &'a str)>,
    assertions: HashMap<&'a str, &'a str>,
    entities: &'a BTreeSet<&'a str>,
    predicates: &'a BTreeSet<&'a str>,
}

impl<'a> Scope<'a> {
    fn new(
        graph: &'a GraphData,
        entities: &'a BTreeSet<&'a str>,
        predicates: &'a BTreeSet<&'a str>,
    ) -> Self {
        Self {
            entity_of: graph
                .nodes
                .iter()
                .map(|n| (n.id.as_str(), n.entity_id.as_str()))
                .collect(),
            edges: graph
                .edges
                .iter()
                .map(|e| (e.id.as_str(), (e.from.as_str(), e.to.as_str())))
                .collect(),
            structures: graph
                .structural_edges
                .iter()
                .map(|e| {
                    (
                        e.id.as_str(),
                        (e.from.as_str(), e.to.as_str(), e.predicate.as_str()),
                    )
                })
                .collect(),
            assertions: graph
                .assertions
                .iter()
                .map(|a| (a.id.as_str(), a.edge_id.as_str()))
                .collect(),
            entities,
            predicates,
        }
    }

    fn entity(&self, node: &str) -> bool {
        self.entity_of
            .get(node)
            .is_some_and(|e| self.entities.contains(e))
    }

    fn touches(&self, from: &str, to: &str) -> bool {
        self.entity(from) || self.entity(to)
    }

    /// True when a fact is about one of the scope's entities and carries one
    /// of its predicates (an empty list admits everything).
    fn admits(&self, fact: &Fact) -> bool {
        let (about, predicate): (bool, Option<&str>) = match fact {
            Fact::Node(n) => (self.entities.contains(n.entity_id.as_str()), None),
            Fact::Edge(e) => (self.touches(&e.from, &e.to), Some(&e.predicate)),
            Fact::StructuralEdge(e) => (self.touches(&e.from, &e.to), Some(&e.predicate)),
            Fact::Assertion(a) => match self.structures.get(a.edge_id.as_str()) {
                Some((from, to, predicate)) => (self.touches(from, to), Some(predicate)),
                None => (false, None),
            },
            Fact::Attachment(a) => {
                let about = match &a.host {
                    MetadataHost::Entity { id } => self.entities.contains(id.as_str()),
                    MetadataHost::Node { id } => self.entity(id),
                    MetadataHost::Edge { id } | MetadataHost::Assertion { id } => {
                        let structure = self
                            .assertions
                            .get(id.as_str())
                            .copied()
                            .unwrap_or(id.as_str());
                        self.edges
                            .get(id.as_str())
                            .is_some_and(|(from, to)| self.touches(from, to))
                            || self
                                .structures
                                .get(structure)
                                .is_some_and(|(from, to, _)| self.touches(from, to))
                    }
                    MetadataHost::Graph => false,
                };
                (about, Some(a.key.as_str()))
            }
            // Spaces are about no entity; embeddings are about their subject.
            // Their "predicate" is the space key, as in the fact index.
            Fact::EmbeddingSpace(_) => (false, None),
            Fact::Embedding(e) => {
                let about = match &e.subject {
                    EmbeddingSubject::Entity { entity_id } => {
                        self.entities.contains(entity_id.as_str())
                    }
                    EmbeddingSubject::Node { node_id } => self.entity(node_id),
                    EmbeddingSubject::Edge { edge_id } => {
                        self.edges
                            .get(edge_id.as_str())
                            .is_some_and(|(from, to)| self.touches(from, to))
                            || self
                                .structures
                                .get(edge_id.as_str())
                                .is_some_and(|(from, to, _)| self.touches(from, to))
                    }
                    EmbeddingSubject::Assertion { assertion_id } => self
                        .assertions
                        .get(assertion_id.as_str())
                        .and_then(|s| self.structures.get(s))
                        .is_some_and(|(from, to, _)| self.touches(from, to)),
                };
                return (self.entities.is_empty() || about)
                    && (self.predicates.is_empty()
                        || self.predicates.contains(e.space.key().as_str()));
            }
        };

        (self.entities.is_empty() || about)
            && (self.predicates.is_empty()
                || predicate.is_some_and(|p| self.predicates.contains(p)))
    }
}
