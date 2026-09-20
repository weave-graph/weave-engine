//! Exact membership maintenance for proof-free single-source Query/Filter values.
//! Snapshot decoding/comparison is O(input); materialization and repinning are O(output).
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub(crate) const STATE_LIMIT: usize = 64 * 1024 * 1024;
const EVALUATOR: &str = "selection-1";

/// Trusted embedding diagnostics. Never serialized into a graph value or delivery.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewSelectionWork {
    pub source_records: usize,
    pub hashed_records: usize,
    pub memberships_evaluated: usize,
    pub crossed_boundary_candidates: usize,
    pub output_nodes_repinned: usize,
    pub output_claims_rendered: usize,
    pub fallback_runs: usize,
    pub oracle_disagreements: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct State {
    evaluator: String,
    definition: String,
    principal: String,
    shape: String,
    revision: String,
    times: Vec<Option<i64>>,
    claims: BTreeMap<String, Member>,
    boundaries: BTreeMap<i64, BTreeSet<String>>,
    endpoints: BTreeMap<String, usize>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    fingerprint: String,
    from: String,
    to: String,
    selected: bool,
    start: Option<i64>,
    end: Option<i64>,
}
pub(crate) struct Plan<'a> {
    pub query: &'a QueryPlan,
    filters: Vec<(&'a Option<String>, Option<i64>)>,
}
impl<'a> Plan<'a> {
    pub fn parse(mut expression: &'a GraphExpression) -> Option<Self> {
        let mut filters = Vec::new();
        loop {
            match expression {
                GraphExpression::Query { query } => return Some(Self { query, filters }),
                GraphExpression::Filter {
                    input,
                    predicate,
                    valid_at,
                } if filters.len() < 32 => {
                    filters.push((predicate, *valid_at));
                    expression = input;
                }
                _ => return None,
            }
        }
    }
    fn times(&self) -> Vec<Option<i64>> {
        std::iter::once(self.query.valid_at)
            .chain(self.filters.iter().map(|(_, t)| *t))
            .collect()
    }
    fn prune_nodes(&self) -> bool {
        self.query.predicate.is_some()
            || self.query.from.is_some()
            || self.query.to.is_some()
            || self.query.valid_at.is_some()
            || self.filters.iter().any(|(p, t)| p.is_some() || t.is_some())
    }
    fn matches(&self, predicate: &str, from: &str, to: &str, time: &Interval) -> bool {
        self.query.predicate.as_ref().is_none_or(|p| p == predicate)
            && self.query.from.as_ref().is_none_or(|p| p == from)
            && self.query.to.as_ref().is_none_or(|p| p == to)
            && self.query.valid_at.is_none_or(|t| time.contains(t))
            && self.filters.iter().all(|(p, t)| {
                p.as_ref().is_none_or(|p| p == predicate) && t.is_none_or(|t| time.contains(t))
            })
    }
}

/// Classification uses raw verified input, before visibility filtering. No diagnostic is public.
pub(crate) fn eligible(data: &GraphData) -> bool {
    data.influence.is_none()
        && data.context_typing.is_none()
        && data.attachments.is_empty()
        && data.nodes.iter().all(|n| {
            n.metadata.is_empty()
                && n.derived_from.is_empty()
                && n.derived_nodes.is_empty()
                && n.context_scope.is_none()
        })
        && data.edges.iter().all(|e| {
            e.metadata.is_empty()
                && e.derived_from.is_empty()
                && e.derived_nodes.is_empty()
                && e.derivations.is_empty()
                && e.structural_ref.is_none()
                && e.assertion_context.is_none()
        })
        && data.structural_edges.iter().all(|e| e.metadata.is_empty())
        && data.assertions.iter().all(|a| {
            a.metadata.is_empty()
                && a.derived_from.is_empty()
                && a.derived_nodes.is_empty()
                && a.derivations.is_empty()
                && a.context.is_none()
        })
}

pub(crate) fn fingerprint(value: &impl Serialize) -> Result<String> {
    struct Hasher(Sha256);
    impl std::io::Write for Hasher {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut hash = Hasher(Sha256::new());
    serde_json::to_writer(&mut hash, value)?;
    Ok(format!("{:x}", hash.0.finalize()))
}

impl State {
    pub fn reusable(&self, definition: &str, principal: &str, raw: &GraphData) -> Result<bool> {
        Ok(self.definition == definition
            && self.principal == principal
            && self.shape == fingerprint(&(&raw.profile, &raw.schema))?)
    }
    /// Redundant index fields must agree even after valid JSON corruption.
    pub fn valid(&self) -> bool {
        if self.evaluator != EVALUATOR || self.claims.len() > STATE_LIMIT / 4096 {
            return false;
        }
        let mut endpoints = BTreeMap::new();
        let mut boundaries: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
        for (id, member) in &self.claims {
            for t in [member.start, member.end].into_iter().flatten() {
                boundaries.entry(t).or_default().insert(id.clone());
            }
            if member.selected {
                for endpoint in [&member.from, &member.to] {
                    *endpoints.entry(endpoint.clone()).or_insert(0usize) += 1;
                }
            }
        }
        endpoints == self.endpoints && boundaries == self.boundaries
    }
}

/// Caller supplies verified raw data inside the same SQL/authority snapshot as publication.
/// The cache changes only which predicates are evaluated; fresh primitive readers always clamp output.
pub(crate) fn evaluate(
    plan: &Plan<'_>,
    definition: &str,
    revision: &str,
    raw: GraphData,
    principal: &str,
    prior: Option<&State>,
    work: &mut ViewSelectionWork,
) -> Result<(QueryResult, State)> {
    if !eligible(&raw) {
        return Err(err("E_SELECTION_SUBSET", "source requires full evaluation"));
    }
    let shape = fingerprint(&(&raw.profile, &raw.schema))?;
    let prior = prior
        .filter(|s| s.definition == definition && s.principal == principal && s.shape == shape);
    work.source_records =
        raw.nodes.len() + raw.edges.len() + raw.structural_edges.len() + raw.assertions.len();
    let graph = visible(raw, principal);
    work.hashed_records = graph.nodes.len()
        + graph.edges.len()
        + graph.structural_edges.len()
        + graph.assertions.len();
    // Charge a conservative index envelope before retaining any cloned identifiers/maps.
    let mut bytes = json_size(&graph, STATE_LIMIT)?;
    let overhead = work
        .source_records
        .checked_mul(4096)
        .ok_or_else(|| err("E_BUDGET", "selection state capacity exceeded"))?;
    bytes = bytes
        .checked_add(overhead)
        .filter(|b| *b <= STATE_LIMIT)
        .ok_or_else(|| err("E_BUDGET", "selection state capacity exceeded"))?;
    let _reserved_bytes = bytes;
    let node_hashes = graph
        .nodes
        .iter()
        .map(|n| Ok((n.id.as_str(), fingerprint(n)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let structures = graph
        .structural_edges
        .iter()
        .map(|e| Ok((e.id.as_str(), (e, fingerprint(e)?))))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let times = plan.times();
    let mut crossed = BTreeSet::new();
    if let Some(old) = prior {
        for (old_time, new_time) in old.times.iter().zip(&times) {
            if old_time == new_time {
                continue;
            }
            if let (Some(old_time), Some(new_time)) = (old_time, new_time) {
                for ids in old
                    .boundaries
                    .range((
                        std::ops::Bound::Excluded((*old_time).min(*new_time)),
                        std::ops::Bound::Included((*old_time).max(*new_time)),
                    ))
                    .map(|(_, ids)| ids)
                {
                    crossed.extend(ids.iter().map(String::as_str));
                }
            } else {
                crossed.extend(old.claims.keys().map(String::as_str));
            }
        }
    }
    work.crossed_boundary_candidates = crossed.len();
    let mut state = State {
        evaluator: EVALUATOR.into(),
        definition: definition.into(),
        principal: principal.into(),
        shape,
        revision: revision.into(),
        times,
        claims: BTreeMap::new(),
        boundaries: BTreeMap::new(),
        endpoints: BTreeMap::new(),
    };
    let mut visit =
        |id: &str, predicate: &str, from: &str, to: &str, interval: &Interval, payload: String| {
            let old = prior.and_then(|s| s.claims.get(id));
            let selected = if let Some(old) =
                old.filter(|m| m.fingerprint == payload && !crossed.contains(id))
            {
                old.selected
            } else {
                work.memberships_evaluated += 1;
                plan.matches(predicate, from, to, interval)
            };
            if selected {
                for endpoint in [from, to] {
                    *state.endpoints.entry(endpoint.into()).or_insert(0) += 1;
                }
            }
            for t in [Some(interval.start), interval.end].into_iter().flatten() {
                state.boundaries.entry(t).or_default().insert(id.into());
            }
            state.claims.insert(
                id.into(),
                Member {
                    fingerprint: payload,
                    from: from.into(),
                    to: to.into(),
                    selected,
                    start: Some(interval.start),
                    end: interval.end,
                },
            );
        };
    for edge in &graph.edges {
        let hash = fingerprint(&(
            edge,
            node_hashes.get(edge.from.as_str()),
            node_hashes.get(edge.to.as_str()),
        ))?;
        visit(
            &edge.id,
            &edge.predicate,
            &edge.from,
            &edge.to,
            &edge.valid_time,
            hash,
        );
    }
    for assertion in &graph.assertions {
        let (edge, hash) = structures
            .get(assertion.edge_id.as_str())
            .ok_or_else(|| err("E_ASSERTION", "structural edge unavailable"))?;
        let hash = fingerprint(&(
            assertion,
            hash,
            node_hashes.get(edge.from.as_str()),
            node_hashes.get(edge.to.as_str()),
        ))?;
        visit(
            &assertion.id,
            &edge.predicate,
            &edge.from,
            &edge.to,
            &assertion.valid_time,
            hash,
        );
    }
    // Match the oracle's materialization-before-selection budget/error boundary.
    let (mut graph, attachment_origins) = materialize(graph, &plan.query.graph_id, revision)?;
    graph
        .edges
        .retain(|e| state.claims.get(&e.id).is_some_and(|m| m.selected));
    if plan.prune_nodes() {
        graph.nodes.retain(|n| state.endpoints.contains_key(&n.id));
    }
    work.output_nodes_repinned = graph.nodes.len();
    work.output_claims_rendered = graph.edges.len();
    let root = GraphRef {
        graph_id: plan.query.graph_id.clone(),
        revision: revision.into(),
    };
    let mut result = QueryResult {
        graph,
        version: VERSION.into(),
        snapshots: BTreeMap::from([(root.graph_id.clone(), root.revision.clone())]),
        input_snapshots: vec![root.clone()],
        coverage: Coverage::Complete,
        diagnostics: vec![],
        provenance: vec![],
        edge_origins: BTreeMap::new(),
        node_origins: BTreeMap::new(),
        attachment_origins,
        metadata_graphs: vec![],
        source_revisions: vec![],
        selected_context: None,
    };
    let mut output_bytes = json_size(&result, MATERIALIZED_LIMIT)?;
    for node in &result.graph.nodes {
        let origin = NodeRef {
            graph_id: root.graph_id.clone(),
            revision: root.revision.clone(),
            node_id: node.id.clone(),
        };
        output_bytes += json_size(
            &(&node.id, [&origin]),
            MATERIALIZED_LIMIT.saturating_sub(output_bytes),
        )?;
        result.node_origins.insert(node.id.clone(), vec![origin]);
    }
    for edge in &result.graph.edges {
        let origin = AssertionRef {
            graph_id: root.graph_id.clone(),
            revision: root.revision.clone(),
            assertion_id: edge.id.clone(),
        };
        output_bytes += json_size(
            &(&edge.id, [&origin], &origin),
            MATERIALIZED_LIMIT.saturating_sub(output_bytes),
        )?;
        result.provenance.push(origin.clone());
        result.edge_origins.insert(edge.id.clone(), vec![origin]);
    }
    if !plan.filters.is_empty() {
        result.provenance = result.edge_origins.values().flatten().cloned().collect();
    }
    json_size(&result, MATERIALIZED_LIMIT)?;
    json_size(&state, STATE_LIMIT)?;
    Ok((result, state))
}
