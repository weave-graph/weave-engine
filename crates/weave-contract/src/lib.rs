//! Versioned, I/O-free boundary between the Weave compiler and runtime.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub const VERSION: &str = "0.3.0";
pub const LEGACY_VERSION: &str = "0.1.0";
fn main_branch() -> String {
    "main".into()
}
fn depth() -> u32 {
    8
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GraphRef {
    pub graph_id: String,
    pub revision: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssertionRef {
    pub graph_id: String,
    pub revision: String,
    pub assertion_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Interval {
    pub start: i64,
    pub end: Option<i64>,
}
impl Interval {
    pub fn contains(&self, t: i64) -> bool {
        t >= self.start && self.end.is_none_or(|end| t < end)
    }
    pub fn valid(&self) -> bool {
        self.end.is_none_or(|end| self.start < end)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    #[default]
    Positive,
    Negative,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub entity_id: String,
    pub space_id: String,
    #[serde(default)]
    pub properties: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub metadata: Vec<GraphRef>,
    /// Empty means public. Nonempty is a disjunction of principal IDs.
    #[serde(default)]
    pub readers: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub id: String,
    pub predicate: String,
    pub from: String,
    pub to: String,
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
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct GraphData {
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct QueryPlan {
    pub graph_id: String,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default = "main_branch")]
    pub branch_id: String,
    #[serde(default)]
    pub predicate: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub valid_at: Option<i64>,
    #[serde(default)]
    pub include_metadata: bool,
    #[serde(default = "depth")]
    pub max_depth: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Bind {
        name: String,
        value: GraphExpression,
    },
    Evaluate {
        value: GraphExpression,
    },
    Join {
        left: QueryPlan,
        right: QueryPlan,
        output_predicate: String,
        match_on: JoinMatch,
    },
    Commit {
        graph_id: String,
        #[serde(default = "main_branch")]
        branch_id: String,
        #[serde(default)]
        expected_head: Option<String>,
        data: GraphData,
    },
    Query {
        query: QueryPlan,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GraphExpression {
    Query {
        query: QueryPlan,
    },
    Reference {
        name: String,
    },
    Filter {
        input: Box<GraphExpression>,
        #[serde(default)]
        predicate: Option<String>,
        #[serde(default)]
        valid_at: Option<i64>,
    },
    Join {
        left: Box<GraphExpression>,
        right: Box<GraphExpression>,
        output_predicate: String,
        match_on: JoinMatch,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JoinMatch {
    EntitySpaceToFrom,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub version: String,
    pub commands: Vec<Command>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    Complete,
    Partial,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueryResult {
    pub version: String,
    pub graph: GraphData,
    pub snapshots: BTreeMap<String, String>,
    #[serde(default)]
    pub input_snapshots: Vec<GraphRef>,
    pub coverage: Coverage,
    pub diagnostics: Vec<Diagnostic>,
    pub provenance: Vec<AssertionRef>,
    #[serde(default)]
    pub edge_origins: BTreeMap<String, Vec<AssertionRef>>,
    pub metadata_graphs: Vec<ResolvedGraph>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedGraph {
    pub reference: GraphRef,
    pub graph: GraphData,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub version: String,
    pub event_id: String,
    pub event_type: String,
    pub graph_id: String,
    pub branch_id: String,
    pub revision: String,
    pub sequence: u64,
    pub actor: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandResult {
    Committed { revision: String, event_id: String },
    Queried { result: QueryResult },
}
