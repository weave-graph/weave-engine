//! Exact claim context selection; default is one context, never a wildcard.
use crate::GraphRef;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextSelection {
    #[default]
    Default,
    Pinned {
        reference: GraphRef,
    },
}
impl ContextSelection {
    pub fn reference(&self) -> Option<&GraphRef> {
        match self {
            Self::Default => None,
            Self::Pinned { reference } => Some(reference),
        }
    }
}
