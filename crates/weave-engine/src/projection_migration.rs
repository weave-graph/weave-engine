//! Explicit trusted native state/artifact/checkpoint transfer between immutable namespaces.
use super::*;
use serde::{Deserialize, Serialize};
const RECORD_LIMIT: usize = 4 * 1024 * 1024;
const STATE_LIMIT: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionMigrationInputs {
    pub source_adapter: String,
    pub source_state_digest: String,
    pub source_manifest_digest: String,
    pub epoch: String,
    /// A binding to semantic state and scoped inputs; unrelated private scan advances are invisible.
    /// The kernel reads and transfers the actual private checkpoint in the commit transaction.
    pub checkpoint_binding: String,
    pub snapshots: Vec<GraphRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectionMigrationKind {
    Upgrade,
    /// Restore the actual prior artifact/state/checkpoint pair recorded at this retired source.
    Rollback {
        restore_from: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionMigrationRequest {
    pub inputs: ProjectionMigrationInputs,
    pub destination: AdapterManifest,
    pub event_schema: String,
    pub nonce: String,
    pub disposition: ProjectionMigrationKind,
    pub state_revision: String,
    /// Actual transformed state for an upgrade; exact recorded prior state for a rollback.
    pub state: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionMigrationReceipt {
    pub source_adapter: String,
    pub destination_adapter: String,
    pub receipt_id: String,
    pub state_binding_digest: String,
    pub duplicate: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Migration {
    request: ProjectionMigrationRequest,
    principal: String,
    source_manifest: AdapterManifest,
    before: ProjectionRebaseRequest,
    before_checkpoint: i64,
    after: ProjectionRebaseRequest,
    after_checkpoint: i64,
    receipt: ProjectionMigrationReceipt,
}
fn integrity() -> Error {
    err(
        "E_LIFECYCLE_INTEGRITY",
        "state migration binding unavailable",
    )
}
fn receipt_id(request: &ProjectionMigrationRequest, principal: &str) -> Result<String> {
    retention::hash(&("weave-projection-migration/1", principal, request))
}
fn validate(record: &Migration, digest: &str) -> Result<()> {
    if retention::hash(record)? != digest
        || record.source_manifest.id != record.request.inputs.source_adapter
        || record.source_manifest.principal != record.principal
        || record.request.destination.principal != record.principal
        || record.request.event_schema != VERSION
        || record.source_manifest.id == record.request.destination.id
        || record.source_manifest.subscriptions != record.request.destination.subscriptions
        || record.source_manifest.output_graphs != record.request.destination.output_graphs
        || !record.source_manifest.effect_destinations.is_empty()
        || !record.request.destination.effect_destinations.is_empty()
        || !record.source_manifest.projection_replay
        || !record.request.destination.projection_replay
        || record.request.inputs.checkpoint_binding
            != retention::hash(&(
                "weave-projection-checkpoint/1",
                &record.source_manifest.id,
                &record.request.inputs.source_state_digest,
                &ProjectionRebaseInputs {
                    adapter: record.source_manifest.id.clone(),
                    epoch: record.request.inputs.epoch.clone(),
                    manifest_digest: record.request.inputs.source_manifest_digest.clone(),
                    snapshots: record.request.inputs.snapshots.clone(),
                },
            ))?
        || matches!(record.request.disposition, ProjectionMigrationKind::Upgrade)
            && (record.after_checkpoint != record.before_checkpoint
                || record.after.inputs.snapshots != record.before.inputs.snapshots)
        || record.before_checkpoint < 0
        || record.after_checkpoint < 0
        || record.before.inputs.adapter != record.source_manifest.id
        || record.before.inputs.manifest_digest != retention::hash(&record.source_manifest)?
        || record.request.inputs.source_state_digest != retention::hash(&record.before)?
        || record.request.inputs.source_manifest_digest != retention::hash(&record.source_manifest)?
        || record.before.inputs.epoch != record.request.inputs.epoch
        || record.after.inputs.adapter != record.request.destination.id
        || record.after.inputs.manifest_digest != retention::hash(&record.request.destination)?
        || record.after.inputs.epoch != record.request.inputs.epoch
        || record.after.state_revision != record.request.state_revision
        || record.after.state != record.request.state
        || record.receipt.source_adapter != record.source_manifest.id
        || record.receipt.destination_adapter != record.request.destination.id
        || record.receipt.duplicate
        || record.receipt.receipt_id != receipt_id(&record.request, &record.principal)?
        || record.receipt.state_binding_digest != retention::hash(&record.after)?
    {
        return Err(integrity());
    }
    Ok(())
}
pub(crate) fn validate_retained_migration(
    source: &str,
    destination: &str,
    principal: &str,
    nonce: &str,
    digest: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let record: Migration = serde_json::from_value(value.clone()).map_err(|_| integrity())?;
    validate(&record, digest)?;
    if record.source_manifest.id != source
        || record.request.destination.id != destination
        || record.principal != principal
        || record.request.nonce != nonce
    {
        return Err(integrity());
    }
    Ok(())
}
impl Engine {
    fn migration_source(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<ProjectionMigrationInputs> {
        let current = self.projection_inputs(adapter, host)?;
        self.require_replay_checkpoint(adapter)?;
        let state = self.projection_state(adapter)?.ok_or_else(|| {
            err(
                "E_REBASE_STATE",
                "migration requires an actual state/checkpoint pair",
            )
        })?;
        for reference in &state.inputs.snapshots {
            self.retention_whole(reference, host)?;
        }
        let digest = retention::hash(&state)?;
        Ok(ProjectionMigrationInputs {
            source_adapter: adapter.into(),
            source_state_digest: digest.clone(),
            source_manifest_digest: current.manifest_digest.clone(),
            epoch: current.epoch.clone(),
            checkpoint_binding: retention::hash(&(
                "weave-projection-checkpoint/1",
                adapter,
                &digest,
                &current,
            ))?,
            snapshots: current.snapshots,
        })
    }
    /// Current authorized transfer inputs without disclosing the local event coordinate.
    pub fn projection_migration_inputs_for(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<ProjectionMigrationInputs> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.migration_source(adapter, host)
    }
    fn migration_record(&self, source: &str) -> Result<Migration> {
        self.read_budget.request()?;
        let row: Option<(Option<String>, String)> = self.conn.query_row(
            "SELECT CASE WHEN length(CAST(body AS BLOB))<=?2 THEN body END,substr(digest,1,129) FROM projection_migrations WHERE source_adapter=?1",
            params![source, RECORD_LIMIT as i64], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?;
        let (body, digest) = row.ok_or_else(integrity)?;
        let body = body.ok_or_else(|| err("E_BUDGET", "migration record exceeds budget"))?;
        self.read_budget.charge(body.len())?;
        let record: Migration = serde_json::from_str(&body).map_err(|_| integrity())?;
        validate(&record, &digest)?;
        if record.source_manifest.id != source {
            return Err(integrity());
        }
        Ok(record)
    }
    /// Trusted pure native upgrade or rollback. This binds supplied actual state; it does not
    /// prove the host's computation. Effects and compiled handler state are excluded.
    pub fn migrate_projection_for(
        &self,
        request: &ProjectionMigrationRequest,
        host: &HostContext,
    ) -> Result<ProjectionMigrationReceipt> {
        self.migrate_projection_boundary(request, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn migrate_projection_test_before_commit(
        &self,
        request: &ProjectionMigrationRequest,
        host: &HostContext,
        before: impl FnOnce(),
    ) -> Result<ProjectionMigrationReceipt> {
        self.migrate_projection_boundary(request, host, before)
    }
    fn migrate_projection_boundary(
        &self,
        request: &ProjectionMigrationRequest,
        host: &HostContext,
        before: impl FnOnce(),
    ) -> Result<ProjectionMigrationReceipt> {
        let _budget = self.read_budget.enter();
        json_size(request, 2 * STATE_LIMIT)?;
        json_size(&request.state, STATE_LIMIT)?;
        if !valid_id(&request.inputs.source_adapter)
            || !valid_id(&request.nonce)
            || !valid_id(&request.state_revision)
            || !valid_id(&request.destination.id)
            || matches!(&request.disposition, ProjectionMigrationKind::Rollback { restore_from } if !valid_id(restore_from))
        {
            return Err(err("E_MIGRATION", "invalid migration identity"));
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let _clock = self.operation_write_scope()?;
        let source = &request.inputs.source_adapter;
        self.require_adapter_host(source, host)?;
        self.read_budget.request()?;
        let prior_source: Option<String> = self.conn.query_row(
            "SELECT source_adapter FROM projection_migrations WHERE source_adapter=?1 OR destination_adapter=?2 OR (principal=?3 AND nonce=?4) LIMIT 1",
            params![source, request.destination.id, host.principal, request.nonce], |r| r.get(0),
        ).optional()?;
        if let Some(source) = prior_source {
            let old = self.migration_record(&source)?;
            if old.request != *request || old.principal != host.principal {
                return Err(err(
                    "E_RECEIPT_CONFLICT",
                    "migration identity binds another transfer",
                ));
            }
            self.require_adapter_host(&old.request.destination.id, host)?;
            if self.dispatch_manifest(&source)?.0 != old.source_manifest
                || self.dispatch_manifest(&old.request.destination.id)?.0 != old.request.destination
            {
                return Err(integrity());
            }
            self.projection_state(&old.request.destination.id)?
                .ok_or_else(integrity)?;
            for reference in old
                .before
                .inputs
                .snapshots
                .iter()
                .chain(&old.after.inputs.snapshots)
            {
                self.retention_whole(reference, host)?;
            }
            tx.commit()?;
            return Ok(ProjectionMigrationReceipt {
                duplicate: true,
                ..old.receipt
            });
        }
        let actual = self.migration_source(source, host)?;
        if actual != request.inputs {
            return Err(err("E_CONFLICT", "migration inputs or checkpoint changed"));
        }
        let (manifest, lifecycle, checkpoint) = self.dispatch_manifest(source)?;
        if !matches!(lifecycle.as_str(), "paused" | "draining") {
            return Err(err(
                "E_MIGRATION_PENDING",
                "pause or drain the source before migration",
            ));
        }
        let pending: i64 = self.conn.query_row(
            "SELECT (SELECT count(*) FROM dispatch_pending WHERE adapter=?1)+(SELECT count(*) FROM governance_delivery_pending WHERE adapter=?1)+(SELECT count(*) FROM effect_intents WHERE adapter=?1 AND state IN ('pending','unknown'))",
            [source], |r| r.get(0),
        )?;
        if pending != 0 {
            return Err(err(
                "E_MIGRATION_PENDING",
                "resolve all in-flight work before migration",
            ));
        }
        if request.event_schema != VERSION
            || request.destination.id == *source
            || request.destination.principal != manifest.principal
            || request.destination.subscriptions != manifest.subscriptions
            || request.destination.output_graphs != manifest.output_graphs
            || !request.destination.effect_destinations.is_empty()
            || !request.destination.projection_replay
        {
            return Err(err(
                "E_MIGRATION_COMPATIBILITY",
                "explicit compatible pure event scopes are required",
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
                "migration needs a fresh immutable namespace",
            ));
        }
        let original = self.projection_state(source)?.ok_or_else(integrity)?;
        let (pins, after_checkpoint) = match &request.disposition {
            ProjectionMigrationKind::Upgrade => (original.inputs.snapshots.clone(), checkpoint),
            ProjectionMigrationKind::Rollback { restore_from } => {
                self.require_adapter_host(restore_from, host)?;
                let archived = self.migration_record(restore_from)?;
                let mut expected = archived.source_manifest.clone();
                expected.id = request.destination.id.clone();
                if self.dispatch_manifest(restore_from)?.0 != archived.source_manifest
                    || request.destination != expected
                    || request.state != archived.before.state
                    || request.state_revision != archived.before.state_revision
                {
                    return Err(err(
                        "E_MIGRATION_COMPATIBILITY",
                        "rollback must restore the recorded artifact/state pair",
                    ));
                }
                if archived.before.inputs.epoch != actual.epoch {
                    return Err(err(
                        "E_CHECKPOINT_EXPIRED",
                        "recorded rollback pair requires a fresh rebuild",
                    ));
                }
                for reference in &archived.before.inputs.snapshots {
                    self.retention_whole(reference, host)?;
                }
                (archived.before.inputs.snapshots, archived.before_checkpoint)
            }
        };
        let after = ProjectionRebaseRequest {
            inputs: ProjectionRebaseInputs {
                adapter: request.destination.id.clone(),
                epoch: actual.epoch,
                manifest_digest: retention::hash(&request.destination)?,
                snapshots: pins,
            },
            state_revision: request.state_revision.clone(),
            state: request.state.clone(),
        };
        json_size(&after, STATE_LIMIT)?;
        let receipt = ProjectionMigrationReceipt {
            source_adapter: source.clone(),
            destination_adapter: request.destination.id.clone(),
            receipt_id: receipt_id(request, &host.principal)?,
            state_binding_digest: retention::hash(&after)?,
            duplicate: false,
        };
        let record = Migration {
            request: request.clone(),
            principal: host.principal.clone(),
            source_manifest: manifest,
            before: original,
            before_checkpoint: checkpoint,
            after: after.clone(),
            after_checkpoint,
            receipt: receipt.clone(),
        };
        let bytes = json_size(&record, RECORD_LIMIT)?;
        let used: (i64, i64) = self.conn.query_row(
            "SELECT count(*),coalesce(sum(length(CAST(body AS BLOB))),0) FROM projection_migrations WHERE principal=?1",
            [&host.principal], |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if used.0 >= 1000
            || used
                .1
                .checked_add(bytes as i64)
                .is_none_or(|n| n > 64 * 1024 * 1024)
        {
            return Err(err("E_BUDGET", "migration receipt quota exceeded"));
        }
        self.install_adapter(&request.destination, host)?;
        self.conn.execute(
            "INSERT INTO retention_stateful_adapters VALUES (?1)",
            [&request.destination.id],
        )?;
        self.conn.execute(
            "INSERT INTO retention_adapter_states VALUES (?1,?2,?3,?4,?5)",
            params![
                request.destination.id,
                after.inputs.epoch,
                serde_json::to_string(&after)?,
                retention::hash(&(&after, after_checkpoint))?,
                after_checkpoint
            ],
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
            "INSERT INTO projection_migrations VALUES (?1,?2,?3,?4,?5,?6)",
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
