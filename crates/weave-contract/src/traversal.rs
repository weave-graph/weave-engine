//! Traversal, time travel and change sets (protocol 0.23.0, A3).
//!
//! Two graph expressions read one graph branch through the fact store's
//! indexes:
//!
//! - [`ExpandPlan`] (`kind: "expand"`) returns the exact k-hop neighborhood of
//!   seed nodes, optionally restricted by predicate, direction and valid time,
//!   at a pinned revision, the current head or the branch as known at a
//!   recorded time (`as_of`).
//! - [`ChangesPlan`] (`kind: "changes"`) returns the facts asserted, retracted
//!   or corrected between two states of a branch, each selected by recorded
//!   time, by an exact revision (a cursor from an earlier change set) or as
//!   the current head. It is the pull change feed.
//!
//! Both are authorized under the evaluating principal's current authority.
//! Hidden facts never appear in, and never change, their output.
use crate::{main_branch, Fact, FactKind};
use serde::{Deserialize, Serialize};

/// Most seed nodes one `expand` may name.
pub const MAX_SEEDS: usize = 1_000;
/// Most hops one `expand` may traverse.
pub const MAX_HOPS: u32 = 16;
/// Most predicates one `expand` or `changes` may name, and most scope entities.
pub const MAX_SCOPE: usize = 1_000;
/// Largest `limit` of an `expand` (returned nodes plus edges) or `changes` (entries).
pub const MAX_LIMIT: u32 = 200_000;
/// Default `limit`.
pub const DEFAULT_LIMIT: u32 = 10_000;

fn default_limit() -> u32 {
    DEFAULT_LIMIT
}

/// Which edges a hop follows, relative to the node it starts from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Edges whose `from` is the node.
    #[default]
    Out,
    /// Edges whose `to` is the node.
    In,
    /// Edges with the node at either end.
    Both,
}

/// k-hop expansion from seed nodes (`kind: "expand"`).
///
/// The result holds every node within `hops` hops of a visible seed along
/// visible edges that match `predicates` (empty means any) and, with
/// `valid_at`, are valid at that instant, plus every such edge that leaves a
/// node fewer than `hops` hops away. Seeds that are missing or hidden make the
/// result partial; they are not distinguished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpandPlan {
    pub graph_id: String,
    #[serde(default = "main_branch")]
    pub branch_id: String,
    /// An exact revision. Absent means the branch head, or `as_of` when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Read the branch as this replica knew it at this recorded time (Unix ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<i64>,
    /// Node IDs to start from (1 to 1,000).
    pub seeds: Vec<String>,
    /// Hops to traverse (1 to 16).
    pub hops: u32,
    #[serde(default)]
    pub direction: Direction,
    /// Edge predicates to follow; empty follows every predicate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub predicates: Vec<String>,
    /// Follow only edges, and keep only nodes and attachments, valid at this instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_at: Option<i64>,
    /// Most nodes plus edges the result may hold; more fails with `E_BUDGET`.
    #[serde(default = "default_limit")]
    pub limit: u32,
}

/// One end of a change set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChangeBound {
    /// The branch as this replica knew it at a recorded time (Unix ms). Before
    /// the branch's first recorded head this is the empty state.
    AsOf { unix_millis: i64 },
    /// An exact revision of the graph, typically the `to_revision` of an
    /// earlier change set.
    Cursor { revision: String },
    /// The branch head when the expression is evaluated.
    Head,
}

fn head() -> ChangeBound {
    ChangeBound::Head
}

/// Facts asserted, retracted or corrected between two states of one branch
/// (`kind: "changes"`).
///
/// The change set is the difference between the two authorized states, fact
/// by fact: a fact visible only in `to` is asserted, one visible only in
/// `from` is retracted, and one visible in both with different content is
/// corrected. Intermediate versions between the two states are not listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangesPlan {
    pub graph_id: String,
    #[serde(default = "main_branch")]
    pub branch_id: String,
    pub from: ChangeBound,
    #[serde(default = "head")]
    pub to: ChangeBound,
    /// Keep only changes about these entities; empty keeps every change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<String>,
    /// Keep only changes of edges with these predicates and attachments with
    /// these keys; empty keeps every change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub predicates: Vec<String>,
    /// Most entries the change set may hold; more fails with `E_BUDGET`.
    #[serde(default = "default_limit")]
    pub limit: u32,
}

/// How one fact differs between the two states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Asserted,
    Retracted,
    Corrected,
}

/// One changed fact. `before` is its authorized value in the `from` state and
/// `after` in the `to` state; each is absent where the fact is not visible.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeEntry {
    pub change: ChangeKind,
    pub kind: FactKind,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Fact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Fact>,
}

/// The change set of a `changes` expression (`QueryResult.changes`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeSet {
    /// The revision of the `from` state; absent for the empty state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_revision: Option<String>,
    /// The revision of the `to` state; absent for the empty state. Pass it as
    /// `{kind: "cursor", revision}` to continue the feed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_revision: Option<String>,
    /// Entries in `(kind, id)` order.
    pub entries: Vec<ChangeEntry>,
}
