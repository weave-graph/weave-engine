//! Recorded selectors are read criteria; claimed witnesses never grant authority.
use crate::{Diagnostic, GraphRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    Baseline,
    Committed,
    Accepted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecordedObservation {
    pub observer: String,
    pub checkpoint: String,
    pub graph: GraphRef,
    pub branch_id: String,
    pub recorded_at_ms: i64,
    pub kind: ObservationKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordedCut {
    Checkpoint {
        observer: String,
        checkpoint: String,
    },
    AtTime {
        observer: String,
        unix_millis: i64,
    },
}

/// Resolve against the actual engine replica and trusted operation snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordedSelection {
    LocalTime {
        unix_millis: i64,
    },
    Checkpoint {
        observer: String,
        checkpoint: String,
    },
}

pub fn validate_selection(value: &RecordedSelection) -> Result<(), Diagnostic> {
    let valid = match value {
        RecordedSelection::LocalTime { unix_millis } => *unix_millis >= 0,
        RecordedSelection::Checkpoint {
            observer,
            checkpoint,
        } => {
            !observer.is_empty()
                && observer.len() <= 512
                && !checkpoint.is_empty()
                && checkpoint.len() <= 512
        }
    };
    if valid {
        Ok(())
    } else {
        Err(Diagnostic {
            code: "E_HISTORY_SELECTOR".into(),
            message: "bounded recorded selector required".into(),
        })
    }
}

/// Check descriptive witnesses before pure computations consume them.
pub fn validate_result(value: &crate::QueryResult) -> Result<(), Diagnostic> {
    crate::accepted_history::validate_result(value)?;
    merge_observations(&value.recorded_observations, &[])?;
    if value
        .recorded_observations
        .iter()
        .any(|w| !value.input_snapshots.contains(&w.graph))
    {
        return Err(Diagnostic {
            code: "E_HISTORY_SELECTOR".into(),
            message: "recorded witness requires its exact input snapshot".into(),
        });
    }
    Ok(())
}

/// Preserve all distinct selections, rejecting conflicting claimed descriptions.
pub fn merge_observations(
    left: &[RecordedObservation],
    right: &[RecordedObservation],
) -> Result<Vec<RecordedObservation>, Diagnostic> {
    if left.len().saturating_add(right.len()) > 1000 {
        return Err(Diagnostic {
            code: "E_BUDGET".into(),
            message: "recorded selection manifest limit".into(),
        });
    }
    let mut values = BTreeMap::new();
    for value in left.iter().chain(right) {
        if value.recorded_at_ms < 0
            || [
                &value.observer,
                &value.checkpoint,
                &value.graph.graph_id,
                &value.graph.revision,
                &value.branch_id,
            ]
            .iter()
            .any(|s| s.is_empty() || s.len() > 512)
        {
            return Err(Diagnostic {
                code: "E_HISTORY_SELECTOR".into(),
                message: "bounded recorded witness required".into(),
            });
        }
        let key = (value.observer.clone(), value.checkpoint.clone());
        if let Some(old) = values.insert(key, value.clone()) {
            if old != *value {
                return Err(Diagnostic {
                    code: "E_HISTORY_SELECTOR".into(),
                    message: "conflicting recorded observation descriptions".into(),
                });
            }
        }
    }
    Ok(values.into_values().collect())
}
