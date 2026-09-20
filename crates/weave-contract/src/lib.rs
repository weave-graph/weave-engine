//! Versioned, I/O-free boundary between the Weave compiler and runtime.
pub mod context_axes;
pub mod context_typing;
pub use context_typing::{ContextTyping, TypedContextWitness};
pub mod handler_registration;
pub mod host_types;
pub mod view_registration;
pub use handler_registration::{
    CompiledHandlerTemplate, HandlerBinding, HandlerEventType, HandlerInput, HandlerRecipe,
};
pub use host_types::{ClusterRequest, IdentityPolicyRef, IdentityResolve};
pub use view_registration::{
    AcceptedGraphSelection, CompiledViewTemplate, CurrentViewSelection, ViewClock, ViewReadTime,
};
pub mod decimal;
pub mod quantity;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub const VERSION: &str = "0.19.0";
pub mod carrier_algebra;
pub mod carrier_profile;
pub mod influence;
pub mod temporal;
pub use influence::GraphInfluence;
pub mod counterpart;
mod geometry_types;
pub use geometry_types::*;
pub mod context;
mod context_types;
pub use context_types::*;
pub mod rules;
mod rules_types;
pub use rules_types::*;
pub mod algebra;
pub mod identity;
pub use identity::SourceRevision;
mod algebra_types;
mod assertions;
mod schema;
pub use algebra_types::*;
pub use assertions::*;
pub use schema::*;
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
    /// Record-local OR alternatives, conjunctive with the declared flat gates.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "crate::carrier_profile::bounded_derivations"
    )]
    pub derivations: Vec<Derivation>,

    /// Conservative whole-snapshot AND gates, independent of record readers.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "influence::bounded_snapshot_refs"
    )]
    pub derived_snapshots: Vec<GraphRef>,
    /// Conservative AND influence from exact source nodes, including isolated nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_nodes: Vec<NodeRef>,
    /// Conservative AND gate for derived node values, independent of readers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_from: Vec<AssertionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_scope: Option<ContextSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
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
    /// Conservative whole-snapshot AND gates, independent of record readers.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "influence::bounded_snapshot_refs"
    )]
    pub derived_snapshots: Vec<GraphRef>,
    /// Global AND gate, independent of alternative derivation groups.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_nodes: Vec<NodeRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assertion_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assertion_context: Option<GraphRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structural_ref: Option<StructuralRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assertion_properties: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derivations: Vec<Derivation>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct GraphData {
    /// Conservative whole-value AND influence, retained even when no objects survive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub influence: Option<GraphInfluence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_typing: Option<ContextTyping>,
    #[serde(default, skip_serializing_if = "GraphProfile::is_legacy")]
    pub profile: GraphProfile,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structural_edges: Vec<StructuralEdge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assertions: Vec<Assertion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<GraphSchema>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MetadataAttachment>,
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
    CommitBatch {
        batch_id: String,
        commits: Vec<SnapshotCommit>,
    },
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CounterpartSelection {
    pub predicate: String,
    pub entity_id: String,
    pub from_space_id: String,
    pub to_space_id: String,
    pub valid_at: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GraphExpression {
    Window {
        input: Box<GraphExpression>,
        window: Interval,
    },
    Sequence {
        left: Box<GraphExpression>,
        right: Box<GraphExpression>,
        window: Interval,
        relation: TemporalRelation,
        match_on: JoinMatch,
    },
    AcceptedGraph {
        selection: AcceptedGraphSelection,
    },
    CurrentView {
        selection: CurrentViewSelection,
    },
    TypedContext {
        input: Box<GraphExpression>,
        reference: GraphRef,
        expected_schema: context_axes::ContextSchema,
    },
    ResolveIdentity {
        selection: IdentityResolve,
    },
    Cluster {
        selection: ClusterRequest,
    },
    Counterparts {
        input: Box<GraphExpression>,
        selection: CounterpartSelection,
    },
    Geometry {
        operation: GeometryOperation,
        valid_at: i64,
    },
    Explain {
        input: Box<GraphExpression>,
    },
    Context {
        input: Box<GraphExpression>,
        selection: ContextSelection,
    },
    Reason {
        input: Box<GraphExpression>,
        rules: RuleSet,
    },
    Union {
        left: Box<GraphExpression>,
        right: Box<GraphExpression>,
    },
    Diff {
        before: Box<GraphExpression>,
        after: Box<GraphExpression>,
    },
    Project {
        input: Box<GraphExpression>,
        node_ids: Vec<String>,
        edge_ids: Vec<String>,
    },
    Support {
        input: Box<GraphExpression>,
        predicate: String,
        from: EntitySpace,
        to: EntitySpace,
        valid_at: i64,
    },
    Metadata {
        input: Box<GraphExpression>,
        host: MetadataHost,
        key: String,
    },
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
/// Pure valid-time relation; does not select or authorize a recorded-time observation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemporalRelation {
    Before,
    Meets,
    Overlaps,
    Within,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JoinMatch {
    EntitySpaceToFrom,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Program {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_revisions: Vec<SourceRevision>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_context: Option<ContextSelection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_revisions: Vec<SourceRevision>,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_origins: BTreeMap<String, Vec<NodeRef>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attachment_origins: BTreeMap<String, Vec<AssertionRef>>,
    pub metadata_graphs: Vec<ResolvedGraph>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedGraph {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attachment_origins: BTreeMap<String, Vec<AssertionRef>>,
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
    Unchanged {
        revision: String,
    },
    BatchUnchanged {
        commits: Vec<CommitReceipt>,
    },
    BatchCommitted {
        manifest_id: String,
        commits: Vec<CommitReceipt>,
    },
    Committed {
        revision: String,
        event_id: String,
    },
    Queried {
        result: Box<QueryResult>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SnapshotCommit {
    pub graph_id: String,
    #[serde(default = "main_branch")]
    pub branch_id: String,
    #[serde(default)]
    pub expected_head: Option<String>,
    pub data: GraphData,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MetadataHost {
    Assertion { id: String },
    Node { id: String },
    Edge { id: String },
    Entity { id: String },
    Graph,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MetadataValue {
    Literal {
        value: serde_json::Value,
    },
    Graph {
        reference: GraphRef,
    },
    LiveGraph {
        graph_id: String,
        #[serde(default = "main_branch")]
        branch_id: String,
    },
    Object {
        graph_id: String,
        object_id: String,
    },
    Artifact {
        uri: String,
        digest: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MetadataAttachment {
    /// Record-local OR alternatives, conjunctive with the declared flat gates.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "crate::carrier_profile::bounded_derivations"
    )]
    pub derivations: Vec<Derivation>,

    /// Conservative assertion influence, independent of origin and host placement.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "influence::bounded_refs"
    )]
    pub derived_from: Vec<AssertionRef>,
    /// Conservative node influence, independent of origin and host placement.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "influence::bounded_refs"
    )]
    pub derived_nodes: Vec<NodeRef>,
    /// Conservative whole-snapshot AND gates, independent of record readers.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "influence::bounded_snapshot_refs"
    )]
    pub derived_snapshots: Vec<GraphRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<GraphRef>,
    pub id: String,
    pub host: MetadataHost,
    pub key: String,
    pub value: MetadataValue,
    pub valid_time: Interval,
    #[serde(default)]
    pub origin: Option<AssertionRef>,
    #[serde(default)]
    pub readers: Vec<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub schema_revision: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestMember {
    pub graph_id: String,
    pub branch_id: String,
    pub revision: String,
    pub parent: Option<String>,
    pub content_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SnapshotManifest {
    pub batch_id: String,
    pub members: Vec<ManifestMember>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitReceipt {
    pub graph_id: String,
    pub branch_id: String,
    pub revision: String,
    pub event_id: Option<String>,
}
