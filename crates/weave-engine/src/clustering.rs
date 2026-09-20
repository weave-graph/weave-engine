//! Authorized, snapshot-scoped graph navigation. No approximate query pruning.
use super::*;
use serde_json::json;
use std::collections::BTreeSet;
use weave_cluster::{Hierarchy, Link, Member, Snapshot};

fn failure(e: weave_cluster::Error) -> Error {
    err(
        e.0,
        "clustering input or operation unavailable within its budget",
    )
}
fn member_id(m: &Member) -> &str {
    match m {
        Member::Leaf(id) | Member::Cluster(id) => id,
    }
}
fn require_repersistable(graph: &GraphData) -> Result<()> {
    if graph
        .nodes
        .iter()
        .any(|n| n.derived_from.len().saturating_add(n.derived_nodes.len()) > 999)
    {
        return Err(err(
            "E_CLUSTER_BUDGET",
            "context-protected influence leaves no repersistence pin",
        ));
    }
    Ok(())
}
fn require_proof_capacity(
    claims: &[AssertionRef],
    nodes: &[NodeRef],
    typing: Option<&ContextTyping>,
) -> Result<()> {
    let (extra_claims, extra_nodes) = typing
        .map(context_typing::gates)
        .transpose()
        .map_err(|d| err(&d.code, &d.message))?
        .unwrap_or_default();
    let claim_count = claims
        .iter()
        .chain(&extra_claims)
        .map(|r| (&r.graph_id, &r.revision, &r.assertion_id))
        .collect::<BTreeSet<_>>()
        .len();
    let node_count = nodes
        .iter()
        .chain(&extra_nodes)
        .map(|r| (&r.graph_id, &r.revision, &r.node_id))
        .collect::<BTreeSet<_>>()
        .len();
    if claim_count + node_count > 999 {
        return Err(err(
            "E_CLUSTER_BUDGET",
            "context-protected influence leaves no repersistence pin",
        ));
    }
    Ok(())
}
impl Engine {
    fn cluster_input(&self, request: &ClusterRequest, host: &HostContext) -> Result<QueryResult> {
        if !valid_id(&request.predicate) || request.levels > 10_000 {
            return Err(err(
                "E_CLUSTER_INPUT",
                "bounded predicate and level count required",
            ));
        }
        request.valid_at.checked_add(1).ok_or_else(|| {
            err(
                "E_CLUSTER_TIME",
                "sample time has no representable exclusive end",
            )
        })?;
        // Query without an edge filter: isolated visible nodes must remain part of the snapshot.
        let query = QueryPlan {
            graph_id: request.source.graph_id.clone(),
            revision: Some(request.source.revision.clone()),
            branch_id: "main".into(),
            predicate: None,
            from: None,
            to: None,
            valid_at: None,
            include_metadata: false,
            max_depth: 0,
        };
        let input = self.query(&query, host)?;
        let mut value = context::select(input, &request.context, &algebra_context(host))
            .map_err(|d| err(&d.code, &d.message))?;
        value.graph.edges.retain(|e| {
            e.predicate == request.predicate
                && e.polarity == Polarity::Positive
                && e.valid_time.contains(request.valid_at)
        });
        let selected: BTreeSet<_> = value.graph.edges.iter().map(|e| e.id.clone()).collect();
        value.edge_origins.retain(|id, _| selected.contains(id));
        Ok(value)
    }

    /// Returns source-linked leaf wrappers plus synthetic manifest, cluster, membership and
    /// existential aggregate-relation objects. Recompute under the current host on every call.
    pub fn cluster_navigation(
        &self,
        request: &ClusterRequest,
        host: &HostContext,
    ) -> Result<QueryResult> {
        let _snapshot = self.optional_read_transaction()?;
        let _clock_scope = self.operation_scope()?;
        let _read_scope = self.read_budget.enter();
        let mut value = self.cluster_input(request, host)?;
        // cluster_input checked this bound.
        let end = request.valid_at + 1;
        // Conservative whole-input influence: matching decisions and counts depend on all
        // selected facts, not only the local cluster's leaves. Reserve one pin for repersistence.
        let mut nodes = BTreeMap::new();
        for reference in value.node_origins.values().flatten() {
            nodes.insert(
                (&reference.graph_id, &reference.revision, &reference.node_id),
                reference,
            );
            if nodes.len() > 999 {
                return Err(err(
                    "E_CLUSTER_BUDGET",
                    "navigation influence budget exceeded",
                ));
            }
        }
        let mut claims = BTreeMap::new();
        for reference in value.edge_origins.values().flatten() {
            claims.insert(
                (
                    &reference.graph_id,
                    &reference.revision,
                    &reference.assertion_id,
                ),
                reference,
            );
            if nodes.len() + claims.len() > 999 {
                return Err(err(
                    "E_CLUSTER_BUDGET",
                    "navigation influence budget exceeded",
                ));
            }
        }
        let node_proofs: Vec<_> = nodes.into_values().cloned().collect();
        let claim_proofs: Vec<_> = claims.into_values().cloned().collect();
        require_proof_capacity(
            &claim_proofs,
            &node_proofs,
            value.graph.context_typing.as_ref(),
        )?;
        let snapshot = Snapshot {
            // Include source graph identity but not whole-revision hash in membership identity.
            perspective: format!(
                "perspective:{:x}",
                Sha256::digest(serde_json::to_vec(&(
                    &request.source.graph_id,
                    &request.predicate
                ))?)
            ),
            context: request.context.clone(),
            valid_at: request.valid_at,
            sources: value.input_snapshots.clone(),
            nodes: value.graph.nodes.iter().map(|n| n.id.clone()).collect(),
            links: value
                .graph
                .edges
                .iter()
                .map(|e| Link {
                    id: e.id.clone(),
                    from: e.from.clone(),
                    to: e.to.clone(),
                    evidence: value.edge_origins.get(&e.id).cloned().unwrap_or_default(),
                })
                .collect(),
            partial: true,
        };
        let mut hierarchy = Hierarchy::new(snapshot).map_err(failure)?;
        let mut records = BTreeMap::new();
        for _ in 0..request
            .levels
            .min(value.graph.nodes.len().saturating_add(1))
        {
            let manifest = hierarchy.advance().map_err(failure)?;
            for member in &manifest.frontier {
                if let Member::Cluster(id) = member {
                    records
                        .entry(id.clone())
                        .or_insert_with(|| hierarchy.cluster(id).unwrap().clone());
                }
            }
            if manifest.evidence_boundary {
                break;
            }
        }
        let manifest = hierarchy.manifest();
        let manifest_id = format!(
            "navigation:{:x}",
            Sha256::digest(serde_json::to_vec(&(
                &hierarchy.snapshot().perspective,
                &request.context,
                request.valid_at,
                &manifest.frontier,
            ))?)
        );
        let mut ids: BTreeSet<_> = value.graph.nodes.iter().map(|n| n.id.clone()).collect();
        let derived = |id: String, properties: BTreeMap<String, serde_json::Value>| Node {
            derived_snapshots: vec![],
            id: id.clone(),
            entity_id: id,
            space_id: "weave:navigation".into(),
            type_id: None,
            properties,
            metadata: vec![],
            readers: vec![host.principal.clone()],
            context_scope: Some(request.context.clone()),
            derived_nodes: node_proofs.clone(),
            derived_from: claim_proofs.clone(),
        };
        // The output envelope intentionally makes no global completeness assertion.
        let mut manifest_json = serde_json::to_value(&manifest)?;
        manifest_json["partial_input"] = json!(true);
        let mut output = GraphData::default();
        let mut bytes = json_size(&output, MATERIALIZED_LIMIT)?;
        for source in &value.graph.nodes {
            let entity = format!(
                "navigation-leaf:{:x}",
                Sha256::digest(serde_json::to_vec(&(&request.source.graph_id, &source.id))?)
            );
            let mut leaf = derived(
                source.id.clone(),
                BTreeMap::from([
                    ("kind".into(), json!("source_leaf")),
                    ("source_node".into(), json!(value.node_origins[&source.id])),
                    ("source_entity".into(), json!(source.entity_id)),
                    ("source_space".into(), json!(source.space_id)),
                ]),
            );
            leaf.entity_id = entity;
            bytes += json_size(&leaf, MATERIALIZED_LIMIT.saturating_sub(bytes))?;
            output.nodes.push(leaf);
        }
        let mut add_node = |node: Node, output: &mut GraphData| -> Result<()> {
            if !ids.insert(node.id.clone()) {
                return Err(err(
                    "E_CLUSTER_COLLISION",
                    "navigation and source node IDs overlap",
                ));
            }
            bytes += json_size(&node, MATERIALIZED_LIMIT.saturating_sub(bytes))?;
            output.nodes.push(node);
            Ok(())
        };
        add_node(
            derived(
                manifest_id.clone(),
                BTreeMap::from([
                    ("kind".into(), json!("navigation_manifest")),
                    ("manifest".into(), manifest_json),
                    ("predicate".into(), json!(request.predicate)),
                    ("valid_at".into(), json!(request.valid_at)),
                    ("source".into(), json!(request.source)),
                ]),
            ),
            &mut output,
        )?;
        for record in records.values() {
            add_node(
                derived(
                    record.id.clone(),
                    BTreeMap::from([
                        ("kind".into(), json!("cluster")),
                        ("record".into(), json!(record)),
                    ]),
                ),
                &mut output,
            )?;
        }
        let mut add_edge =
            |from: &str, to: &str, predicate: &str, evidence: Vec<String>| -> Result<()> {
                let id = format!(
                    "navigation-edge:{:x}",
                    Sha256::digest(serde_json::to_vec(&(from, to, predicate, &evidence))?)
                );
                let edge = Edge {
                    derived_snapshots: vec![],
                    derived_nodes: node_proofs.clone(),
                    id,
                    from: from.into(),
                    to: to.into(),
                    predicate: predicate.into(),
                    type_id: None,
                    valid_time: Interval {
                        start: request.valid_at,
                        end: Some(end),
                    },
                    polarity: Polarity::Positive,
                    properties: BTreeMap::new(),
                    assertion_properties: BTreeMap::from([
                        ("source_links".into(), json!(evidence)),
                        ("approximate_navigation".into(), json!(true)),
                    ]),
                    metadata: vec![],
                    readers: vec![host.principal.clone()],
                    derived_from: claim_proofs.clone(),
                    derivations: if claim_proofs.is_empty() {
                        vec![]
                    } else {
                        vec![Derivation {
                            node_premises: node_proofs.clone(),
                            operator: "weave:cluster-navigation:v1".into(),
                            premises: claim_proofs.clone(),
                            parameters: BTreeMap::from([
                                ("from".into(), json!(from)),
                                ("to".into(), json!(to)),
                                ("predicate".into(), json!(predicate)),
                                ("source_links".into(), json!(evidence)),
                                ("context".into(), json!(request.context)),
                                ("valid_at".into(), json!(request.valid_at)),
                            ]),
                            input_snapshots: value.input_snapshots.clone(),
                        }]
                    },
                    assertion_source: None,
                    assertion_context: request.context.reference().cloned(),
                    structural_ref: None,
                };
                bytes += json_size(&edge, MATERIALIZED_LIMIT.saturating_sub(bytes))?;
                output.edges.push(edge);
                Ok(())
            };
        for member in &manifest.frontier {
            add_edge(
                &manifest_id,
                member_id(member),
                "weave:cluster:frontier",
                vec![],
            )?;
        }
        for record in records.values() {
            for child in &record.children {
                add_edge(
                    &record.id,
                    member_id(child),
                    "weave:cluster:member",
                    record.contributing_links.clone(),
                )?;
            }
        }
        for link in hierarchy.aggregate_links().map_err(failure)? {
            add_edge(
                member_id(&link.from),
                member_id(&link.to),
                "weave:cluster:exists",
                link.contributing_links,
            )?;
        }
        value.node_origins = output
            .nodes
            .iter()
            .map(|n| (n.id.clone(), vec![]))
            .collect();
        value.edge_origins = output
            .edges
            .iter()
            .map(|e| (e.id.clone(), claim_proofs.clone()))
            .collect();
        value.provenance = claim_proofs;
        output.context_typing = value.graph.context_typing.clone();
        output.influence = weave_contract::influence::input_influence(&value.graph)
            .map_err(|d| err(&d.code, &d.message))?;
        value.graph = output;
        value.attachment_origins.clear();
        value.metadata_graphs.clear();
        // Fixed scoped coverage prevents an unavailable private dependency from changing the
        // navigation envelope relative to an absent private object. Exact pins remain observable.
        value.coverage = Coverage::Partial;
        value.diagnostics = vec![Diagnostic { code: "I_CLUSTER_SCOPED".into(), message: "Navigation covers authorized available evidence; exact queries must inspect source evidence".into() }];
        context_typing::protect_result_generated(&mut value)
            .map_err(|d| err(&d.code, &d.message))?;
        weave_contract::influence::protect_generated_result(&mut value, MATERIALIZED_LIMIT)
            .map_err(|d| err(&d.code, &d.message))?;
        require_repersistable(&value.graph)?;
        validate_graph(&value.graph)?;
        json_size(&value, MATERIALIZED_LIMIT)?;
        Ok(value)
    }
    /// Compare two currently authorized historical frontiers. Recomputed on each call;
    /// the returned summary node retains both inputs' proof gates when persisted.
    pub fn cluster_lineage(
        &self,
        before: &ClusterRequest,
        after: &ClusterRequest,
        host: &HostContext,
    ) -> Result<QueryResult> {
        let _read_scope = self.read_budget.enter();
        if before.source.graph_id != after.source.graph_id
            || before.predicate != after.predicate
            || before.context != after.context
        {
            return Err(err(
                "E_CLUSTER_SCOPE",
                "lineage requires the same source domain, predicate and context",
            ));
        }
        let transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let _clock_scope = self.operation_scope()?;
        let mut left = self.cluster_input(before, host)?;
        let mut right = self.cluster_input(after, host)?;
        let hierarchy = |request: &ClusterRequest, value: &QueryResult| -> Result<Hierarchy> {
            let mut h = Hierarchy::new(Snapshot {
                perspective: format!(
                    "perspective:{:x}",
                    Sha256::digest(serde_json::to_vec(&(
                        &request.source.graph_id,
                        &request.predicate
                    ))?)
                ),
                context: request.context.clone(),
                valid_at: request.valid_at,
                // Match navigation record revisions, including descriptor/proof pins.
                sources: value.input_snapshots.clone(),
                nodes: value.graph.nodes.iter().map(|n| n.id.clone()).collect(),
                links: value
                    .graph
                    .edges
                    .iter()
                    .map(|e| Link {
                        id: e.id.clone(),
                        from: e.from.clone(),
                        to: e.to.clone(),
                        evidence: value.edge_origins.get(&e.id).cloned().unwrap_or_default(),
                    })
                    .collect(),
                partial: true,
            })
            .map_err(failure)?;
            for _ in 0..request
                .levels
                .min(value.graph.nodes.len().saturating_add(1))
            {
                if h.advance().map_err(failure)?.evidence_boundary {
                    break;
                }
            }
            Ok(h)
        };
        let old = hierarchy(before, &left)?;
        let new = hierarchy(after, &right)?;
        // cluster_input reads every leaf from the exact requested source graph;
        // additional pins describe proof dependencies, not other leaf-ID domains.
        let lineage = old
            .lineage_to_in_domain(&new, &before.source.graph_id)
            .map_err(failure)?;
        let mut nodes = BTreeMap::new();
        let mut assertions = BTreeMap::new();
        for value in [&left, &right] {
            for r in value.node_origins.values().flatten() {
                nodes.insert((&r.graph_id, &r.revision, &r.node_id), r);
                if nodes.len() + assertions.len() > 999 {
                    return Err(err("E_CLUSTER_BUDGET", "lineage influence budget exceeded"));
                }
            }
            for r in value.edge_origins.values().flatten() {
                assertions.insert((&r.graph_id, &r.revision, &r.assertion_id), r);
                if nodes.len() + assertions.len() > 999 {
                    return Err(err("E_CLUSTER_BUDGET", "lineage influence budget exceeded"));
                }
            }
        }
        let derived_nodes: Vec<_> = nodes.into_values().cloned().collect();
        let derived_from: Vec<_> = assertions.into_values().cloned().collect();
        let typing = context_typing::merge(
            left.graph.context_typing.as_ref(),
            right.graph.context_typing.as_ref(),
        )
        .map_err(|d| err(&d.code, &d.message))?;
        require_proof_capacity(&derived_from, &derived_nodes, typing.as_ref())?;
        let proof_bytes = json_size(&(&derived_nodes, &derived_from), MATERIALIZED_LIMIT)?;
        json_size(
            &lineage,
            MATERIALIZED_LIMIT.saturating_sub(proof_bytes + 4096),
        )?;
        let id = format!(
            "lineage:{:x}",
            Sha256::digest(serde_json::to_vec(&lineage)?)
        );
        let node = Node {
            derived_snapshots: vec![],
            id: id.clone(),
            entity_id: id.clone(),
            space_id: "weave:navigation".into(),
            type_id: None,
            properties: BTreeMap::from([
                ("kind".into(), json!("cluster_lineage")),
                ("lineage".into(), serde_json::to_value(&lineage)?),
            ]),
            metadata: vec![],
            readers: vec![host.principal.clone()],
            context_scope: Some(before.context.clone()),
            derived_nodes,
            derived_from: derived_from.clone(),
        };
        // Merge only envelopes; same graph IDs at distinct revisions may contain
        // deliberately different source objects and must not be unioned as objects.
        for value in [&mut left, &mut right] {
            let typing = value.graph.context_typing.take();
            let influence = weave_contract::influence::input_influence(&value.graph)
                .map_err(|d| err(&d.code, &d.message))?;
            value.graph = GraphData {
                context_typing: typing,
                influence,
                ..GraphData::default()
            };
            value.node_origins.clear();
            value.edge_origins.clear();
            value.attachment_origins.clear();
            value.metadata_graphs.clear();
            value.provenance.clear();
        }
        let mut result = algebra::union(left, right, &algebra_context(host))
            .map_err(|d| err(&d.code, &d.message))?;
        result.graph.nodes.push(node);
        result.node_origins.insert(id, vec![]);
        result.provenance = derived_from;
        result.coverage = Coverage::Partial;
        result.diagnostics = vec![Diagnostic {
            code: "I_CLUSTER_SCOPED".into(),
            message: "Lineage compares currently authorized evidence at both pins; overlaps are navigation, not accepted identity".into(),
        }];
        context_typing::protect_result_generated(&mut result)
            .map_err(|d| err(&d.code, &d.message))?;
        weave_contract::influence::protect_generated_result(&mut result, MATERIALIZED_LIMIT)
            .map_err(|d| err(&d.code, &d.message))?;
        require_repersistable(&result.graph)?;
        validate_graph(&result.graph)?;
        json_size(&result, MATERIALIZED_LIMIT)?;
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        Ok(result)
    }
}
