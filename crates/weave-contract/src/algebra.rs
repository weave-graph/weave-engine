//! Bounded, pure operators over runtime-authorized graph values.
use crate::*;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

fn err(code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: message.into(),
    }
}
fn key(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("contract values serialize");
    format!("{:x}", Sha256::digest(bytes))
}
struct Counter {
    remaining: usize,
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(io::Error::other("algebra byte limit"));
        }
        self.remaining -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Budget {
    bytes: Counter,
    objects: usize,
}
impl Budget {
    fn new(ctx: &AlgebraContext) -> Self {
        Self {
            bytes: Counter {
                remaining: ctx.max_output_bytes,
            },
            objects: ctx.max_objects,
        }
    }
    fn add(&mut self, value: &impl Serialize) -> Result<(), Diagnostic> {
        self.objects = self
            .objects
            .checked_sub(1)
            .ok_or_else(|| err("E_ALGEBRA_LIMIT", "Graph algebra object budget exceeded"))?;
        serde_json::to_writer(&mut self.bytes, value)
            .map_err(|_| err("E_ALGEBRA_LIMIT", "Graph algebra byte budget exceeded"))
    }
}
fn checked(mut result: QueryResult, ctx: &AlgebraContext) -> Result<QueryResult, Diagnostic> {
    if ctx.principal.is_empty() {
        return Err(err(
            "E_ALGEBRA_AUTH",
            "A trusted host principal is required",
        ));
    }
    for node in &mut result.graph.nodes {
        node.readers = vec![ctx.principal.clone()];
    }
    for edge in &mut result.graph.edges {
        edge.readers = vec![ctx.principal.clone()];
    }
    for attachment in &mut result.graph.attachments {
        attachment.readers = vec![ctx.principal.clone()];
    }
    if let Some(d) = validate_schema_graph(&result.graph).into_iter().next() {
        return Err(d);
    }
    preflight(&result, ctx)?;
    Ok(result)
}
fn preflight(result: &QueryResult, ctx: &AlgebraContext) -> Result<(), Diagnostic> {
    if result.graph.profile != GraphProfile::Legacy
        || !result.graph.structural_edges.is_empty()
        || !result.graph.assertions.is_empty()
    {
        return Err(err(
            "E_ALGEBRA_PROFILE",
            "Algebra consumes materialized graph values, not raw explicit snapshots",
        ));
    }
    let count = result
        .graph
        .nodes
        .len()
        .saturating_add(result.graph.edges.len())
        .saturating_add(result.graph.attachments.len());
    if count > ctx.max_objects {
        return Err(err("E_ALGEBRA_LIMIT", "Input graph exceeds object budget"));
    }
    Budget::new(ctx).add(result)
}
fn unique<T: Serialize + Clone>(values: impl IntoIterator<Item = T>) -> Vec<T> {
    values
        .into_iter()
        .map(|v| (key(&v), v))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect()
}
/// Merge descriptive source identities without accepting conflicting labels.
pub fn merge_source_revisions(
    left: &[SourceRevision],
    right: &[SourceRevision],
) -> Result<Vec<SourceRevision>, Diagnostic> {
    let mut sources = BTreeMap::new();
    for source in left.iter().chain(right) {
        let label = (source.name.clone(), source.revision.clone());
        if let Some(old) = sources.insert(label, source.clone()) {
            if old.digest != source.digest {
                return Err(err(
                    "E_SOURCE_REVISION",
                    "Source label has conflicting digests",
                ));
            }
        }
    }
    Ok(sources.into_values().collect())
}
fn envelope(left: &QueryResult, right: Option<&QueryResult>) -> Result<QueryResult, Diagnostic> {
    let mut out = left.clone();
    out.source_revisions = merge_source_revisions(
        &left.source_revisions,
        right.map_or(&[], |r| r.source_revisions.as_slice()),
    )?;
    out.graph = GraphData::default();
    out.node_origins.clear();
    out.edge_origins.clear();
    out.attachment_origins.clear();
    if let Some(r) = right {
        out.input_snapshots = unique(
            left.input_snapshots
                .iter()
                .chain(&r.input_snapshots)
                .cloned(),
        );
        out.provenance = unique(left.provenance.iter().chain(&r.provenance).cloned());
        out.metadata_graphs = unique(
            left.metadata_graphs
                .iter()
                .chain(&r.metadata_graphs)
                .cloned(),
        );
        out.diagnostics = unique(left.diagnostics.iter().chain(&r.diagnostics).cloned());
        if r.coverage == Coverage::Partial {
            out.coverage = Coverage::Partial;
        }
        // Legacy map cannot represent multiple revisions; full input_snapshots is authoritative.
        for (g, rev) in &r.snapshots {
            out.snapshots
                .entry(g.clone())
                .or_insert_with(|| rev.clone());
        }
    }
    out.version = VERSION.into();
    Ok(out)
}
const TYPE_PREFIX: &str = "weave:algebra:type:";
fn type_key(schema: &GraphSchema, id: &str) -> String {
    if id.starts_with(TYPE_PREFIX) {
        id.into()
    } else {
        format!("{TYPE_PREFIX}{}", key(&(schema, id)))
    }
}
fn merge_schema(
    a: Option<&GraphSchema>,
    b: Option<&GraphSchema>,
) -> Result<Option<GraphSchema>, Diagnostic> {
    match (a, b) {
        (None, None) => Ok(None),
        (Some(a), Some(b)) if a == b => Ok(Some(a.clone())),
        (Some(a), Some(b)) => {
            let mut nodes = BTreeMap::new();
            let mut edges = BTreeMap::new();
            for s in [a, b] {
                for (id, def) in &s.nodes {
                    let name = type_key(s, id);
                    if nodes
                        .insert(name, def.clone())
                        .is_some_and(|old| old != *def)
                    {
                        return Err(err(
                            "E_SCHEMA_COLLISION",
                            "Conflicting canonical node types",
                        ));
                    }
                }
                for (id, def) in &s.edges {
                    let mut d = def.clone();
                    d.from_type = type_key(s, &def.from_type);
                    d.to_type = type_key(s, &def.to_type);
                    if edges
                        .insert(type_key(s, id), d.clone())
                        .is_some_and(|old| old != d)
                    {
                        return Err(err(
                            "E_SCHEMA_COLLISION",
                            "Conflicting canonical edge types",
                        ));
                    }
                }
            }
            Ok(Some(GraphSchema {
                id: format!("weave:algebra:schema:{}", key(&(&nodes, &edges))),
                revision: "1".into(),
                nodes,
                edges,
            }))
        }
        _ => Err(err(
            "E_SCHEMA_ALGEBRA",
            "Typed and untyped graph values cannot be combined without an explicit mapping",
        )),
    }
}
fn node_key(input: &QueryResult, node: &Node) -> Result<String, Diagnostic> {
    let origins = input.node_origins.get(&node.id).ok_or_else(|| {
        err(
            "E_ORIGIN_MISSING",
            "Graph node lacks runtime-provided pinned provenance",
        )
    })?;
    Ok(format!(
        "node:{}",
        key(&(unique(origins.clone()), &node.entity_id, &node.space_id))
    ))
}
fn edge_key(input: &QueryResult, edge: &Edge) -> Result<String, Diagnostic> {
    let origins = input.edge_origins.get(&edge.id).filter(|v| !v.is_empty());
    if edge.derivations.is_empty() {
        let origins = origins.ok_or_else(|| {
            err(
                "E_ORIGIN_MISSING",
                "Graph edge lacks runtime-provided pinned provenance",
            )
        })?;
        // The interval matters: traversal may restrict the same immutable assertion.
        Ok(format!(
            "edge:{}",
            key(&(unique(origins.clone()), &edge.valid_time))
        ))
    } else {
        Ok(format!(
            "derived:{}",
            key(&(
                &edge.predicate,
                &edge.valid_time,
                &edge.polarity,
                &edge.properties,
                &edge.assertion_properties,
                &edge.assertion_source,
                &edge.assertion_context,
                &edge.structural_ref,
                &edge.derivations
            ))
        ))
    }
}
/// Exact immutable origin membership, never identity inference or corroboration counting.
pub fn union(
    left: QueryResult,
    right: QueryResult,
    ctx: &AlgebraContext,
) -> Result<QueryResult, Diagnostic> {
    preflight(&left, ctx)?;
    preflight(&right, ctx)?;
    let mut out = envelope(&left, Some(&right))?;
    out.graph.schema = merge_schema(left.graph.schema.as_ref(), right.graph.schema.as_ref())?;
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    let mut edges: BTreeMap<String, Edge> = BTreeMap::new();
    let mut attachments: BTreeMap<String, MetadataAttachment> = BTreeMap::new();
    let mut budget = Budget::new(ctx);
    for input in [&left, &right] {
        let remap_type = input
            .graph
            .schema
            .as_ref()
            .filter(|s| Some(*s) != out.graph.schema.as_ref());
        let mut node_ids = BTreeMap::new();
        let mut edge_ids = BTreeMap::new();
        for n in &input.graph.nodes {
            let id = node_key(input, n)?;
            node_ids.insert(n.id.clone(), id.clone());
            let mut node = n.clone();
            node.id = id.clone();
            node.readers = vec![ctx.principal.clone()];
            if let (Some(s), Some(t)) = (remap_type, &node.type_id) {
                node.type_id = Some(type_key(s, t));
            }
            if let Some(old) = nodes.get(&id) {
                if old != &node {
                    return Err(err(
                        "E_ORIGIN_CONFLICT",
                        "Same pinned node origin has inconsistent payload",
                    ));
                }
            } else {
                budget.add(&(&node, &input.node_origins[&n.id]))?;
                nodes.insert(id.clone(), node);
                out.node_origins
                    .insert(id, input.node_origins[&n.id].clone());
            }
        }
        for e in &input.graph.edges {
            let base = edge_key(input, e)?;
            let mut edge = e.clone();
            edge.from = node_ids
                .get(&e.from)
                .ok_or_else(|| err("E_ENDPOINT", "Missing algebra endpoint"))?
                .clone();
            edge.to = node_ids
                .get(&e.to)
                .ok_or_else(|| err("E_ENDPOINT", "Missing algebra endpoint"))?
                .clone();
            let id = format!("edge:{}", key(&(&base, &edge.from, &edge.to)));
            edge_ids.insert(e.id.clone(), id.clone());
            edge.id = id.clone();
            edge.readers = vec![ctx.principal.clone()];
            if let (Some(s), Some(t)) = (remap_type, &edge.type_id) {
                edge.type_id = Some(type_key(s, t));
            }
            if let Some(old) = edges.get(&id) {
                if old != &edge {
                    return Err(err(
                        "E_ORIGIN_CONFLICT",
                        "Same pinned edge origin has inconsistent payload",
                    ));
                }
            } else {
                let origins = input.edge_origins.get(&e.id).cloned().unwrap_or_default();
                budget.add(&(&edge, &origins))?;
                edges.insert(id.clone(), edge);
                out.edge_origins.insert(id, origins);
            }
        }
        for a in &input.graph.attachments {
            let mut attachment = a.clone();
            attachment.readers = vec![ctx.principal.clone()];
            attachment.host = match &a.host {
                MetadataHost::Node { id } => MetadataHost::Node {
                    id: node_ids
                        .get(id)
                        .ok_or_else(|| err("E_ATTACHMENT_HOST", "Missing node attachment host"))?
                        .clone(),
                },
                MetadataHost::Assertion { id } => MetadataHost::Assertion {
                    id: edge_ids
                        .get(id)
                        .ok_or_else(|| {
                            err("E_ATTACHMENT_HOST", "Missing assertion attachment host")
                        })?
                        .clone(),
                },
                MetadataHost::Edge { id } => MetadataHost::Edge {
                    id: edge_ids
                        .get(id)
                        .ok_or_else(|| err("E_ATTACHMENT_HOST", "Missing edge attachment host"))?
                        .clone(),
                },
                other => other.clone(),
            };
            let origins = input
                .attachment_origins
                .get(&a.id)
                .cloned()
                .unwrap_or_default();
            attachment.id.clear();
            let id = format!("attachment:{}", key(&(&attachment, &origins)));
            attachment.id = id.clone();
            if let std::collections::btree_map::Entry::Vacant(entry) = attachments.entry(id.clone())
            {
                budget.add(&(&attachment, &origins))?;
                out.attachment_origins.insert(id.clone(), origins);
                entry.insert(attachment);
            }
        }
    }
    out.graph.nodes = nodes.into_values().collect();
    out.graph.edges = edges.into_values().collect();
    out.graph.attachments = attachments.into_values().collect();
    checked(out, ctx)
}
/// Projection is structural: requested edges retain endpoints, permissions and origins.
pub fn project(
    input: QueryResult,
    node_ids: &[String],
    edge_ids: &[String],
    ctx: &AlgebraContext,
) -> Result<QueryResult, Diagnostic> {
    preflight(&input, ctx)?;
    let mut out = input;
    let mut ns: BTreeSet<_> = node_ids.iter().cloned().collect();
    let es: BTreeSet<_> = edge_ids.iter().cloned().collect();
    if ns
        .iter()
        .any(|id| !out.graph.nodes.iter().any(|n| &n.id == id))
        || es
            .iter()
            .any(|id| !out.graph.edges.iter().any(|e| &e.id == id))
    {
        return Err(err(
            "E_PROJECT_MEMBER",
            "Projection names an unavailable member",
        ));
    }
    out.graph.edges.retain(|e| es.contains(&e.id));
    for e in &out.graph.edges {
        ns.insert(e.from.clone());
        ns.insert(e.to.clone());
    }
    out.graph.nodes.retain(|n| ns.contains(&n.id));
    let entities: BTreeSet<_> = out
        .graph
        .nodes
        .iter()
        .map(|n| n.entity_id.clone())
        .collect();
    out.graph.attachments.retain(|a| match &a.host {
        MetadataHost::Graph => true,
        MetadataHost::Node { id } => ns.contains(id),
        MetadataHost::Edge { id } | MetadataHost::Assertion { id } => es.contains(id),
        MetadataHost::Entity { id } => entities.contains(id),
    });
    out.node_origins.retain(|id, _| ns.contains(id));
    out.edge_origins.retain(|id, _| es.contains(id));
    let ats: BTreeSet<_> = out.graph.attachments.iter().map(|a| a.id.clone()).collect();
    out.attachment_origins.retain(|id, _| ats.contains(id));
    // Retain execution provenance and snapshots, including dependencies of selected derived members.
    checked(out, ctx)
}
/// Snapshot membership delta. Removed members keep their original assertion polarity.
pub fn diff(
    before: QueryResult,
    after: QueryResult,
    ctx: &AlgebraContext,
) -> Result<QueryResult, Diagnostic> {
    preflight(&before, ctx)?;
    preflight(&after, ctx)?;
    let before_edges: BTreeSet<_> = before
        .graph
        .edges
        .iter()
        .map(|e| edge_key(&before, e))
        .collect::<Result<_, _>>()?;
    let after_edges: BTreeSet<_> = after
        .graph
        .edges
        .iter()
        .map(|e| edge_key(&after, e))
        .collect::<Result<_, _>>()?;
    let before_nodes: BTreeSet<_> = before
        .graph
        .nodes
        .iter()
        .map(|n| node_key(&before, n))
        .collect::<Result<_, _>>()?;
    let after_nodes: BTreeSet<_> = after
        .graph
        .nodes
        .iter()
        .map(|n| node_key(&after, n))
        .collect::<Result<_, _>>()?;
    let mut node_statuses = BTreeMap::new();
    let mut statuses = BTreeMap::new();
    for (input, status) in [(&before, "removed"), (&after, "added")] {
        for n in &input.graph.nodes {
            let id = node_key(input, n)?;
            if !(before_nodes.contains(&id) && after_nodes.contains(&id)) {
                node_statuses.insert(id, status);
            }
        }
        for e in &input.graph.edges {
            let k = edge_key(input, e)?;
            if !(before_edges.contains(&k) && after_edges.contains(&k)) {
                let from = node_key(
                    input,
                    input
                        .graph
                        .nodes
                        .iter()
                        .find(|n| n.id == e.from)
                        .ok_or_else(|| err("E_ENDPOINT", "Missing diff endpoint"))?,
                )?;
                let to = node_key(
                    input,
                    input
                        .graph
                        .nodes
                        .iter()
                        .find(|n| n.id == e.to)
                        .ok_or_else(|| err("E_ENDPOINT", "Missing diff endpoint"))?,
                )?;
                statuses.insert(format!("edge:{}", key(&(&k, from, to))), status);
            }
        }
    }
    let mut out = union(before, after, ctx)?;
    let mut budget = Budget::new(ctx);
    budget.add(&out)?;
    for (id, status) in statuses {
        let edge = out
            .graph
            .edges
            .iter()
            .find(|e| e.id == id)
            .expect("union retains members");
        let attachment = MetadataAttachment {
            id: format!("weave:diff:{}", key(&(&id, status))),
            host: MetadataHost::Edge { id: id.clone() },
            key: "weave:diff:membership".into(),
            value: MetadataValue::Literal {
                value: json!(status),
            },
            valid_time: edge.valid_time.clone(),
            origin: None,
            readers: vec![ctx.principal.clone()],
            required: false,
            schema_revision: None,
        };
        let origins = out.edge_origins.get(&id).cloned().unwrap_or_default();
        budget.add(&(&attachment, &origins))?;
        out.attachment_origins
            .insert(attachment.id.clone(), origins);
        out.graph.attachments.push(attachment);
    }
    for (id, status) in node_statuses {
        let attachment = MetadataAttachment {
            id: format!("weave:diff:{}", key(&(&id, status))),
            host: MetadataHost::Node { id },
            key: "weave:diff:membership".into(),
            value: MetadataValue::Literal {
                value: json!(status),
            },
            valid_time: Interval {
                start: i64::MIN,
                end: None,
            },
            origin: None,
            readers: vec![ctx.principal.clone()],
            required: false,
            schema_revision: None,
        };
        budget.add(&attachment)?;
        out.graph.attachments.push(attachment);
    }
    checked(out, ctx)
}
/// Four-valued evidence at a single instant, with no closed-world absence inference.
pub fn support(
    input: QueryResult,
    predicate: &str,
    from: &EntitySpace,
    to: &EntitySpace,
    valid_at: i64,
    ctx: &AlgebraContext,
) -> Result<QueryResult, Diagnostic> {
    preflight(&input, ctx)?;
    let nodes: BTreeMap<_, _> = input.graph.nodes.iter().map(|n| (&n.id, n)).collect();
    let mut positive = Vec::new();
    let mut negative = Vec::new();
    for edge in &input.graph.edges {
        if edge.predicate != predicate || !edge.valid_time.contains(valid_at) {
            continue;
        }
        let (Some(a), Some(b)) = (nodes.get(&edge.from), nodes.get(&edge.to)) else {
            return Err(err("E_ENDPOINT", "Missing support endpoint"));
        };
        if a.entity_id != from.entity_id
            || a.space_id != from.space_id
            || b.entity_id != to.entity_id
            || b.space_id != to.space_id
        {
            continue;
        }
        if edge.assertion_context.is_some() {
            return Err(err(
                "E_CONTEXT_REQUIRED",
                "Contextual support requires an explicit compatible context selection",
            ));
        }
        let origins = input
            .edge_origins
            .get(&edge.id)
            .cloned()
            .unwrap_or_default();
        if edge.derivations.is_empty() && origins.is_empty() {
            return Err(err(
                "E_ORIGIN_MISSING",
                "Support premise lacks runtime provenance",
            ));
        }
        let groups = edge_alternatives(edge, &origins);
        match edge.polarity {
            Polarity::Positive => positive.extend(groups),
            Polarity::Negative => negative.extend(groups),
        }
    }
    let state = match (positive.is_empty(), negative.is_empty()) {
        (true, true) => "unknown",
        (false, true) => "supported",
        (true, false) => "refuted",
        (false, false) => "conflicted",
    };
    let parameters: BTreeMap<String, Value> = [
        ("predicate".into(), json!(predicate)),
        ("from".into(), json!(from)),
        ("to".into(), json!(to)),
        ("valid_at".into(), json!(valid_at)),
        ("state".into(), json!(state)),
    ]
    .into();
    let mut out = envelope(&input, None)?;
    let mut budget = Budget::new(ctx);
    let mut derivations = Vec::new();
    let mut append = |parents: Vec<&Derivation>| -> Result<(), Diagnostic> {
        if derivations.len() >= 128 {
            return Err(err("E_ALGEBRA_LIMIT", "Support alternatives exceed budget"));
        }
        let premises = unique(parents.iter().flat_map(|d| d.premises.iter().cloned()));
        let mut parameters = parameters.clone();
        parameters.insert("inputs".into(), json!(parents));
        let d = Derivation {
            operator: "weave:support".into(),
            premises: premises.clone(),
            parameters: parameters.clone(),
            input_snapshots: premise_snapshots(premises.iter()),
        };
        budget.add(&d)?;
        derivations.push(d);
        Ok(())
    };
    match state {
        "conflicted" => {
            if positive.len().saturating_mul(negative.len()) > ctx.max_objects.min(128) {
                return Err(err("E_ALGEBRA_LIMIT", "Support alternatives exceed budget"));
            }
            for p in &positive {
                for n in &negative {
                    append(vec![p, n])?;
                }
            }
        }
        "supported" => {
            for p in &positive {
                append(vec![p])?;
            }
        }
        "refuted" => {
            for n in &negative {
                append(vec![n])?;
            }
        }
        _ => {}
    }
    let id = format!("support:{}", key(&(&parameters, &input.input_snapshots)));
    let node = Node {
        id: id.clone(),
        type_id: Some("Support".into()),
        entity_id: id.clone(),
        space_id: "weave:analysis".into(),
        properties: [
            ("state".into(), json!(state)),
            ("valid_at".into(), json!(valid_at)),
            (
                "coverage".into(),
                json!(if input.coverage == Coverage::Complete {
                    "complete"
                } else {
                    "partial"
                }),
            ),
        ]
        .into(),
        metadata: vec![],
        readers: vec![ctx.principal.clone()],
    };
    budget.add(&node)?;
    out.graph.schema = Some(GraphSchema {
        id: "weave:support:status".into(),
        revision: "1".into(),
        nodes: [(
            "Support".into(),
            NodeSchema {
                properties: [
                    (
                        "state".into(),
                        PropertySchema {
                            value_type: ScalarType::String,
                            required: true,
                            nullable: false,
                        },
                    ),
                    (
                        "valid_at".into(),
                        PropertySchema {
                            value_type: ScalarType::Integer,
                            required: true,
                            nullable: false,
                        },
                    ),
                    (
                        "coverage".into(),
                        PropertySchema {
                            value_type: ScalarType::String,
                            required: true,
                            nullable: false,
                        },
                    ),
                ]
                .into(),
                space_id: Some("weave:analysis".into()),
                allow_extra_properties: false,
            },
        )]
        .into(),
        edges: [(
            "SupportEvidence".into(),
            EdgeSchema {
                from_type: "Support".into(),
                to_type: "Support".into(),
                properties: BTreeMap::new(),
                allow_cross_space: false,
                allow_extra_properties: false,
            },
        )]
        .into(),
    });
    out.graph.nodes.push(node);
    // This is a derived value identity, not a claim that a persisted source node exists.
    out.node_origins.insert(
        id.clone(),
        unique(input.node_origins.values().flatten().cloned()),
    );
    if !derivations.is_empty() {
        let origins = unique(derivations.iter().flat_map(|d| d.premises.clone()));
        let edge = Edge {
            structural_ref: None,
            assertion_source: None,
            assertion_context: None,
            assertion_properties: BTreeMap::new(),
            id: format!("{id}:evidence"),
            type_id: Some("SupportEvidence".into()),
            predicate: "weave:support:evidence".into(),
            from: id.clone(),
            to: id,
            valid_time: Interval {
                start: valid_at,
                end: valid_at.checked_add(1),
            },
            polarity: Polarity::Positive,
            properties: BTreeMap::new(),
            metadata: vec![],
            readers: vec![ctx.principal.clone()],
            derived_from: origins.clone(),
            derivations,
        };
        budget.add(&edge)?;
        out.edge_origins.insert(edge.id.clone(), origins);
        out.graph.edges.push(edge);
    }
    checked(out, ctx)
}

fn premise_snapshots<'a>(premises: impl Iterator<Item = &'a AssertionRef>) -> Vec<GraphRef> {
    unique(premises.map(|p| GraphRef {
        graph_id: p.graph_id.clone(),
        revision: p.revision.clone(),
    }))
}

fn edge_alternatives(edge: &Edge, origins: &[AssertionRef]) -> Vec<Derivation> {
    if edge.derivations.is_empty() {
        vec![Derivation {
            operator: "weave:source".into(),
            premises: origins.to_vec(),
            parameters: BTreeMap::new(),
            input_snapshots: premise_snapshots(origins.iter()),
        }]
    } else {
        edge.derivations.clone()
    }
}

/// Compose OR-of-AND derivations for a two-premise operator. Preserve the parent
/// operator parameters as a trace in addition to flattened, authorizable leaves.
#[allow(clippy::too_many_arguments)]
pub fn combine_derivations(
    left: &Edge,
    left_origins: &[AssertionRef],
    right: &Edge,
    right_origins: &[AssertionRef],
    operator: &str,
    parameters: BTreeMap<String, Value>,
    _snapshots: &[GraphRef],
    ctx: &AlgebraContext,
) -> Result<Vec<Derivation>, Diagnostic> {
    let left = edge_alternatives(left, left_origins);
    let right = edge_alternatives(right, right_origins);
    if left.len().saturating_mul(right.len()) > ctx.max_objects.min(128) {
        return Err(err(
            "E_ALGEBRA_LIMIT",
            "Derivation alternatives exceed budget",
        ));
    }
    let mut budget = Budget::new(ctx);
    let mut output = Vec::new();
    for l in &left {
        for r in &right {
            let mut params = parameters.clone();
            params.insert("inputs".into(), json!([l, r]));
            let d = Derivation {
                operator: operator.into(),
                premises: unique(l.premises.iter().chain(&r.premises).cloned()),
                parameters: params,
                input_snapshots: premise_snapshots(l.premises.iter().chain(&r.premises)),
            };
            budget.add(&d)?;
            output.push(d);
        }
    }
    Ok(unique(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> AlgebraContext {
        AlgebraContext {
            principal: "reader".into(),
            max_objects: 100,
            max_output_bytes: 1_000_000,
        }
    }
    fn fixture(graph: &str, polarity: &str, start: i64, end: i64) -> QueryResult {
        let mut result:QueryResult=serde_json::from_value(json!({"version":VERSION,"graph":{"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"e","predicate":"p","from":"a","to":"b","valid_time":{"start":start,"end":end},"polarity":polarity}]},"snapshots":{},"input_snapshots":[{"graph_id":graph,"revision":"r"}],"coverage":"complete","diagnostics":[],"provenance":[],"metadata_graphs":[]})).unwrap();
        result.node_origins = [
            (
                "a".into(),
                vec![NodeRef {
                    graph_id: graph.into(),
                    revision: "r".into(),
                    node_id: "a".into(),
                }],
            ),
            (
                "b".into(),
                vec![NodeRef {
                    graph_id: graph.into(),
                    revision: "r".into(),
                    node_id: "b".into(),
                }],
            ),
        ]
        .into();
        let origin = AssertionRef {
            graph_id: graph.into(),
            revision: "r".into(),
            assertion_id: "e".into(),
        };
        result.edge_origins.insert("e".into(), vec![origin.clone()]);
        result.provenance.push(origin);
        result
    }
    fn state(r: &QueryResult) -> &str {
        r.graph.nodes[0].properties["state"].as_str().unwrap()
    }
    fn target(entity: &str) -> EntitySpace {
        EntitySpace {
            entity_id: entity.into(),
            space_id: "s".into(),
        }
    }
    #[test]
    fn union_dedups_same_origin_without_conflating_graph_local_ids() {
        let a = fixture("a", "positive", 0, 10);
        let b = fixture("b", "positive", 0, 10);
        let ab = union(a.clone(), b, &ctx()).unwrap();
        assert_eq!(ab.graph.nodes.len(), 4);
        assert_eq!(ab.graph.edges.len(), 2);
        let aba = union(ab, a, &ctx()).unwrap();
        assert_eq!(aba.graph.nodes.len(), 4);
        assert_eq!(aba.graph.edges.len(), 2);
        assert_eq!(aba.input_snapshots.len(), 2);
        assert!(aba.graph.nodes.iter().all(|n| n.readers == ["reader"]));
    }
    #[test]
    fn projection_keeps_endpoints_and_named_metadata() {
        let mut a = fixture("a", "positive", 0, 10);
        a.graph.attachments.push(serde_json::from_value(json!({"id":"m","host":{"kind":"edge","id":"e"},"key":"note","value":{"kind":"literal","value":"evidence"},"valid_time":{"start":0,"end":10}})).unwrap());
        let p = project(a, &[], &["e".into()], &ctx()).unwrap();
        assert_eq!(p.graph.nodes.len(), 2);
        assert_eq!(p.graph.attachments.len(), 1);
        assert_eq!(p.edge_origins.len(), 1);
        assert!(project(p, &["missing".into()], &[], &ctx()).is_err());
    }
    #[test]
    fn diff_removal_does_not_invent_negative_evidence() {
        let a = fixture("a", "positive", 0, 10);
        let empty = project(a.clone(), &[], &[], &ctx()).unwrap();
        let delta = diff(a, empty, &ctx()).unwrap();
        assert_eq!(delta.graph.edges[0].polarity, Polarity::Positive);
        assert!(delta
            .graph
            .attachments
            .iter()
            .any(|a| matches!(&a.value,MetadataValue::Literal{value} if value=="removed")));
    }
    #[test]
    fn support_four_states_are_time_specific_and_open_world() {
        let p = fixture("a", "positive", 0, 10);
        let n = fixture("b", "negative", 5, 15);
        let combined = union(p.clone(), n, &ctx()).unwrap();
        for (at, expected) in [
            (0, "supported"),
            (5, "conflicted"),
            (10, "refuted"),
            (20, "unknown"),
        ] {
            let result = support(
                combined.clone(),
                "p",
                &target("A"),
                &target("B"),
                at,
                &ctx(),
            )
            .unwrap();
            assert_eq!(state(&result), expected);
            assert!(validate_schema_graph(&result.graph).is_empty());
        }
        let mut partial = p;
        partial.coverage = Coverage::Partial;
        let r = support(partial, "missing", &target("A"), &target("B"), 0, &ctx()).unwrap();
        assert_eq!(state(&r), "unknown");
        assert_eq!(r.coverage, Coverage::Partial);
    }
    #[test]
    fn support_preserves_alternative_and_joint_premises() {
        let p = fixture("a", "positive", 0, 10);
        let q = fixture("b", "positive", 0, 10);
        let n = fixture("c", "negative", 0, 10);
        let r = support(
            union(union(p, q, &ctx()).unwrap(), n, &ctx()).unwrap(),
            "p",
            &target("A"),
            &target("B"),
            5,
            &ctx(),
        )
        .unwrap();
        assert_eq!(r.graph.edges[0].derivations.len(), 2);
        assert!(r.graph.edges[0]
            .derivations
            .iter()
            .all(|d| d.premises.len() == 2));
    }
    #[test]
    fn byte_object_and_missing_origin_limits_fail_closed() {
        let a = fixture("a", "positive", 0, 10);
        let mut small = ctx();
        small.max_output_bytes = 100;
        assert_eq!(
            union(a.clone(), a.clone(), &small).unwrap_err().code,
            "E_ALGEBRA_LIMIT"
        );
        small = ctx();
        small.max_objects = 1;
        assert!(project(a.clone(), &[], &[], &small).is_err());
        let mut invalid = a.clone();
        invalid.node_origins.clear();
        assert_eq!(
            union(a, invalid, &ctx()).unwrap_err().code,
            "E_ORIGIN_MISSING"
        );
    }
    #[test]
    fn joins_preserve_parent_parameters_and_alternative_groups() {
        let mut a = fixture("a", "positive", 0, 10);
        let b = fixture("b", "positive", 0, 10);
        a.graph.edges[0].derivations = vec![Derivation {
            operator: "rule:r".into(),
            premises: a.edge_origins["e"].clone(),
            parameters: [("threshold".into(), json!(3))].into(),
            input_snapshots: a.input_snapshots.clone(),
        }];
        let d = combine_derivations(
            &a.graph.edges[0],
            &a.edge_origins["e"],
            &b.graph.edges[0],
            &b.edge_origins["e"],
            "join",
            BTreeMap::new(),
            &[],
            &ctx(),
        )
        .unwrap();
        assert_eq!(d[0].premises.len(), 2);
        assert_eq!(d[0].parameters["inputs"][0]["parameters"]["threshold"], 3);
    }
    #[test]
    fn typed_union_preserves_nominal_types_across_nested_composition() {
        fn typed(mut q: QueryResult, schema: &str) -> QueryResult {
            q.graph.schema=Some(serde_json::from_value(json!({"id":schema,"revision":"1","nodes":{"N":{}},"edges":{"E":{"from_type":"N","to_type":"N"}}})).unwrap());
            for n in &mut q.graph.nodes {
                n.type_id = Some("N".into());
            }
            q.graph.edges[0].type_id = Some("E".into());
            q
        }
        let a = typed(fixture("a", "positive", 0, 10), "schemaA");
        let b = typed(fixture("b", "positive", 0, 10), "schemaB");
        let ab = union(a.clone(), b, &ctx()).unwrap();
        assert_eq!(ab.graph.schema.as_ref().unwrap().nodes.len(), 2);
        let aba = union(ab, a, &ctx()).unwrap();
        assert_eq!(aba.graph.nodes.len(), 4);
        assert!(validate_schema_graph(&aba.graph).is_empty());
        assert_eq!(
            union(aba, fixture("u", "positive", 0, 10), &ctx())
                .unwrap_err()
                .code,
            "E_SCHEMA_ALGEBRA"
        );
    }
    #[test]
    fn alternate_groups_do_not_expose_unrelated_snapshot_names() {
        let a = fixture("a", "positive", 0, 10);
        let b = fixture("b", "positive", 0, 10);
        let extra = GraphRef {
            graph_id: "private-other-path".into(),
            revision: "hidden".into(),
        };
        let d = combine_derivations(
            &a.graph.edges[0],
            &a.edge_origins["e"],
            &b.graph.edges[0],
            &b.edge_origins["e"],
            "join",
            BTreeMap::new(),
            &[extra],
            &ctx(),
        )
        .unwrap();
        assert_eq!(d[0].input_snapshots.len(), 2);
        assert!(!serde_json::to_string(&d)
            .unwrap()
            .contains("private-other-path"));
    }
    #[test]
    fn contextual_support_is_not_silently_treated_as_universal() {
        let mut input = fixture("a", "positive", 0, 10);
        input.graph.edges[0].assertion_context = Some(GraphRef {
            graph_id: "scenario".into(),
            revision: "1".into(),
        });
        assert_eq!(
            support(input.clone(), "p", &target("A"), &target("B"), 5, &ctx())
                .unwrap_err()
                .code,
            "E_CONTEXT_REQUIRED"
        );
        let result = support(input, "other", &target("A"), &target("B"), 5, &ctx()).unwrap();
        assert_eq!(state(&result), "unknown");
    }
    #[test]
    fn source_manifest_merges_reject_conflicting_labels() {
        let source = SourceRevision {
            name: "R".into(),
            revision: "1".into(),
            digest: "a".into(),
        };
        let changed = SourceRevision {
            digest: "b".into(),
            ..source.clone()
        };
        assert_eq!(
            merge_source_revisions(std::slice::from_ref(&source), &[changed])
                .unwrap_err()
                .code,
            "E_SOURCE_REVISION"
        );
        assert_eq!(
            merge_source_revisions(std::slice::from_ref(&source), std::slice::from_ref(&source))
                .unwrap()
                .len(),
            1
        );
    }
}
