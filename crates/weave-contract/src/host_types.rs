//! Pinned native graph-service selectors. Parameters select installed state; they grant no authority.
use crate::{ContextSelection, GraphRef, NodeRef};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IdentityPolicyRef {
    pub id: String,
    pub revision: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IdentityResolve {
    pub mapping_id: String,
    pub revision: String,
    pub policy: IdentityPolicyRef,
    pub source: NodeRef,
    pub target_space: String,
    pub valid_at: i64,
    pub context: ContextSelection,
}
/// Source is an exact immutable pin, never a caller-supplied authorized snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClusterRequest {
    pub source: GraphRef,
    pub context: ContextSelection,
    pub valid_at: i64,
    pub predicate: String,
    /// Number of lazy levels to materialize, bounded by the input node count.
    pub levels: usize,
}
