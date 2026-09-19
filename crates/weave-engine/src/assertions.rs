//! Explicit assertion materialization keeps structural identity separate from source claim identity.
use super::*;
pub(crate) fn assertion_edge(
    assertion: &Assertion,
    edge: &StructuralEdge,
    reference: Option<StructuralRef>,
) -> Edge {
    let mut metadata = edge.metadata.clone();
    for r in assertion.metadata.iter().chain(assertion.context.iter()) {
        if !metadata.contains(r) {
            metadata.push(r.clone());
        }
    }
    Edge {
        assertion_source: Some(assertion.source.clone()),
        assertion_context: assertion.context.clone(),
        structural_ref: reference,
        assertion_properties: assertion.properties.clone(),
        type_id: edge.type_id.clone(),
        id: assertion.id.clone(),
        predicate: edge.predicate.clone(),
        from: edge.from.clone(),
        to: edge.to.clone(),
        valid_time: assertion.valid_time.clone(),
        polarity: assertion.polarity.clone(),
        properties: edge.properties.clone(),
        metadata,
        readers: if assertion.readers.is_empty() {
            edge.readers.clone()
        } else if edge.readers.is_empty() {
            assertion.readers.clone()
        } else {
            assertion
                .readers
                .iter()
                .filter(|p| edge.readers.contains(p))
                .cloned()
                .collect()
        },
        derived_from: assertion.derived_from.clone(),
        derivations: assertion.derivations.clone(),
    }
}
pub(crate) fn materialize(
    mut data: GraphData,
    graph: &str,
    revision: &str,
) -> Result<(GraphData, BTreeMap<String, Vec<AssertionRef>>)> {
    let mut node_bytes = json_size(&data, MATERIALIZED_LIMIT)?;
    for node in &mut data.nodes {
        let origin = NodeRef {
            graph_id: graph.into(),
            revision: revision.into(),
            node_id: node.id.clone(),
        };
        if !node.derived_nodes.contains(&origin) {
            if node
                .derived_from
                .len()
                .saturating_add(node.derived_nodes.len())
                >= 1000
            {
                return Err(err(
                    "E_BUDGET",
                    "Node influence expansion exceeds output limit",
                ));
            }
            node_bytes += json_size(&origin, MATERIALIZED_LIMIT.saturating_sub(node_bytes))?;
            node.derived_nodes.push(origin);
        }
    }
    if data.profile == GraphProfile::Legacy {
        return Ok((data, BTreeMap::new()));
    }
    let structures: BTreeMap<_, _> = data.structural_edges.iter().map(|e| (&e.id, e)).collect();
    let mut out = GraphData {
        nodes: data.nodes.clone(),
        schema: data.schema.clone(),
        ..GraphData::default()
    };
    let mut bytes = json_size(&out, MATERIALIZED_LIMIT)?;
    for assertion in &data.assertions {
        let structure = structures
            .get(&assertion.edge_id)
            .ok_or_else(|| err("E_ASSERTION", "assertion structural edge unavailable"))?;
        let mut edge = assertion_edge(
            assertion,
            structure,
            Some(StructuralRef {
                graph_id: graph.into(),
                revision: revision.into(),
                edge_id: structure.id.clone(),
            }),
        );
        let source = AssertionRef {
            graph_id: graph.into(),
            revision: revision.into(),
            assertion_id: assertion.id.clone(),
        };
        if edge.derivations.is_empty() {
            let mut premises = edge.derived_from.clone();
            if !premises.contains(&source) {
                premises.push(source.clone());
            }
            edge.derivations = vec![Derivation {
                operator: "weave:assertion".into(),
                premises,
                parameters: BTreeMap::new(),
                input_snapshots: vec![GraphRef {
                    graph_id: graph.into(),
                    revision: revision.into(),
                }],
            }];
        } else {
            for group in &mut edge.derivations {
                if !group.premises.contains(&source) {
                    group.premises.push(source.clone());
                }
                let snapshot = GraphRef {
                    graph_id: graph.into(),
                    revision: revision.into(),
                };
                if !group.input_snapshots.contains(&snapshot) {
                    group.input_snapshots.push(snapshot);
                }
            }
        }
        edge.derived_from = Vec::new();
        for p in edge.derivations.iter().flat_map(|g| &g.premises) {
            if !edge.derived_from.contains(p) {
                edge.derived_from.push(p.clone());
            }
        }
        bytes += json_size(&edge, MATERIALIZED_LIMIT.saturating_sub(bytes))?;
        out.edges.push(edge);
    }
    let mut origins = BTreeMap::new();
    let mut ids = HashSet::new();
    for attachment in &data.attachments {
        let targets: Vec<MetadataHost> = match &attachment.host {
            MetadataHost::Edge { id } => data
                .assertions
                .iter()
                .filter(|a| &a.edge_id == id)
                .map(|a| MetadataHost::Edge { id: a.id.clone() })
                .collect(),
            MetadataHost::Assertion { id } => vec![MetadataHost::Edge { id: id.clone() }],
            other => vec![other.clone()],
        };
        for target in targets {
            let mut copy = attachment.clone();
            if matches!(attachment.host, MetadataHost::Edge { .. }) {
                copy.id = format!(
                    "materialized-attachment:{:x}",
                    Sha256::digest(serde_json::to_vec(&(&attachment.id, &target))?)
                );
            }
            copy.host = target;
            if !ids.insert(copy.id.clone()) {
                return Err(err(
                    "E_ATTACHMENT",
                    "materialized attachment identity collision",
                ));
            }
            let origin = vec![AssertionRef {
                graph_id: graph.into(),
                revision: revision.into(),
                assertion_id: attachment.id.clone(),
            }];
            bytes += json_size(&(&copy, &origin), MATERIALIZED_LIMIT.saturating_sub(bytes))?;
            origins.insert(copy.id.clone(), origin);
            out.attachments.push(copy);
        }
    }
    Ok((out, origins))
}
pub(crate) fn validate_explicit(data: &GraphData) -> Result<()> {
    if !data.edges.is_empty() {
        return Err(err(
            "E_PROFILE",
            "explicit graphs cannot contain legacy claim edges",
        ));
    }
    if data.structural_edges.len() > 100_000 || data.assertions.len() > 100_000 {
        return Err(err("E_BUDGET", "explicit graph object budget exceeded"));
    }
    let mut ids = HashSet::new();
    let nodes: HashSet<_> = data.nodes.iter().map(|n| &n.id).collect();
    for edge in &data.structural_edges {
        if !valid_id(&edge.id)
            || !valid_id(&edge.predicate)
            || !ids.insert(&edge.id)
            || !nodes.contains(&edge.from)
            || !nodes.contains(&edge.to)
            || edge.readers.iter().any(|p| !valid_id(p))
        {
            return Err(err(
                "E_STRUCTURAL_EDGE",
                "invalid or duplicate structural relationship",
            ));
        }
    }
    let edges = ids.clone();
    for assertion in &data.assertions {
        validate_assertion_provenance(assertion)?;
        if !valid_id(&assertion.id)
            || !valid_id(&assertion.source)
            || !ids.insert(&assertion.id)
            || !edges.contains(&assertion.edge_id)
            || !assertion.valid_time.valid()
            || assertion.readers.iter().any(|p| !valid_id(p))
        {
            return Err(err(
                "E_ASSERTION",
                "assertion identity, source, interval and existing structural edge required",
            ));
        }
    }
    let assertions: HashSet<_> = data.assertions.iter().map(|a| &a.id).collect();
    for a in &data.attachments {
        if !ids.insert(&a.id) {
            return Err(err(
                "E_ATTACHMENT",
                "structural edge, assertion and attachment IDs must be disjoint",
            ));
        }
        let valid = match &a.host {
            MetadataHost::Edge { id } => edges.contains(id),
            MetadataHost::Assertion { id } => assertions.contains(id),
            MetadataHost::Node { id } => nodes.contains(id),
            MetadataHost::Entity { id } => data.nodes.iter().any(|n| &n.entity_id == id),
            MetadataHost::Graph => true,
        };
        if !valid || !valid_id(&a.id) || !valid_id(&a.key) || !a.valid_time.valid() {
            return Err(err("E_ATTACHMENT", "invalid explicit attachment"));
        }
    }
    // Validate all structural properties even when the relationship has no supporting claims.
    for reference in data
        .structural_edges
        .iter()
        .flat_map(|e| &e.metadata)
        .chain(
            data.assertions
                .iter()
                .flat_map(|a| a.metadata.iter().chain(a.context.iter())),
        )
    {
        if !valid_id(&reference.graph_id) || !valid_id(&reference.revision) {
            return Err(err("E_REFERENCE", "explicit metadata must pin valid IDs"));
        }
    }
    let (materialized, _) = materialize(data.clone(), "validation", "validation")?;
    validate_graph(&materialized)
}

impl Engine {
    /// Exact first-class structural lookup, independent of whether the relationship has claims.
    pub fn resolve_structural(
        &self,
        reference: &StructuralRef,
        host: &HostContext,
    ) -> Result<Option<StructuralEdge>> {
        let _read_scope = self.read_budget.enter();
        let Some(data) = self.load(&reference.graph_id, &reference.revision)? else {
            return Ok(None);
        };
        let (visible, _) = self.authorized_nodes(data, host)?;
        Ok(visible
            .structural_edges
            .into_iter()
            .find(|e| e.id == reference.edge_id))
    }
    /// Exact source-assertion lookup. Source is claimed provenance, not authenticated identity.
    pub fn resolve_assertion(
        &self,
        reference: &AssertionRef,
        host: &HostContext,
    ) -> Result<Option<Assertion>> {
        let _read_scope = self.read_budget.enter();
        let Some(data) = self.load(&reference.graph_id, &reference.revision)? else {
            return Ok(None);
        };
        let (visible, _) = self.authorized(data, host)?;
        Ok(visible
            .assertions
            .into_iter()
            .find(|a| a.id == reference.assertion_id))
    }
}

fn validate_assertion_provenance(assertion: &Assertion) -> Result<()> {
    if assertion.derivations.len() > 128
        || assertion
            .derivations
            .iter()
            .any(|g| !valid_id(&g.operator) || g.premises.is_empty() || g.premises.len() > 1000)
    {
        return Err(err("E_DERIVATION", "invalid explicit derivation groups"));
    }
    for p in assertion
        .derived_from
        .iter()
        .chain(assertion.derivations.iter().flat_map(|g| &g.premises))
    {
        if !valid_id(&p.graph_id) || !valid_id(&p.revision) || !valid_id(&p.assertion_id) {
            return Err(err("E_DERIVATION", "invalid explicit derivation reference"));
        }
    }
    if !assertion.derivations.is_empty() {
        let flat: HashSet<_> = assertion
            .derived_from
            .iter()
            .map(|p| (&p.graph_id, &p.revision, &p.assertion_id))
            .collect();
        let grouped: HashSet<_> = assertion
            .derivations
            .iter()
            .flat_map(|g| {
                g.premises
                    .iter()
                    .map(|p| (&p.graph_id, &p.revision, &p.assertion_id))
            })
            .collect();
        if flat != grouped {
            return Err(err(
                "E_DERIVATION",
                "explicit flat index must equal grouped premises",
            ));
        }
    }
    Ok(())
}
