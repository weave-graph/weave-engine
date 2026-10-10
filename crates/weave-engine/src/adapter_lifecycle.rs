//! Explicit trusted owner cleanup. Cancellation is an audited disposition, never an effect retry.
use super::*;
use serde::{Deserialize, Serialize};
const RECORD_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryCancellationReason {
    StaleOutput,
    Superseded,
    OwnerStop,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeliveryCancellationRequest {
    pub adapter: String,
    pub event: String,
    /// Compare the actual pending lease, even after expiry; a renewed worker is fenced separately.
    pub expected_lease: String,
    pub nonce: String,
    pub reason: DeliveryCancellationReason,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeliveryCancellationReceipt {
    pub adapter: String,
    pub event: String,
    pub receipt_id: String,
    pub rebuild_required: bool,
    pub duplicate: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancellation {
    request: DeliveryCancellationRequest,
    principal: String,
    manifest_digest: String,
    source: GraphRef,
    prior_state: Option<ProjectionRebaseRequest>,
    #[serde(default, skip_serializing_if = "is_false")]
    compiled_rebuild: bool,
    receipt: DeliveryCancellationReceipt,
}
fn is_false(value: &bool) -> bool {
    !*value
}
fn integrity() -> Error {
    err("E_LIFECYCLE_INTEGRITY", "lifecycle record unavailable")
}
pub(crate) fn validate_retained_cancellation(
    adapter: &str,
    event: &str,
    nonce: &str,
    source: &GraphRef,
    digest: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let record: Cancellation = serde_json::from_value(value.clone()).map_err(|_| integrity())?;
    if retention::hash(&record)? != digest
        || record.request.adapter != adapter
        || record.request.event != event
        || record.request.nonce != nonce
        || record.source != *source
        || record.receipt.adapter != adapter
        || record.receipt.event != event
        || record.receipt.duplicate
        || record.receipt.rebuild_required
            != (record.prior_state.is_some() || record.compiled_rebuild)
        || record.receipt.receipt_id
            != retention::hash(&(
                "weave-delivery-cancellation/1",
                &record.principal,
                &record.request,
            ))?
        || record.prior_state.as_ref().is_some_and(|state| {
            state.inputs.adapter != adapter
                || state.inputs.manifest_digest != record.manifest_digest
        })
    {
        return Err(integrity());
    }
    Ok(())
}
impl Engine {
    pub(crate) fn initialize_adapter_lifecycle(&self, version: i64) -> Result<()> {
        let present: i64 = self.conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('delivery_cancellations','projection_rebuild_requests','projection_migrations')", [], |r| r.get(0))?;
        if (version < 23 && present != 0) || (version >= 23 && present != 3) {
            return Err(integrity());
        }
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS delivery_cancellations(adapter TEXT NOT NULL REFERENCES dispatch_adapters(id),event_id TEXT NOT NULL,nonce TEXT NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(adapter,event_id),UNIQUE(adapter,nonce));
CREATE TABLE IF NOT EXISTS projection_rebuild_requests(adapter TEXT PRIMARY KEY REFERENCES dispatch_adapters(id),event_id TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS projection_migrations(source_adapter TEXT PRIMARY KEY REFERENCES dispatch_adapters(id),destination_adapter TEXT UNIQUE NOT NULL REFERENCES dispatch_adapters(id),principal TEXT NOT NULL,nonce TEXT NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,UNIQUE(principal,nonce));")?;
        Ok(())
    }
    pub(crate) fn require_uncanceled_delivery(&self, adapter: &str, event: &str) -> Result<()> {
        let canceled: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM delivery_cancellations WHERE adapter=?1 AND event_id=?2)",
            params![adapter, event],
            |r| r.get(0),
        )?;
        if canceled {
            return Err(err(
                "E_DELIVERY_CANCELED",
                "delivery was explicitly canceled",
            ));
        }
        Ok(())
    }
    /// Fixed-owner cleanup returns no source payload and remains possible after source read expiry.
    /// Only pure graph handlers are supported. Pending/unknown effects are never disposed here.
    pub fn cancel_handler_delivery_for(
        &self,
        request: &DeliveryCancellationRequest,
        host: &HostContext,
    ) -> Result<DeliveryCancellationReceipt> {
        self.cancel_handler_delivery_boundary(request, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn cancel_handler_delivery_test_before_commit(
        &self,
        request: &DeliveryCancellationRequest,
        host: &HostContext,
        before: impl FnOnce(),
    ) -> Result<DeliveryCancellationReceipt> {
        self.cancel_handler_delivery_boundary(request, host, before)
    }
    fn cancel_handler_delivery_boundary(
        &self,
        request: &DeliveryCancellationRequest,
        host: &HostContext,
        before: impl FnOnce(),
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
        .all(|x| valid_id(x))
        {
            return Err(err("E_CANCELLATION", "invalid cleanup identity"));
        }
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_write_scope()?;
        self.require_adapter_host(&request.adapter, host)?;
        self.reject_governed_effect_adapter(&request.adapter)?;
        let (manifest, _, checkpoint) = self.dispatch_manifest(&request.adapter)?;
        if !manifest.effect_destinations.is_empty() {
            return Err(err(
                "E_CANCELLATION_MODE",
                "pure graph handler cleanup required",
            ));
        }
        self.read_budget.request()?;
        let prior: Option<(Option<String>,String)> = self.conn.query_row("SELECT CASE WHEN length(CAST(body AS BLOB))<=?3 THEN body END,substr(digest,1,129) FROM delivery_cancellations WHERE adapter=?1 AND (event_id=?2 OR nonce=?4)",params![request.adapter,request.event,RECORD_LIMIT as i64,request.nonce],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((body, digest)) = prior {
            let body = body.ok_or_else(|| err("E_BUDGET", "cleanup record exceeds budget"))?;
            self.read_budget.charge(body.len())?;
            let old: Cancellation = serde_json::from_str(&body).map_err(|_| integrity())?;
            if retention::hash(&old)? != digest
                || old.principal != host.principal
                || old.manifest_digest != retention::hash(&manifest)?
            {
                return Err(integrity());
            }
            if old.request != *request {
                return Err(err(
                    "E_RECEIPT_CONFLICT",
                    "cleanup identity binds another disposition",
                ));
            }
            tx.commit()?;
            return Ok(DeliveryCancellationReceipt {
                duplicate: true,
                ..old.receipt
            });
        }
        let completed: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM handler_receipts WHERE adapter=?1 AND event_id=?2)",
            params![request.adapter, request.event],
            |r| r.get(0),
        )?;
        if completed {
            return Err(err(
                "E_CANCELLATION",
                "completed delivery cannot be canceled",
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
            return Err(err("E_CONFLICT", "pending delivery or lease changed"));
        }
        let effects: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM effect_intents WHERE adapter=?1 AND state IN ('pending','unknown'))",[&request.adapter],|r|r.get(0))?;
        if effects {
            return Err(err(
                "E_CANCELLATION_MODE",
                "resolve effect work through its own protocol",
            ));
        }
        let (graph, branch, revision, sequence): (String, String, String, i64) =
            self.conn.query_row(
                "SELECT graph_id,branch_id,revision,sequence FROM events WHERE event_id=?1",
                [&request.event],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        if !manifest
            .subscriptions
            .iter()
            .any(|s| s.graph_id == graph && s.branch_id == branch)
        {
            return Err(integrity());
        }
        let prior_state = self.projection_state(&request.adapter)?;
        let compiled_rebuild = self.has_compiled_replay_state(&request.adapter)?;
        let required: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM retention_stateful_adapters WHERE adapter=?1)",
            [&request.adapter],
            |r| r.get(0),
        )?;
        if required && prior_state.is_none() {
            return Err(integrity());
        }
        let receipt = DeliveryCancellationReceipt {
            adapter: request.adapter.clone(),
            event: request.event.clone(),
            receipt_id: retention::hash(&(
                "weave-delivery-cancellation/1",
                &host.principal,
                request,
            ))?,
            rebuild_required: prior_state.is_some() || compiled_rebuild,
            duplicate: false,
        };
        let record = Cancellation {
            request: request.clone(),
            principal: host.principal.clone(),
            manifest_digest: retention::hash(&manifest)?,
            source: GraphRef {
                graph_id: graph,
                revision,
            },
            prior_state,
            compiled_rebuild,
            receipt: receipt.clone(),
        };
        let bytes = json_size(&record, RECORD_LIMIT)?;
        let used:(i64,i64)=self.conn.query_row("SELECT count(*),coalesce(sum(length(CAST(body AS BLOB))),0) FROM delivery_cancellations WHERE adapter=?1",[&request.adapter],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if used.0 >= 10000
            || used
                .1
                .checked_add(bytes as i64)
                .is_none_or(|n| n > 64 * 1024 * 1024)
        {
            return Err(err("E_BUDGET", "cleanup receipt quota exceeded"));
        }
        self.conn.execute(
            "INSERT INTO delivery_cancellations VALUES (?1,?2,?3,?4,?5)",
            params![
                request.adapter,
                request.event,
                request.nonce,
                serde_json::to_string(&record)?,
                retention::hash(&record)?
            ],
        )?;
        self.advance_projection_scan_checkpoint(&request.adapter, sequence.max(checkpoint))?;
        if receipt.rebuild_required {
            self.conn.execute("INSERT INTO projection_rebuild_requests VALUES (?1,?2) ON CONFLICT(adapter) DO UPDATE SET event_id=excluded.event_id",params![request.adapter,request.event])?;
        }
        self.conn.execute(
            "DELETE FROM dispatch_pending WHERE adapter=?1",
            [&request.adapter],
        )?;
        before();
        tx.commit()?;
        Ok(receipt)
    }
}
