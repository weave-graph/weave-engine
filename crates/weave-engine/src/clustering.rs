//! Authorized, snapshot-scoped graph navigation. No approximate query pruning.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use weave_cluster::{Hierarchy, Link, Member, Snapshot};

/// Host API; source is an exact immutable pin, never a caller-supplied authorized snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClusterRequest {
    pub source: GraphRef,
    pub context: ContextSelection,
    pub valid_at: i64,
    pub predicate: String,
    /// Number of lazy levels to materialize, bounded by the input node count.
    pub levels: usize,
}
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
impl Engine {
    /// Returns source-linked leaf wrappers plus synthetic manifest, cluster, membership and
    /// existential aggregate-relation objects. Recompute under the current host on every call.
    pub fn cluster_navigation(
        &self,
        request: &ClusterRequest,
        host: &HostContext,
    ) -> Result<QueryResult> {
        let _read_scope = self.read_budget.enter();
        if !valid_id(&request.predicate) || request.levels > 10_000 {
            return Err(err(
                "E_CLUSTER_INPUT",
                "bounded predicate and level count required",
            ));
        }
        let end = request.valid_at.checked_add(1).ok_or_else(|| {
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
        value.graph = output;
        value.attachment_origins.clear();
        value.metadata_graphs.clear();
        // Fixed scoped coverage prevents an unavailable private dependency from changing the
        // navigation envelope relative to an absent private object. Exact pins remain observable.
        value.coverage = Coverage::Partial;
        value.diagnostics = vec![Diagnostic { code: "I_CLUSTER_SCOPED".into(), message: "Navigation covers authorized available evidence; exact queries must inspect source evidence".into() }];
        validate_graph(&value.graph)?;
        json_size(&value, MATERIALIZED_LIMIT)?;
        Ok(value)
    }
}
