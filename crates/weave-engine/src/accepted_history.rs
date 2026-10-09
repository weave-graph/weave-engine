//! Replica-local governed acceptance history, distinct from graph receipt/branch recording.
use super::*;
use serde::{Deserialize, Serialize};

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
fn unavailable() -> Error {
    err("E_GOV_HISTORY_UNAVAILABLE", "accepted history unavailable")
}
const MAX_HISTORY: usize = 1000;
impl Engine {
    fn accepted_history_scope(
        &self,
        view: &str,
        observer: &str,
        host: &HostContext,
    ) -> Result<Option<String>> {
        if ![view, observer, &host.principal]
            .iter()
            .all(|id| valid_id(id))
            || observer != self.runtime_source_identity()?
        {
            return Err(unavailable());
        }
        let head = self.gov_head(view).map_err(|_| unavailable())?;
        let policy = self
            .gov_policy(view, &head.policy)
            .map_err(|_| unavailable())?;
        if !policy.readers.is_empty()
            && !policy.readers.contains(&host.principal)
            && !policy.proposers.contains(&host.principal)
        {
            return Err(unavailable());
        }
        Ok(head.decision_id)
    }
    fn accepted_history_state(
        &self,
        view: &str,
        decision: &str,
        time: i64,
        host: &HostContext,
    ) -> Result<AcceptedViewHistoryResult> {
        let observation = self
            .accepted_history_observation(view, decision, time)
            .map_err(|_| unavailable())?;
        let result = self
            .query_accepted_view(
                &AcceptedViewSelection {
                    view_id: view.into(),
                    decision_id: Some(decision.into()),
                },
                host,
            )
            .map_err(|_| unavailable())?;
        if !result.input_snapshots.contains(&observation.occurrence)
            || !result.input_snapshots.contains(&observation.source)
        {
            return Err(unavailable());
        }
        let state = AcceptedViewHistoryResult {
            observation,
            result,
        };
        json_size(&state, MATERIALIZED_LIMIT)?;
        Ok(state)
    }
    /// Select the exact local accepted occurrence, never a global or received-time cut.
    pub fn query_accepted_history_for(
        &self,
        view: &str,
        cut: &AcceptedViewHistoryCut,
        host: &HostContext,
    ) -> Result<AcceptedViewHistoryResult> {
        let _transaction = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        let observer = match cut {
            AcceptedViewHistoryCut::Decision { observer, .. }
            | AcceptedViewHistoryCut::AtTime { observer, .. } => observer,
        };
        let tip = self
            .accepted_history_scope(view, observer, host)?
            .ok_or_else(unavailable)?;
        if let AcceptedViewHistoryCut::Decision { decision_id, .. } = cut {
            if !valid_id(decision_id) {
                return Err(unavailable());
            }
            let (_, time) = self.governance_history_link(view, decision_id)?;
            return self.accepted_history_state(view, decision_id, time, host);
        }
        let AcceptedViewHistoryCut::AtTime { unix_millis, .. } = cut else {
            unreachable!()
        };
        if *unix_millis < 0 || *unix_millis > self.operation_time()? {
            return Err(err(
                "E_GOV_HISTORY_TIME",
                "acceptance cut outside observed clock",
            ));
        }
        let mut current = Some(tip);
        let mut seen = HashSet::new();
        let mut later = None;
        while let Some(decision) = current {
            if seen.len() == MAX_HISTORY || !seen.insert(decision.clone()) {
                return Err(unavailable());
            }
            let (parent, time) = self.governance_history_link(view, &decision)?;
            self.accepted_history_observation(view, &decision, time)
                .map_err(|_| unavailable())?;
            if later.is_some_and(|later| time > later) {
                return Err(unavailable());
            }
            if time <= *unix_millis {
                return self.accepted_history_state(view, &decision, time, host);
            }
            later = Some(time);
            current = parent;
        }
        Err(unavailable())
    }
    /// Authorized start state plus all actual accepted occurrences, with no hidden-version skips.
    pub fn accepted_history_range_for(
        &self,
        view: &str,
        observer: &str,
        interval: &Interval,
        limit: usize,
        host: &HostContext,
    ) -> Result<AcceptedViewHistoryRange> {
        if !interval.valid()
            || interval.start < 0
            || interval.end.is_none()
            || !(1..=MAX_HISTORY).contains(&limit)
        {
            return Err(err(
                "E_GOV_HISTORY_RANGE",
                "finite acceptance interval and bounded limit required",
            ));
        }
        let _transaction = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        let end = interval.end.expect("finite range");
        if end > self.operation_time()? {
            return Err(err(
                "E_GOV_HISTORY_TIME",
                "acceptance range outside observed clock",
            ));
        }
        let start_state = self
            .query_accepted_history_for(
                view,
                &AcceptedViewHistoryCut::AtTime {
                    observer: observer.into(),
                    unix_millis: interval.start,
                },
                host,
            )
            .map_err(|_| unavailable())?;
        let mut current = self.accepted_history_scope(view, observer, host)?;
        let mut remaining_bytes = MATERIALIZED_LIMIT
            .saturating_sub(json_size(&start_state, MATERIALIZED_LIMIT)?)
            .saturating_sub(256);
        let mut changes = Vec::new();
        let mut seen = HashSet::new();
        let mut later = None;
        while let Some(decision) = current {
            if seen.len() == MAX_HISTORY || !seen.insert(decision.clone()) {
                return Err(unavailable());
            }
            let (parent, time) = self.governance_history_link(view, &decision)?;
            self.accepted_history_observation(view, &decision, time)
                .map_err(|_| unavailable())?;
            if later.is_some_and(|later| time > later) {
                return Err(unavailable());
            }
            if time < interval.start {
                break;
            }
            if time < end {
                if changes.len() == limit {
                    return Err(unavailable());
                }
                let state = self.accepted_history_state(view, &decision, time, host)?;
                let bytes = json_size(&state, remaining_bytes).map_err(|_| unavailable())?;
                remaining_bytes = remaining_bytes
                    .checked_sub(bytes.saturating_add(1))
                    .ok_or_else(unavailable)?;
                changes.push(state);
            }
            later = Some(time);
            current = parent;
        }
        changes.reverse();
        Ok(AcceptedViewHistoryRange {
            interval: interval.clone(),
            start_state,
            changes,
        })
    }
}
