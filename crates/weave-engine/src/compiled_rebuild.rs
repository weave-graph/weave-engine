//! Explicit kernel-computed pure snapshot reconstruction; no caller result authority.
use super::*;
use compiled_handlers::Registration;
use serde::{Deserialize, Serialize};
const RECORD_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledRebuildInputs {
    pub binding: CompiledMigrationInputs,
    pub expected_output: Option<GraphRef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledRebuildRequest {
    pub inputs: CompiledRebuildInputs,
    pub nonce: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledRebuildReceipt {
    pub receipt_id: String,
    pub definition_digest: String,
    pub epoch: String,
    pub output: GraphRef,
    pub coverage: Coverage,
    pub diagnostics: Vec<Diagnostic>,
    pub duplicate: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    request: CompiledRebuildRequest,
    registration: Registration,
    receipt: CompiledRebuildReceipt,
    checkpoint: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayState {
    binding: CompiledMigrationInputs,
    receipt_id: String,
    checkpoint: i64,
}
fn integrity() -> Error {
    err(
        "E_REBUILD_INTEGRITY",
        "compiled reconstruction binding unavailable",
    )
}
fn receipt_id(request: &CompiledRebuildRequest) -> Result<String> {
    retention::hash(&("weave-compiled-reconstruction/1", request))
}
fn validate_record(record: &Record, digest: &str) -> Result<()> {
    if retention::hash(record)? != digest
        || record.checkpoint < 0
        || record.receipt.duplicate
        || record.request.inputs.binding.source_adapter != record.registration.manifest.id
        || record.request.inputs.binding.source_binding_digest
            != compiled_lifecycle::binding(&record.registration)?
        || record.receipt.definition_digest != record.registration.template.definition_digest
        || record.receipt.epoch != record.request.inputs.binding.epoch
        || record.receipt.receipt_id != receipt_id(&record.request)?
        || record.receipt.output.graph_id != record.registration.output.graph_id
        || !valid_id(&record.request.nonce)
        || record.request.inputs.binding.primary_input.graph_id
            != record.registration.template.input.graph_id
        || !record
            .request
            .inputs
            .binding
            .input_snapshots
            .contains(&record.request.inputs.binding.primary_input)
        || record.request.inputs.binding.checkpoint_binding
            != retention::hash(&(
                "weave-compiled-checkpoint/1",
                &record.request.inputs.binding.source_adapter,
                &record.request.inputs.binding.source_binding_digest,
                &record.request.inputs.binding.epoch,
                &record.request.inputs.binding.primary_input,
                &record.request.inputs.binding.input_snapshots,
            ))?
    {
        return Err(integrity());
    }
    handler_registration::validate_handler_template(&record.registration.template)
        .map_err(|_| integrity())?;
    Ok(())
}
pub(crate) fn validate_retained_rebuild(
    engine: &Engine,
    table: &str,
    adapter: &str,
    nonce: &str,
    digest: &str,
    value: &serde_json::Value,
) -> Result<()> {
    if table == "compiled_rebuild_receipts" {
        let record: Record = serde_json::from_value(value.clone()).map_err(|_| integrity())?;
        validate_record(&record, digest)?;
        if record.request.inputs.binding.source_adapter != adapter
            || record.request.nonce != nonce
            || engine.compiled_rebuild_record(&record.receipt.receipt_id)? != record
        {
            return Err(integrity());
        }
    } else {
        let state: ReplayState = serde_json::from_value(value.clone()).map_err(|_| integrity())?;
        if retention::hash(&state)? != digest
            || state.binding.source_adapter != adapter
            || engine.compiled_replay_state(adapter)?.as_ref() != Some(&state)
        {
            return Err(integrity());
        }
    }
    Ok(())
}
impl Engine {
    pub(crate) fn initialize_compiled_rebuild(&self, version: i64) -> Result<()> {
        let present: i64=self.conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('compiled_rebuild_receipts','compiled_replay_states')",[],|r|r.get(0))?;
        if (version < 25 && present != 0) || (version >= 25 && present != 2) {
            return Err(integrity());
        }
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS compiled_rebuild_receipts(adapter TEXT NOT NULL REFERENCES compiled_handlers(adapter),nonce TEXT NOT NULL,receipt_id TEXT UNIQUE NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(adapter,nonce));
CREATE TABLE IF NOT EXISTS compiled_replay_states(adapter TEXT PRIMARY KEY REFERENCES compiled_handlers(adapter),epoch TEXT NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,checkpoint INTEGER NOT NULL);")?;
        Ok(())
    }
    fn compiled_rebuild_record(&self, id: &str) -> Result<Record> {
        self.read_budget.request()?;
        let row: Option<(Option<String>,String)>=self.conn.query_row("SELECT CASE WHEN length(CAST(body AS BLOB))<=?2 THEN body END,substr(digest,1,129) FROM compiled_rebuild_receipts WHERE receipt_id=?1",params![id,RECORD_LIMIT as i64],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (body, digest) = row.ok_or_else(integrity)?;
        let body =
            body.ok_or_else(|| err("E_BUDGET", "compiled reconstruction record exceeds budget"))?;
        self.read_budget.charge(body.len())?;
        let record: Record = serde_json::from_str(&body).map_err(|_| integrity())?;
        validate_record(&record, &digest)?;
        if record.receipt.receipt_id != id
            || self.handler_registration(&record.registration.manifest.id)? != record.registration
        {
            return Err(integrity());
        }
        Ok(record)
    }
    fn compiled_replay_state(&self, adapter: &str) -> Result<Option<ReplayState>> {
        self.read_budget.request()?;
        let row:Option<(String,Option<String>,String,i64)>=self.conn.query_row("SELECT substr(epoch,1,49),CASE WHEN length(CAST(body AS BLOB))<=?2 THEN body END,substr(digest,1,129),checkpoint FROM compiled_replay_states WHERE adapter=?1",params![adapter,RECORD_LIMIT as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((epoch, body, digest, checkpoint)) = row else {
            return Ok(None);
        };
        let body = body.ok_or_else(|| err("E_BUDGET", "compiled replay state exceeds budget"))?;
        self.read_budget.charge(body.len())?;
        let state: ReplayState = serde_json::from_str(&body).map_err(|_| integrity())?;
        let registration = self.handler_registration(adapter)?;
        let (_, _, actual_checkpoint) = self.dispatch_manifest(adapter)?;
        let origin = self.compiled_rebuild_record(&state.receipt_id)?;
        if retention::hash(&state)? != digest
            || state.checkpoint != checkpoint
            || checkpoint != actual_checkpoint
            || checkpoint < 0
            || state.binding.source_adapter != adapter
            || state.binding.epoch != epoch
            || state.binding.source_binding_digest != compiled_lifecycle::binding(&registration)?
            || origin.receipt.epoch != epoch
            || origin.registration.manifest.principal != registration.manifest.principal
            || origin.registration.template.input != registration.template.input
            || origin.registration.output != registration.output
            || origin.registration.template.protocol != registration.template.protocol
            || state.binding.primary_input.graph_id != registration.template.input.graph_id
            || !state
                .binding
                .input_snapshots
                .contains(&state.binding.primary_input)
            || state.binding.checkpoint_binding
                != retention::hash(&(
                    "weave-compiled-checkpoint/1",
                    &state.binding.source_adapter,
                    &state.binding.source_binding_digest,
                    &state.binding.epoch,
                    &state.binding.primary_input,
                    &state.binding.input_snapshots,
                ))?
        {
            return Err(integrity());
        }
        Ok(Some(state))
    }
    fn store_compiled_replay_state(&self, state: &ReplayState) -> Result<()> {
        json_size(state, RECORD_LIMIT)?;
        self.conn.execute("INSERT INTO compiled_replay_states VALUES (?1,?2,?3,?4,?5) ON CONFLICT(adapter) DO UPDATE SET epoch=excluded.epoch,body=excluded.body,digest=excluded.digest,checkpoint=excluded.checkpoint",params![state.binding.source_adapter,state.binding.epoch,serde_json::to_string(state)?,retention::hash(state)?,state.checkpoint])?;
        Ok(())
    }
    pub(crate) fn compiled_replay_ready(&self, adapter: &str) -> Result<bool> {
        Ok(self.compiled_replay_state(adapter)?.is_some_and(|s| {
            self.retention_replay_epoch()
                .is_ok_and(|epoch| s.binding.epoch == epoch)
        }))
    }
    pub(crate) fn has_compiled_replay_state(&self, adapter: &str) -> Result<bool> {
        Ok(self.compiled_replay_state(adapter)?.is_some())
    }
    pub(crate) fn advance_compiled_replay_checkpoint(
        &self,
        adapter: &str,
        checkpoint: i64,
    ) -> Result<()> {
        if let Some(mut state) = self.compiled_replay_state(adapter)? {
            state.checkpoint = checkpoint;
            self.store_compiled_replay_state(&state)?;
        }
        Ok(())
    }
    pub(crate) fn transfer_compiled_replay_state(
        &self,
        source: &str,
        destination: &Registration,
        checkpoint: i64,
    ) -> Result<()> {
        if let Some(mut state) = self.compiled_replay_state(source)? {
            state.binding.source_adapter = destination.manifest.id.clone();
            state.binding.source_binding_digest = compiled_lifecycle::binding(destination)?;
            state.binding.checkpoint_binding = retention::hash(&(
                "weave-compiled-checkpoint/1",
                &state.binding.source_adapter,
                &state.binding.source_binding_digest,
                &state.binding.epoch,
                &state.binding.primary_input,
                &state.binding.input_snapshots,
            ))?;
            state.checkpoint = checkpoint;
            self.store_compiled_replay_state(&state)?;
        }
        Ok(())
    }
    fn rebuild_inputs(&self, adapter: &str, host: &HostContext) -> Result<CompiledRebuildInputs> {
        let binding = self.compiled_bound_inputs(adapter, host)?;
        self.compiled_replay_state(adapter)?;
        let registration = self.handler_registration(adapter)?;
        let expected_output = self
            .head(
                &registration.output.graph_id,
                &registration.output.branch_id,
            )?
            .map(|revision| GraphRef {
                graph_id: registration.output.graph_id,
                revision,
            });
        if let Some(reference) = &expected_output {
            self.retention_whole(reference, host)?;
        }
        Ok(CompiledRebuildInputs {
            binding,
            expected_output,
        })
    }
    pub fn compiled_rebuild_inputs_for(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<CompiledRebuildInputs> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.rebuild_inputs(adapter, host)
    }
    pub fn rebuild_compiled_handler_for(
        &mut self,
        request: &CompiledRebuildRequest,
        host: &HostContext,
    ) -> Result<CompiledRebuildReceipt> {
        self.rebuild_compiled_boundary(request, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn rebuild_compiled_handler_test_before_commit(
        &mut self,
        request: &CompiledRebuildRequest,
        host: &HostContext,
        before: impl FnOnce(),
    ) -> Result<CompiledRebuildReceipt> {
        self.rebuild_compiled_boundary(request, host, before)
    }
    fn rebuild_compiled_boundary(
        &mut self,
        request: &CompiledRebuildRequest,
        host: &HostContext,
        before: impl FnOnce(),
    ) -> Result<CompiledRebuildReceipt> {
        let _budget = self.read_budget.enter();
        json_size(request, 2 * 1024 * 1024)?;
        if !valid_id(&request.nonce) || !valid_id(&request.inputs.binding.source_adapter) {
            return Err(err("E_REBUILD", "invalid reconstruction identity"));
        }
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _clock = self.operation_write_scope()?;
            let adapter = &request.inputs.binding.source_adapter;
            self.require_adapter_host(adapter, host)?;
            self.reject_governed_effect_adapter(adapter)?;
            let prior:Option<String>=self.conn.query_row("SELECT receipt_id FROM compiled_rebuild_receipts WHERE adapter=?1 AND nonce=?2",params![adapter,request.nonce],|r|r.get(0)).optional()?;
            if let Some(id) = prior {
                let record = self.compiled_rebuild_record(&id)?;
                if record.request != *request {
                    return Err(err(
                        "E_RECEIPT_CONFLICT",
                        "reconstruction identity binds other inputs",
                    ));
                }
                for reference in &record.request.inputs.binding.input_snapshots {
                    self.retention_whole(reference, host)?;
                }
                self.retention_whole(&record.receipt.output, host)?;
                return Ok(CompiledRebuildReceipt {
                    duplicate: true,
                    ..record.receipt
                });
            }
            let actual = self.rebuild_inputs(adapter, host)?;
            if actual != request.inputs {
                return Err(err("E_CONFLICT", "compiled reconstruction inputs changed"));
            }
            let (_, lifecycle, _) = self.dispatch_manifest(adapter)?;
            if !matches!(lifecycle.as_str(), "paused" | "draining") {
                return Err(err(
                    "E_REBUILD_PENDING",
                    "pause or drain before reconstruction",
                ));
            }
            let pending:i64=self.conn.query_row("SELECT (SELECT count(*) FROM dispatch_pending WHERE adapter=?1)+(SELECT count(*) FROM governance_delivery_pending WHERE adapter=?1)+(SELECT count(*) FROM effect_intents WHERE adapter=?1 AND state IN ('pending','unknown'))",[adapter],|r|r.get(0))?;
            if pending != 0 {
                return Err(err(
                    "E_REBUILD_PENDING",
                    "resolve in-flight work before reconstruction",
                ));
            }
            let registration = self.handler_registration(adapter)?;
            let (input, closure) = self.handler_input_snapshot(
                &registration,
                &actual.binding.primary_input,
                &registration.template.input.branch_id,
            )?;
            let id = receipt_id(request)?;
            let (data, coverage, diagnostics) =
                self.compiled_materialization(&registration, input, &closure, &id)?;
            let checkpoint: i64 =
                self.conn
                    .query_row("SELECT coalesce(max(sequence),0) FROM events", [], |r| {
                        r.get(0)
                    })?;
            let program = Program {
                version: VERSION.into(),
                source_revisions: registration.template.source_revisions.clone(),
                commands: vec![Command::Commit {
                    graph_id: registration.output.graph_id.clone(),
                    branch_id: registration.output.branch_id.clone(),
                    expected_head: actual.expected_output.as_ref().map(|r| r.revision.clone()),
                    data,
                }],
            };
            self.execute(&program, host)?;
            let output = GraphRef {
                graph_id: registration.output.graph_id.clone(),
                revision: self
                    .head(
                        &registration.output.graph_id,
                        &registration.output.branch_id,
                    )?
                    .ok_or_else(integrity)?,
            };
            let receipt = CompiledRebuildReceipt {
                receipt_id: id.clone(),
                definition_digest: registration.template.definition_digest.clone(),
                epoch: actual.binding.epoch.clone(),
                output,
                coverage,
                diagnostics,
                duplicate: false,
            };
            let record = Record {
                request: request.clone(),
                registration,
                receipt: receipt.clone(),
                checkpoint,
            };
            let bytes = json_size(&record, RECORD_LIMIT)?;
            let (count,used):(i64,i64)=self.conn.query_row("SELECT count(*),coalesce(sum(length(CAST(body AS BLOB))),0) FROM compiled_rebuild_receipts WHERE adapter=?1",[adapter],|r|Ok((r.get(0)?,r.get(1)?)))?;
            if count >= 128
                || used
                    .checked_add(bytes as i64)
                    .is_none_or(|n| n > 64 * 1024 * 1024)
            {
                return Err(err(
                    "E_BUDGET",
                    "compiled reconstruction receipt quota exceeded",
                ));
            }
            self.conn.execute(
                "INSERT INTO compiled_rebuild_receipts VALUES (?1,?2,?3,?4,?5)",
                params![
                    adapter,
                    request.nonce,
                    id,
                    serde_json::to_string(&record)?,
                    retention::hash(&record)?
                ],
            )?;
            self.conn.execute(
                "UPDATE dispatch_adapters SET checkpoint=?2 WHERE id=?1",
                params![adapter, checkpoint],
            )?;
            self.store_compiled_replay_state(&ReplayState {
                binding: actual.binding,
                receipt_id: receipt.receipt_id.clone(),
                checkpoint,
            })?;
            self.conn.execute(
                "DELETE FROM projection_rebuild_requests WHERE adapter=?1",
                [adapter],
            )?;
            before();
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
