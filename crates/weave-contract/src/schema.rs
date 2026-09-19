//! Portable schema validation shared by source compilers and runtime clients.
use crate::{Diagnostic, Edge, GraphData, GraphProfile, Interval, Polarity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSchema {
    pub id: String,
    pub revision: String,
    #[serde(default)]
    pub nodes: BTreeMap<String, NodeSchema>,
    #[serde(default)]
    pub edges: BTreeMap<String, EdgeSchema>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSchema {
    #[serde(default)]
    pub properties: BTreeMap<String, PropertySchema>,
    #[serde(default)]
    pub space_id: Option<String>,
    #[serde(default)]
    pub allow_extra_properties: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeSchema {
    pub from_type: String,
    pub to_type: String,
    #[serde(default)]
    pub properties: BTreeMap<String, PropertySchema>,
    #[serde(default)]
    pub allow_cross_space: bool,
    #[serde(default)]
    pub allow_extra_properties: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertySchema {
    pub value_type: ScalarType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub nullable: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalarType {
    /// Finite IEEE754 binary64; this is not exact decimal arithmetic.
    Float,
    /// Exact bounded decimal, encoded as a canonical string.
    Decimal,
    /// Exact amount with a required nominal unit descriptor.
    Quantity(crate::quantity::UnitDescriptor),
    String,
    Integer,
    Boolean,
}

fn error(errors: &mut Vec<Diagnostic>, code: &str, message: String) {
    errors.push(Diagnostic {
        code: code.into(),
        message,
    });
}
fn properties(
    object: &str,
    values: &BTreeMap<String, serde_json::Value>,
    shape: &BTreeMap<String, PropertySchema>,
    allow_extra: bool,
    errors: &mut Vec<Diagnostic>,
) {
    for (key, declaration) in shape {
        match values.get(key) {
            None if declaration.required => error(
                errors,
                "E_SCHEMA_REQUIRED",
                format!("'{object}' requires property '{key}'"),
            ),
            Some(value) => {
                let valid = if value.is_null() {
                    declaration.nullable
                } else {
                    match &declaration.value_type {
                        ScalarType::Decimal => crate::decimal::Decimal::deserialize(value).is_ok(),
                        ScalarType::Quantity(unit) => crate::quantity::Quantity::deserialize(value)
                            .is_ok_and(|quantity| quantity.unit() == unit),
                        ScalarType::String => value.is_string(),
                        ScalarType::Integer => value.as_i64().is_some(),
                        ScalarType::Boolean => value.is_boolean(),
                        ScalarType::Float => value.as_f64().is_some_and(f64::is_finite),
                    }
                };
                if !valid {
                    error(
                        errors,
                        "E_SCHEMA_PROPERTY_TYPE",
                        format!(
                            "'{object}' property '{key}' does not match {:?} (nullable: {})",
                            declaration.value_type, declaration.nullable
                        ),
                    );
                }
            }
            _ => {}
        }
    }
    if !allow_extra {
        for key in values.keys() {
            if !shape.contains_key(key) {
                error(
                    errors,
                    "E_SCHEMA_PROPERTY",
                    format!("'{object}' has undeclared property '{key}'"),
                );
            }
        }
    }
}
/// Validate an embedded schema and every typed object without I/O or authority.
/// Runtime authorization and graph structural/history validation are additional obligations.
pub fn validate_schema_graph(graph: &GraphData) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    for node in &graph.nodes {
        if let Some(scope) = &node.context_scope {
            if let Err(d) = crate::context::validate_selection(scope) {
                errors.push(d);
            }
        }
    }
    let scopes: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .filter_map(|node| {
            node.context_scope
                .as_ref()
                .map(|scope| (node.id.as_str(), scope))
        })
        .collect();
    let mut check_scope = |from: &str, to: &str, context: Option<&crate::GraphRef>| {
        if [from, to].iter().any(|id| {
            scopes
                .get(id)
                .is_some_and(|scope| scope.reference() != context)
        }) {
            error(
                &mut errors,
                "E_CONTEXT_SCOPE",
                "Claim endpoints have an incompatible value context".into(),
            );
        }
    };
    for edge in &graph.edges {
        check_scope(&edge.from, &edge.to, edge.assertion_context.as_ref());
    }
    let structural_by_id: BTreeMap<_, _> = graph
        .structural_edges
        .iter()
        .map(|edge| (&edge.id, edge))
        .collect();
    for assertion in &graph.assertions {
        if let Some(edge) = structural_by_id.get(&assertion.edge_id) {
            check_scope(&edge.from, &edge.to, assertion.context.as_ref());
        }
    }
    let structural_values: Vec<Edge>;
    let edges = match graph.profile {
        GraphProfile::Legacy => {
            if !graph.structural_edges.is_empty() || !graph.assertions.is_empty() {
                error(
                    &mut errors,
                    "E_ASSERTION_PROFILE",
                    "Legacy snapshots cannot carry explicit structural edges or assertions".into(),
                );
            }
            &graph.edges
        }
        GraphProfile::Explicit => {
            if !graph.edges.is_empty() {
                error(
                    &mut errors,
                    "E_ASSERTION_PROFILE",
                    "Explicit snapshots cannot carry legacy edges".into(),
                );
            }
            let mut ids: std::collections::BTreeSet<_> =
                graph.nodes.iter().map(|n| n.id.as_str()).collect();
            let edge_ids: std::collections::BTreeSet<_> = graph
                .structural_edges
                .iter()
                .map(|e| e.id.as_str())
                .collect();
            for edge in &graph.structural_edges {
                if edge.id.is_empty() || !ids.insert(&edge.id) {
                    error(
                        &mut errors,
                        "E_ASSERTION_ID",
                        "Structural IDs must be nonempty and unambiguous".into(),
                    );
                }
            }
            for assertion in &graph.assertions {
                if assertion.id.is_empty() || !ids.insert(&assertion.id) {
                    error(
                        &mut errors,
                        "E_ASSERTION_ID",
                        "Assertion IDs must be nonempty and unambiguous".into(),
                    );
                }
                if !edge_ids.contains(assertion.edge_id.as_str()) {
                    error(
                        &mut errors,
                        "E_ASSERTION_EDGE",
                        format!(
                            "Assertion '{}' references an absent structural edge",
                            assertion.id
                        ),
                    );
                }
                if assertion.source.is_empty() {
                    error(
                        &mut errors,
                        "E_ASSERTION_SOURCE",
                        format!("Assertion '{}' requires a source label", assertion.id),
                    );
                }
                if !assertion.valid_time.valid() {
                    error(
                        &mut errors,
                        "E_ASSERTION_INTERVAL",
                        format!("Assertion '{}' has an empty valid interval", assertion.id),
                    );
                }
            }
            for attachment in &graph.attachments {
                if attachment.id.is_empty() || !ids.insert(&attachment.id) {
                    error(
                        &mut errors,
                        "E_ASSERTION_ID",
                        "Attachment IDs must be nonempty and unambiguous".into(),
                    );
                }
            }
            structural_values = graph
                .structural_edges
                .iter()
                .map(|e| Edge {
                    id: e.id.clone(),
                    structural_ref: None,
                    assertion_source: None,
                    assertion_context: None,
                    assertion_properties: BTreeMap::new(),
                    type_id: e.type_id.clone(),
                    predicate: e.predicate.clone(),
                    from: e.from.clone(),
                    to: e.to.clone(),
                    properties: e.properties.clone(),
                    metadata: e.metadata.clone(),
                    readers: e.readers.clone(),
                    valid_time: Interval {
                        start: 0,
                        end: None,
                    },
                    polarity: Polarity::Positive,
                    derived_from: vec![],
                    derivations: vec![],
                })
                .collect();
            &structural_values
        }
    };
    let node_ids: std::collections::BTreeSet<_> =
        graph.nodes.iter().map(|n| n.id.as_str()).collect();
    for edge in edges {
        if !node_ids.contains(edge.from.as_str()) || !node_ids.contains(edge.to.as_str()) {
            error(
                &mut errors,
                "E_SCHEMA_ENDPOINT",
                format!("Edge '{}' has unavailable endpoints", edge.id),
            );
        }
    }
    let Some(schema) = &graph.schema else {
        if graph.nodes.iter().any(|n| n.type_id.is_some())
            || edges.iter().any(|e| e.type_id.is_some())
        {
            error(
                &mut errors,
                "E_SCHEMA_MISSING",
                "Typed objects require an explicit graph schema".into(),
            );
        }
        return errors;
    };
    if schema.id.is_empty() || schema.revision.is_empty() {
        error(
            &mut errors,
            "E_SCHEMA_ID",
            "Schema ID and revision must be nonempty".into(),
        );
    }
    for (name, definition) in &schema.nodes {
        if name.is_empty()
            || definition.space_id.as_deref() == Some("")
            || definition.properties.keys().any(String::is_empty)
        {
            error(
                &mut errors,
                "E_SCHEMA_ID",
                format!("Node schema '{name}' has an empty identifier"),
            );
        }
    }
    for (name, definition) in &schema.edges {
        if name.is_empty() || definition.properties.keys().any(String::is_empty) {
            error(
                &mut errors,
                "E_SCHEMA_ID",
                format!("Edge schema '{name}' has an empty identifier"),
            );
        }
        if !schema.nodes.contains_key(&definition.from_type)
            || !schema.nodes.contains_key(&definition.to_type)
        {
            error(
                &mut errors,
                "E_SCHEMA_ENDPOINT_TYPE",
                format!("Edge schema '{name}' references an unknown node type"),
            );
        }
    }
    let nodes: BTreeMap<_, _> = graph.nodes.iter().map(|n| (&n.id, n)).collect();
    for node in &graph.nodes {
        let Some(definition) = node.type_id.as_ref().and_then(|id| schema.nodes.get(id)) else {
            error(
                &mut errors,
                "E_SCHEMA_NODE_TYPE",
                format!("Node '{}' has missing or unknown type", node.id),
            );
            continue;
        };
        if definition
            .space_id
            .as_ref()
            .is_some_and(|space| space != &node.space_id)
        {
            error(
                &mut errors,
                "E_SCHEMA_SPACE",
                format!("Node '{}' is outside its declared space", node.id),
            );
        }
        properties(
            &node.id,
            &node.properties,
            &definition.properties,
            definition.allow_extra_properties,
            &mut errors,
        );
    }
    for edge in edges {
        let Some(definition) = edge.type_id.as_ref().and_then(|id| schema.edges.get(id)) else {
            error(
                &mut errors,
                "E_SCHEMA_EDGE_TYPE",
                format!("Edge '{}' has missing or unknown type", edge.id),
            );
            continue;
        };
        match (nodes.get(&edge.from), nodes.get(&edge.to)) {
            (Some(from), Some(to)) => {
                if from.type_id.as_ref() != Some(&definition.from_type)
                    || to.type_id.as_ref() != Some(&definition.to_type)
                {
                    error(
                        &mut errors,
                        "E_SCHEMA_ENDPOINT_TYPE",
                        format!(
                            "Edge '{}' endpoints do not match its declared node types",
                            edge.id
                        ),
                    );
                }
                if !definition.allow_cross_space && from.space_id != to.space_id {
                    error(
                        &mut errors,
                        "E_SCHEMA_CROSS_SPACE",
                        format!(
                            "Edge '{}' crosses spaces without an explicit schema declaration",
                            edge.id
                        ),
                    );
                }
            }
            _ => error(
                &mut errors,
                "E_SCHEMA_ENDPOINT",
                format!("Edge '{}' has unavailable typed endpoints", edge.id),
            ),
        }
        properties(
            &edge.id,
            &edge.properties,
            &definition.properties,
            definition.allow_extra_properties,
            &mut errors,
        );
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> GraphData {
        serde_json::from_value(serde_json::json!({
            "schema":{"id":"infra","revision":"1","nodes":{"Device":{"properties":{"name":{"value_type":"string","required":true},"count":{"value_type":"integer","nullable":true}},"space_id":"ops"},"Gateway":{}},"edges":{"Link":{"from_type":"Device","to_type":"Gateway","properties":{"active":{"value_type":"boolean","required":true}}}}},
            "nodes":[{"id":"d","type_id":"Device","entity_id":"device","space_id":"ops","properties":{"name":"D","count":null}},{"id":"g","type_id":"Gateway","entity_id":"gateway","space_id":"ops"}],
            "edges":[{"id":"e","type_id":"Link","predicate":"connected","from":"d","to":"g","valid_time":{"start":0,"end":10},"properties":{"active":true}}]
        })).unwrap()
    }
    #[test]
    fn typed_graph_and_nullable_optional_property_pass() {
        assert!(validate_schema_graph(&fixture()).is_empty());
    }
    #[test]
    fn direct_client_cannot_bypass_types_endpoints_or_space_checks() {
        let mut graph = fixture();
        graph.nodes[0]
            .properties
            .insert("name".into(), serde_json::json!(7));
        graph.edges[0].to = "d".into();
        graph.nodes[1].space_id = "physical".into();
        let errors = validate_schema_graph(&graph);
        assert!(errors.iter().any(|d| d.code == "E_SCHEMA_PROPERTY_TYPE"));
        assert!(errors.iter().any(|d| d.code == "E_SCHEMA_ENDPOINT_TYPE"));
        graph.edges[0].to = "g".into();
        assert!(validate_schema_graph(&graph)
            .iter()
            .any(|d| d.code == "E_SCHEMA_CROSS_SPACE"));
    }
    #[test]
    fn missing_required_unknown_fields_and_unresolved_schema_fail() {
        let mut graph = fixture();
        graph.nodes[0].properties.remove("name");
        graph.nodes[0]
            .properties
            .insert("undeclared".into(), serde_json::json!(true));
        let errors = validate_schema_graph(&graph);
        assert!(errors.iter().any(|d| d.code == "E_SCHEMA_REQUIRED"));
        assert!(errors.iter().any(|d| d.code == "E_SCHEMA_PROPERTY"));
        graph.schema = None;
        assert_eq!(validate_schema_graph(&graph)[0].code, "E_SCHEMA_MISSING");
    }
    #[test]
    fn integer_requires_signed_64_bit_and_nonnullable_null_is_rejected() {
        let mut graph = fixture();
        graph.nodes[0]
            .properties
            .insert("count".into(), serde_json::json!(1.5));
        graph.nodes[0]
            .properties
            .insert("name".into(), serde_json::Value::Null);
        assert_eq!(
            validate_schema_graph(&graph)
                .iter()
                .filter(|d| d.code == "E_SCHEMA_PROPERTY_TYPE")
                .count(),
            2
        );
        graph.nodes[0]
            .properties
            .insert("count".into(), serde_json::json!(u64::MAX));
        assert!(validate_schema_graph(&graph)
            .iter()
            .any(|d| d.message.contains("count")));
    }
    #[test]
    fn explicit_structure_is_not_an_implicit_assertion() {
        let mut g = fixture();
        let e = g.edges.remove(0);
        g.profile = GraphProfile::Explicit;
        g.structural_edges.push(crate::StructuralEdge {
            id: e.id.clone(),
            predicate: e.predicate,
            from: e.from,
            to: e.to,
            type_id: e.type_id,
            properties: e.properties,
            metadata: e.metadata,
            readers: e.readers,
        });
        assert!(validate_schema_graph(&g).is_empty());
        assert!(g.assertions.is_empty());
        for (id, polarity) in [("positive", "positive"), ("negative", "negative")] {
            g.assertions.push(serde_json::from_value(serde_json::json!({"id":id,"edge_id":"e","source":id,"valid_time":{"start":0,"end":10},"polarity":polarity})).unwrap());
        }
        assert!(validate_schema_graph(&g).is_empty());
        g.assertions[1].edge_id = "missing".into();
        assert!(validate_schema_graph(&g)
            .iter()
            .any(|e| e.code == "E_ASSERTION_EDGE"));
        g.profile = GraphProfile::Legacy;
        assert!(validate_schema_graph(&g)
            .iter()
            .any(|e| e.code == "E_ASSERTION_PROFILE"));
    }
    #[test]
    fn float_is_finite_binary64_and_does_not_weaken_integer_fields() {
        let mut graph = fixture();
        let node_type = graph.nodes[0].type_id.clone().unwrap();
        graph
            .schema
            .as_mut()
            .unwrap()
            .nodes
            .get_mut(&node_type)
            .unwrap()
            .properties
            .insert(
                "measure".into(),
                PropertySchema {
                    value_type: ScalarType::Float,
                    required: true,
                    nullable: false,
                },
            );
        graph.nodes[0]
            .properties
            .insert("measure".into(), serde_json::json!(1.25));
        // Other nodes sharing the type also require the field.
        for node in &mut graph.nodes {
            if node.type_id.as_ref() == Some(&node_type) {
                node.properties
                    .insert("measure".into(), serde_json::json!(1.25));
            }
        }
        assert!(validate_schema_graph(&graph).is_empty());
        graph.nodes[0]
            .properties
            .insert("measure".into(), serde_json::json!("NaN"));
        assert!(validate_schema_graph(&graph)
            .iter()
            .any(|d| d.code == "E_SCHEMA_PROPERTY_TYPE"));
        assert!(serde_json::from_str::<serde_json::Value>("1e400").is_err());
    }
}
