//! Version transfer for actual sealed pure stateless registrations. No external host state.
use super::*;
use compiled_handlers::{Output, Registration};
use serde::{Deserialize, Serialize};
const RECORD_LIMIT: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompiledMigrationInputs {
    pub source_adapter: String,
    pub source_binding_digest: String,
    pub epoch: String,
    pub primary_input: GraphRef,
    pub input_snapshots: Vec<GraphRef>,
    /// Scoped semantic binding; the private checkpoint itself remains inside the transaction.
    pub checkpoint_binding: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompiledMigrationRequest {
    pub inputs: CompiledMigrationInputs,
    pub destination: AdapterManifest,
    pub template: CompiledHandlerTemplate,
    pub output: HandlerOutputBinding,
    pub nonce: String,
    pub disposition: ProjectionMigrationKind,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompiledMigrationReceipt {
    pub source_adapter: String,
    pub destination_adapter: String,
    pub receipt_id: String,
    pub definition_digest: String,
    pub duplicate: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Migration {
    request: CompiledMigrationRequest,
    principal: String,
    before: Registration,
    before_checkpoint: i64,
    after: Registration,
    after_checkpoint: i64,
    receipt: CompiledMigrationReceipt,
}
fn integrity() -> Error {
    err(
        "E_LIFECYCLE_INTEGRITY",
        "compiled migration binding unavailable",
    )
}
fn binding(registration: &Registration) -> Result<String> {
    retention::hash(&("weave-handler-registration-binding/1", registration))
}
fn request_id(request: &CompiledMigrationRequest, principal: &str) -> Result<String> {
    retention::hash(&("weave-compiled-migration/1", principal, request))
}
fn validate(record: &Migration, digest: &str) -> Result<()> {
    handler_registration::validate_handler_template(&record.before.template)
        .map_err(|_| integrity())?;
    handler_registration::validate_handler_template(&record.after.template)
        .map_err(|_| integrity())?;
    if retention::hash(record)? != digest
        || record.before_checkpoint < 0
        || record.after_checkpoint < 0
        || record.before.manifest.id != record.request.inputs.source_adapter
        || record.before.manifest.id == record.after.manifest.id
        || record.before.manifest.principal != record.principal
        || record.after.manifest.principal != record.principal
        || record.before.template.input != record.after.template.input
        || record.before.template.protocol != record.after.template.protocol
        || record.before.output != record.after.output
        || record.before.manifest.subscriptions != record.after.manifest.subscriptions
        || record.before.manifest.output_graphs != record.after.manifest.output_graphs
        || !record.before.manifest.projection_replay
        || !record.after.manifest.projection_replay
        || !record.before.manifest.effect_destinations.is_empty()
        || !record.after.manifest.effect_destinations.is_empty()
        || record.before.manifest.artifact_digest != record.before.template.definition_digest
        || record.after.manifest.artifact_digest != record.after.template.definition_digest
        || record.after.template != record.request.template
        || record.after.manifest != record.request.destination
        || record.after.output.slot != record.request.output.slot
        || record.after.output.graph_id != record.request.output.graph_id
        || record.after.output.branch_id != record.request.output.branch_id
        || record.request.inputs.primary_input.graph_id != record.before.template.input.graph_id
        || !record
            .request
            .inputs
            .input_snapshots
            .contains(&record.request.inputs.primary_input)
        || record.request.inputs.source_binding_digest != binding(&record.before)?
        || record.request.inputs.checkpoint_binding
            != retention::hash(&(
                "weave-compiled-checkpoint/1",
                &record.request.inputs.source_adapter,
                &record.request.inputs.source_binding_digest,
                &record.request.inputs.epoch,
                &record.request.inputs.primary_input,
                &record.request.inputs.input_snapshots,
            ))?
        || matches!(record.request.disposition, ProjectionMigrationKind::Upgrade)
            && record.before_checkpoint != record.after_checkpoint
        || record.receipt.source_adapter != record.before.manifest.id
        || record.receipt.destination_adapter != record.after.manifest.id
        || record.receipt.definition_digest != record.after.template.definition_digest
        || record.receipt.receipt_id != request_id(&record.request, &record.principal)?
        || record.receipt.duplicate
    {
        return Err(integrity());
    }
    Ok(())
}
pub(crate) fn validate_retained_migration(
    engine: &Engine,
    source: &str,
    destination: &str,
    principal: &str,
    nonce: &str,
    digest: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let record: Migration = serde_json::from_value(value.clone()).map_err(|_| integrity())?;
    validate(&record, digest)?;
    if record.before.manifest.id != source
        || record.after.manifest.id != destination
        || record.principal != principal
        || record.request.nonce != nonce
        || engine
            .handler_registration(source)
            .map_err(|_| integrity())?
            != record.before
        || engine
            .handler_registration(destination)
            .map_err(|_| integrity())?
            != record.after
    {
        return Err(integrity());
    }
    Ok(())
}
impl Engine {
    pub(crate) fn initialize_compiled_lifecycle(&self, version: i64) -> Result<()> {
        let present: bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='compiled_migrations')",[],|r|r.get(0))?;
        if (version < 24 && present) || (version >= 24 && !present) {
            return Err(integrity());
        }
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS compiled_migrations(source_adapter TEXT PRIMARY KEY REFERENCES compiled_handlers(adapter),destination_adapter TEXT UNIQUE NOT NULL REFERENCES compiled_handlers(adapter),principal TEXT NOT NULL,nonce TEXT NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,UNIQUE(principal,nonce));")?;
        Ok(())
    }
    fn compiled_transfer_inputs(
        &self,
        source: &str,
        host: &HostContext,
    ) -> Result<CompiledMigrationInputs> {
        self.require_adapter_host(source, host)?;
        self.reject_governed_effect_adapter(source)?;
        self.require_replay_checkpoint(source)?;
        let state: i64=self.conn.query_row("SELECT (SELECT count(*) FROM retention_stateful_adapters WHERE adapter=?1)+(SELECT count(*) FROM retention_adapter_states WHERE adapter=?1)+(SELECT count(*) FROM retention_projection_receipts WHERE adapter=?1)",[source],|r|r.get(0))?;
        if state != 0 {
            return Err(err(
                "E_MIGRATION_MODE",
                "stateless compiled registration required",
            ));
        }
        let registration = self.handler_registration(source)?;
        let input = &registration.template.input;
        let revision = self
            .head(&input.graph_id, &input.branch_id)?
            .ok_or_else(|| err("E_HANDLER_INPUT", "handler input unavailable"))?;
        let primary_input = GraphRef {
            graph_id: input.graph_id.clone(),
            revision,
        };
        let (_, snapshots) =
            self.handler_input_snapshot(&registration, &primary_input, &input.branch_id)?;
        let epoch = self.retention_replay_epoch()?;
        let source_binding_digest = binding(&registration)?;
        Ok(CompiledMigrationInputs {
            source_adapter: source.into(),
            source_binding_digest: source_binding_digest.clone(),
            epoch: epoch.clone(),
            primary_input: primary_input.clone(),
            input_snapshots: snapshots.clone(),
            checkpoint_binding: retention::hash(&(
                "weave-compiled-checkpoint/1",
                source,
                &source_binding_digest,
                &epoch,
                &primary_input,
                &snapshots,
            ))?,
        })
    }
    pub fn compiled_migration_inputs_for(
        &self,
        source: &str,
        host: &HostContext,
    ) -> Result<CompiledMigrationInputs> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.compiled_transfer_inputs(source, host)
    }
    fn compiled_migration_record(&self, source: &str) -> Result<Migration> {
        self.read_budget.request()?;
        let row: Option<(Option<String>,String)>=self.conn.query_row("SELECT CASE WHEN length(CAST(body AS BLOB))<=?2 THEN body END,substr(digest,1,129) FROM compiled_migrations WHERE source_adapter=?1",params![source,RECORD_LIMIT as i64],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (body, digest) = row.ok_or_else(integrity)?;
        let body =
            body.ok_or_else(|| err("E_BUDGET", "compiled migration record exceeds budget"))?;
        self.read_budget.charge(body.len())?;
        let record: Migration = serde_json::from_str(&body).map_err(|_| integrity())?;
        validate(&record, &digest)?;
        if record.before.manifest.id != source
            || self.handler_registration(source)? != record.before
            || self.handler_registration(&record.after.manifest.id)? != record.after
        {
            return Err(integrity());
        }
        Ok(record)
    }
    pub fn migrate_compiled_handler_for(
        &self,
        request: &CompiledMigrationRequest,
        host: &HostContext,
    ) -> Result<CompiledMigrationReceipt> {
        self.migrate_compiled_boundary(request, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn migrate_compiled_handler_test_before_commit(
        &self,
        request: &CompiledMigrationRequest,
        host: &HostContext,
        before: impl FnOnce(),
    ) -> Result<CompiledMigrationReceipt> {
        self.migrate_compiled_boundary(request, host, before)
    }
    fn migrate_compiled_boundary(
        &self,
        request: &CompiledMigrationRequest,
        host: &HostContext,
        before: impl FnOnce(),
    ) -> Result<CompiledMigrationReceipt> {
        let _budget = self.read_budget.enter();
        json_size(request, 3 * 1024 * 1024)?;
        if !valid_id(&request.inputs.source_adapter)
            || !valid_id(&request.destination.id)
            || !valid_id(&request.nonce)
            || matches!(&request.disposition,ProjectionMigrationKind::Rollback {restore_from} if !valid_id(restore_from))
        {
            return Err(err("E_MIGRATION", "invalid compiled migration identity"));
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let _clock = self.operation_write_scope()?;
        let source = &request.inputs.source_adapter;
        self.require_adapter_host(source, host)?;
        self.reject_governed_effect_adapter(source)?;
        self.read_budget.request()?;
        let prior:Option<String>=self.conn.query_row("SELECT source_adapter FROM compiled_migrations WHERE source_adapter=?1 OR destination_adapter=?2 OR (principal=?3 AND nonce=?4) LIMIT 1",params![source,request.destination.id,host.principal,request.nonce],|r|r.get(0)).optional()?;
        if let Some(source) = prior {
            let old = self.compiled_migration_record(&source)?;
            if old.request != *request || old.principal != host.principal {
                return Err(err(
                    "E_RECEIPT_CONFLICT",
                    "compiled migration identity binds another transfer",
                ));
            }
            self.require_adapter_host(&old.after.manifest.id, host)?;
            for reference in &old.request.inputs.input_snapshots {
                self.retention_whole(reference, host)?;
            }
            tx.commit()?;
            return Ok(CompiledMigrationReceipt {
                duplicate: true,
                ..old.receipt
            });
        }
        let actual = self.compiled_transfer_inputs(source, host)?;
        if actual != request.inputs {
            return Err(err("E_CONFLICT", "compiled input or binding changed"));
        }
        let (manifest, lifecycle, checkpoint) = self.dispatch_manifest(source)?;
        if !matches!(lifecycle.as_str(), "paused" | "draining") {
            return Err(err(
                "E_MIGRATION_PENDING",
                "pause or drain the source before migration",
            ));
        }
        let pending:i64=self.conn.query_row("SELECT (SELECT count(*) FROM dispatch_pending WHERE adapter=?1)+(SELECT count(*) FROM governance_delivery_pending WHERE adapter=?1)+(SELECT count(*) FROM effect_intents WHERE adapter=?1 AND state IN ('pending','unknown'))",[source],|r|r.get(0))?;
        if pending != 0 {
            return Err(err(
                "E_MIGRATION_PENDING",
                "resolve in-flight work before migration",
            ));
        }
        let original = self.handler_registration(source)?;
        let output = Output {
            slot: request.output.slot.clone(),
            graph_id: request.output.graph_id.clone(),
            branch_id: request.output.branch_id.clone(),
        };
        if request.destination.id == *source
            || request.destination.principal != manifest.principal
            || request.destination.subscriptions != manifest.subscriptions
            || request.destination.output_graphs != manifest.output_graphs
            || !request.destination.effect_destinations.is_empty()
            || !request.destination.projection_replay
            || request.template.input != original.template.input
            || request.template.protocol != original.template.protocol
            || output != original.output
        {
            return Err(err(
                "E_MIGRATION_COMPATIBILITY",
                "compatible sealed input and output scopes required",
            ));
        }
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM dispatch_adapters WHERE id=?1)",
            [&request.destination.id],
            |r| r.get(0),
        )?;
        if exists {
            return Err(err(
                "E_ADAPTER_VERSION",
                "compiled migration needs a fresh namespace",
            ));
        }
        let after_checkpoint = match &request.disposition {
            ProjectionMigrationKind::Upgrade => checkpoint,
            ProjectionMigrationKind::Rollback { restore_from } => {
                self.require_adapter_host(restore_from, host)?;
                let archived = self.compiled_migration_record(restore_from)?;
                let mut expected = archived.before.manifest.clone();
                expected.id = request.destination.id.clone();
                if request.destination != expected
                    || request.template != archived.before.template
                    || output != archived.before.output
                {
                    return Err(err(
                        "E_MIGRATION_COMPATIBILITY",
                        "rollback must restore the recorded compiled artifact pair",
                    ));
                }
                if archived.request.inputs.epoch != actual.epoch {
                    return Err(err(
                        "E_CHECKPOINT_EXPIRED",
                        "recorded compiled rollback requires a fresh rebuild",
                    ));
                }
                for reference in &archived.request.inputs.input_snapshots {
                    self.retention_whole(reference, host)?;
                }
                archived.before_checkpoint
            }
        };
        let after = Registration {
            template: request.template.clone(),
            output,
            manifest: request.destination.clone(),
        };
        let receipt = CompiledMigrationReceipt {
            source_adapter: source.clone(),
            destination_adapter: request.destination.id.clone(),
            receipt_id: request_id(request, &host.principal)?,
            definition_digest: request.template.definition_digest.clone(),
            duplicate: false,
        };
        let record = Migration {
            request: request.clone(),
            principal: host.principal.clone(),
            before: original,
            before_checkpoint: checkpoint,
            after,
            after_checkpoint,
            receipt: receipt.clone(),
        };
        let bytes = json_size(&record, RECORD_LIMIT)?;
        let used:(i64,i64)=self.conn.query_row("SELECT count(*),coalesce(sum(length(CAST(body AS BLOB))),0) FROM compiled_migrations WHERE principal=?1",[&host.principal],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if used.0 >= 128
            || used
                .1
                .checked_add(bytes as i64)
                .is_none_or(|n| n > 64 * 1024 * 1024)
        {
            return Err(err("E_BUDGET", "compiled migration receipt quota exceeded"));
        }
        self.install_compiled_handler_in_transaction(
            &request.destination,
            &request.template,
            &request.output,
            host,
        )?;
        self.conn.execute(
            "UPDATE dispatch_adapters SET state='paused',checkpoint=?2 WHERE id=?1",
            params![request.destination.id, after_checkpoint],
        )?;
        self.conn.execute(
            "UPDATE dispatch_adapters SET state='removed' WHERE id=?1",
            [source],
        )?;
        self.conn.execute(
            "INSERT INTO compiled_migrations VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                source,
                request.destination.id,
                host.principal,
                request.nonce,
                serde_json::to_string(&record)?,
                retention::hash(&record)?
            ],
        )?;
        before();
        tx.commit()?;
        Ok(receipt)
    }
}
