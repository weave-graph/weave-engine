//! Observation of actual historical actor outcomes: no new tool or effect work.
use super::*;
use serde::{Deserialize, Serialize};
const RECORD_LIMIT: usize = 16 * 1024 * 1024;
/// Default delivery disposition, without exposing an internal replay frontier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordedActorDeliveryMode {
    Compute,
    Observe,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedActorObservation {
    pub adapter: String,
    pub event: String,
    pub lease: String,
    pub prior_state_digest: String,
    pub nonce: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedActorObservationReceipt {
    pub receipt_id: String,
    pub state_digest: String,
    pub source_adapter: String,
    pub source_receipt: RecordedActorReceipt,
    pub handler: HandlerReceipt,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Witness {
    adapter: String,
    definition: RecordedActorDefinition,
    after: RecordedActorState,
    receipt: RecordedActorReceipt,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservationRecord {
    definition: RecordedActorDefinition,
    request: RecordedActorObservation,
    fence: actor_lifecycle::Fence,
    witness: Witness,
    before: RecordedActorState,
    after: RecordedActorState,
    checkpoint: i64,
    receipt: RecordedActorObservationReceipt,
}
fn integrity() -> Error {
    err(
        "E_ACTOR_OBSERVATION_INTEGRITY",
        "actor observation binding unavailable",
    )
}
fn unavailable() -> Error {
    err(
        "E_REPLAY_UNAVAILABLE",
        "actual historical actor outcome unavailable",
    )
}
fn empty_program() -> Program {
    Program {
        version: VERSION.into(),
        commands: vec![],
        source_revisions: vec![],
    }
}
fn receipt_id(request: &RecordedActorObservation) -> Result<String> {
    retention::hash(&(
        "weave-recorded-actor-observation/1",
        &request.adapter,
        &request.event,
        &request.prior_state_digest,
        &request.nonce,
    ))
}
fn observed_state(
    witness: &Witness,
    definition: &RecordedActorDefinition,
    before: &RecordedActorState,
) -> Result<RecordedActorState> {
    let mut after = witness.after.clone();
    after.inputs.adapter = definition.manifest.id.clone();
    after.inputs.registration_digest = retention::hash(definition)?;
    after.inputs.prior_state_digest = Some(retention::hash(before)?);
    recorded_actors::validate_state(&after, definition)?;
    Ok(after)
}
fn validate_basic(
    record: &ObservationRecord,
    adapter: &str,
    event: &str,
    digest: &str,
) -> Result<()> {
    recorded_actors::validate_definition(&record.definition)?;
    recorded_actors::validate_state(&record.before, &record.definition)?;
    recorded_actors::validate_state(&record.witness.after, &record.witness.definition)?;
    if retention::hash(record)? != digest
        || record.request.adapter != adapter
        || record.request.event != event
        || record.definition.manifest.id != adapter
        || !valid_id(&record.request.nonce)
        || !record.request.lease.is_empty()
        || record.fence.adapter != adapter
        || record.checkpoint < 0
        || record.checkpoint > record.fence.through
        || record.witness.adapter != record.witness.definition.manifest.id
        || !actor_lifecycle::compatible(&record.definition, &record.witness.definition)
        || record.request.prior_state_digest != retention::hash(&record.before)?
        || record.before.inputs.epoch != record.after.inputs.epoch
        || record.after != observed_state(&record.witness, &record.definition, &record.before)?
        || record.receipt.receipt_id != receipt_id(&record.request)?
        || record.receipt.state_digest != retention::hash(&record.after)?
        || record.receipt.source_adapter != record.witness.adapter
        || record.receipt.source_receipt != record.witness.receipt
        || record.receipt.handler.duplicate
        || !record.receipt.handler.results.is_empty()
    {
        return Err(integrity());
    }
    Ok(())
}
pub(crate) fn validate_retained_observation(
    engine: &Engine,
    adapter: &str,
    event: &str,
    digest: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let record: ObservationRecord =
        serde_json::from_value(value.clone()).map_err(|_| integrity())?;
    validate_basic(&record, adapter, event, digest)?;
    let definition = engine.actor_definition(adapter)?;
    let host = HostContext::new(
        &definition.manifest.principal,
        definition.manifest.output_graphs.clone(),
    );
    if engine.actor_observation(adapter, event, &host)?.as_ref() != Some(&record) {
        return Err(integrity());
    }
    Ok(())
}
impl Engine {
    pub fn recorded_actor_delivery_mode_for(
        &self,
        adapter: &str,
        event: &str,
        lease: &str,
        host: &HostContext,
    ) -> Result<RecordedActorDeliveryMode> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        let definition = self.actor_definition_for(adapter, host)?;
        if !matches!(
            self.dispatch_manifest(adapter)?.1.as_str(),
            "running" | "draining"
        ) {
            return Err(err("E_PAUSED", "actor is not running"));
        }
        self.require_recorded_actor_ready(adapter)?;
        self.check_lease(adapter, event, lease)?;
        self.require_uncanceled_delivery(adapter, event)?;
        let (graph, branch, revision, sequence) = self
            .scoped_event(&definition.manifest, event)?
            .ok_or_else(unavailable)?;
        self.actor_closure(
            &definition,
            &GraphRef {
                graph_id: graph,
                revision,
            },
            &branch,
            host,
        )?;
        self.actor_observation_current_inputs(&definition, host)?;
        Ok(
            if self
                .actor_fence(adapter)?
                .is_some_and(|f| sequence <= f.through)
            {
                RecordedActorDeliveryMode::Observe
            } else {
                RecordedActorDeliveryMode::Compute
            },
        )
    }

    fn actor_observation_current_inputs(
        &self,
        definition: &RecordedActorDefinition,
        host: &HostContext,
    ) -> Result<()> {
        for scope in &definition.manifest.subscriptions {
            let revision = self
                .head(&scope.graph_id, &scope.branch_id)?
                .ok_or_else(unavailable)?;
            self.actor_closure(
                definition,
                &GraphRef {
                    graph_id: scope.graph_id.clone(),
                    revision,
                },
                &scope.branch_id,
                host,
            )?;
        }
        Ok(())
    }
    fn actor_observation_cell(
        &self,
        adapter: &str,
        event: &str,
    ) -> Result<Option<ObservationRecord>> {
        self.read_budget.request()?;
        let row: Option<(String, Option<String>, String)> = self.conn.query_row("SELECT substr(nonce,1,513),CASE WHEN length(CAST(body AS BLOB))<=?3 THEN body END,substr(digest,1,129) FROM recorded_actor_observations WHERE adapter=?1 AND event_id=?2", params![adapter,event,RECORD_LIMIT as i64], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let Some((nonce, body, digest)) = row else {
            return Ok(None);
        };
        let body = body.ok_or_else(|| err("E_BUDGET", "actor observation record exceeds limit"))?;
        self.read_budget.charge(body.len())?;
        let record: ObservationRecord = serde_json::from_str(&body).map_err(|_| integrity())?;
        validate_basic(&record, adapter, event, &digest)?;
        let sequence: i64 = self
            .conn
            .query_row(
                "SELECT sequence FROM events WHERE event_id=?1",
                [event],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(integrity)?;
        self.read_budget.request()?;
        let actual: Option<(String,String)> = self.conn.query_row("SELECT substr(request_hash,1,129),substr(results,1,129) FROM handler_receipts WHERE adapter=?1 AND event_id=?2", params![adapter,event], |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let expected = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&empty_program())?)
        );
        if nonce != record.request.nonce
            || self.actor_definition(adapter)? != record.definition
            || self.actor_fence(adapter)?.as_ref() != Some(&record.fence)
            || sequence != record.checkpoint
            || actual != Some((expected, "[]".into()))
        {
            return Err(integrity());
        }
        Ok(Some(record))
    }
    /// Follow immutable actual receipts/fences with bounded cycle detection.
    fn actor_observation_witness(
        &self,
        source: &str,
        event: &str,
        observer: &RecordedActorDefinition,
        host: &HostContext,
    ) -> Result<Witness> {
        let mut next = source.to_string();
        let mut seen = HashSet::new();
        let mut observations = Vec::new();
        for _ in 0..16 {
            if !seen.insert(next.clone()) {
                return Err(integrity());
            }
            self.require_adapter_host(&next, host)?;
            self.reject_governed_effect_adapter(&next)?;
            let definition = self.actor_definition(&next)?;
            self.actor_observation_current_inputs(&definition, host)?;
            if !actor_lifecycle::compatible(observer, &definition) {
                return Err(integrity());
            }
            let (graph, branch, revision, sequence) = self
                .scoped_event(&definition.manifest, event)?
                .ok_or_else(unavailable)?;
            self.actor_closure(
                &definition,
                &GraphRef {
                    graph_id: graph,
                    revision,
                },
                &branch,
                host,
            )?;
            let current = self.actor_state(&next)?.ok_or_else(integrity)?;
            for pin in &current.inputs.input_snapshots {
                self.retention_whole(pin, host)?;
            }
            if let Some(record) = self.actor_record(&next, event)? {
                if record.checkpoint != sequence {
                    return Err(integrity());
                }
                for pin in record
                    .before
                    .inputs
                    .input_snapshots
                    .iter()
                    .chain(record.after.inputs.input_snapshots.iter())
                {
                    self.retention_whole(pin, host)?;
                }
                let witness = Witness {
                    adapter: next,
                    definition,
                    after: record.after,
                    receipt: record.receipt,
                };
                if observations
                    .iter()
                    .any(|r: &ObservationRecord| r.witness != witness)
                {
                    return Err(integrity());
                }
                return Ok(witness);
            }
            if let Some(record) = self.actor_observation_cell(&next, event)? {
                for pin in record
                    .before
                    .inputs
                    .input_snapshots
                    .iter()
                    .chain(record.after.inputs.input_snapshots.iter())
                {
                    self.retention_whole(pin, host)?;
                }
                observations.push(record);
            }
            let fence = self.actor_fence(&next)?.ok_or_else(unavailable)?;
            if sequence > fence.through {
                return Err(unavailable());
            }
            next = fence.source;
        }
        Err(err("E_BUDGET", "actor observation lineage exceeds limit"))
    }
    fn actor_observation(
        &self,
        adapter: &str,
        event: &str,
        host: &HostContext,
    ) -> Result<Option<ObservationRecord>> {
        let Some(record) = self.actor_observation_cell(adapter, event)? else {
            return Ok(None);
        };
        let witness =
            self.actor_observation_witness(&record.fence.source, event, &record.definition, host)?;
        if witness != record.witness {
            return Err(integrity());
        }
        for pin in record
            .before
            .inputs
            .input_snapshots
            .iter()
            .chain(record.after.inputs.input_snapshots.iter())
        {
            self.retention_whole(pin, host)?;
        }
        Ok(Some(record))
    }
    pub fn recorded_actor_observation_for(
        &self,
        adapter: &str,
        event: &str,
        host: &HostContext,
    ) -> Result<RecordedActorObservationReceipt> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.require_adapter_host(adapter, host)?;
        self.reject_governed_effect_adapter(adapter)?;
        self.actor_observation_current_inputs(&self.actor_definition(adapter)?, host)?;
        let current = self.actor_state(adapter)?.ok_or_else(integrity)?;
        for pin in &current.inputs.input_snapshots {
            self.retention_whole(pin, host)?;
        }
        Ok(self
            .actor_observation(adapter, event, host)?
            .ok_or_else(unavailable)?
            .receipt)
    }
    pub fn observe_recorded_actor_for(
        &mut self,
        request: &RecordedActorObservation,
        host: &HostContext,
    ) -> Result<RecordedActorObservationReceipt> {
        self.observe_recorded_actor_boundary(request, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn observe_recorded_actor_test_before_commit(
        &mut self,
        request: &RecordedActorObservation,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<RecordedActorObservationReceipt> {
        self.observe_recorded_actor_boundary(request, host, before_commit)
    }
    fn observe_recorded_actor_boundary(
        &mut self,
        request: &RecordedActorObservation,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<RecordedActorObservationReceipt> {
        let _budget = self.read_budget.enter();
        json_size(request, 4096)?;
        if !valid_id(&request.nonce) {
            return Err(err("E_ACTOR_OBSERVATION", "invalid observation nonce"));
        }
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _clock = self.operation_write_scope()?;
            self.require_adapter_host(&request.adapter, host)?;
            self.reject_governed_effect_adapter(&request.adapter)?;
            let definition = self.actor_definition(&request.adapter)?;
            self.actor_observation_current_inputs(&definition, host)?;
            self.require_uncanceled_delivery(&request.adapter, &request.event)?;
            let current = self.actor_state(&request.adapter)?.ok_or_else(integrity)?;
            for pin in &current.inputs.input_snapshots {
                self.retention_whole(pin, host)?;
            }
            let mut canonical = request.clone();
            canonical.lease.clear();
            if let Some(record) = self.actor_observation(&request.adapter, &request.event, host)? {
                if record.request != canonical {
                    return Err(err(
                        "E_RECEIPT_CONFLICT",
                        "occurrence has another observation",
                    ));
                }
                let mut receipt = record.receipt;
                receipt.handler.duplicate = true;
                before_commit();
                return Ok(receipt);
            }
            self.actor_definition_for(&request.adapter, host)?;
            self.require_recorded_actor_ready(&request.adapter)?;
            self.check_lease(&request.adapter, &request.event, &request.lease)?;
            let (_, _, _, sequence) = self
                .scoped_event(&definition.manifest, &request.event)?
                .ok_or_else(unavailable)?;
            let fence = self
                .actor_fence(&request.adapter)?
                .ok_or_else(unavailable)?;
            if sequence > fence.through {
                return Err(err(
                    "E_ACTOR_OBSERVATION",
                    "occurrence is new computation beyond observation fence",
                ));
            }
            if retention::hash(&current)? != request.prior_state_digest {
                return Err(err("E_CONFLICT", "actor state changed"));
            }
            let witness =
                self.actor_observation_witness(&fence.source, &request.event, &definition, host)?;
            if witness.after.inputs.epoch != current.inputs.epoch {
                return Err(err(
                    "E_CHECKPOINT_EXPIRED",
                    "historical actor state requires explicit reconstruction",
                ));
            }
            let intent_count: i64 = self.conn.query_row(
                "SELECT count(*) FROM effect_intents WHERE adapter=?1 AND event_id=?2",
                params![request.adapter, request.event],
                |r| r.get(0),
            )?;
            if intent_count != 0
                || self
                    .actor_record(&request.adapter, &request.event)?
                    .is_some()
            {
                return Err(integrity());
            }
            let (count,used):(i64,i64) = self.conn.query_row("SELECT count(*),coalesce(sum(length(CAST(body AS BLOB))),0) FROM recorded_actor_observations WHERE adapter=?1", [&request.adapter], |r|Ok((r.get(0)?,r.get(1)?)))?;
            if count >= 128 {
                return Err(err("E_BUDGET", "actor observation count exceeded"));
            }
            let token =
                recorded_actors::CompletionToken::observed(&request.adapter, &request.event);
            let handler = self.complete_handler_with_actor(
                &request.adapter,
                &request.event,
                &request.lease,
                &empty_program(),
                &token,
            )?;
            if handler.duplicate {
                return Err(integrity());
            }
            let after = observed_state(&witness, &definition, &current)?;
            let receipt = RecordedActorObservationReceipt {
                receipt_id: receipt_id(&canonical)?,
                state_digest: retention::hash(&after)?,
                source_adapter: witness.adapter.clone(),
                source_receipt: witness.receipt.clone(),
                handler,
            };
            let record = ObservationRecord {
                definition,
                request: canonical,
                fence,
                witness,
                before: current,
                after: after.clone(),
                checkpoint: sequence,
                receipt: receipt.clone(),
            };
            let body = serde_json::to_string(&record)?;
            if body.len() > RECORD_LIMIT
                || used
                    .checked_add(body.len() as i64)
                    .is_none_or(|n| n > 64 * 1024 * 1024)
            {
                return Err(err("E_BUDGET", "actor observation byte quota exceeded"));
            }
            validate_basic(
                &record,
                &request.adapter,
                &request.event,
                &retention::hash(&record)?,
            )?;
            self.put_actor_state(&after, sequence)?;
            self.conn.execute(
                "INSERT INTO recorded_actor_observations VALUES (?1,?2,?3,?4,?5)",
                params![
                    request.adapter,
                    request.event,
                    request.nonce,
                    body,
                    retention::hash(&record)?
                ],
            )?;
            before_commit();
            Ok(receipt)
        }));
        let result = operation_clock::rollback_unwind(outcome, &self.conn, "ROLLBACK");
        match result {
            Ok(value) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                self.conn.execute_batch("ROLLBACK")?;
                Err(error)
            }
        }
    }
}
