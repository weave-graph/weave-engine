//! Portable schema validation shared by source compilers and runtime clients.
use crate::{Diagnostic, GraphData};
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
                    match declaration.value_type {
                        ScalarType::String => value.is_string(),
                        ScalarType::Integer => value.as_i64().is_some(),
                        ScalarType::Boolean => value.is_boolean(),
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
    let Some(schema) = &graph.schema else {
        if graph.nodes.iter().any(|n| n.type_id.is_some())
            || graph.edges.iter().any(|e| e.type_id.is_some())
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
    for edge in &graph.edges {
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
}
