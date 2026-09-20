//! Pure valid-time selection over already-authorized materialized graph values.
//! Derived wrappers preserve original payload attribution and conditional metadata access.
use crate::*;
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

type Result<T> = std::result::Result<T, Diagnostic>;
const MAX_PAIRS: usize = 1_000_000;
const MAX_ALTERNATIVES: usize = 128;

impl TemporalRelation {
    pub fn matches(self, left: &Interval, right: &Interval) -> Result<bool> {
        valid(left)?;
        valid(right)?;
        Ok(match self {
            Self::Before => left.end.is_some_and(|end| end < right.start),
            Self::Meets => left.end == Some(right.start),
            Self::Overlaps => intersection(left, right).is_some(),
            Self::Within => {
                left.start >= right.start
                    && match (left.end, right.end) {
                        (_, None) => true,
                        (Some(a), Some(b)) => a <= b,
                        (None, Some(_)) => false,
                    }
            }
        })
    }
}
fn error(code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: message.into(),
    }
}
fn valid(interval: &Interval) -> Result<()> {
    if interval.valid() {
        Ok(())
    } else {
        Err(error(
            "E_INTERVAL_BOUNDS",
            "Temporal intervals must be nonempty",
        ))
    }
}
fn intersection(left: &Interval, right: &Interval) -> Option<Interval> {
    let start = left.start.max(right.start);
    let end = match (left.end, right.end) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, None) | (None, a) => a,
    };
    if end.is_some_and(|end| start >= end) {
        None
    } else {
        Some(Interval { start, end })
    }
}
struct Bytes(usize);
impl Write for Bytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| io::Error::other("temporal bytes"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Budget {
    bytes: Bytes,
    objects: usize,
    carrier: crate::carrier_algebra::Budget,
}
impl Budget {
    fn new(ctx: &AlgebraContext) -> Self {
        Self {
            bytes: Bytes(ctx.max_output_bytes.min(32 * 1024 * 1024)),
            objects: ctx.max_objects,
            carrier: crate::carrier_algebra::Budget::new(crate::carrier_algebra::Limits {
                bytes: ctx.max_output_bytes,
                ..crate::carrier_algebra::Limits::default()
            }),
        }
    }
    fn charge(&mut self, value: &impl Serialize) -> Result<()> {
        serde_json::to_writer(&mut self.bytes, value)
            .map_err(|_| error("E_TEMPORAL_BUDGET", "Temporal byte budget exceeded"))
    }
    // Charge borrowed source payload and all inserted fields before cloning. The
    // fixed allowance covers generated IDs and fixed JSON keys, never source data.
    fn record(&mut self, value: &impl Serialize) -> Result<()> {
        self.objects = self
            .objects
            .checked_sub(1)
            .ok_or_else(|| error("E_TEMPORAL_BUDGET", "Temporal object budget exceeded"))?;
        self.bytes.0 = self
            .bytes
            .0
            .checked_sub(2048)
            .ok_or_else(|| error("E_TEMPORAL_BUDGET", "Temporal byte budget exceeded"))?;
        self.charge(value)
    }
}
fn identity(kind: &str, value: &impl Serialize, limit: usize) -> Result<String> {
    Ok(format!(
        "temporal:{kind}:{}",
        crate::identity::digest("weave-temporal-v1", value, limit)?
    ))
}
fn merge(a: &GraphInfluence, b: &GraphInfluence) -> Result<GraphInfluence> {
    Ok(crate::influence::merge(Some(a), Some(b))?.unwrap_or_default())
}
fn pins(gates: &GraphInfluence) -> Vec<GraphRef> {
    let mut refs: Vec<_> = gates
        .assertions
        .iter()
        .map(|p| GraphRef {
            graph_id: p.graph_id.clone(),
            revision: p.revision.clone(),
        })
        .chain(gates.nodes.iter().map(|p| GraphRef {
            graph_id: p.graph_id.clone(),
            revision: p.revision.clone(),
        }))
        .chain(gates.snapshots.iter().cloned())
        .collect();
    refs.sort_by(|a, b| (&a.graph_id, &a.revision).cmp(&(&b.graph_id, &b.revision)));
    refs.dedup();
    refs
}
fn validate_input(input: &QueryResult, ctx: &AlgebraContext) -> Result<()> {
    if ctx.principal.is_empty() || ctx.principal.len() > 512 {
        return Err(error(
            "E_TEMPORAL_AUTH",
            "A bounded trusted principal is required",
        ));
    }
    crate::algebra::preflight(input, ctx)?;
    if let Some(diagnostic) = validate_schema_graph(&input.graph).into_iter().next() {
        return Err(diagnostic);
    }
    if let Some(scope) = &input.selected_context {
        crate::context::validate_selection(scope)?;
    }
    for origins in input.node_origins.values() {
        crate::influence::validate_refs(&[], origins)?;
    }
    for origins in input
        .edge_origins
        .values()
        .chain(input.attachment_origins.values())
    {
        crate::influence::validate_refs(origins, &[])?;
    }
    let nodes: BTreeSet<_> = input.graph.nodes.iter().map(|n| &n.id).collect();
    let entities: BTreeSet<_> = input.graph.nodes.iter().map(|n| &n.entity_id).collect();
    let edges: BTreeSet<_> = input.graph.edges.iter().map(|e| &e.id).collect();
    let attachments: BTreeSet<_> = input.graph.attachments.iter().map(|a| &a.id).collect();
    if nodes.len() != input.graph.nodes.len()
        || edges.len() != input.graph.edges.len()
        || attachments.len() != input.graph.attachments.len()
    {
        return Err(error(
            "E_TEMPORAL_ID",
            "Temporal input contains duplicate local IDs",
        ));
    }
    for edge in &input.graph.edges {
        valid(&edge.valid_time)?;
        if !nodes.contains(&edge.from) || !nodes.contains(&edge.to) {
            return Err(error("E_ENDPOINT", "Temporal input endpoint is missing"));
        }
    }
    for attachment in &input.graph.attachments {
        valid(&attachment.valid_time)?;
        let exists = match &attachment.host {
            MetadataHost::Graph => true,
            MetadataHost::Node { id } => nodes.contains(id),
            MetadataHost::Edge { id } | MetadataHost::Assertion { id } => edges.contains(id),
            MetadataHost::Entity { id } => entities.contains(id),
        };
        if !exists {
            return Err(error(
                "E_ATTACHMENT_HOST",
                "Temporal attachment host is missing",
            ));
        }
    }
    Ok(())
}
fn envelope(
    left: &QueryResult,
    right: Option<&QueryResult>,
    ctx: &AlgebraContext,
) -> Result<QueryResult> {
    // Combined input precharge precedes all envelope collection clones.
    let mut budget = Budget::new(ctx);
    budget.charge(left)?;
    if let Some(right) = right {
        budget.charge(right)?;
    }
    let mut out = QueryResult {
        version: VERSION.into(),
        graph: GraphData::default(),
        selected_context: left.selected_context.clone(),
        source_revisions: left.source_revisions.clone(),
        snapshots: left.snapshots.clone(),
        input_snapshots: left.input_snapshots.clone(),
        coverage: left.coverage.clone(),
        diagnostics: left.diagnostics.clone(),
        provenance: left.provenance.clone(),
        edge_origins: BTreeMap::new(),
        node_origins: BTreeMap::new(),
        attachment_origins: BTreeMap::new(),
        metadata_graphs: left.metadata_graphs.clone(),
    };
    out.graph.schema = left.graph.schema.clone();
    out.graph.context_typing = left.graph.context_typing.clone();
    out.graph.influence = crate::influence::input_influence(&left.graph)?;
    if let Some(right) = right {
        if left.graph.schema != right.graph.schema {
            return Err(error(
                "E_SCHEMA_INCOMPATIBLE",
                "Temporal selection requires equal complete schemas",
            ));
        }
        out.selected_context = crate::context::compatible_context(
            left.selected_context.as_ref(),
            right.selected_context.as_ref(),
        )?;
        out.graph.context_typing = crate::context_typing::merge(
            left.graph.context_typing.as_ref(),
            right.graph.context_typing.as_ref(),
        )?;
        out.graph.influence = crate::influence::merge(
            out.graph.influence.as_ref(),
            crate::influence::input_influence(&right.graph)?.as_ref(),
        )?;
        out.source_revisions = crate::algebra::merge_source_revisions(
            &left.source_revisions,
            &right.source_revisions,
        )?;
        for (graph, revision) in &right.snapshots {
            out.snapshots
                .entry(graph.clone())
                .or_insert_with(|| revision.clone());
        }
        out.input_snapshots
            .extend(right.input_snapshots.iter().cloned());
        out.provenance.extend(right.provenance.iter().cloned());
        out.metadata_graphs
            .extend(right.metadata_graphs.iter().cloned());
        out.diagnostics.extend(right.diagnostics.iter().cloned());
        if right.coverage == Coverage::Partial {
            out.coverage = Coverage::Partial;
        }
    }
    out.input_snapshots
        .sort_by(|a, b| (&a.graph_id, &a.revision).cmp(&(&b.graph_id, &b.revision)));
    out.input_snapshots.dedup();
    out.provenance.sort_by(|a, b| {
        crate::influence::assertion_key(a).cmp(&crate::influence::assertion_key(b))
    });
    out.provenance.dedup();
    Ok(out)
}
fn base_gates(out: &QueryResult) -> Result<GraphInfluence> {
    let mut gates = out.graph.influence.clone().unwrap_or_default();
    if let Some(typing) = &out.graph.context_typing {
        let (assertions, nodes) = crate::context_typing::gates(typing)?;
        gates = merge(
            &gates,
            &GraphInfluence {
                derivations: vec![],
                assertions,
                nodes,
                snapshots: vec![],
            },
        )?;
    }
    Ok(gates)
}
fn node_gates(input: &QueryResult, node: &Node) -> Result<GraphInfluence> {
    let declared = GraphInfluence {
        derivations: node.derivations.clone(),
        assertions: node.derived_from.clone(),
        nodes: node.derived_nodes.clone(),
        snapshots: node.derived_snapshots.clone(),
    };
    let originals = GraphInfluence {
        nodes: input
            .node_origins
            .get(&node.id)
            .cloned()
            .unwrap_or_default(),
        ..GraphInfluence::default()
    };
    let gates = merge(&declared, &originals)?;
    if gates.assertions.is_empty()
        && gates.nodes.is_empty()
        && gates.snapshots.is_empty()
        && gates.derivations.is_empty()
    {
        return Err(error(
            "E_ORIGIN_MISSING",
            "Temporal endpoint needs real source evidence",
        ));
    }
    Ok(gates)
}
fn alternatives(input: &QueryResult, edge: &Edge, budget: &mut Budget) -> Result<Vec<Derivation>> {
    let origins = input
        .edge_origins
        .get(&edge.id)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if edge.derivations.len().max(1) > MAX_ALTERNATIVES {
        return Err(error(
            "E_TEMPORAL_BUDGET",
            "Temporal proof alternatives exceeded",
        ));
    }
    budget.charge(&(edge, origins))?;
    let groups = crate::algebra::edge_alternatives(edge, origins)?;
    for group in &groups {
        crate::influence::validate_record_refs(
            &group.premises,
            &group.node_premises,
            &group.snapshot_premises,
        )?;
        if group.premises.is_empty()
            && group.node_premises.is_empty()
            && group.snapshot_premises.is_empty()
        {
            return Err(error(
                "E_ORIGIN_MISSING",
                "Temporal assertion needs a real premise",
            ));
        }
        if group.premises.iter().any(|p| !origins.contains(p)) {
            return Err(error(
                "E_PROVENANCE",
                "Temporal premises are missing from the origin index",
            ));
        }
        crate::identity::source_fingerprint(group)?;
    }
    Ok(groups)
}

fn local_groups(
    gates: &GraphInfluence,
    trace: &Derivation,
    budget: &mut Budget,
) -> Result<Vec<Derivation>> {
    use crate::carrier_algebra as c;
    budget.charge(&(gates, trace))?;
    let carrier = c::from_influence(gates, &mut budget.carrier)?;
    let distributed = c::distribute(&carrier, &c::Carrier::default(), &mut budget.carrier)?;
    let mut groups = distributed.alternatives;
    for group in &mut groups {
        budget.charge(&(&*group, trace))?;
        let parents = serde_json::to_value(&*group).expect("carrier");
        group.operator = trace.operator.clone();
        group.parameters = trace.parameters.clone();
        group.parameters.insert("carrier".into(), parents);
    }
    budget.charge(&groups)?;
    Ok(groups)
}

struct Builder<'a> {
    out: QueryResult,
    budget: Budget,
    ctx: &'a AlgebraContext,
    base: GraphInfluence,
    work: usize,
}
impl<'a> Builder<'a> {
    fn new(out: QueryResult, ctx: &'a AlgebraContext) -> Result<Self> {
        let mut budget = Budget::new(ctx);
        budget.charge(&out)?;
        let base = base_gates(&out)?;
        Ok(Self {
            out,
            budget,
            ctx,
            base,
            work: MAX_PAIRS,
        })
    }
    fn step(&mut self) -> Result<()> {
        self.work = self
            .work
            .checked_sub(1)
            .ok_or_else(|| error("E_TEMPORAL_BUDGET", "Temporal work budget exceeded"))?;
        Ok(())
    }
    fn record_node(
        &mut self,
        source: &Node,
        gates: &GraphInfluence,
        proof: &Derivation,
        role: &str,
    ) -> Result<String> {
        self.budget
            .record(&(source, gates, proof, role, &self.out.selected_context))?;
        let mut node = source.clone();
        node.id.clear();
        node.derived_from.clear();
        node.derived_nodes.clear();
        node.derived_snapshots.clear();
        node.derivations = local_groups(gates, proof, &mut self.budget)?;
        node.context_scope = Some(self.out.selected_context.clone().unwrap_or_default());
        node.readers = vec![self.ctx.principal.clone()];
        node.id = identity("node", &(&node, proof, role), self.ctx.max_output_bytes)?;
        let id = node.id.clone();
        self.out.node_origins.insert(id.clone(), vec![]);
        self.out.graph.nodes.push(node);
        Ok(id)
    }
    fn occurrence(
        &mut self,
        input: &QueryResult,
        edge: &Edge,
        clipped: &Interval,
        proof: &Derivation,
        gates: &GraphInfluence,
        role: &str,
    ) -> Result<(String, BTreeMap<String, String>)> {
        let mut map = BTreeMap::new();
        for id in [&edge.from, &edge.to] {
            if !map.contains_key(id) {
                let node = input
                    .graph
                    .nodes
                    .iter()
                    .find(|n| &n.id == id)
                    .expect("validated endpoint");
                let derived = self.record_node(node, gates, proof, role)?;
                map.insert(id.clone(), derived);
            }
        }
        self.budget
            .record(&(edge, clipped, proof, gates, gates, gates, &map, role))?;
        let mut wrapper = edge.clone();
        wrapper.id.clear();
        wrapper.from = map[&edge.from].clone();
        wrapper.to = map[&edge.to].clone();
        wrapper.valid_time = clipped.clone();
        wrapper.assertion_source = None;
        wrapper.structural_ref = None;
        wrapper.assertion_context = self
            .out
            .selected_context
            .as_ref()
            .and_then(|c| c.reference().cloned());
        wrapper.readers = vec![self.ctx.principal.clone()];
        wrapper.derived_from = crate::carrier_algebra::assertion_index(gates);
        wrapper.derived_nodes.clear();
        wrapper.derived_snapshots.clear();
        let mut occurrence_proof = proof.clone();
        occurrence_proof
            .parameters
            .insert("occurrence_role".into(), json!(role));
        wrapper.derivations = local_groups(gates, &occurrence_proof, &mut self.budget)?;
        wrapper.id = identity("edge", &(&wrapper, role), self.ctx.max_output_bytes)?;
        let id = wrapper.id.clone();
        self.out
            .edge_origins
            .insert(id.clone(), wrapper.derived_from.clone());
        self.out.graph.edges.push(wrapper);
        Ok((id, map))
    }
    fn attachment(
        &mut self,
        source: &MetadataAttachment,
        input: &QueryResult,
        host: MetadataHost,
        clipped: Interval,
        gates: &GraphInfluence,
        proof: Option<&Derivation>,
    ) -> Result<()> {
        crate::context::ensure_consumable(
            input.selected_context.as_ref(),
            source.context.as_ref(),
        )?;
        let originals = input
            .attachment_origins
            .get(&source.id)
            .cloned()
            .unwrap_or_default();
        let mut own = GraphInfluence {
            derivations: source.derivations.clone(),
            assertions: if source.derivations.is_empty() {
                originals
            } else {
                originals
                    .into_iter()
                    .filter(|r| r.assertion_id == source.id)
                    .collect()
            },
            nodes: source.derived_nodes.clone(),
            snapshots: source.derived_snapshots.clone(),
        };
        own.assertions.extend(source.derived_from.iter().cloned());
        own.assertions.extend(source.origin.iter().cloned());
        crate::influence::canonicalize(&mut own);
        crate::influence::validate(&own)?;
        if own.assertions.is_empty()
            && own.nodes.is_empty()
            && own.snapshots.is_empty()
            && own.derivations.is_empty()
        {
            return Err(error(
                "E_ORIGIN_MISSING",
                "Temporal attachment needs source evidence",
            ));
        }
        let mut gates = merge(gates, &own)?;
        for node in &input.graph.nodes {
            self.step()?;
            let depends = match &source.host {
                MetadataHost::Node { id } => &node.id == id,
                MetadataHost::Entity { id } => &node.entity_id == id,
                _ => false,
            };
            if depends {
                gates = merge(&gates, &node_gates(input, node)?)?;
            }
        }
        self.budget
            .record(&(source, &host, &clipped, &gates, &gates, &gates, proof))?;
        let mut attachment = source.clone();
        attachment.id.clear();
        attachment.host = host;
        attachment.valid_time = clipped;
        attachment.origin = None;
        let trace = proof.cloned().unwrap_or_else(|| Derivation {
            operator: "weave:window:attachment/v1".into(),
            premises: vec![],
            node_premises: vec![],
            snapshot_premises: vec![],
            parameters: BTreeMap::from([("original_attachment".into(), json!(source.id))]),
            input_snapshots: vec![],
        });
        attachment.derivations = local_groups(&gates, &trace, &mut self.budget)?;
        // Retain only already-declared flat carriers; new pair gates stay record-local.
        attachment.derived_from = source.derived_from.clone();
        attachment.derived_nodes = source.derived_nodes.clone();
        attachment.derived_snapshots = source.derived_snapshots.clone();
        attachment.readers = vec![self.ctx.principal.clone()];
        attachment.id = identity(
            "attachment",
            &(&attachment, proof),
            self.ctx.max_output_bytes,
        )?;
        self.out.attachment_origins.insert(
            attachment.id.clone(),
            crate::carrier_algebra::assertion_index(&gates),
        );
        self.out.graph.attachments.push(attachment);
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn carry(
        &mut self,
        input: &QueryResult,
        edge: &Edge,
        new_edge: &str,
        nodes: &BTreeMap<String, String>,
        window: &Interval,
        gates: &GraphInfluence,
        proof: &Derivation,
        graph_hosts: bool,
    ) -> Result<()> {
        let entities: BTreeSet<_> = input
            .graph
            .nodes
            .iter()
            .filter(|n| nodes.contains_key(&n.id))
            .map(|n| &n.entity_id)
            .collect();
        for attachment in &input.graph.attachments {
            self.step()?;
            let Some(clipped) = intersection(&attachment.valid_time, window) else {
                continue;
            };
            let host = match &attachment.host {
                MetadataHost::Graph if graph_hosts => MetadataHost::Graph,
                MetadataHost::Node { id } if nodes.contains_key(id) => MetadataHost::Node {
                    id: nodes[id].clone(),
                },
                MetadataHost::Edge { id } if id == &edge.id => MetadataHost::Edge {
                    id: new_edge.into(),
                },
                MetadataHost::Assertion { id } if id == &edge.id => MetadataHost::Assertion {
                    id: new_edge.into(),
                },
                MetadataHost::Entity { id } if entities.contains(id) => {
                    MetadataHost::Entity { id: id.clone() }
                }
                _ => continue,
            };
            self.attachment(attachment, input, host, clipped, gates, Some(proof))?;
        }
        Ok(())
    }
    fn finish(mut self) -> Result<QueryResult> {
        // Duplicate occurrences from equivalent alternatives have identical payloads.
        self.out.graph.nodes.sort_by(|a, b| a.id.cmp(&b.id));
        self.out.graph.nodes.dedup_by(|a, b| a == b);
        self.out.graph.edges.sort_by(|a, b| a.id.cmp(&b.id));
        self.out.graph.edges.dedup_by(|a, b| a == b);
        self.out.graph.attachments.sort_by(|a, b| a.id.cmp(&b.id));
        self.out.graph.attachments.dedup_by(|a, b| a == b);
        self.out
            .provenance
            .extend(self.out.edge_origins.values().flatten().cloned());
        self.out
            .provenance
            .extend(self.out.attachment_origins.values().flatten().cloned());
        self.out.provenance.sort_by(|a, b| {
            crate::influence::assertion_key(a).cmp(&crate::influence::assertion_key(b))
        });
        self.out.provenance.dedup();
        self.out
            .input_snapshots
            .extend(crate::influence::snapshots(&self.out.graph));
        self.out
            .input_snapshots
            .sort_by(|a, b| (&a.graph_id, &a.revision).cmp(&(&b.graph_id, &b.revision)));
        self.out.input_snapshots.dedup();
        validate_input(&self.out, self.ctx)?;
        Budget::new(self.ctx).charge(&self.out)?;
        Ok(self.out)
    }
}
fn edge_context(input: &QueryResult, edge: &Edge) -> Result<()> {
    crate::context::ensure_consumable(
        input.selected_context.as_ref(),
        edge.assertion_context.as_ref(),
    )?;
    for id in [&edge.from, &edge.to] {
        let node = input
            .graph
            .nodes
            .iter()
            .find(|n| &n.id == id)
            .expect("validated endpoint");
        if let Some(scope) = &node.context_scope {
            crate::context::compatible_context(input.selected_context.as_ref(), Some(scope))?;
        }
    }
    Ok(())
}
fn endpoint_gates(
    input: &QueryResult,
    edge: &Edge,
    base: &GraphInfluence,
) -> Result<GraphInfluence> {
    let mut gates = merge(
        base,
        &GraphInfluence {
            nodes: edge.derived_nodes.clone(),
            snapshots: edge.derived_snapshots.clone(),
            ..GraphInfluence::default()
        },
    )?;
    for id in [&edge.from, &edge.to] {
        let node = input
            .graph
            .nodes
            .iter()
            .find(|n| &n.id == id)
            .expect("validated endpoint");
        gates = merge(&gates, &node_gates(input, node)?)?;
    }
    Ok(gates)
}
fn derivation(
    operator: &str,
    parameters: serde_json::Value,
    groups: &[&Derivation],
    base: &GraphInfluence,
    budget: &mut Budget,
) -> Result<(Derivation, GraphInfluence)> {
    let mut gates = base.clone();
    for group in groups {
        gates = merge(
            &gates,
            &GraphInfluence {
                derivations: vec![],
                assertions: group.premises.clone(),
                nodes: group.node_premises.clone(),
                snapshots: group.snapshot_premises.clone(),
            },
        )?;
    }
    // Repeated proof indexes are precharged independently, before retention.
    budget.charge(&(&gates, &gates, &gates, groups, &parameters))?;
    let proof = Derivation {
        snapshot_premises: gates.snapshots.clone(),
        operator: operator.into(),
        premises: gates.assertions.clone(),
        node_premises: gates.nodes.clone(),
        parameters: BTreeMap::from([
            ("selection".into(), parameters),
            (
                "inputs".into(),
                serde_json::to_value(groups).expect("derivations serialize"),
            ),
        ]),
        input_snapshots: pins(&gates),
    };
    Ok((proof, gates))
}

/// Clip signed assertion occurrences and attachments, retaining untimed structure.
pub fn window(input: QueryResult, window: &Interval, ctx: &AlgebraContext) -> Result<QueryResult> {
    valid(window)?;
    validate_input(&input, ctx)?;
    let mut builder = Builder::new(envelope(&input, None, ctx)?, ctx)?;
    // Untimed original nodes are structural, not a claim selected by an edge match.
    for node in &input.graph.nodes {
        builder.budget.record(node)?;
        builder.out.graph.nodes.push(node.clone());
        if let Some(origins) = input.node_origins.get(&node.id) {
            builder.budget.charge(origins)?;
            builder
                .out
                .node_origins
                .insert(node.id.clone(), origins.clone());
        }
    }
    for edge in &input.graph.edges {
        builder.step()?;
        let Some(clipped) = intersection(&edge.valid_time, window) else {
            continue;
        };
        edge_context(&input, edge)?;
        let base = endpoint_gates(&input, edge, &builder.base)?;
        let groups = alternatives(&input, edge, &mut builder.budget)?;
        for group in &groups {
            let (proof, gates) = derivation(
                "weave:window/v1",
                json!({"window":window,"interval":edge.valid_time,"structural_ref":edge.structural_ref}),
                &[group],
                &base,
                &mut builder.budget,
            )?;
            let (id, nodes) =
                builder.occurrence(&input, edge, &clipped, &proof, &gates, "window")?;
            builder.carry(&input, edge, &id, &nodes, window, &gates, &proof, false)?;
        }
    }
    // Graph/entity/node attachments also belong to retained original structure.
    // Edge/assertion attachments were rebound only to their clipped occurrences.
    for attachment in &input.graph.attachments {
        builder.step()?;
        if matches!(
            attachment.host,
            MetadataHost::Edge { .. } | MetadataHost::Assertion { .. }
        ) {
            continue;
        }
        if let Some(clipped) = intersection(&attachment.valid_time, window) {
            let base = builder.base.clone();
            builder.attachment(
                attachment,
                &input,
                attachment.host.clone(),
                clipped,
                &base,
                None,
            )?;
        }
    }
    builder.finish()
}

/// Select pairs without asserting a simultaneous conclusion over their gap.
pub fn sequence(
    left: QueryResult,
    right: QueryResult,
    window: &Interval,
    relation: TemporalRelation,
    match_on: &JoinMatch,
    ctx: &AlgebraContext,
) -> Result<QueryResult> {
    valid(window)?;
    validate_input(&left, ctx)?;
    validate_input(&right, ctx)?;
    let JoinMatch::EntitySpaceToFrom = match_on;
    if left
        .graph
        .edges
        .len()
        .checked_mul(right.graph.edges.len())
        .is_none_or(|n| n > MAX_PAIRS)
    {
        return Err(error(
            "E_TEMPORAL_BUDGET",
            "Temporal candidate pair budget exceeded",
        ));
    }
    let mut builder = Builder::new(envelope(&left, Some(&right), ctx)?, ctx)?;
    let left_nodes: BTreeMap<_, _> = left.graph.nodes.iter().map(|n| (&n.id, n)).collect();
    let right_nodes: BTreeMap<_, _> = right.graph.nodes.iter().map(|n| (&n.id, n)).collect();
    for le in &left.graph.edges {
        if le.polarity != Polarity::Positive {
            continue;
        }
        let Some(lc) = intersection(&le.valid_time, window) else {
            continue;
        };
        for re in &right.graph.edges {
            builder.step()?;
            if re.polarity != Polarity::Positive {
                continue;
            }
            let Some(rc) = intersection(&re.valid_time, window) else {
                continue;
            };
            let ln = left_nodes[&le.to];
            let rn = right_nodes[&re.from];
            if ln.entity_id != rn.entity_id
                || ln.space_id != rn.space_id
                || !relation.matches(&le.valid_time, &re.valid_time)?
            {
                continue;
            }
            edge_context(&left, le)?;
            edge_context(&right, re)?;
            let base = endpoint_gates(&right, re, &endpoint_gates(&left, le, &builder.base)?)?;
            let lg = alternatives(&left, le, &mut builder.budget)?;
            let rg = alternatives(&right, re, &mut builder.budget)?;
            if lg
                .len()
                .checked_mul(rg.len())
                .is_none_or(|n| n > MAX_ALTERNATIVES)
            {
                return Err(error(
                    "E_TEMPORAL_BUDGET",
                    "Temporal alternative product exceeded",
                ));
            }
            for a in &lg {
                for b in &rg {
                    let (proof, gates) = derivation(
                        "weave:sequence/v1",
                        json!({"window":window,"relation":relation,"match_on":match_on,"left_interval":le.valid_time,"right_interval":re.valid_time,"left_structure":le.structural_ref,"right_structure":re.structural_ref,"left_origins":left.edge_origins.get(&le.id),"right_origins":right.edge_origins.get(&re.id)}),
                        &[a, b],
                        &base,
                        &mut builder.budget,
                    )?;
                    let (lid, lnodes) =
                        builder.occurrence(&left, le, &lc, &proof, &gates, "left")?;
                    let (rid, rnodes) =
                        builder.occurrence(&right, re, &rc, &proof, &gates, "right")?;
                    builder.carry(&left, le, &lid, &lnodes, window, &gates, &proof, true)?;
                    builder.carry(&right, re, &rid, &rnodes, window, &gates, &proof, true)?;
                }
            }
        }
    }
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> AlgebraContext {
        AlgebraContext {
            principal: "reader".into(),
            max_objects: 10_000,
            max_output_bytes: 32 * 1024 * 1024,
        }
    }
    fn interval(start: i64, end: Option<i64>) -> Interval {
        Interval { start, end }
    }
    fn assertion(graph: &str, id: &str) -> AssertionRef {
        AssertionRef {
            graph_id: graph.into(),
            revision: "r1".into(),
            assertion_id: id.into(),
        }
    }
    fn input(graph: &str, start: i64, end: Option<i64>, left: bool) -> QueryResult {
        let entities = if left {
            ["start", "middle"]
        } else {
            ["middle", "end"]
        };
        let mut result: QueryResult = serde_json::from_value(json!({
            "version":VERSION,"graph":{
                "nodes":[{"id":"a","entity_id":entities[0],"space_id":"s"},{"id":"b","entity_id":entities[1],"space_id":"s"}],
                "edges":[{"id":"e","predicate":"event","from":"a","to":"b","valid_time":{"start":start,"end":end}}]
            },"snapshots":{graph:"r1"},"input_snapshots":[{"graph_id":graph,"revision":"r1"}],
            "coverage":"complete","diagnostics":[],"provenance":[],"metadata_graphs":[]
        })).unwrap();
        result
            .edge_origins
            .insert("e".into(), vec![assertion(graph, "e")]);
        for id in ["a", "b"] {
            result.node_origins.insert(
                id.into(),
                vec![NodeRef {
                    graph_id: graph.into(),
                    revision: "r1".into(),
                    node_id: id.into(),
                }],
            );
        }
        result
    }
    fn pairs(
        left: QueryResult,
        right: QueryResult,
        window: &Interval,
        relation: TemporalRelation,
    ) -> QueryResult {
        sequence(
            left,
            right,
            window,
            relation,
            &JoinMatch::EntitySpaceToFrom,
            &ctx(),
        )
        .unwrap()
    }
    fn matches(result: &QueryResult) -> usize {
        result
            .graph
            .edges
            .iter()
            .filter(|edge| {
                edge.derivations.iter().any(|proof| {
                    proof.operator == "weave:sequence/v1"
                        && proof.parameters.get("occurrence_role") == Some(&json!("left"))
                })
            })
            .count()
    }
    fn attach(input: &mut QueryResult) {
        for (i, host) in [
            MetadataHost::Graph,
            MetadataHost::Node { id: "a".into() },
            MetadataHost::Edge { id: "e".into() },
            MetadataHost::Assertion { id: "e".into() },
            MetadataHost::Entity {
                id: input.graph.nodes[0].entity_id.clone(),
            },
        ]
        .into_iter()
        .enumerate()
        {
            let id = format!("m{i}");
            let source = assertion(input.input_snapshots[0].graph_id.as_str(), &id);
            let attachment: MetadataAttachment = serde_json::from_value(json!({"id":id,"host":host,"key":format!("key{i}"),"value":{"kind":"literal","value":{"payload":"exact"}},"valid_time":{"start":-5,"end":100}})).unwrap();
            input.attachment_origins.insert(id, vec![source]);
            input.graph.attachments.push(attachment);
        }
    }
    #[test]
    fn finite_set_oracle_and_open_boundaries() {
        let mut intervals = vec![];
        for start in -2..=3 {
            for end in start + 1..=4 {
                intervals.push(interval(start, Some(end)));
            }
            intervals.push(interval(start, None));
        }
        for a in &intervals {
            for b in &intervals {
                let sa: BTreeSet<_> = (-3..=6).filter(|t| a.contains(*t)).collect();
                let sb: BTreeSet<_> = (-3..=6).filter(|t| b.contains(*t)).collect();
                assert_eq!(
                    TemporalRelation::Overlaps.matches(a, b).unwrap(),
                    !sa.is_disjoint(&sb)
                );
                assert_eq!(
                    TemporalRelation::Within.matches(a, b).unwrap(),
                    sa.is_subset(&sb)
                );
                assert_eq!(
                    TemporalRelation::Before.matches(a, b).unwrap(),
                    a.end.is_some_and(|end| end < b.start)
                );
                assert_eq!(
                    TemporalRelation::Meets.matches(a, b).unwrap(),
                    a.end == Some(b.start)
                );
            }
        }
        let a = interval(i64::MIN, Some(i64::MAX));
        let b = interval(i64::MAX, None);
        assert!(TemporalRelation::Meets.matches(&a, &b).unwrap());
        assert!(!TemporalRelation::Overlaps.matches(&a, &b).unwrap());
        assert!(!TemporalRelation::Before.matches(&b, &a).unwrap());
        assert_eq!(
            TemporalRelation::Within
                .matches(&interval(1, Some(1)), &b)
                .unwrap_err()
                .code,
            "E_INTERVAL_BOUNDS"
        );
    }
    #[test]
    fn sequence_before_is_not_simultaneous_and_meets_is_explicit() {
        let out = pairs(
            input("L", 0, Some(5), true),
            input("R", 10, Some(15), false),
            &interval(0, Some(20)),
            TemporalRelation::Before,
        );
        assert_eq!(matches(&out), 1);
        assert_eq!(out.graph.edges.len(), 2);
        assert!(out.graph.edges.iter().all(
            |e| e.valid_time == interval(0, Some(5)) || e.valid_time == interval(10, Some(15))
        ));
        assert!(intersection(
            &out.graph.edges[0].valid_time,
            &out.graph.edges[1].valid_time
        )
        .is_none());
        for (relation, count) in [
            (TemporalRelation::Meets, 1),
            (TemporalRelation::Before, 0),
            (TemporalRelation::Overlaps, 0),
        ] {
            assert_eq!(
                matches(&pairs(
                    input("L", 0, Some(5), true),
                    input("R", 5, Some(10), false),
                    &interval(0, Some(20)),
                    relation
                )),
                count
            );
        }
    }
    #[test]
    fn relation_precedes_clipping_and_window_preserves_signed_evidence() {
        let narrow = interval(12, Some(18));
        let l = input("L", 0, Some(20), true);
        let r = input("R", 10, Some(30), false);
        assert_eq!(
            matches(&pairs(
                l.clone(),
                r.clone(),
                &narrow,
                TemporalRelation::Within
            )),
            0
        );
        let wl = window(l, &narrow, &ctx()).unwrap();
        let wr = window(r, &narrow, &ctx()).unwrap();
        assert_eq!(
            matches(&pairs(wl, wr, &narrow, TemporalRelation::Within)),
            1
        );
        let mut signed = input("L", 0, Some(20), true);
        signed.graph.edges[0].polarity = Polarity::Negative;
        let clipped = window(signed, &narrow, &ctx()).unwrap();
        assert_eq!(clipped.graph.edges[0].polarity, Polarity::Negative);
        assert_eq!(clipped.graph.edges[0].valid_time, narrow);
        assert_ne!(clipped.graph.edges[0].id, "e");
        assert_eq!(
            clipped
                .graph
                .nodes
                .iter()
                .filter(|n| n.id == "a" || n.id == "b")
                .count(),
            2
        );
    }
    #[test]
    fn every_sequence_record_retains_both_premises_after_envelope_stripping() {
        let mut l = input("L", 0, Some(5), true);
        attach(&mut l);
        let mut r = input("R", 10, Some(15), false);
        attach(&mut r);
        let mut out = pairs(l, r, &interval(0, Some(20)), TemporalRelation::Before);
        out.graph.influence = None;
        out.graph.context_typing = None;
        let left = assertion("L", "e");
        let private = assertion("R", "e");
        for node in &mut out.graph.nodes {
            node.readers.clear();
            assert!(!node.derivations.is_empty());
            assert!(node
                .derivations
                .iter()
                .all(|g| g.premises.contains(&left) && g.premises.contains(&private)));
            assert!(out.node_origins[&node.id].is_empty());
        }
        for edge in &mut out.graph.edges {
            edge.readers.clear();
            assert!(edge.derived_from.contains(&left) && edge.derived_from.contains(&private));
            assert!(edge
                .derivations
                .iter()
                .all(|d| d.premises.contains(&private)));
        }
        for attachment in &mut out.graph.attachments {
            attachment.readers.clear();
            assert!(
                !attachment.derivations.is_empty()
                    && attachment
                        .derivations
                        .iter()
                        .all(|g| g.premises.contains(&left) && g.premises.contains(&private))
            );
            assert!(attachment.origin.is_none());
        }
        assert!(out
            .graph
            .attachments
            .iter()
            .any(|a| matches!(a.host, MetadataHost::Node { .. })));
        assert!(out
            .graph
            .attachments
            .iter()
            .any(|a| matches!(a.host, MetadataHost::Edge { .. })));
        assert!(out
            .graph
            .attachments
            .iter()
            .any(|a| matches!(a.host, MetadataHost::Assertion { .. })));
        assert!(out
            .graph
            .attachments
            .iter()
            .any(|a| matches!(a.host, MetadataHost::Entity { .. })));
        assert!(out
            .graph
            .attachments
            .iter()
            .all(|a| a.valid_time == interval(0, Some(20))));
    }
    #[test]
    fn alternatives_remain_separate_not_a_conjunction() {
        let mut l = input("L", 0, Some(5), true);
        let a = assertion("A", "a");
        let b = assertion("B", "b");
        l.edge_origins
            .insert("e".into(), vec![a.clone(), b.clone()]);
        l.graph.edges[0].derivations = [a.clone(), b.clone()]
            .into_iter()
            .map(|p| Derivation {
                snapshot_premises: vec![],
                operator: "source-choice".into(),
                premises: vec![p],
                node_premises: vec![],
                parameters: BTreeMap::new(),
                input_snapshots: vec![],
            })
            .collect();
        let out = pairs(
            l,
            input("R", 10, Some(15), false),
            &interval(0, Some(20)),
            TemporalRelation::Before,
        );
        assert_eq!(matches(&out), 2);
        for node in &out.graph.nodes {
            assert_ne!(
                node.derivations.iter().any(|g| g.premises.contains(&a)),
                node.derivations.iter().any(|g| g.premises.contains(&b))
            );
        }
        for edge in &out.graph.edges {
            assert_ne!(
                edge.derived_from.contains(&a),
                edge.derived_from.contains(&b)
            );
        }
        for att in &out.graph.attachments {
            assert_ne!(
                att.derivations.iter().any(|g| g.premises.contains(&a)),
                att.derivations.iter().any(|g| g.premises.contains(&b))
            );
        }
    }
    #[test]
    fn selected_attachment_alternatives_survive_project_union_without_globalization() {
        let mut l = input("L", 0, Some(5), true);
        attach(&mut l);
        let a = assertion("A", "a");
        let b = assertion("B", "b");
        l.edge_origins
            .insert("e".into(), vec![a.clone(), b.clone()]);
        l.graph.edges[0].derivations = [a.clone(), b.clone()]
            .into_iter()
            .map(|p| Derivation {
                operator: "choice".into(),
                premises: vec![p],
                node_premises: vec![],
                snapshot_premises: vec![],
                parameters: BTreeMap::new(),
                input_snapshots: vec![],
            })
            .collect();
        let out = pairs(
            l,
            input("R", 10, Some(15), false),
            &interval(0, Some(20)),
            TemporalRelation::Before,
        );
        let selected = out
            .graph
            .edges
            .iter()
            .find(|e| {
                e.derivations.iter().any(|g| {
                    g.premises.contains(&a)
                        && g.parameters.get("occurrence_role") == Some(&json!("left"))
                })
            })
            .unwrap();
        let projected =
            crate::algebra::project(out.clone(), &[], std::slice::from_ref(&selected.id), &ctx())
                .unwrap();
        let united = crate::algebra::union(projected.clone(), projected, &ctx()).unwrap();
        assert!(united
            .graph
            .influence
            .as_ref()
            .is_none_or(|i| i.assertions.is_empty() && i.derivations.is_empty()));
        assert!(united.graph.attachments.iter().all(|m| {
            m.derived_from.is_empty()
                && m.derivations
                    .iter()
                    .all(|g| g.premises.contains(&a) != g.premises.contains(&b))
        }));
        assert!(united
            .graph
            .attachments
            .iter()
            .any(|m| matches!(m.host, MetadataHost::Node { .. })));
        assert!(united
            .graph
            .attachments
            .iter()
            .any(|m| matches!(m.host, MetadataHost::Edge { .. })));
    }
    #[test]
    fn empty_output_preserves_explicit_influence_without_promoting_input_pins() {
        let mut l = input("L", 0, Some(5), true);
        l.graph.influence = Some(GraphInfluence {
            snapshots: vec![GraphRef {
                graph_id: "private-descriptor".into(),
                revision: "r1".into(),
            }],
            ..GraphInfluence::default()
        });
        let out = pairs(
            l,
            input("R", 0, Some(5), false),
            &interval(0, Some(20)),
            TemporalRelation::Before,
        );
        assert!(out.graph.nodes.is_empty() && out.graph.edges.is_empty());
        assert_eq!(out.graph.influence.unwrap().snapshots.len(), 1);
        assert!(out
            .input_snapshots
            .iter()
            .any(|r| r.graph_id == "private-descriptor"));
    }
    #[test]
    fn every_window_attachment_host_is_clipped_and_source_gated() {
        let mut source = input("L", 0, Some(10), true);
        attach(&mut source);
        let out = window(source, &interval(2, Some(8)), &ctx()).unwrap();
        for att in &out.graph.attachments {
            assert_eq!(att.valid_time, interval(2, Some(8)));
            assert!(att.origin.is_none());
            assert!(att
                .derivations
                .iter()
                .all(|g| g.premises.iter().any(|r| r.assertion_id.starts_with('m'))));
        }
        assert!(out
            .graph
            .attachments
            .iter()
            .any(|a| matches!(a.host, MetadataHost::Graph)));
        for att in &out.graph.attachments {
            if let MetadataHost::Edge { id } | MetadataHost::Assertion { id } = &att.host {
                assert!(out.graph.edges.iter().any(|e| &e.id == id));
            }
        }
    }
    #[test]
    fn invalid_inputs_schema_conflicts_and_budgets_fail_closed() {
        let l = input("L", 0, Some(5), true);
        let r = input("R", 10, Some(15), false);
        let mut invalid = l.clone();
        invalid.graph.edges[0].valid_time.end = Some(0);
        assert_eq!(
            window(invalid, &interval(0, None), &ctx())
                .unwrap_err()
                .code,
            "E_INTERVAL_BOUNDS"
        );
        let mut no_proof = l.clone();
        no_proof.edge_origins.clear();
        assert_eq!(
            window(no_proof, &interval(0, None), &ctx())
                .unwrap_err()
                .code,
            "E_ORIGIN_MISSING"
        );
        let mut tiny = ctx();
        tiny.max_objects = 3;
        assert_eq!(
            sequence(
                l.clone(),
                r.clone(),
                &interval(0, None),
                TemporalRelation::Before,
                &JoinMatch::EntitySpaceToFrom,
                &tiny
            )
            .unwrap_err()
            .code,
            "E_TEMPORAL_BUDGET"
        );
        let mut typed = l.clone();
        typed.graph.schema = Some(GraphSchema {
            id: "S".into(),
            revision: "1".into(),
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
        });
        assert!(sequence(
            typed,
            r,
            &interval(0, None),
            TemporalRelation::Before,
            &JoinMatch::EntitySpaceToFrom,
            &ctx()
        )
        .is_err());
        let mut malformed = l;
        malformed.graph.edges[0].to = "missing".into();
        assert_eq!(
            window(malformed, &interval(0, None), &ctx())
                .unwrap_err()
                .code,
            "E_SCHEMA_ENDPOINT"
        );
    }
}
