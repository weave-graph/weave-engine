//! Explicit declared identity-bridge selection, not an accepted identity resolver.
//! Input is an already authorized graph value; no catalog lookup or implicit mapping.
use crate::*;
use std::collections::BTreeMap;
fn error(code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: message.into(),
    }
}
pub fn validate_selection(selection: &CounterpartSelection) -> Result<(), Diagnostic> {
    if [
        &selection.predicate,
        &selection.entity_id,
        &selection.from_space_id,
        &selection.to_space_id,
    ]
    .iter()
    .any(|s| s.is_empty() || s.len() > 512 || s.chars().any(char::is_control))
    {
        return Err(error(
            "E_IDENTITY_SELECTION",
            "Bridge selection requires bounded explicit identifiers",
        ));
    }
    if selection.from_space_id == selection.to_space_id {
        return Err(error(
            "E_SPACE",
            "Counterpart selection requires distinct source and target spaces",
        ));
    }
    Ok(())
}
pub fn select(
    input: QueryResult,
    selection: &CounterpartSelection,
    ctx: &AlgebraContext,
) -> Result<QueryResult, Diagnostic> {
    crate::algebra::preflight(&input, ctx)?;
    validate_selection(selection)?;
    let nodes: BTreeMap<_, _> = input.graph.nodes.iter().map(|n| (&n.id, n)).collect();
    let mut selected = Vec::new();
    for edge in &input.graph.edges {
        if edge.predicate != selection.predicate
            || edge.polarity != Polarity::Positive
            || !edge.valid_time.contains(selection.valid_at)
        {
            continue;
        }
        let (Some(from), Some(to)) = (nodes.get(&edge.from), nodes.get(&edge.to)) else {
            return Err(error(
                "E_IDENTITY_ENDPOINT",
                "Bridge endpoint is unavailable",
            ));
        };
        if from.entity_id != selection.entity_id
            || from.space_id != selection.from_space_id
            || to.space_id != selection.to_space_id
        {
            continue;
        }
        if to.entity_id != selection.entity_id {
            return Err(error(
                "E_IDENTITY",
                "Declared counterpart endpoints have different entity identities",
            ));
        }
        crate::context::ensure_consumable(
            input.selected_context.as_ref(),
            edge.assertion_context.as_ref(),
        )?;
        for endpoint in [from, to] {
            if let Some(scope) = &endpoint.context_scope {
                crate::context::compatible_context(input.selected_context.as_ref(), Some(scope))?;
            }
        }
        selected.push(edge.id.clone());
    }
    // Preserve the original assertion, schema, endpoint manifestations and proof groups.
    // Selection is not a new mapping assertion and never copies represented state.
    crate::algebra::project(input, &[], &selected, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn ctx() -> AlgebraContext {
        AlgebraContext {
            principal: "reader".into(),
            max_objects: 100,
            max_output_bytes: 1_000_000,
        }
    }
    fn selection() -> CounterpartSelection {
        CounterpartSelection {
            predicate: "counterpart".into(),
            entity_id: "device".into(),
            from_space_id: "physical".into(),
            to_space_id: "operations".into(),
            valid_at: 5,
        }
    }
    fn value() -> QueryResult {
        let mut q:QueryResult=serde_json::from_value(json!({"version":VERSION,"graph":{"nodes":[{"id":"p","entity_id":"device","space_id":"physical"},{"id":"o","entity_id":"device","space_id":"operations"}],"edges":[{"id":"bridge","predicate":"counterpart","from":"p","to":"o","valid_time":{"start":0,"end":10},"assertion_source":"mapping-proposal"}]},"snapshots":{"Mappings":"r1"},"input_snapshots":[{"graph_id":"Mappings","revision":"r1"}],"coverage":"complete","diagnostics":[],"provenance":[],"metadata_graphs":[]})).unwrap();
        q.edge_origins.insert(
            "bridge".into(),
            vec![AssertionRef {
                graph_id: "Mappings".into(),
                revision: "r1".into(),
                assertion_id: "bridge".into(),
            }],
        );
        q
    }
    #[test]
    fn explicit_directed_bridge_preserves_manifestations_and_assertion_identity() {
        let input = value();
        let output = select(input.clone(), &selection(), &ctx()).unwrap();
        assert_eq!(
            output.graph.edges,
            input
                .graph
                .edges
                .iter()
                .cloned()
                .map(|mut e| {
                    e.readers = vec!["reader".into()];
                    e
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(output.graph.nodes.len(), 2);
        assert_eq!(output.edge_origins, input.edge_origins);
        let mut reverse = selection();
        std::mem::swap(&mut reverse.from_space_id, &mut reverse.to_space_id);
        assert!(select(value(), &reverse, &ctx())
            .unwrap()
            .graph
            .edges
            .is_empty());
    }
    #[test]
    fn no_similarity_resolution_negative_absence_or_implicit_context() {
        let mut q = value();
        q.graph.nodes[1].entity_id = "different".into();
        assert_eq!(
            select(q, &selection(), &ctx()).unwrap_err().code,
            "E_IDENTITY"
        );
        let mut q = value();
        q.graph.edges[0].polarity = Polarity::Negative;
        assert!(select(q, &selection(), &ctx())
            .unwrap()
            .graph
            .edges
            .is_empty());
        let mut at = selection();
        at.valid_at = 10;
        assert!(select(value(), &at, &ctx()).unwrap().graph.edges.is_empty());
        let mut q = value();
        let pin = GraphRef {
            graph_id: "World".into(),
            revision: "r".into(),
        };
        q.graph.edges[0].assertion_context = Some(pin.clone());
        assert_eq!(
            select(q.clone(), &selection(), &ctx()).unwrap_err().code,
            "E_CONTEXT_REQUIRED"
        );
        q.selected_context = Some(ContextSelection::Pinned { reference: pin });
        assert_eq!(
            select(q.clone(), &selection(), &ctx())
                .unwrap()
                .selected_context,
            q.selected_context
        );
        let mut bad = selection();
        bad.to_space_id = bad.from_space_id.clone();
        assert_eq!(select(value(), &bad, &ctx()).unwrap_err().code, "E_SPACE");
    }
    #[test]
    fn typed_partial_bridge_value_preserves_schema_and_dependency_records() {
        let mut q = value();
        q.coverage = Coverage::Partial;
        q.graph.schema=Some(serde_json::from_value(json!({"id":"S","revision":"1","nodes":{"Manifestation":{}},"edges":{"Bridge":{"from_type":"Manifestation","to_type":"Manifestation","allow_cross_space":true}}})).unwrap());
        for n in &mut q.graph.nodes {
            n.type_id = Some("Manifestation".into());
        }
        q.graph.edges[0].type_id = Some("Bridge".into());
        q.graph.nodes[0].derived_from = q.edge_origins["bridge"].clone();
        let out = select(q.clone(), &selection(), &ctx()).unwrap();
        assert_eq!(out.coverage, Coverage::Partial);
        assert_eq!(out.graph.schema, q.graph.schema);
        assert_eq!(
            out.graph.nodes[0].derived_from,
            q.graph.nodes[0].derived_from
        );
        assert!(validate_schema_graph(&out.graph).is_empty());
    }
}
