//! Fixed-owner actor cleanup: never forget an unknown physical outcome.
use super::*;
use serde::{Deserialize, Serialize};
const RECORD_LIMIT: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancellation {
    request: DeliveryCancellationRequest,
    principal: String,
    definition: RecordedActorDefinition,
    source: GraphRef,
    before: RecordedActorState,
    before_checkpoint: i64,
    checkpoint: i64,
    effects: Vec<EffectDisposition>,
    receipt: DeliveryCancellationReceipt,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectDisposition {
    id: String,
    before_state: String,
    before_digest: String,
    after_digest: String,
}
fn integrity() -> Error {
    err(
        "E_ACTOR_CANCELLATION_INTEGRITY",
        "actor disposition binding unavailable",
    )
}
fn receipt_id(request: &DeliveryCancellationRequest, principal: &str) -> Result<String> {
    retention::hash(&("weave-recorded-actor-cancellation/1", principal, request))
}
fn canceled_response(
    request: &DeliveryCancellationRequest,
    principal: &str,
) -> Result<serde_json::Value> {
    Ok(
        serde_json::json!({"owner_disposition":receipt_id(request,principal)?,"outcome":"not_dispatched"}),
    )
}
fn validate_record(record: &Cancellation, adapter: &str, event: &str, digest: &str) -> Result<()> {
    recorded_actors::validate_definition(&record.definition)?;
    recorded_actors::validate_state(&record.before, &record.definition)?;
    if retention::hash(record)? != digest
        || record.request.adapter != adapter
        || record.request.event != event
        || record.definition.manifest.id != adapter
        || record.definition.manifest.principal != record.principal
        || ![
            &record.request.adapter,
            &record.request.event,
            &record.request.expected_lease,
            &record.request.nonce,
        ]
        .iter()
        .all(|s| valid_id(s))
        || record.before_checkpoint < 0
        || record.checkpoint <= record.before_checkpoint
        || record.receipt.adapter != adapter
        || record.receipt.event != event
        || record.receipt.duplicate
        || !record.receipt.rebuild_required
        || record.receipt.receipt_id != receipt_id(&record.request, &record.principal)?
        || record.effects.len() > 32
    {
        return Err(integrity());
    }
    let mut ids = Vec::new();
    for effect in &record.effects {
        if !valid_id(&effect.id)
            || !matches!(
                effect.before_state.as_str(),
                "pending" | "confirmed" | "failed"
            )
            || !effect.before_digest.starts_with("sha256:")
            || effect.before_digest.len() != 71
            || !effect.after_digest.starts_with("sha256:")
            || effect.after_digest.len() != 71
        {
            return Err(integrity());
        }
        ids.push(effect.id.clone());
    }
    let mut ordered = ids.clone();
    ordered.sort();
    ordered.dedup();
    if ordered != ids {
        return Err(integrity());
    }
    Ok(())
}
pub(crate) fn validate_retained_actor_cancellation(
    engine: &Engine,
    adapter: &str,
    event: &str,
    digest: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let record: Cancellation = serde_json::from_value(value.clone()).map_err(|_| integrity())?;
    validate_record(&record, adapter, event, digest)?;
    if engine.actor_cancellation(adapter, event)?.as_ref() != Some(&record) {
        return Err(integrity());
    }
    Ok(())
}
impl Engine {
    pub(crate) fn initialize_actor_disposition(&self, version: i64) -> Result<()> {
        let present:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='recorded_actor_cancellations')",[],|r|r.get(0))?;
        if (version < 28 && present) || (version >= 28 && !present) {
            return Err(integrity());
        }
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS recorded_actor_cancellations(adapter TEXT NOT NULL REFERENCES recorded_actor_definitions(adapter),event_id TEXT NOT NULL,nonce TEXT NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(adapter,event_id),UNIQUE(adapter,nonce));")?;
        Ok(())
    }
    fn actor_cancellation(&self, adapter: &str, event: &str) -> Result<Option<Cancellation>> {
        self.read_budget.request()?;
        let row:Option<(String,Option<String>,String)>=self.conn.query_row("SELECT substr(nonce,1,513),CASE WHEN length(CAST(body AS BLOB))<=?3 THEN body END,substr(digest,1,129) FROM recorded_actor_cancellations WHERE adapter=?1 AND event_id=?2",params![adapter,event,RECORD_LIMIT as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let Some((nonce, body, digest)) = row else {
            return Ok(None);
        };
        let body = body.ok_or_else(|| err("E_BUDGET", "actor disposition exceeds limit"))?;
        self.read_budget.charge(body.len())?;
        let record: Cancellation = serde_json::from_str(&body).map_err(|_| integrity())?;
        validate_record(&record, adapter, event, &digest)?;
        let occurrence: Option<(String, String, String, i64)> = self
            .conn
            .query_row(
                "SELECT graph_id,branch_id,revision,sequence FROM events WHERE event_id=?1",
                [event],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let (graph, branch, revision, sequence) = occurrence.ok_or_else(integrity)?;
        let (_, _, checkpoint) = self.dispatch_manifest(adapter)?;
        if nonce != record.request.nonce
            || self.actor_definition(adapter)? != record.definition
            || record.source
                != (GraphRef {
                    graph_id: graph.clone(),
                    revision,
                })
            || sequence != record.checkpoint
            || !record
                .definition
                .manifest
                .subscriptions
                .contains(&SubscriptionScope {
                    graph_id: graph,
                    branch_id: branch,
                })
            || checkpoint < record.checkpoint
            || self.actor_state(adapter)?.is_none()
        {
            return Err(integrity());
        }
        let actual = self.actor_disposition_effects(adapter, event)?;
        if actual.len() != record.effects.len() {
            return Err(integrity());
        }
        for (after, witness) in actual.iter().zip(&record.effects) {
            let mut before = after.clone();
            if witness.before_state == "pending" {
                if after.state != "failed"
                    || after.response.as_ref()
                        != Some(&canceled_response(&record.request, &record.principal)?)
                {
                    return Err(integrity());
                }
                before.state = "pending".into();
                before.response = None;
            } else if after.state != witness.before_state {
                return Err(integrity());
            }
            if after.id != witness.id
                || retention::hash(after)? != witness.after_digest
                || retention::hash(&before)? != witness.before_digest
            {
                return Err(integrity());
            }
        }
        let acknowledged: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM handler_receipts WHERE adapter=?1 AND event_id=?2)",
            params![adapter, event],
            |r| r.get(0),
        )?;
        if acknowledged {
            return Err(integrity());
        }
        Ok(Some(record))
    }
    fn actor_pending_effects_for_disposition(
        &self,
        adapter: &str,
        event: &str,
    ) -> Result<Vec<EffectIntent>> {
        self.read_budget.request()?;
        let unresolved: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM effect_intents WHERE adapter=?1 AND state='unknown')",
            [adapter],
            |r| r.get(0),
        )?;
        if unresolved {
            return Err(err(
                "E_EFFECT_UNKNOWN",
                "reconcile actual external outcomes before actor disposition",
            ));
        }
        let elsewhere:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM effect_intents WHERE adapter=?1 AND event_id<>?2 AND state='pending')",params![adapter,event],|r|r.get(0))?;
        if elsewhere {
            return Err(integrity());
        }
        self.actor_disposition_effects(adapter, event)
    }
    fn actor_disposition_effects(&self, adapter: &str, event: &str) -> Result<Vec<EffectIntent>> {
        let definition = self.actor_definition(adapter)?;
        let mut statement=self.conn.prepare("SELECT substr(id,1,513) FROM effect_intents WHERE adapter=?1 AND event_id=?2 ORDER BY id LIMIT 33")?;
        let ids = statement
            .query_map(params![adapter, event], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if ids.len() > 32 {
            return Err(err("E_BUDGET", "actor disposition effect count exceeded"));
        }
        let mut effects = Vec::new();
        for id in ids {
            self.read_budget.request()?;
            let size:i64=self.conn.query_row("SELECT length(CAST(payload AS BLOB))+coalesce(length(CAST(response AS BLOB)),0) FROM effect_intents WHERE id=?1",[&id],|r|r.get(0))?;
            if size > 2 * 1024 * 1024 {
                return Err(err("E_BUDGET", "actor disposition effect exceeds limit"));
            }
            self.read_budget.charge(size as usize)?;
            let effect = self.effect_intent(&id)?.ok_or_else(integrity)?;
            if effect.adapter != adapter
                || effect.event_id != event
                || !definition
                    .manifest
                    .effect_destinations
                    .contains(&effect.destination)
                || effect.id
                    != format!(
                        "effect:{:x}",
                        Sha256::digest(serde_json::to_vec(&(adapter, &effect.idempotency_key))?)
                    )
                || !matches!(effect.state.as_str(), "pending" | "confirmed" | "failed")
                || (effect.state == "pending") != effect.response.is_none()
            {
                return Err(integrity());
            }
            effects.push(effect);
        }
        json_size(&effects, 65 * 1024 * 1024)?;
        Ok(effects)
    }
    pub fn cancel_recorded_actor_delivery_for(
        &self,
        request: &DeliveryCancellationRequest,
        host: &HostContext,
    ) -> Result<DeliveryCancellationReceipt> {
        self.cancel_recorded_actor_boundary(request, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn cancel_recorded_actor_delivery_test_before_commit(
        &self,
        request: &DeliveryCancellationRequest,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<DeliveryCancellationReceipt> {
        self.cancel_recorded_actor_boundary(request, host, before_commit)
    }
    fn cancel_recorded_actor_boundary(
        &self,
        request: &DeliveryCancellationRequest,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<DeliveryCancellationReceipt> {
        let _budget = self.read_budget.enter();
        json_size(request, 4096)?;
        if ![
            &request.adapter,
            &request.event,
            &request.expected_lease,
            &request.nonce,
        ]
        .iter()
        .all(|s| valid_id(s))
        {
            return Err(err("E_CANCELLATION", "invalid actor cleanup identity"));
        }
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _clock = self.operation_write_scope()?;
            self.require_adapter_host(&request.adapter, host)?;
            self.reject_governed_effect_adapter(&request.adapter)?;
            let definition = self.actor_definition(&request.adapter)?;
            if let Some(old) = self.actor_cancellation(&request.adapter, &request.event)? {
                if old.request != *request || old.principal != host.principal {
                    return Err(err(
                        "E_RECEIPT_CONFLICT",
                        "actor cleanup identity binds another disposition",
                    ));
                }
                let mut receipt = old.receipt;
                receipt.duplicate = true;
                before_commit();
                return Ok(receipt);
            }
            self.require_uncanceled_delivery(&request.adapter, &request.event)?;
            let (_, lifecycle, checkpoint) = self.dispatch_manifest(&request.adapter)?;
            if lifecycle == "removed" {
                return Err(err("E_CANCELLATION", "active recorded actor required"));
            }
            let before = self.actor_state(&request.adapter)?.ok_or_else(integrity)?;
            let completed:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM handler_receipts WHERE adapter=?1 AND event_id=?2) OR EXISTS(SELECT 1 FROM recorded_actor_cancellations WHERE adapter=?1 AND nonce=?3)",params![request.adapter,request.event,request.nonce],|r|r.get(0))?;
            if completed {
                return Err(err(
                    "E_RECEIPT_CONFLICT",
                    "actor delivery or nonce already disposed",
                ));
            }
            let pending: Option<(String, String)> = self
                .conn
                .query_row(
                    "SELECT event_id,lease FROM dispatch_pending WHERE adapter=?1",
                    [&request.adapter],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if pending.as_ref() != Some(&(request.event.clone(), request.expected_lease.clone())) {
                return Err(err("E_CONFLICT", "actor occurrence or lease changed"));
            }
            let (graph, branch, revision, sequence): (String, String, String, i64) =
                self.conn.query_row(
                    "SELECT graph_id,branch_id,revision,sequence FROM events WHERE event_id=?1",
                    [&request.event],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?;
            if sequence <= checkpoint
                || !definition
                    .manifest
                    .subscriptions
                    .contains(&SubscriptionScope {
                        graph_id: graph.clone(),
                        branch_id: branch,
                    })
            {
                return Err(integrity());
            }
            let effects =
                self.actor_pending_effects_for_disposition(&request.adapter, &request.event)?;
            let response = serde_json::to_string(&canceled_response(request, &host.principal)?)?;
            for effect in &effects {
                if effect.state == "pending" {
                    let changed=self.conn.execute("UPDATE effect_intents SET state='failed',response=?2 WHERE id=?1 AND state='pending' AND response IS NULL",params![effect.id,response])?;
                    if changed != 1 {
                        return Err(integrity());
                    }
                }
            }
            let mut witnesses = Vec::new();
            for before in &effects {
                let mut after = before.clone();
                if before.state == "pending" {
                    after.state = "failed".into();
                    after.response = Some(canceled_response(request, &host.principal)?);
                }
                witnesses.push(EffectDisposition {
                    id: before.id.clone(),
                    before_state: before.state.clone(),
                    before_digest: retention::hash(before)?,
                    after_digest: retention::hash(&after)?,
                });
            }
            let receipt = DeliveryCancellationReceipt {
                adapter: request.adapter.clone(),
                event: request.event.clone(),
                receipt_id: receipt_id(request, &host.principal)?,
                rebuild_required: true,
                duplicate: false,
            };
            let record = Cancellation {
                request: request.clone(),
                principal: host.principal.clone(),
                definition,
                source: GraphRef {
                    graph_id: graph,
                    revision,
                },
                before: before.clone(),
                before_checkpoint: checkpoint,
                checkpoint: sequence,
                effects: witnesses,
                receipt: receipt.clone(),
            };
            let bytes = json_size(&record, RECORD_LIMIT)?;
            let (count,total):(i64,i64)=self.conn.query_row("SELECT count(*),coalesce(sum(length(CAST(body AS BLOB))),0) FROM recorded_actor_cancellations WHERE adapter=?1",[&request.adapter],|r|Ok((r.get(0)?,r.get(1)?)))?;
            if count >= 128
                || total
                    .checked_add(bytes as i64)
                    .is_none_or(|n| n > 64 * 1024 * 1024)
            {
                return Err(err("E_BUDGET", "actor disposition quota exceeded"));
            }
            validate_record(
                &record,
                &request.adapter,
                &request.event,
                &retention::hash(&record)?,
            )?;
            self.advance_projection_scan_checkpoint(&request.adapter, sequence)?;
            self.conn.execute(
                "UPDATE dispatch_adapters SET state='paused' WHERE id=?1",
                [&request.adapter],
            )?;
            self.conn.execute("INSERT INTO projection_rebuild_requests VALUES (?1,?2) ON CONFLICT(adapter) DO UPDATE SET event_id=excluded.event_id",params![request.adapter,request.event])?;
            self.conn.execute(
                "DELETE FROM dispatch_pending WHERE adapter=?1",
                [&request.adapter],
            )?;
            self.conn.execute(
                "INSERT INTO recorded_actor_cancellations VALUES (?1,?2,?3,?4,?5)",
                params![
                    request.adapter,
                    request.event,
                    request.nonce,
                    serde_json::to_string(&record)?,
                    retention::hash(&record)?
                ],
            )?;
            before_commit();
            Ok(receipt)
        }));
        let result = operation_clock::rollback_unwind(outcome, &self.conn, "ROLLBACK");
        match result {
            Ok(receipt) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(receipt)
            }
            Err(error) => {
                self.conn.execute_batch("ROLLBACK")?;
                Err(error)
            }
        }
    }
}
