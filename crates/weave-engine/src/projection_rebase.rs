//! Explicit trusted projection-state rebuild. No effect-enabled historical replay.
use super::*;
use serde::{Deserialize, Serialize};
const STATE_LIMIT: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionRebaseInputs {
    pub adapter: String,
    /// Opaque retention-policy epoch; never a global event coordinate or row count.
    pub epoch: String,
    pub manifest_digest: String,
    pub snapshots: Vec<GraphRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionRebaseRequest {
    pub inputs: ProjectionRebaseInputs,
    pub state_revision: String,
    pub state: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionRebaseReceipt {
    pub epoch: String,
    pub binding_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionCompletionRequest {
    pub adapter: String,
    pub event: String,
    pub lease: String,
    pub prior_state_digest: String,
    pub state_revision: String,
    pub state: serde_json::Value,
    pub input_snapshots: Vec<GraphRef>,
    pub program: Program,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct StoredCompletion {
    request_digest: String,
    before: ProjectionRebaseRequest,
    after: ProjectionRebaseRequest,
    handler_results_digest: String,
}
pub(crate) fn validate_retained_receipt(
    adapter: &str,
    request_digest: &str,
    body_digest: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let stored: StoredCompletion = serde_json::from_value(value.clone()).map_err(|_| {
        err(
            "E_REBASE_INTEGRITY",
            "projection receipt binding unavailable",
        )
    })?;
    if retention::hash(&stored)? != body_digest
        || stored.request_digest != request_digest
        || stored.before.inputs.adapter != adapter
        || stored.after.inputs.adapter != adapter
        || stored.before.inputs.manifest_digest != stored.after.inputs.manifest_digest
        || stored.before.inputs.epoch != stored.after.inputs.epoch
    {
        return Err(err(
            "E_REBASE_INTEGRITY",
            "projection receipt binding unavailable",
        ));
    }
    Ok(())
}
/// Only this module can create an authorization for a checked state completion.
pub(crate) struct CompletionToken {
    adapter: String,
    event: String,
}
impl CompletionToken {
    pub(crate) fn authorizes(&self, adapter: &str, event: &str) -> bool {
        self.adapter == adapter && self.event == event
    }
}
fn completion_digest(request: &ProjectionCompletionRequest) -> Result<String> {
    // A redelivered occurrence has a new lease but the same immutable operation.
    retention::hash(&(
        &request.adapter,
        &request.event,
        &request.prior_state_digest,
        &request.state_revision,
        &request.state,
        &request.input_snapshots,
        &request.program,
    ))
}
impl Engine {
    fn projection_manifest(&self, adapter: &str, host: &HostContext) -> Result<AdapterManifest> {
        self.require_adapter_host(adapter, host)?;
        self.reject_governed_effect_adapter(adapter)?;
        if self.is_compiled_handler(adapter)? {
            return Err(err(
                "E_REBASE_MODE",
                "native projection state requires a native projection manifest",
            ));
        }
        let (manifest, state, _) = self.dispatch_manifest(adapter)?;
        if !manifest.projection_replay
            || !manifest.effect_destinations.is_empty()
            || state == "removed"
        {
            return Err(err(
                "E_REBASE_MODE",
                "a pure projection manifest is required",
            ));
        }
        Ok(manifest)
    }
    fn projection_inputs(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<ProjectionRebaseInputs> {
        let manifest = self.projection_manifest(adapter, host)?;
        let mut snapshots = Vec::new();
        for scope in &manifest.subscriptions {
            let revision = self
                .head(&scope.graph_id, &scope.branch_id)?
                .ok_or_else(|| err("E_UNAVAILABLE", "rebuild input unavailable"))?;
            let reference = GraphRef {
                graph_id: scope.graph_id.clone(),
                revision,
            };
            self.retention_whole(&reference, host)?;
            snapshots.push(reference);
        }
        snapshots.sort_by(|a, b| (&a.graph_id, &a.revision).cmp(&(&b.graph_id, &b.revision)));
        snapshots.dedup();
        Ok(ProjectionRebaseInputs {
            adapter: adapter.into(),
            epoch: self.retention_replay_epoch()?,
            manifest_digest: retention::hash(&manifest)?,
            snapshots,
        })
    }
    /// Current authorized rebuild inputs for one fixed-owner projection, with no global offset.
    pub fn projection_rebase_inputs_for(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<ProjectionRebaseInputs> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.projection_inputs(adapter, host)
    }
    /// Atomically install explicitly rebuilt state and its matching checkpoint/artifact pair.
    /// The trusted adapter supplies its actual projection state; the kernel checks pins and authority.
    pub fn rebase_projection_for(
        &self,
        request: &ProjectionRebaseRequest,
        host: &HostContext,
    ) -> Result<ProjectionRebaseReceipt> {
        self.rebase_projection_boundary(request, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn rebase_projection_test_before_commit(
        &self,
        request: &ProjectionRebaseRequest,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<ProjectionRebaseReceipt> {
        self.rebase_projection_boundary(request, host, before_commit)
    }
    fn rebase_projection_boundary(
        &self,
        request: &ProjectionRebaseRequest,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<ProjectionRebaseReceipt> {
        let _budget = self.read_budget.enter();
        json_size(request, STATE_LIMIT)?;
        if !valid_id(&request.state_revision) {
            return Err(err("E_REBASE_STATE", "invalid state revision"));
        }
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_write_scope()?;
        let actual = self.projection_inputs(&request.inputs.adapter, host)?;
        if actual != request.inputs {
            return Err(err("E_CONFLICT", "projection inputs or policy changed"));
        }
        let adapter = &actual.adapter;
        let pending: i64 = self.conn.query_row("SELECT (SELECT count(*) FROM dispatch_pending WHERE adapter=?1)+(SELECT count(*) FROM governance_delivery_pending WHERE adapter=?1)+(SELECT count(*) FROM effect_intents WHERE adapter=?1 AND state IN ('pending','unknown'))",[adapter],|r|r.get(0))?;
        if pending != 0 {
            return Err(err(
                "E_REBASE_PENDING",
                "resolve in-flight work before rebasing",
            ));
        }
        let last: i64 =
            self.conn
                .query_row("SELECT coalesce(max(sequence),0) FROM events", [], |r| {
                    r.get(0)
                })?;
        let digest = retention::hash(request)?;
        self.conn.execute(
            "INSERT INTO retention_stateful_adapters VALUES (?1) ON CONFLICT(adapter) DO NOTHING",
            [adapter],
        )?;
        self.conn.execute("INSERT INTO retention_adapter_states VALUES (?1,?2,?3,?4,?5) ON CONFLICT(adapter) DO UPDATE SET epoch=excluded.epoch,body=excluded.body,digest=excluded.digest,checkpoint=excluded.checkpoint",params![adapter,actual.epoch,serde_json::to_string(request)?,retention::hash(&(request,last))?,last])?;
        self.conn.execute(
            "UPDATE dispatch_adapters SET checkpoint=?2 WHERE id=?1",
            params![adapter, last],
        )?;
        before_commit();
        tx.commit()?;
        Ok(ProjectionRebaseReceipt {
            epoch: actual.epoch,
            binding_digest: digest,
        })
    }
    pub(crate) fn projection_state(
        &self,
        adapter: &str,
    ) -> Result<Option<ProjectionRebaseRequest>> {
        self.read_budget.request()?;
        let row: Option<(String,Option<String>,String,i64)> = self.conn.query_row("SELECT substr(epoch,1,49),CASE WHEN length(CAST(body AS BLOB))<=?2 THEN body END,substr(digest,1,129),checkpoint FROM retention_adapter_states WHERE adapter=?1",params![adapter,STATE_LIMIT as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((epoch, body, digest, checkpoint)) = row else {
            return Ok(None);
        };
        let body = body.ok_or_else(|| err("E_BUDGET", "projection state exceeds budget"))?;
        self.read_budget.charge(body.len())?;
        let state: ProjectionRebaseRequest = serde_json::from_str(&body)?;
        let (manifest, _, actual_checkpoint) = self.dispatch_manifest(adapter)?;
        if state.inputs.adapter != adapter
            || state.inputs.epoch != epoch
            || checkpoint < 0
            || checkpoint != actual_checkpoint
            || retention::hash(&(&state, checkpoint))? != digest
            || state.inputs.manifest_digest != retention::hash(&manifest)?
        {
            return Err(err(
                "E_REBASE_INTEGRITY",
                "projection state binding unavailable",
            ));
        }
        Ok(Some(state))
    }
    /// Skipped private/nonmatching events advance the private coordinate without
    /// changing the public state binding or epoch. Both coordinates commit together.
    pub(crate) fn advance_projection_scan_checkpoint(
        &self,
        adapter: &str,
        checkpoint: i64,
    ) -> Result<()> {
        let state = self.projection_state(adapter)?;
        self.conn.execute(
            "UPDATE dispatch_adapters SET checkpoint=?2 WHERE id=?1",
            params![adapter, checkpoint],
        )?;
        if let Some(state) = state {
            self.conn.execute(
                "UPDATE retention_adapter_states SET checkpoint=?2,digest=?3 WHERE adapter=?1",
                params![adapter, checkpoint, retention::hash(&(&state, checkpoint))?],
            )?;
        }
        Ok(())
    }
    pub(crate) fn require_replay_checkpoint(&self, adapter: &str) -> Result<()> {
        let (_, policy) = retention::state(&self.conn)?;
        let state = self.projection_state(adapter)?;
        let required: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM retention_stateful_adapters WHERE adapter=?1)",
            [adapter],
            |r| r.get(0),
        )?;
        if required && state.is_none() {
            return Err(err(
                "E_REBASE_INTEGRITY",
                "current projection state unavailable",
            ));
        }
        if policy == RetentionPolicy::default() && state.is_none() {
            return Ok(());
        }
        let ready = state.is_some_and(|state| {
            self.retention_replay_epoch()
                .is_ok_and(|epoch| state.inputs.epoch == epoch)
        });
        if !ready {
            return Err(err(
                "E_CHECKPOINT_EXPIRED",
                "explicit snapshot rebuild is required",
            ));
        }
        Ok(())
    }
    /// Recheck current whole-input authority before returning even an empty stored projection state.
    pub fn projection_state_for(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<ProjectionRebaseRequest> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.projection_manifest(adapter, host)?;
        self.require_replay_checkpoint(adapter)?;
        let state = self
            .projection_state(adapter)?
            .ok_or_else(|| err("E_REBASE_STATE", "projection state unavailable"))?;
        for reference in &state.inputs.snapshots {
            self.retention_whole(reference, host)?;
        }
        Ok(state)
    }
    fn completion_state(
        &self,
        request: &ProjectionCompletionRequest,
        before: &ProjectionRebaseRequest,
        results: &[CommandResult],
        event: GraphRef,
        host: &HostContext,
    ) -> Result<ProjectionRebaseRequest> {
        let mut snapshots = before.inputs.snapshots.clone();
        snapshots.extend(request.input_snapshots.iter().cloned());
        snapshots.push(event);
        let mut values = Vec::new();
        let mut revisions = Vec::new();
        for result in results {
            match result {
                CommandResult::Queried { result } => values.push(result.as_ref()),
                CommandResult::HistoryRanged { range, .. } => {
                    values.push(&range.start_state);
                    values.extend(range.changes.iter());
                }
                CommandResult::Committed { revision, .. }
                | CommandResult::Unchanged { revision } => revisions.push(revision.as_str()),
                CommandResult::BatchCommitted { commits, .. }
                | CommandResult::BatchUnchanged { commits } => {
                    snapshots.extend(commits.iter().map(|c| GraphRef {
                        graph_id: c.graph_id.clone(),
                        revision: c.revision.clone(),
                    }));
                }
            }
        }
        for value in values {
            self.require_current_result_authority(value, host)?;
            snapshots.extend(value.input_snapshots.iter().cloned());
            snapshots.extend(value.snapshots.iter().map(|(graph_id, revision)| GraphRef {
                graph_id: graph_id.clone(),
                revision: revision.clone(),
            }));
            snapshots.extend(value.metadata_graphs.iter().map(|g| g.reference.clone()));
            snapshots.extend(value.recorded_observations.iter().map(|o| o.graph.clone()));
            for witness in &value.accepted_observations {
                snapshots.push(witness.occurrence.clone());
                snapshots.push(witness.source.clone());
            }
        }
        for revision in revisions {
            let graph_id: String = self.conn.query_row(
                "SELECT graph_id FROM revisions WHERE revision=?1",
                [revision],
                |r| r.get(0),
            )?;
            snapshots.push(GraphRef {
                graph_id,
                revision: revision.into(),
            });
        }
        snapshots.sort_by(|a, b| (&a.graph_id, &a.revision).cmp(&(&b.graph_id, &b.revision)));
        snapshots.dedup();
        if snapshots.len() > 1000 {
            return Err(err("E_BUDGET", "projection input pin limit exceeded"));
        }
        for reference in &snapshots {
            self.retention_whole(reference, host)?;
        }
        let after = ProjectionRebaseRequest {
            inputs: ProjectionRebaseInputs {
                snapshots,
                ..before.inputs.clone()
            },
            state_revision: request.state_revision.clone(),
            state: request.state.clone(),
        };
        json_size(&after, STATE_LIMIT)?;
        Ok(after)
    }
    /// A trusted native projection supplies actual state and commands; the kernel commits
    /// state, retained inputs, outputs, receipt and delivery checkpoint in one transaction.
    pub fn complete_projection_for(
        &mut self,
        request: &ProjectionCompletionRequest,
        host: &HostContext,
    ) -> Result<HandlerReceipt> {
        self.complete_projection_boundary(request, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn complete_projection_test_before_commit(
        &mut self,
        request: &ProjectionCompletionRequest,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<HandlerReceipt> {
        self.complete_projection_boundary(request, host, before_commit)
    }
    fn complete_projection_boundary(
        &mut self,
        request: &ProjectionCompletionRequest,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<HandlerReceipt> {
        let _budget = self.read_budget.enter();
        json_size(request, STATE_LIMIT)?;
        if !valid_id(&request.state_revision) || request.input_snapshots.len() > 1000 {
            return Err(err(
                "E_REBASE_STATE",
                "invalid projection state or input pins",
            ));
        }
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _clock = self.operation_write_scope()?;
            let manifest = self.projection_manifest(&request.adapter, host)?;
            let (graph_id, _, revision, _) = self
                .scoped_event(&manifest, &request.event)?
                .ok_or_else(|| err("E_UNAVAILABLE", "delivery unavailable"))?;
            let event = GraphRef { graph_id, revision };
            let digest = completion_digest(request)?;
            self.read_budget.request()?;
            let row: Option<(String,Option<String>,String)> = self.conn.query_row(
                "SELECT substr(request_digest,1,129),CASE WHEN length(CAST(body AS BLOB))<=?3 THEN body END,substr(digest,1,129) FROM retention_projection_receipts WHERE adapter=?1 AND event_id=?2",
                params![request.adapter,request.event,(3*STATE_LIMIT) as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            let token = CompletionToken {
                adapter: request.adapter.clone(),
                event: request.event.clone(),
            };
            if let Some((old, body, body_digest)) = row {
                self.projection_state(&request.adapter)?.ok_or_else(|| {
                    err("E_REBASE_INTEGRITY", "current projection state unavailable")
                })?;
                if old != digest {
                    return Err(err(
                        "E_RECEIPT_CONFLICT",
                        "processed projection has different state or commands",
                    ));
                }
                let body =
                    body.ok_or_else(|| err("E_BUDGET", "projection receipt exceeds budget"))?;
                self.read_budget.charge(body.len())?;
                let stored: StoredCompletion = serde_json::from_str(&body)?;
                if retention::hash(&stored)? != body_digest
                    || stored.request_digest != digest
                    || retention::hash(&stored.before)? != request.prior_state_digest
                    || stored.before.inputs.adapter != request.adapter
                    || stored.before.inputs.manifest_digest != retention::hash(&manifest)?
                {
                    return Err(err(
                        "E_REBASE_INTEGRITY",
                        "projection receipt binding unavailable",
                    ));
                }
                let exists: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM handler_receipts WHERE adapter=?1 AND event_id=?2)",params![request.adapter,request.event],|r|r.get(0))?;
                if !exists {
                    return Err(err(
                        "E_REBASE_INTEGRITY",
                        "projection completion receipt unavailable",
                    ));
                }
                let receipt = self.complete_handler_with_projection(
                    &request.adapter,
                    &request.event,
                    &request.lease,
                    &request.program,
                    Some(&token),
                )?;
                if !receipt.duplicate
                    || retention::hash(&receipt.results)? != stored.handler_results_digest
                    || self.completion_state(
                        request,
                        &stored.before,
                        &receipt.results,
                        event,
                        host,
                    )? != stored.after
                {
                    return Err(err(
                        "E_REBASE_INTEGRITY",
                        "projection receipt binding unavailable",
                    ));
                }
                // Historical duplicates return their receipt without restoring older state.
                before_commit();
                return Ok(receipt);
            }
            // This genuine leased occurrence was already selected before any
            // policy transition. Its retained inputs may finish once; the next
            // new poll still requires an explicit rebuild for the new epoch.
            self.check_lease(&request.adapter, &request.event, &request.lease)?;
            let before = self.projection_state(&request.adapter)?.ok_or_else(|| {
                err(
                    "E_REBASE_STATE",
                    "explicit projection state rebuild is required",
                )
            })?;
            if retention::hash(&before)? != request.prior_state_digest {
                return Err(err("E_CONFLICT", "projection state or policy changed"));
            }
            for reference in before
                .inputs
                .snapshots
                .iter()
                .chain(request.input_snapshots.iter())
            {
                self.retention_whole(reference, host)?;
            }
            let used: (i64,i64) = self.conn.query_row("SELECT count(*),coalesce(sum(length(CAST(body AS BLOB))),0) FROM retention_projection_receipts WHERE adapter=?1",[&request.adapter],|r|Ok((r.get(0)?,r.get(1)?)))?;
            if used.0 >= 10_000 {
                return Err(err("E_BUDGET", "projection receipt count limit exceeded"));
            }
            let receipt = self.complete_handler_with_projection(
                &request.adapter,
                &request.event,
                &request.lease,
                &request.program,
                Some(&token),
            )?;
            if receipt.duplicate {
                return Err(err(
                    "E_REBASE_INTEGRITY",
                    "projection state receipt is missing",
                ));
            }
            let after = self.completion_state(request, &before, &receipt.results, event, host)?;
            let stored = StoredCompletion {
                request_digest: digest.clone(),
                before,
                after: after.clone(),
                handler_results_digest: retention::hash(&receipt.results)?,
            };
            let body = serde_json::to_string(&stored)?;
            if body.len() > 3 * STATE_LIMIT
                || used
                    .1
                    .checked_add(body.len() as i64)
                    .is_none_or(|n| n > 64 * 1024 * 1024)
            {
                return Err(err("E_BUDGET", "projection receipt byte limit exceeded"));
            }
            let checkpoint = self.dispatch_manifest(&request.adapter)?.2;
            self.conn.execute(
                "UPDATE retention_adapter_states SET body=?2,digest=?3,checkpoint=?4 WHERE adapter=?1",
                params![
                    request.adapter,
                    serde_json::to_string(&after)?,
                    retention::hash(&(&after,checkpoint))?,checkpoint
                ],
            )?;
            self.conn.execute(
                "INSERT INTO retention_projection_receipts VALUES (?1,?2,?3,?4,?5)",
                params![
                    request.adapter,
                    request.event,
                    digest,
                    body,
                    retention::hash(&stored)?
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
