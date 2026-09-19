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
        let Some(attachment) = select_attachment(&input, &resolved_host, key).cloned() else {
            return Ok(missing(
                input,
                "metadata attachment unavailable or conflicted",
            ));
        };
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
        // Selecting this attachment influences even an empty/unavailable target value.
        // Keep that path restriction independently of emitted object membership.
        input.graph.influence = weave_contract::influence::merge(
            input.graph.influence.as_ref(),
            Some(&GraphInfluence {
                assertions: origin.clone(),
                nodes: vec![],
            }),
        )
        .map_err(|d| err(&d.code, &d.message))?;
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
        let influence = weave_contract::influence::merge(
            input.graph.influence.as_ref(),
            graph.influence.as_ref(),
        )
        .map_err(|d| err(&d.code, &d.message))?;
        let mut typing = context_typing::merge(
            input.graph.context_typing.as_ref(),
            graph.context_typing.as_ref(),
        )
        .map_err(|d| err(&d.code, &d.message))?;
        if let Some(t) = &mut typing {
            t.selected = None;
        }
        graph.context_typing = typing.clone();
        graph.influence = influence.clone();
        // A contextual path remains qualified even for an empty target. Default navigation
        // does not implicitly assign the parent scope to independently qualified target claims.
        input.selected_context = path_context
            .clone()
            .map(|reference| ContextSelection::Pinned { reference });
        let mut window = attachment.valid_time.clone();
        if let MetadataHost::Edge { id } = &attachment.host {
            if let Some(edge) = input.graph.edges.iter().find(|e| &e.id == id) {
                let Some(common) = intersect(&window, &edge.valid_time) else {
                    input.graph = GraphData {
                        context_typing: typing,
                        influence,
                        ..GraphData::default()
                    };
                    input.node_origins.clear();
                    input.edge_origins.clear();
                    input.attachment_origins.clear();
                    input.provenance.clear();
                    context_typing::protect_result_generated(&mut input)
                        .map_err(|d| err(&d.code, &d.message))?;
                    weave_contract::influence::protect_generated_result(
                        &mut input,
                        MATERIALIZED_LIMIT,
                    )
                    .map_err(|d| err(&d.code, &d.message))?;
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
                    node_premises: edge.derived_nodes.clone(),
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
                        .chain(group.node_premises.iter().map(|p| GraphRef {
                            graph_id: p.graph_id.clone(),
                            revision: p.revision.clone(),
                        }))
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
        context_typing::protect_result_generated(&mut input)
            .map_err(|d| err(&d.code, &d.message))?;
        weave_contract::influence::protect_generated_result(&mut input, MATERIALIZED_LIMIT)
            .map_err(|d| err(&d.code, &d.message))?;
        // Navigation changed each node payload: retain source references as proof gates,
        // and give the path-qualified wrapper its own identity instead of false source origins.
        let mut node_ids = BTreeMap::new();
        for node in &mut input.graph.nodes {
            let old = node.id.clone();
            node.id = format!(
                "metadata-node:{:x}",
                Sha256::digest(serde_json::to_vec(&node)?)
            );
            node_ids.insert(old, node.id.clone());
        }
        for edge in &mut input.graph.edges {
            edge.from = node_ids[&edge.from].clone();
            edge.to = node_ids[&edge.to].clone();
        }
        for attachment in &mut input.graph.attachments {
            if let MetadataHost::Node { id } = &mut attachment.host {
                *id = node_ids[id].clone();
            }
        }
        input.node_origins = input
            .graph
            .nodes
            .iter()
            .map(|n| (n.id.clone(), vec![]))
            .collect();
        json_size(&input, MATERIALIZED_LIMIT)?;
        Ok(input)
    }
}
// This compatibility shorthand is deliberately limited to a direct, already-authorized
// metadata materialization. Proof dependencies are not aliases for source object identity.
fn select_attachment<'a>(
    input: &'a QueryResult,
    host: &MetadataHost,
    key: &str,
) -> Option<&'a MetadataAttachment> {
    let mut exact = input
        .graph
        .attachments
        .iter()
        .filter(|a| &a.host == host && a.key == key);
    if let Some(first) = exact.next() {
        return exact.next().is_none().then_some(first);
    }
    let MetadataHost::Node { .. } = host else {
        return None;
    };
    if input.snapshots.len() != 1 {
        return None;
    }
    let (graph_id, revision) = input.snapshots.iter().next()?;
    let target = GraphRef {
        graph_id: graph_id.clone(),
        revision: revision.clone(),
    };
    let mut resolved = input
        .metadata_graphs
        .iter()
        .filter(|m| m.reference == target);
    let original = resolved.next()?;
    if resolved.next().is_some() {
        return None;
    }
    let mut originals = original
        .graph
        .attachments
        .iter()
        .filter(|a| &a.host == host && a.key == key);
    let source = originals.next()?;
    if originals.next().is_some() {
        return None;
    }
    let proof = AssertionRef {
        graph_id: target.graph_id,
        revision: target.revision,
        assertion_id: source.id.clone(),
    };
    // The current envelope must attest this exact pinned original attachment.
    // Legacy ResolvedGraph records need no remapping envelope; if one is present,
    // it must agree. Neither authored `origin` nor derived_nodes supplies this role.
    if original
        .attachment_origins
        .get(&source.id)
        .is_some_and(|origins| !origins.contains(&proof))
        || !input.attachment_origins.get(&source.id)?.contains(&proof)
    {
        return None;
    }
    let mut current = input.graph.attachments.iter().filter(|a| a.id == source.id);
    let attachment = current.next()?;
    if current.next().is_some()
        || attachment.key != key
        || attachment.value != source.value
        || attachment.context != source.context
    {
        return None;
    }
    let MetadataHost::Node { id } = &attachment.host else {
        return None;
    };
    if !id.starts_with("metadata-node:") || !input.node_origins.get(id)?.is_empty() {
        return None;
    }
    let mut nodes = input.graph.nodes.iter().filter(|n| &n.id == id);
    nodes.next()?;
    nodes.next().is_none().then_some(attachment)
}

fn missing(mut input: QueryResult, message: &str) -> QueryResult {
    let mut typing = input.graph.context_typing.take();
    if let Some(t) = &mut typing {
        t.selected = None;
    }
    let influence = input.graph.influence.take();
    input.graph = GraphData {
        influence,
        context_typing: typing,
        ..GraphData::default()
    };
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

#[cfg(test)]
mod selector_tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str) -> MetadataHost {
        MetadataHost::Node { id: id.into() }
    }

    // These envelopes model engine-produced materializations. Raw programs cannot
    // submit QueryResult provenance; mutations below test conservative internal handling.
    fn materialized() -> QueryResult {
        let original: GraphData = serde_json::from_value(json!({
            "nodes":[{"id":"b","entity_id":"B","space_id":"s"}],
            "attachments":[{"id":"bc","host":{"kind":"node","id":"b"},
                "key":"next","value":{"kind":"graph","reference":{"graph_id":"C","revision":"c1"}},
                "valid_time":{"start":0,"end":10}}]
        }))
        .unwrap();
        let proof = AssertionRef {
            graph_id: "B".into(),
            revision: "b1".into(),
            assertion_id: "bc".into(),
        };
        let mut graph = original.clone();
        graph.nodes[0].id = "metadata-node:wrapper".into();
        graph.attachments[0].host = node("metadata-node:wrapper");
        let origins = BTreeMap::from([("bc".into(), vec![proof])]);
        QueryResult {
            version: VERSION.into(),
            selected_context: None,
            source_revisions: vec![],
            graph,
            snapshots: BTreeMap::from([("B".into(), "b1".into())]),
            input_snapshots: vec![],
            coverage: Coverage::Complete,
            diagnostics: vec![],
            provenance: vec![],
            edge_origins: BTreeMap::new(),
            node_origins: BTreeMap::from([("metadata-node:wrapper".into(), vec![])]),
            attachment_origins: origins.clone(),
            metadata_graphs: vec![ResolvedGraph {
                reference: GraphRef {
                    graph_id: "B".into(),
                    revision: "b1".into(),
                },
                graph: original,
                attachment_origins: origins,
            }],
        }
    }

    fn assert_unavailable(input: QueryResult) {
        assert!(select_attachment(&input, &node("b"), "next").is_none());
        let result = Engine::memory()
            .unwrap()
            .metadata_value(input, &node("b"), "next", &HostContext::new("reader", []))
            .unwrap();
        assert_eq!(result.coverage, Coverage::Partial);
        assert!(result.graph.nodes.is_empty());
        assert_eq!(
            result.diagnostics.last().unwrap().code,
            "E_METADATA_UNAVAILABLE"
        );
    }

    #[test]
    fn original_selector_requires_current_provenance_and_consistent_original_remapping() {
        let input = materialized();
        assert_eq!(
            select_attachment(&input, &node("b"), "next").unwrap().id,
            "bc"
        );
        let mut legacy = input.clone();
        legacy.metadata_graphs[0].attachment_origins.clear();
        assert!(select_attachment(&legacy, &node("b"), "next").is_some());
        for envelope in [false, true] {
            let mut bad = input.clone();
            let origins = if envelope {
                &mut bad.attachment_origins
            } else {
                &mut bad.metadata_graphs[0].attachment_origins
            };
            origins.get_mut("bc").unwrap()[0].revision = "other".into();
            assert_unavailable(bad);
        }
        let mut bad = input.clone();
        bad.snapshots.insert("B".into(), "other".into());
        assert_unavailable(bad);
        let mut bad = input;
        bad.graph.attachments[0].value = MetadataValue::Graph {
            reference: GraphRef {
                graph_id: "Other".into(),
                revision: "r".into(),
            },
        };
        assert_unavailable(bad);
    }

    #[test]
    fn ambiguous_original_current_or_snapshot_candidates_do_not_fall_back() {
        let input = materialized();
        let mut bad = input.clone();
        let mut second = bad.metadata_graphs[0].graph.attachments[0].clone();
        second.id = "bc2".into();
        bad.metadata_graphs[0].graph.attachments.push(second);
        assert_unavailable(bad);
        let mut bad = input.clone();
        bad.graph.attachments.push(bad.graph.attachments[0].clone());
        assert_unavailable(bad);
        let mut bad = input.clone();
        bad.metadata_graphs.push(bad.metadata_graphs[0].clone());
        assert_unavailable(bad);
        let mut bad = input;
        bad.snapshots.insert("Other".into(), "r".into());
        assert_unavailable(bad);
    }

    #[test]
    fn dependency_refs_saved_values_and_remapped_attachments_are_not_aliases() {
        let input = materialized();
        let source = NodeRef {
            graph_id: "B".into(),
            revision: "b1".into(),
            node_id: "b".into(),
        };
        let mut bad = input.clone();
        bad.graph.nodes[0].derived_nodes.push(source.clone());
        bad.attachment_origins.clear();
        assert_unavailable(bad);
        let mut bad = input.clone();
        bad.node_origins
            .insert("metadata-node:wrapper".into(), vec![source]);
        assert_unavailable(bad);
        let mut bad = input.clone();
        bad.snapshots = BTreeMap::from([("Saved".into(), "s1".into())]);
        assert_unavailable(bad);
        let mut bad = input.clone();
        bad.graph.attachments[0].id = "derived-attachment:remapped".into();
        assert_unavailable(bad);
        let mut bad = input;
        bad.graph.nodes.clear();
        assert_unavailable(bad);
    }

    #[test]
    fn exact_hosts_take_precedence_and_ambiguity_never_uses_original_fallback() {
        let mut input = materialized();
        assert_eq!(
            select_attachment(&input, &node("metadata-node:wrapper"), "next")
                .unwrap()
                .id,
            "bc"
        );
        let mut exact = input.graph.attachments[0].clone();
        exact.id = "exact".into();
        exact.host = node("b");
        input.graph.attachments.push(exact.clone());
        assert_eq!(
            select_attachment(&input, &node("b"), "next").unwrap().id,
            "exact"
        );
        exact.id = "conflicting-exact".into();
        input.graph.attachments.push(exact);
        assert_unavailable(input);
    }
}
