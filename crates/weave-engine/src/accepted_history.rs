//! Replica-local governed acceptance history, distinct from graph receipt/branch recording.
use super::*;
fn unavailable() -> Error {
    err("E_GOV_HISTORY_UNAVAILABLE", "accepted history unavailable")
}
const MAX_HISTORY: usize = 1000;
impl Engine {
    pub fn query_accepted_selection_for(
        &self,
        view: &str,
        selection: &AcceptedSelection,
        host: &HostContext,
    ) -> Result<QueryResult> {
        let _transaction = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        weave_contract::accepted_history::validate_selection(selection)
            .map_err(|d| err(&d.code, &d.message))?;
        let cut = match selection {
            AcceptedSelection::LocalTime { unix_millis } => AcceptedViewHistoryCut::AtTime {
                observer: self.runtime_source_identity()?,
                unix_millis: *unix_millis,
            },
            AcceptedSelection::Decision {
                observer,
                decision_id,
            } => AcceptedViewHistoryCut::Decision {
                observer: observer.clone(),
                decision_id: decision_id.clone(),
            },
        };
        let state = self.query_accepted_history_for(view, &cut, host)?;
        let mut result = state.result;
        result.accepted_observations.push(state.observation);
        weave_contract::accepted_history::validate_result(&result)
            .map_err(|d| err(&d.code, &d.message))?;
        json_size(&result, MATERIALIZED_LIMIT)?;
        Ok(result)
    }
    pub(crate) fn require_accepted_result_authority(
        &self,
        value: &QueryResult,
        host: &HostContext,
    ) -> Result<()> {
        weave_contract::accepted_history::validate_result(value).map_err(|_| unavailable())?;
        for observation in &value.accepted_observations {
            let actual = self.query_accepted_history_for(
                &observation.view_id,
                &AcceptedViewHistoryCut::Decision {
                    observer: observation.observer.clone(),
                    decision_id: observation.decision_id.clone(),
                },
                host,
            )?;
            if &actual.observation != observation {
                return Err(unavailable());
            }
        }
        Ok(())
    }
    pub fn query_accepted_range_for(
        &self,
        view: &str,
        observer: &str,
        interval: &Interval,
        limit: usize,
        valid_at: Option<i64>,
        host: &HostContext,
    ) -> Result<HistoryRangeValue> {
        let _transaction = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        let range = self.accepted_history_range_for(view, observer, interval, limit, host)?;
        fn value(state: AcceptedViewHistoryResult) -> QueryResult {
            let mut result = state.result;
            result.accepted_observations.push(state.observation);
            result
        }
        let mut output = HistoryRangeValue {
            axis: HistoryAxis::Accepted,
            interval: range.interval,
            start_state: value(range.start_state),
            changes: range.changes.into_iter().map(value).collect(),
        };
        if valid_at.is_some() {
            for value in std::iter::once(&mut output.start_state).chain(output.changes.iter_mut()) {
                *value = self.expression(
                    &GraphExpression::Filter {
                        input: Box::new(GraphExpression::Reference {
                            name: "state".into(),
                        }),
                        predicate: None,
                        valid_at,
                    },
                    &BTreeMap::from([("state".into(), value.clone())]),
                    host,
                    0,
                    &mut 1000,
                )?;
            }
        }
        json_size(&output, MATERIALIZED_LIMIT)?;
        Ok(output)
    }

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
        if *unix_millis < self.retention_history_floor()? {
            self.query_accepted_view(
                &AcceptedViewSelection {
                    view_id: view.into(),
                    decision_id: None,
                },
                host,
            )
            .map_err(|_| unavailable())?;
            return Err(err(
                "E_GOV_HISTORY_EXPIRED",
                "acceptance cut precedes retained history",
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
            .map_err(|error| {
                if error.code == "E_GOV_HISTORY_EXPIRED" {
                    error
                } else {
                    unavailable()
                }
            })?;
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
