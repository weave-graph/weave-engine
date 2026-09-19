//! Pure selection of already authorized graph claims. No context graph is read,
//! no permission is minted, and no claim is relabeled or broadcast.
use crate::*;
fn error(code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: message.into(),
    }
}
pub fn validate_selection(selection: &ContextSelection) -> Result<(), Diagnostic> {
    if let Some(reference) = selection.reference() {
        if [&reference.graph_id, &reference.revision]
            .iter()
            .any(|s| s.is_empty() || s.len() > 512 || s.chars().any(char::is_control))
        {
            return Err(error(
                "E_CONTEXT_REFERENCE",
                "Context selection requires a bounded exact graph/revision reference",
            ));
        }
    }
    Ok(())
}
/// None is legacy default only for context-free premises, never a wildcard.
pub fn ensure_consumable(
    selection: Option<&ContextSelection>,
    claim_context: Option<&GraphRef>,
) -> Result<(), Diagnostic> {
    match selection {
        None if claim_context.is_some() => Err(error(
            "E_CONTEXT_REQUIRED",
            "Contextual evidence requires an explicit exact selection",
        )),
        Some(scope) if scope.reference() != claim_context => Err(error(
            "E_CONTEXT_SCOPE",
            "Claim is outside the selected context",
        )),
        _ => Ok(()),
    }
}
/// A join cannot silently broadcast a default input into a pinned world.
pub fn compatible_context(
    left: Option<&ContextSelection>,
    right: Option<&ContextSelection>,
) -> Result<Option<ContextSelection>, Diagnostic> {
    let default = ContextSelection::Default;
    if left.unwrap_or(&default) != right.unwrap_or(&default) {
        return Err(error(
            "E_CONTEXT_INCOMPATIBLE",
            "Composition requires the same exact context on both inputs",
        ));
    }
    Ok(left.or(right).cloned())
}
pub fn select(
    input: QueryResult,
    selection: &ContextSelection,
    ctx: &AlgebraContext,
) -> Result<QueryResult, Diagnostic> {
    crate::algebra::preflight(&input, ctx)?;
    validate_selection(selection)?;
    if input
        .selected_context
        .as_ref()
        .is_some_and(|scope| scope != selection)
    {
        return Err(error(
            "E_CONTEXT_SCOPE",
            "Reselect from the original unscoped value; prior selection discarded other contexts",
        ));
    }
    let ids: Vec<_> = input
        .graph
        .edges
        .iter()
        .filter(|edge| edge.assertion_context.as_ref() == selection.reference())
        .map(|edge| edge.id.clone())
        .collect();
    let node_ids: Vec<_> = input
        .graph
        .nodes
        .iter()
        .filter(|node| {
            node.context_scope
                .as_ref()
                .is_none_or(|scope| scope == selection)
        })
        .map(|node| node.id.clone())
        .collect();
    let selected_edges: std::collections::BTreeSet<_> = ids.iter().collect();
    let selected_nodes: std::collections::BTreeSet<_> = node_ids.iter().collect();
    if input.graph.edges.iter().any(|edge| {
        selected_edges.contains(&edge.id)
            && (!selected_nodes.contains(&edge.from) || !selected_nodes.contains(&edge.to))
    }) {
        return Err(error(
            "E_CONTEXT_SCOPE",
            "Selected claim endpoints belong to an incompatible context",
        ));
    }
    let mut out = crate::algebra::project(input, &node_ids, &ids, ctx)?;
    out.graph
        .attachments
        .retain(|attachment| attachment.context.as_ref() == selection.reference());
    let retained: std::collections::BTreeSet<_> =
        out.graph.attachments.iter().map(|a| a.id.clone()).collect();
    out.attachment_origins.retain(|id, _| retained.contains(id));
    out.selected_context = Some(selection.clone());
    crate::algebra::preflight(&out, ctx)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn pin(revision: &str) -> ContextSelection {
        ContextSelection::Pinned {
            reference: GraphRef {
                graph_id: "World".into(),
                revision: revision.into(),
            },
        }
    }
    fn ctx() -> AlgebraContext {
        AlgebraContext {
            principal: "reader".into(),
            max_objects: 1000,
            max_output_bytes: 1_000_000,
        }
    }
    fn value() -> QueryResult {
        let mut value:QueryResult=serde_json::from_value(json!({"version":VERSION,"graph":{"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"default","predicate":"p","from":"a","to":"b","valid_time":{"start":0,"end":10}},{"id":"world","predicate":"p","from":"a","to":"b","assertion_context":{"graph_id":"World","revision":"1"},"valid_time":{"start":0,"end":10},"polarity":"negative"}],"attachments":[{"id":"m","host":{"kind":"edge","id":"world"},"key":"evidence","value":{"kind":"literal","value":"scoped"},"context":{"graph_id":"World","revision":"1"},"valid_time":{"start":0,"end":10}},{"id":"d","host":{"kind":"edge","id":"world"},"key":"other","value":{"kind":"literal","value":"default"},"valid_time":{"start":0,"end":10}}]},"snapshots":{"G":"r"},"input_snapshots":[{"graph_id":"G","revision":"r"}],"coverage":"complete","diagnostics":[],"provenance":[],"metadata_graphs":[]})).unwrap();
        for edge in &value.graph.edges {
            value.edge_origins.insert(
                edge.id.clone(),
                vec![AssertionRef {
                    graph_id: "G".into(),
                    revision: "r".into(),
                    assertion_id: edge.id.clone(),
                }],
            );
        }
        for node in &value.graph.nodes {
            value.node_origins.insert(
                node.id.clone(),
                vec![NodeRef {
                    graph_id: "G".into(),
                    revision: "r".into(),
                    node_id: node.id.clone(),
                }],
            );
        }
        value
    }
    #[test]
    fn exact_scope_filters_claims_and_metadata_without_broadcast() {
        let result = select(value(), &pin("1"), &ctx()).unwrap();
        assert_eq!(result.graph.edges.len(), 1);
        assert_eq!(result.graph.edges[0].id, "world");
        assert_eq!(
            result.graph.edges[0].assertion_context,
            pin("1").reference().cloned()
        );
        assert_eq!(result.graph.attachments.len(), 1);
        assert_eq!(result.graph.attachments[0].id, "m");
        let default = select(value(), &ContextSelection::Default, &ctx()).unwrap();
        assert_eq!(default.graph.edges[0].id, "default");
        assert!(default.graph.attachments.is_empty());
        let empty = select(value(), &pin("2"), &ctx()).unwrap();
        assert!(empty.graph.edges.is_empty());
        assert_eq!(empty.selected_context, Some(pin("2")));
        assert_eq!(
            select(empty, &pin("1"), &ctx()).unwrap_err().code,
            "E_CONTEXT_SCOPE"
        );
    }
    #[test]
    fn support_consumes_only_selected_scope_and_keeps_it_on_conclusion() {
        let from = EntitySpace {
            entity_id: "A".into(),
            space_id: "s".into(),
        };
        let to = EntitySpace {
            entity_id: "B".into(),
            space_id: "s".into(),
        };
        assert_eq!(
            crate::algebra::support(value(), "p", &from, &to, 5, &ctx())
                .unwrap_err()
                .code,
            "E_CONTEXT_REQUIRED"
        );
        let scoped = select(value(), &pin("1"), &ctx()).unwrap();
        let support = crate::algebra::support(scoped, "p", &from, &to, 5, &ctx()).unwrap();
        assert_eq!(support.selected_context, Some(pin("1")));
        assert!(support
            .graph
            .edges
            .iter()
            .all(|e| e.assertion_context == pin("1").reference().cloned()));
        assert!(serde_json::to_string(&support).unwrap().contains("refuted"));
    }
    #[test]
    fn mixed_union_loses_scope_and_diff_rejects_mismatched_scopes() {
        let a = select(value(), &pin("1"), &ctx()).unwrap();
        let b = select(value(), &ContextSelection::Default, &ctx()).unwrap();
        let union = crate::algebra::union(a.clone(), b.clone(), &ctx()).unwrap();
        assert_eq!(union.selected_context, None);
        assert_eq!(union.graph.edges.len(), 2);
        assert_eq!(
            crate::algebra::diff(a, b, &ctx()).unwrap_err().code,
            "E_CONTEXT_INCOMPATIBLE"
        );
        assert!(compatible_context(Some(&pin("1")), None).is_err());
        assert_eq!(
            compatible_context(None, Some(&ContextSelection::Default)).unwrap(),
            Some(ContextSelection::Default)
        );
    }
    #[test]
    fn scoped_rule_outputs_keep_exact_context_and_are_reusable() {
        let scoped = select(value(), &pin("1"), &ctx()).unwrap();
        let var = |s: &str| RuleTerm::Variable { name: s.into() };
        let rules = RuleSet {
            id: "R".into(),
            revision: "1".into(),
            rules: vec![Rule {
                id: "derive".into(),
                body: vec![RuleAtom {
                    predicate: "p".into(),
                    from: var("x"),
                    to: var("y"),
                    polarity: Polarity::Negative,
                }],
                head: RuleAtom {
                    predicate: "q".into(),
                    from: var("x"),
                    to: var("y"),
                    polarity: Polarity::Positive,
                },
                allow_cross_space: false,
            }],
        };
        let limits = RuleBudget {
            max_steps: 1000,
            max_rounds: 10,
            max_facts: 100,
            max_derivations: 128,
        };
        let result = crate::rules::reason(scoped, &rules, &limits, &ctx()).unwrap();
        assert!(result.graph.edges.iter().any(|e| e.predicate == "q"));
        assert!(result
            .graph
            .edges
            .iter()
            .all(|e| e.assertion_context == pin("1").reference().cloned()));
        assert_eq!(
            crate::rules::reason(result.clone(), &rules, &limits, &ctx())
                .unwrap()
                .graph,
            result.graph
        );
    }
    #[test]
    fn unknown_status_nodes_retain_scope_after_union_clears_envelope() {
        let from = EntitySpace {
            entity_id: "A".into(),
            space_id: "s".into(),
        };
        let to = EntitySpace {
            entity_id: "B".into(),
            space_id: "s".into(),
        };
        let a = crate::algebra::support(
            select(value(), &pin("empty-a"), &ctx()).unwrap(),
            "p",
            &from,
            &to,
            5,
            &ctx(),
        )
        .unwrap();
        let b = crate::algebra::support(
            select(value(), &pin("empty-b"), &ctx()).unwrap(),
            "p",
            &from,
            &to,
            5,
            &ctx(),
        )
        .unwrap();
        let union = crate::algebra::union(a, b, &ctx()).unwrap();
        assert!(union.selected_context.is_none());
        assert_eq!(union.graph.nodes.len(), 2);
        let revisions: std::collections::BTreeSet<_> = union
            .graph
            .nodes
            .iter()
            .map(|n| n.properties["context_revision"].as_str().unwrap())
            .collect();
        assert_eq!(revisions, ["empty-a", "empty-b"].into());
        let default = select(union.clone(), &ContextSelection::Default, &ctx()).unwrap();
        assert!(default.graph.nodes.is_empty());
        let selected = select(union.clone(), &pin("empty-a"), &ctx()).unwrap();
        assert_eq!(selected.graph.nodes.len(), 1);
        assert_eq!(selected.graph.nodes[0].context_scope, Some(pin("empty-a")));
        assert!(union
            .graph
            .nodes
            .iter()
            .all(|n| n.properties["context_graph_id"] == "World"
                && n.properties["state"] == "unknown"));
    }
    #[test]
    fn mixed_explanation_nodes_and_links_retain_each_conclusion_scope() {
        let explained = crate::identity::explain(&value(), &ctx()).unwrap();
        assert!(explained.selected_context.is_none());
        let default = select(explained.clone(), &ContextSelection::Default, &ctx()).unwrap();
        assert!(!default.graph.nodes.is_empty());
        assert!(default
            .graph
            .nodes
            .iter()
            .all(|n| n.context_scope == Some(ContextSelection::Default)));
        assert!(default
            .graph
            .edges
            .iter()
            .all(|e| e.assertion_context.is_none()));
        let scoped = select(explained, &pin("1"), &ctx()).unwrap();
        assert!(!scoped.graph.nodes.is_empty());
        assert!(scoped
            .graph
            .nodes
            .iter()
            .all(|n| n.context_scope == Some(pin("1"))));
        assert!(scoped
            .graph
            .edges
            .iter()
            .all(|e| e.assertion_context == pin("1").reference().cloned()));
    }
}
