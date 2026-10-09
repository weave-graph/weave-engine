//! Local acceptance selection descriptions never install governance authority.
use crate::{Diagnostic, GraphRef, Interval, QueryResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AcceptedViewHistoryCut {
    Decision {
        observer: String,
        decision_id: String,
    },
    AtTime {
        observer: String,
        unix_millis: i64,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedViewObservation {
    pub observer: String,
    pub view_id: String,
    pub decision_id: String,
    pub accepted_at_ms: i64,
    /// The genuine protected governance occurrence, not a caller-authored label.
    pub occurrence: GraphRef,
    pub source: GraphRef,
    pub branch_id: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedViewHistoryResult {
    pub observation: AcceptedViewObservation,
    pub result: QueryResult,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedViewHistoryRange {
    pub interval: Interval,
    pub start_state: AcceptedViewHistoryResult,
    /// All accepted occurrences in the half-open interval, in ancestry order.
    pub changes: Vec<AcceptedViewHistoryResult>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AcceptedSelection {
    LocalTime {
        unix_millis: i64,
    },
    Decision {
        observer: String,
        decision_id: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAxis {
    Recorded,
    Accepted,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRangeValue {
    pub axis: HistoryAxis,
    pub interval: Interval,
    pub start_state: QueryResult,
    pub changes: Vec<QueryResult>,
}
fn invalid() -> Diagnostic {
    Diagnostic {
        code: "E_ACCEPTED_SELECTOR".into(),
        message: "bounded accepted selection required".into(),
    }
}
fn id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512
}
pub fn validate_selection(value: &AcceptedSelection) -> Result<(), Diagnostic> {
    let valid = match value {
        AcceptedSelection::LocalTime { unix_millis } => *unix_millis >= 0,
        AcceptedSelection::Decision {
            observer,
            decision_id,
        } => id(observer) && id(decision_id),
    };
    if valid {
        Ok(())
    } else {
        Err(invalid())
    }
}
pub fn validate_range(interval: &Interval, limit: usize) -> Result<(), Diagnostic> {
    if interval.valid()
        && interval.start >= 0
        && interval.end.is_some()
        && (1..=1000).contains(&limit)
    {
        Ok(())
    } else {
        Err(Diagnostic {
            code: "E_HISTORY_RANGE".into(),
            message: "finite half-open interval and bounded history limit required".into(),
        })
    }
}
pub fn merge_observations(
    left: &[AcceptedViewObservation],
    right: &[AcceptedViewObservation],
) -> Result<Vec<AcceptedViewObservation>, Diagnostic> {
    if left.len().saturating_add(right.len()) > 1000 {
        return Err(Diagnostic {
            code: "E_BUDGET".into(),
            message: "accepted selection manifest limit".into(),
        });
    }
    let mut values = BTreeMap::new();
    for value in left.iter().chain(right) {
        if value.accepted_at_ms < 0
            || [
                &value.observer,
                &value.view_id,
                &value.decision_id,
                &value.branch_id,
                &value.occurrence.graph_id,
                &value.occurrence.revision,
                &value.source.graph_id,
                &value.source.revision,
            ]
            .iter()
            .any(|s| !id(s))
        {
            return Err(invalid());
        }
        let key = (
            value.observer.clone(),
            value.view_id.clone(),
            value.decision_id.clone(),
        );
        if let Some(old) = values.insert(key, value.clone()) {
            if old != *value {
                return Err(invalid());
            }
        }
    }
    Ok(values.into_values().collect())
}
pub fn validate_result(value: &QueryResult) -> Result<(), Diagnostic> {
    merge_observations(&value.accepted_observations, &[])?;
    if value.accepted_observations.iter().any(|w| {
        !value.input_snapshots.contains(&w.source) || !value.input_snapshots.contains(&w.occurrence)
    }) {
        return Err(invalid());
    }
    Ok(())
}
