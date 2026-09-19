//! Deterministic identities and principal-scoped explanation graphs.
//! These hashes identify computation inputs; they never confer access or trust.
use crate::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{self, Write};

const MAX_IDENTITY_BYTES: usize = 16 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRevision {
    pub name: String,
    pub revision: String,
    pub digest: String,
}
/// Installed by the admission host. A plan cannot mint a policy context.
#[derive(Debug, Clone)]
pub struct ViewContext {
    pub principal: String,
    pub policy_revision: String,
    pub identity_maps: Vec<GraphRef>,
    pub artifacts: Vec<String>,
}
struct HashWriter {
    hash: Sha256,
    remaining: usize,
}
impl Write for HashWriter {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.remaining = self
            .remaining
            .checked_sub(b.len())
            .ok_or_else(|| io::Error::other("identity byte budget"))?;
        self.hash.update(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn failure(code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: message.into(),
    }
}
fn digest(
    domain: &str,
    value: &(impl Serialize + ?Sized),
    limit: usize,
) -> Result<String, Diagnostic> {
    let mut writer = HashWriter {
        hash: Sha256::new(),
        remaining: limit,
    };
    serde_json::to_writer(&mut writer, &(domain, value)).map_err(|_| {
        failure(
            "E_IDENTITY_LIMIT",
            "Identity input exceeds its serialized byte budget",
        )
    })?;
    Ok(format!("sha256:{:x}", writer.hash.finalize()))
}
fn ordered<T: Clone + Serialize>(values: &[T]) -> Result<Vec<T>, Diagnostic> {
    // Bound before allocating canonical membership keys.
    digest("bounded-set", values, MAX_IDENTITY_BYTES)?;
    let mut set = BTreeMap::new();
    for v in values {
        set.insert(
            serde_json::to_vec(v)
                .map_err(|_| failure("E_IDENTITY_ENCODING", "Cannot encode identity input"))?,
            v.clone(),
        );
    }
    Ok(set.into_values().collect())
}
/// Structural canonical IR identity, not a claim of algebraic equivalence.
/// Operator order is retained; schema/source collections are canonical sets.
pub fn plan_fingerprint(
    expression: &GraphExpression,
    schemas: &[GraphSchema],
    sources: &[SourceRevision],
) -> Result<String, Diagnostic> {
    digest(
        "weave-plan-v1",
        &(VERSION, expression, ordered(schemas)?, ordered(sources)?),
        MAX_IDENTITY_BYTES,
    )
}
/// Ordered commands retain effect order. Source revision labels are descriptive,
/// and cannot authenticate the author or grant execution authority.
pub fn program_fingerprint(
    program: &Program,
    schemas: &[GraphSchema],
) -> Result<String, Diagnostic> {
    let mut program = program.clone();
    program.source_revisions = ordered(&program.source_revisions)?;
    digest(
        "weave-program-v1",
        &(
            VERSION,
            &program,
            ordered(schemas)?,
            ordered(&program.source_revisions)?,
        ),
        MAX_IDENTITY_BYTES,
    )
}
pub fn result_fingerprint(
    plan_id: &str,
    result: &QueryResult,
    context: &ViewContext,
) -> Result<String, Diagnostic> {
    if plan_id.is_empty() || context.principal.is_empty() || context.policy_revision.is_empty() {
        return Err(failure(
            "E_VIEW_CONTEXT",
            "Result identity requires plan, principal and host policy revision",
        ));
    }
    digest("bounded-result", result, MAX_IDENTITY_BYTES)?;
    let mut normalized = result.clone();
    normalized.input_snapshots = ordered(&result.input_snapshots)?;
    normalized.provenance = ordered(&result.provenance)?;
    normalized.source_revisions = ordered(&result.source_revisions)?;
    normalized.metadata_graphs = ordered(&result.metadata_graphs)?;
    for origins in normalized.edge_origins.values_mut() {
        *origins = ordered(origins)?;
    }
    for origins in normalized.node_origins.values_mut() {
        *origins = ordered(origins)?;
    }
    for origins in normalized.attachment_origins.values_mut() {
        *origins = ordered(origins)?;
    }
    // Hash the full result envelope, including origins, coverage and diagnostics.
    // Display array order is retained; this is not minimal algebraic equivalence.
    digest(
        "weave-result-v1",
        &(
            VERSION,
            plan_id,
            &context.principal,
            &context.policy_revision,
            ordered(&context.identity_maps)?,
            ordered(&context.artifacts)?,
            &normalized,
        ),
        MAX_IDENTITY_BYTES,
    )
}
/// Digest an already normalized source unit. The caller must use its AST types
/// to remove locations; arbitrary user JSON keys are always semantic here.
pub fn source_fingerprint(source: &impl Serialize) -> Result<String, Diagnostic> {
    digest("weave-source-v1", source, MAX_IDENTITY_BYTES)
}
fn size(value: &(impl Serialize + ?Sized), limit: usize) -> Result<usize, Diagnostic> {
    let mut w = HashWriter {
        hash: Sha256::new(),
        remaining: limit,
    };
    serde_json::to_writer(&mut w, value)
        .map_err(|_| failure("E_EXPLAIN_LIMIT", "Explanation exceeds byte budget"))?;
    Ok(limit - w.remaining)
}
/// Explain only an already authorized result. No reads, hidden premise lookup or
/// policy grants occur. Every alternative remains its own AND group in the graph.
pub fn explain(input: &QueryResult, ctx: &AlgebraContext) -> Result<QueryResult, Diagnostic> {
    if ctx.principal.is_empty() {
        return Err(failure(
            "E_EXPLAIN_CONTEXT",
            "Explanation requires a trusted principal",
        ));
    }
    size(input, ctx.max_output_bytes)?;
    let mut remaining = ctx.max_output_bytes;
    let mut objects = 0usize;
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    let mut edges = Vec::new();
    let mut edge_origins = BTreeMap::new();
    let mut add_node = |id: String,
                        kind: &str,
                        scope: &crate::ContextSelection,
                        dependencies: &[AssertionRef]|
     -> Result<(), Diagnostic> {
        if let std::collections::btree_map::Entry::Vacant(entry) = nodes.entry(id.clone()) {
            size(&(kind, scope, dependencies), remaining)?;
            let n = Node {
                derived_from: ordered(dependencies)?,
                context_scope: Some(scope.clone()),
                id: id.clone(),
                entity_id: id,
                space_id: "weave:explanation".into(),
                type_id: Some("Part".into()),
                properties: [("kind".into(), json!(kind))].into(),
                metadata: vec![],
                readers: vec![ctx.principal.clone()],
            };
            remaining = remaining
                .checked_sub(size(&n, remaining)?)
                .ok_or_else(|| failure("E_EXPLAIN_LIMIT", "Explanation byte budget exceeded"))?;
            objects += 1;
            if objects > ctx.max_objects {
                return Err(failure(
                    "E_EXPLAIN_LIMIT",
                    "Explanation object budget exceeded",
                ));
            }
            entry.insert(n);
        }
        Ok(())
    };
    // Collect bounded node/edge records separately so every emitted edge is gated
    // by exactly the alternative it explains, not a flattened union of alternatives.
    let mut records = Vec::new();
    let mut record_bytes = ctx.max_output_bytes;
    let mut add_record = |link: (
        String,
        String,
        &'static str,
        String,
        crate::ContextSelection,
    ),
                          group: &Derivation|
     -> Result<(), Diagnostic> {
        if records.len() >= ctx.max_objects {
            return Err(failure(
                "E_EXPLAIN_LIMIT",
                "Explanation object budget exceeded",
            ));
        }
        let bytes = size(&(&link, group), record_bytes)?;
        record_bytes = record_bytes.checked_sub(bytes).ok_or_else(|| {
            failure(
                "E_EXPLAIN_LIMIT",
                "Explanation construction exceeds byte budget",
            )
        })?;
        records.push((link, group.clone()));
        Ok(())
    };
    for edge in &input.graph.edges {
        let scope = match &edge.assertion_context {
            Some(reference) => crate::ContextSelection::Pinned {
                reference: reference.clone(),
            },
            None => crate::ContextSelection::Default,
        };
        if input
            .selected_context
            .as_ref()
            .is_some_and(|selected| selected != &scope)
        {
            return Err(failure(
                "E_CONTEXT_SCOPE",
                "Explanation input contradicts its selected context",
            ));
        }
        let groups = if edge.derivations.is_empty() {
            vec![Derivation {
                operator: "weave:source".into(),
                premises: input
                    .edge_origins
                    .get(&edge.id)
                    .cloned()
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| {
                        failure(
                            "E_ORIGIN_MISSING",
                            "Explanation premise lacks pinned provenance",
                        )
                    })?,
                parameters: [("attribution".into(), json!({"structural_ref":edge.structural_ref,"source":edge.assertion_source,"context":edge.assertion_context,"assertion_properties":edge.assertion_properties}))].into(),
                input_snapshots: vec![],
            }]
        } else {
            edge.derivations.clone()
        };
        let conclusion = format!(
            "conclusion:{}",
            digest(
                "conclusion",
                &(&edge.id, &edge.structural_ref, &scope),
                ctx.max_output_bytes
            )?
        );
        let conclusion_dependencies = ordered(
            &groups
                .iter()
                .flat_map(|group| group.premises.iter().cloned())
                .collect::<Vec<_>>(),
        )?;
        add_node(
            conclusion.clone(),
            "conclusion",
            &scope,
            &conclusion_dependencies,
        )?;
        for group in groups {
            let group_id = format!(
                "derivation:{}",
                digest(
                    "derivation",
                    &(&edge.id, &group, &scope),
                    ctx.max_output_bytes
                )?
            );
            add_node(group_id.clone(), "derivation", &scope, &group.premises)?;
            add_record(
                (
                    conclusion.clone(),
                    group_id.clone(),
                    "alternative",
                    serde_json::to_string(&group)
                        .map_err(|_| failure("E_IDENTITY_ENCODING", "Cannot encode derivation"))?,
                    scope.clone(),
                ),
                &group,
            )?;
            for premise in &group.premises {
                let id = format!(
                    "premise:{}",
                    digest(
                        "premise",
                        &(premise, &scope, &group_id),
                        ctx.max_output_bytes
                    )?
                );
                add_node(id.clone(), "premise", &scope, &group.premises)?;
                add_record(
                    (
                        group_id.clone(),
                        id,
                        "joint_premise",
                        serde_json::to_string(premise)
                            .map_err(|_| failure("E_IDENTITY_ENCODING", "Cannot encode premise"))?,
                        scope.clone(),
                    ),
                    &group,
                )?;
            }
        }
    }
    #[allow(clippy::drop_non_drop)]
    drop(add_node);
    for ((from, to, label, payload, scope), group) in records {
        let id = format!(
            "explanation:{}",
            digest(
                "explanation-edge",
                &(&from, &to, label, &payload),
                ctx.max_output_bytes
            )?
        );
        let premises = group.premises.clone();
        let snapshots = ordered(
            &premises
                .iter()
                .map(|p| GraphRef {
                    graph_id: p.graph_id.clone(),
                    revision: p.revision.clone(),
                })
                .collect::<Vec<_>>(),
        )?;
        let record = Edge {
            id: id.clone(),
            structural_ref: None,
            assertion_properties: BTreeMap::new(),
            assertion_source: None,
            assertion_context: scope.reference().cloned(),
            type_id: Some("Evidence".into()),
            predicate: format!("weave:explanation:{label}"),
            from,
            to,
            valid_time: Interval {
                start: i64::MIN,
                end: None,
            },
            polarity: Polarity::Positive,
            properties: [("payload".into(), json!(payload))].into(),
            metadata: vec![],
            readers: vec![ctx.principal.clone()],
            derived_from: premises.clone(),
            derivations: vec![Derivation {
                operator: "weave:explain".into(),
                premises: premises.clone(),
                parameters: [("explained".into(), json!(group))].into(),
                input_snapshots: snapshots,
            }],
        };
        remaining = remaining
            .checked_sub(size(&record, remaining)?)
            .ok_or_else(|| failure("E_EXPLAIN_LIMIT", "Explanation byte budget exceeded"))?;
        objects += 1;
        if objects > ctx.max_objects {
            return Err(failure(
                "E_EXPLAIN_LIMIT",
                "Explanation object budget exceeded",
            ));
        }
        edge_origins.insert(id, premises);
        edges.push(record);
    }
    let schema = GraphSchema {
        id: "weave:explanation".into(),
        revision: "1".into(),
        nodes: [(
            "Part".into(),
            NodeSchema {
                properties: [(
                    "kind".into(),
                    PropertySchema {
                        value_type: ScalarType::String,
                        required: true,
                        nullable: false,
                    },
                )]
                .into(),
                space_id: Some("weave:explanation".into()),
                allow_extra_properties: false,
            },
        )]
        .into(),
        edges: [(
            "Evidence".into(),
            EdgeSchema {
                from_type: "Part".into(),
                to_type: "Part".into(),
                properties: [(
                    "payload".into(),
                    PropertySchema {
                        value_type: ScalarType::String,
                        required: true,
                        nullable: false,
                    },
                )]
                .into(),
                allow_cross_space: false,
                allow_extra_properties: false,
            },
        )]
        .into(),
    };
    let provenance = ordered(&edge_origins.values().flatten().cloned().collect::<Vec<_>>())?;
    let input_snapshots = ordered(
        &provenance
            .iter()
            .map(|p| GraphRef {
                graph_id: p.graph_id.clone(),
                revision: p.revision.clone(),
            })
            .collect::<Vec<_>>(),
    )?;
    let node_origins = nodes.keys().map(|id| (id.clone(), Vec::new())).collect();
    let result = QueryResult {
        selected_context: input.selected_context.clone(),
        source_revisions: input.source_revisions.clone(),
        version: VERSION.into(),
        graph: GraphData {
            schema: Some(schema),
            nodes: nodes.into_values().collect(),
            edges,
            ..GraphData::default()
        },
        snapshots: input.snapshots.clone(),
        input_snapshots,
        coverage: input.coverage.clone(),
        diagnostics: input.diagnostics.clone(),
        provenance,
        edge_origins,
        node_origins,
        attachment_origins: BTreeMap::new(),
        metadata_graphs: vec![],
    };
    size(&result, ctx.max_output_bytes)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan() -> GraphExpression {
        GraphExpression::Query {
            query: serde_json::from_value(json!({"graph_id":"G"})).unwrap(),
        }
    }
    fn value() -> QueryResult {
        serde_json::from_value(json!({"version":VERSION,"graph":{"nodes":[{"id":"n","entity_id":"n","space_id":"s"}],"edges":[{"id":"derived","predicate":"p","from":"n","to":"n","valid_time":{"start":0,"end":10},"derivations":[{"operator":"rule-v1","premises":[{"graph_id":"A","revision":"a1","assertion_id":"x"}],"parameters":{"threshold":3}},{"operator":"rule-v1","premises":[{"graph_id":"B","revision":"b1","assertion_id":"y"}],"parameters":{"threshold":4}}]}]},"snapshots":{"G":"r1"},"input_snapshots":[{"graph_id":"G","revision":"r1"}],"coverage":"complete","diagnostics":[],"provenance":[],"metadata_graphs":[]})).unwrap()
    }
    fn host() -> ViewContext {
        ViewContext {
            principal: "reader".into(),
            policy_revision: "local-readers-v1".into(),
            identity_maps: vec![],
            artifacts: vec![],
        }
    }
    #[test]
    fn semantic_literal_fields_and_source_revisions_change_identity() {
        assert_ne!(
            source_fingerprint(&json!({"properties":{"span":1}})).unwrap(),
            source_fingerprint(&json!({"properties":{"span":2}})).unwrap()
        );
        let source = SourceRevision {
            name: "F".into(),
            revision: "1".into(),
            digest: "source-digest".into(),
        };
        let mut changed = source.clone();
        changed.revision = "2".into();
        assert_ne!(
            plan_fingerprint(&plan(), &[], &[source]).unwrap(),
            plan_fingerprint(&plan(), &[], &[changed]).unwrap()
        );
    }
    #[test]
    fn exact_dependencies_and_trusted_context_change_result_identity() {
        let p = plan_fingerprint(&plan(), &[], &[]).unwrap();
        let r = value();
        let context = host();
        let original = result_fingerprint(&p, &r, &context).unwrap();
        let mut changed = context.clone();
        changed.principal = "other".into();
        assert_ne!(original, result_fingerprint(&p, &r, &changed).unwrap());
        changed = context.clone();
        changed.policy_revision = "policy2".into();
        assert_ne!(original, result_fingerprint(&p, &r, &changed).unwrap());
        changed = context.clone();
        changed.identity_maps.push(GraphRef {
            graph_id: "map".into(),
            revision: "2".into(),
        });
        assert_ne!(original, result_fingerprint(&p, &r, &changed).unwrap());
        changed = context.clone();
        changed.artifacts.push("model-v2".into());
        assert_ne!(original, result_fingerprint(&p, &r, &changed).unwrap());
        let mut updated = r.clone();
        updated.input_snapshots[0].revision = "r2".into();
        assert_ne!(
            original,
            result_fingerprint(&p, &updated, &context).unwrap()
        );
        updated = r.clone();
        updated.coverage = Coverage::Partial;
        assert_ne!(
            original,
            result_fingerprint(&p, &updated, &context).unwrap()
        );
        updated = r.clone();
        updated.edge_origins.insert(
            "derived".into(),
            vec![AssertionRef {
                graph_id: "extra".into(),
                revision: "1".into(),
                assertion_id: "a".into(),
            }],
        );
        assert_ne!(
            original,
            result_fingerprint(&p, &updated, &context).unwrap()
        );
        updated = r.clone();
        updated.node_origins.insert(
            "n".into(),
            vec![NodeRef {
                graph_id: "extra".into(),
                revision: "1".into(),
                node_id: "n".into(),
            }],
        );
        assert_ne!(
            original,
            result_fingerprint(&p, &updated, &context).unwrap()
        );
        assert_eq!(original, result_fingerprint(&p, &r, &context).unwrap());
    }
    #[test]
    fn command_effect_order_is_not_normalized_away() {
        let a = Command::Bind {
            name: "A".into(),
            value: plan(),
        };
        let b = Command::Bind {
            name: "B".into(),
            value: plan(),
        };
        let p = Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![a.clone(), b.clone()],
        };
        let q = Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![b, a],
        };
        assert_ne!(
            program_fingerprint(&p, &[]).unwrap(),
            program_fingerprint(&q, &[]).unwrap()
        );
    }
    #[test]
    fn explanation_is_a_typed_graph_with_isolated_alternatives() {
        let ctx = AlgebraContext {
            principal: "reader".into(),
            max_objects: 100,
            max_output_bytes: 100_000,
        };
        let explanation = explain(&value(), &ctx).unwrap();
        assert!(validate_schema_graph(&explanation.graph).is_empty());
        assert_eq!(
            explanation
                .graph
                .edges
                .iter()
                .filter(|e| e.predicate == "weave:explanation:alternative")
                .count(),
            2
        );
        for edge in &explanation.graph.edges {
            assert_eq!(edge.readers, ["reader"]);
            assert_eq!(edge.derivations.len(), 1);
            assert_eq!(edge.derivations[0].premises.len(), 1);
            let text = serde_json::to_string(edge).unwrap();
            assert!(
                !(text.contains("\"revision\":\"a1\"") && text.contains("\"revision\":\"b1\""))
            );
        }
        assert!(explanation
            .graph
            .nodes
            .iter()
            .all(|n| n.readers == ["reader"]));
        assert_eq!(explanation.provenance.len(), 2);
        let combined =
            crate::algebra::union(explanation.clone(), explanation.clone(), &ctx).unwrap();
        assert_eq!(combined.graph.edges.len(), explanation.graph.edges.len());
        let mut small = ctx;
        small.max_objects = 2;
        assert_eq!(
            explain(&value(), &small).unwrap_err().code,
            "E_EXPLAIN_LIMIT"
        );
    }
}
