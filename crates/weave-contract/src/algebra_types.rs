//! Portable graph-algebra types. Origin envelopes are populated by the runtime,
//! never trusted from an executable plan, and are not cryptographic signatures.
use crate::{AssertionRef, GraphRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeRef {
    pub graph_id: String,
    pub revision: String,
    pub node_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntitySpace {
    pub entity_id: String,
    pub space_id: String,
}
/// An AND group of premises. Multiple groups on an edge are OR alternatives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Derivation {
    pub operator: String,
    pub premises: Vec<AssertionRef>,
    #[serde(default)]
    pub parameters: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub input_snapshots: Vec<GraphRef>,
}
/// Trusted host context; intentionally not deserializable from plans.
#[derive(Debug, Clone)]
pub struct AlgebraContext {
    pub principal: String,
    pub max_objects: usize,
    pub max_output_bytes: usize,
}
