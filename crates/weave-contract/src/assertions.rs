//! Explicit structural relationships and separately attributable source claims.
use crate::*;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphProfile {
    #[default]
    Legacy,
    Explicit,
}
impl GraphProfile {
    pub fn is_legacy(&self) -> bool {
        *self == Self::Legacy
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StructuralRef {
    pub graph_id: String,
    pub revision: String,
    pub edge_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StructuralEdge {
    pub id: String,
    pub predicate: String,
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub metadata: Vec<GraphRef>,
    #[serde(default)]
    pub readers: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Assertion {
    /// Conservative whole-snapshot AND gates, independent of record readers.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "influence::bounded_snapshot_refs"
    )]
    pub derived_snapshots: Vec<GraphRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_nodes: Vec<NodeRef>,
    pub id: String,
    pub edge_id: String,
    /// Claimed source label, not cryptographic authentication.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<GraphRef>,
    pub valid_time: Interval,
    #[serde(default)]
    pub polarity: Polarity,
    #[serde(default)]
    pub properties: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub metadata: Vec<GraphRef>,
    #[serde(default)]
    pub readers: Vec<String>,
    #[serde(default)]
    pub derived_from: Vec<AssertionRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derivations: Vec<Derivation>,
}
