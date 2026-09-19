use super::*;
pub(crate) fn pin_live_attachments(
    data: &mut GraphData,
    pins: &BTreeMap<(String, String), Option<String>>,
) {
    for attachment in &mut data.attachments {
        if let MetadataValue::LiveGraph {
            graph_id,
            branch_id,
        } = &attachment.value
        {
            if let Some(Some(revision)) = pins.get(&(graph_id.clone(), branch_id.clone())) {
                attachment.value = MetadataValue::Graph {
                    reference: GraphRef {
                        graph_id: graph_id.clone(),
                        revision: revision.clone(),
                    },
                };
            }
        }
    }
}
impl Engine {
    pub(crate) fn metadata_value(
        &self,
        mut input: QueryResult,
        host: &MetadataHost,
        key: &str,
        principal: &HostContext,
    ) -> Result<QueryResult> {
        let resolved_host = match host {
            MetadataHost::Assertion { id }
                if input
                    .graph
                    .edges
                    .iter()
                    .any(|e| &e.id == id && e.structural_ref.is_some()) =>
            {
                MetadataHost::Edge { id: id.clone() }
            }
            other => other.clone(),
        };
        let candidates: Vec<_> = input
            .graph
            .attachments
            .iter()
            .filter(|a| a.host == resolved_host && a.key == key)
            .take(2)
            .cloned()
            .collect();
        if candidates.len() != 1 {
            return Ok(missing(
                input,
                "metadata attachment unavailable or conflicted",
            ));
        }
        let attachment = &candidates[0];
        context::ensure_consumable(input.selected_context.as_ref(), attachment.context.as_ref())
            .map_err(|d| err(&d.code, &d.message))?;
        if let MetadataHost::Edge { id } = &attachment.host {
            if let Some(edge) = input.graph.edges.iter().find(|e| &e.id == id) {
                context::ensure_consumable(
                    input.selected_context.as_ref(),
                    edge.assertion_context.as_ref(),
                )
                .map_err(|d| err(&d.code, &d.message))?;
            }
        }
        if let MetadataHost::Node { id } = &attachment.host {
            if let Some(node) = input.graph.nodes.iter().find(|n| &n.id == id) {
                if let Some(scope) = &node.context_scope {
                    context::compatible_context(input.selected_context.as_ref(), Some(scope))
                        .map_err(|d| err(&d.code, &d.message))?;
                }
            }
        }
        let path_context = attachment.context.clone();
        let MetadataValue::Graph { reference } = &attachment.value else {
            return Ok(missing(input, "metadata graph unavailable"));
        };
        let Some(origin) = input.attachment_origins.get(&attachment.id).cloned() else {
            return Ok(missing(input, "metadata attachment origin unavailable"));
        };
        // Retain the current original snapshot for a subsequent back-reference through a cycle.
        if input.snapshots.len() == 1 {
            let (graph_id, revision) = input.snapshots.iter().next().expect("one snapshot");
            let original = GraphRef {
                graph_id: graph_id.clone(),
                revision: revision.clone(),
            };
            if !input
                .metadata_graphs
                .iter()
                .any(|m| m.reference == original)
            {
                input.metadata_graphs.push(ResolvedGraph {
                    attachment_origins: input.attachment_origins.clone(),
                    reference: original,
                    graph: input.graph.clone(),
                });
            }
        }
        let Some(resolved) = input
            .metadata_graphs
            .iter()
            .find(|m| &m.reference == reference)
        else {
            return Ok(missing(
                input,
                "metadata graph not materialized; query with include_metadata",
            ));
        };
        let resolved_origins = resolved.attachment_origins.clone();
        let mut graph = resolved.graph.clone();
        if let Some(required) = &path_context {
            if graph.nodes.iter().any(|n| {
                n.context_scope
                    .as_ref()
                    .is_some_and(|scope| scope.reference() != Some(required))
            }) || graph
                .edges
                .iter()
                .any(|e| e.assertion_context.as_ref() != Some(required))
                || graph
                    .attachments
                    .iter()
                    .any(|a| a.context.as_ref() != Some(required))
            {
                return Err(err(
                    "E_CONTEXT_MISMATCH",
                    "metadata target qualifiers do not match contextual access path",
                ));
            }
        }
        // A contextual path remains qualified even for an empty target. Default navigation
        // does not implicitly assign the parent scope to independently qualified target claims.
        input.selected_context = path_context
            .clone()
            .map(|reference| ContextSelection::Pinned { reference });
        let mut window = attachment.valid_time.clone();
        if let MetadataHost::Edge { id } = &attachment.host {
            if let Some(edge) = input.graph.edges.iter().find(|e| &e.id == id) {
                let Some(common) = intersect(&window, &edge.valid_time) else {
                    input.graph = GraphData::default();
                    input.node_origins.clear();
                    input.edge_origins.clear();
                    input.attachment_origins.clear();
                    input.provenance.clear();
                    return Ok(input);
                };
                window = common;
            }
        }
        graph.edges = graph
            .edges
            .into_iter()
            .filter_map(|mut e| {
                e.valid_time = intersect(&e.valid_time, &window)?;
                Some(e)
            })
            .collect();
        graph.attachments = graph
            .attachments
            .into_iter()
            .filter_map(|mut a| {
                a.valid_time = intersect(&a.valid_time, &window)?;
                Some(a)
            })
            .collect();
        prune_attachments(&mut graph);

        // This access path has its own visibility; it does not relabel the stored target graph.
        graph
            .nodes
            .iter_mut()
            .for_each(|n| n.readers = vec![principal.principal.clone()]);
        graph
            .edges
            .iter_mut()
            .for_each(|e| e.readers = vec![principal.principal.clone()]);
        graph
            .attachments
            .iter_mut()
            .for_each(|a| a.readers = vec![principal.principal.clone()]);
        input.snapshots =
            BTreeMap::from([(reference.graph_id.clone(), reference.revision.clone())]);
        input.graph = graph;
        input.edge_origins.clear();
        input.attachment_origins.clear();
        input.provenance.clear();
        input.node_origins.clear();
        let mut bytes = json_size(&input, MATERIALIZED_LIMIT)?;
        for node in &mut input.graph.nodes {
            for dependency in &origin {
                if !node.derived_from.contains(dependency) {
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
                    bytes += json_size(dependency, MATERIALIZED_LIMIT.saturating_sub(bytes))?;
                    node.derived_from.push(dependency.clone());
                }
            }
            let origins = vec![NodeRef {
                graph_id: reference.graph_id.clone(),
                revision: reference.revision.clone(),
                node_id: node.id.clone(),
            }];
            bytes += json_size(
                &(&node.id, &origins),
                MATERIALIZED_LIMIT.saturating_sub(bytes),
            )?;
            if !node.derived_nodes.contains(&origins[0]) {
                bytes += json_size(&origins[0], MATERIALIZED_LIMIT.saturating_sub(bytes))?;
                node.derived_nodes.push(origins[0].clone());
            }
            input.node_origins.insert(node.id.clone(), origins);
        }
        for dependency in &origin {
            bytes += json_size(dependency, MATERIALIZED_LIMIT.saturating_sub(bytes))?;
            input.provenance.push(dependency.clone());
        }
        for edge in &mut input.graph.edges {
            let source = AssertionRef {
                graph_id: reference.graph_id.clone(),
                revision: reference.revision.clone(),
                assertion_id: edge.id.clone(),
            };
            let mut dependencies = origin.clone();
            if !dependencies.contains(&source) {
                dependencies.push(source.clone());
                bytes += json_size(&source, MATERIALIZED_LIMIT.saturating_sub(bytes))?;
                input.provenance.push(source);
            }
            bytes += json_size(
                &(&edge.id, &dependencies),
                MATERIALIZED_LIMIT.saturating_sub(bytes),
            )?;
            if edge.derivations.is_empty() {
                edge.derivations = vec![Derivation {
                    operator: "weave:metadata".into(),
                    premises: dependencies.clone(),
                    parameters: BTreeMap::from([(
                        "key".into(),
                        serde_json::Value::String(key.into()),
                    )]),
                    input_snapshots: dependencies
                        .iter()
                        .map(|p| GraphRef {
                            graph_id: p.graph_id.clone(),
                            revision: p.revision.clone(),
                        })
                        .collect(),
                }];
            } else {
                for group in &mut edge.derivations {
                    for p in &origin {
                        if !group.premises.contains(p) {
                            group.premises.push(p.clone());
                        }
                    }
                    group.input_snapshots = group
                        .premises
                        .iter()
                        .map(|p| GraphRef {
                            graph_id: p.graph_id.clone(),
                            revision: p.revision.clone(),
                        })
                        .collect();
                }
                dependencies.clear();
                for p in edge.derivations.iter().flat_map(|g| &g.premises) {
                    if !dependencies.contains(p) {
                        dependencies.push(p.clone());
                    }
                }
            }
            edge.derived_from = dependencies.clone();
            bytes += json_size(&edge.derivations, MATERIALIZED_LIMIT.saturating_sub(bytes))?;
            input.edge_origins.insert(edge.id.clone(), dependencies);
        }
        for attachment in &input.graph.attachments {
            let source = AssertionRef {
                graph_id: reference.graph_id.clone(),
                revision: reference.revision.clone(),
                assertion_id: attachment.id.clone(),
            };
            let mut dependencies = origin.clone();
            for p in resolved_origins.get(&attachment.id).into_iter().flatten() {
                if !dependencies.contains(p) {
                    dependencies.push(p.clone());
                }
            }
            if !resolved_origins.contains_key(&attachment.id) && !dependencies.contains(&source) {
                dependencies.push(source);
            }
            bytes += json_size(
                &(&attachment.id, &dependencies),
                MATERIALIZED_LIMIT.saturating_sub(bytes),
            )?;
            input
                .attachment_origins
                .insert(attachment.id.clone(), dependencies);
        }
        json_size(&input, MATERIALIZED_LIMIT)?;
        Ok(input)
    }
}
fn missing(mut input: QueryResult, message: &str) -> QueryResult {
    input.graph = GraphData::default();
    input.node_origins.clear();
    input.edge_origins.clear();
    input.attachment_origins.clear();
    input.provenance.clear();
    partial(&mut input, "E_METADATA_UNAVAILABLE", message);
    input
}

fn intersect(a: &Interval, b: &Interval) -> Option<Interval> {
    let start = a.start.max(b.start);
    let end = match (a.end, b.end) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    };
    if end.is_some_and(|end| start >= end) {
        None
    } else {
        Some(Interval { start, end })
    }
}
pub(crate) fn carry_node_attachments(
    input: &QueryResult,
    original: &Node,
    output: &Node,
    principal: &HostContext,
    attachments: &mut BTreeMap<String, MetadataAttachment>,
    origins: &mut BTreeMap<String, Vec<AssertionRef>>,
    bytes: &mut usize,
) -> Result<()> {
    for attachment in &input.graph.attachments {
        let relevant = match &attachment.host {
            MetadataHost::Node { id } => id == &original.id,
            MetadataHost::Entity { id } => id == &original.entity_id,
            _ => false,
        };
        if !relevant {
            continue;
        }
        let mut copy = attachment.clone();
        copy.id = format!(
            "derived-attachment:{:x}",
            Sha256::digest(serde_json::to_vec(&(
                &input.input_snapshots,
                &attachment.id,
                &output.id
            ))?)
        );
        if attachments.contains_key(&copy.id) {
            continue;
        }
        if matches!(copy.host, MetadataHost::Node { .. }) {
            copy.host = MetadataHost::Node {
                id: output.id.clone(),
            };
        }
        copy.readers = vec![principal.principal.clone()];
        let source = input
            .attachment_origins
            .get(&attachment.id)
            .cloned()
            .ok_or_else(|| err("E_PROVENANCE", "attachment origin unavailable"))?;
        *bytes += json_size(&copy, MATERIALIZED_LIMIT.saturating_sub(*bytes))?;
        *bytes += json_size(
            &(&copy.id, &source),
            MATERIALIZED_LIMIT.saturating_sub(*bytes),
        )?;
        origins.insert(copy.id.clone(), source);
        attachments.insert(copy.id.clone(), copy);
    }
    Ok(())
}
