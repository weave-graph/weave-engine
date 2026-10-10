//! Compatible recorded actor transfer and private observation fences.
use super::*;
use serde::{Deserialize, Serialize};
const RECORD_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedActorMigrationInputs {
    pub source_adapter: String,
    pub definition_digest: String,
    pub state_digest: String,
    pub current_inputs: RecordedActorInputs,
    pub checkpoint_binding: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedActorMigration {
    pub inputs: RecordedActorMigrationInputs,
    pub destination: RecordedActorDefinition,
    pub nonce: String,
    pub disposition: ProjectionMigrationKind,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedActorMigrationReceipt {
    pub source_adapter: String,
    pub destination_adapter: String,
    pub receipt_id: String,
    pub state_digest: String,
    pub definition_digest: String,
    pub duplicate: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fence {
    pub(crate) adapter: String,
    pub(crate) source: String,
    pub(crate) through: i64,
    pub(crate) migration_id: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationRecord {
    request: RecordedActorMigration,
    principal: String,
    source: RecordedActorDefinition,
    before: RecordedActorState,
    before_checkpoint: i64,
    before_fence: Option<Fence>,
    restored_from: Option<String>,
    restored: RecordedActorState,
    restored_checkpoint: i64,
    after: RecordedActorState,
    after_fence: Option<Fence>,
    receipt: RecordedActorMigrationReceipt,
}
fn integrity() -> Error {
    err(
        "E_ACTOR_LIFECYCLE_INTEGRITY",
        "actor lifecycle binding unavailable",
    )
}
fn binding(inputs: &RecordedActorMigrationInputs) -> Result<String> {
    retention::hash(&(
        "weave-recorded-actor-checkpoint/1",
        &inputs.source_adapter,
        &inputs.definition_digest,
        &inputs.state_digest,
        &inputs.current_inputs,
    ))
}
fn receipt_id(request: &RecordedActorMigration, principal: &str) -> Result<String> {
    retention::hash(&("weave-recorded-actor-migration/1", principal, request))
}
pub(crate) fn compatible(a: &RecordedActorDefinition, b: &RecordedActorDefinition) -> bool {
    a.event_schema == b.event_schema
        && a.state_protocol == b.state_protocol
        && a.metadata_depth == b.metadata_depth
        && a.manifest.principal == b.manifest.principal
        && a.manifest.subscriptions == b.manifest.subscriptions
        && a.manifest.output_graphs == b.manifest.output_graphs
        && a.manifest.effect_destinations == b.manifest.effect_destinations
        && !a.manifest.projection_replay
        && !b.manifest.projection_replay
}
fn rebound(
    state: &RecordedActorState,
    definition: &RecordedActorDefinition,
) -> Result<RecordedActorState> {
    let mut out = state.clone();
    out.inputs.adapter = definition.manifest.id.clone();
    out.inputs.registration_digest = retention::hash(definition)?;
    out.inputs.prior_state_digest = Some(retention::hash(state)?);
    Ok(out)
}
fn validate_record(record: &MigrationRecord, digest: &str) -> Result<()> {
    let request = &record.request;
    recorded_actors::validate_state(&record.before, &record.source)?;
    recorded_actors::validate_state(&record.after, &request.destination)?;
    let mut current_state = record.before.clone();
    current_state.inputs = request.inputs.current_inputs.clone();
    recorded_actors::validate_state(&current_state, &record.source)?;
    recorded_actors::validate_definition(&record.source).map_err(|_| integrity())?;
    recorded_actors::validate_definition(&request.destination).map_err(|_| integrity())?;
    if retention::hash(record)? != digest
        || record.before_checkpoint < 0
        || record.restored_checkpoint < 0
        || record.source.manifest.id != request.inputs.source_adapter
        || record.source.manifest.principal != record.principal
        || !compatible(&record.source, &request.destination)
        || request.destination.manifest.id == request.inputs.source_adapter
        || !valid_id(&request.nonce)
        || request.inputs.definition_digest != retention::hash(&record.source)?
        || request.inputs.state_digest != retention::hash(&record.before)?
        || request.inputs.current_inputs.adapter != request.inputs.source_adapter
        || request.inputs.current_inputs.registration_digest != request.inputs.definition_digest
        || request.inputs.current_inputs.prior_state_digest.as_ref()
            != Some(&request.inputs.state_digest)
        || request.inputs.current_inputs.epoch != record.before.inputs.epoch
        || request.inputs.checkpoint_binding != binding(&request.inputs)?
        || record.before.inputs.registration_digest != request.inputs.definition_digest
        || record.before.inputs.adapter != request.inputs.source_adapter
        || record.after != rebound(&record.restored, &request.destination)?
        || record.after.inputs.epoch != record.before.inputs.epoch
        || record.receipt.duplicate
        || record.receipt.receipt_id != receipt_id(request, &record.principal)?
        || record.receipt.source_adapter != request.inputs.source_adapter
        || record.receipt.destination_adapter != request.destination.manifest.id
        || record.receipt.state_digest != retention::hash(&record.after)?
        || record.receipt.definition_digest != retention::hash(&request.destination)?
    {
        return Err(integrity());
    }
    match &request.disposition {
        ProjectionMigrationKind::Upgrade => {
            if record.restored_from.is_some()
                || record.restored != record.before
                || record.restored_checkpoint != record.before_checkpoint
            {
                return Err(integrity());
            }
            let expected = record.before_fence.as_ref().map(|f| Fence {
                adapter: request.destination.manifest.id.clone(),
                source: f.source.clone(),
                through: f.through,
                migration_id: record.receipt.receipt_id.clone(),
            });
            if record.after_fence != expected {
                return Err(integrity());
            }
        }
        ProjectionMigrationKind::Rollback { restore_from } => {
            if record.restored_from.as_ref() != Some(restore_from)
                || record.after_fence
                    != Some(Fence {
                        adapter: request.destination.manifest.id.clone(),
                        source: request.inputs.source_adapter.clone(),
                        through: record
                            .before_checkpoint
                            .max(record.before_fence.as_ref().map_or(0, |f| f.through)),
                        migration_id: record.receipt.receipt_id.clone(),
                    })
            {
                return Err(integrity());
            }
        }
    }
    Ok(())
}
pub(crate) fn validate_retained_actor_lifecycle(
    engine: &Engine,
    table: &str,
    adapter: &str,
    digest: &str,
    value: &serde_json::Value,
) -> Result<()> {
    if table == "recorded_actor_migrations" {
        let record: MigrationRecord =
            serde_json::from_value(value.clone()).map_err(|_| integrity())?;
        validate_record(&record, digest)?;
        if engine.actor_migration(adapter)?.as_ref() != Some(&record) {
            return Err(integrity());
        }
    } else if table == "recorded_actor_replay_fences" {
        let fence: Fence = serde_json::from_value(value.clone()).map_err(|_| integrity())?;
        if retention::hash(&fence)? != digest
            || engine.actor_fence(adapter)?.as_ref() != Some(&fence)
        {
            return Err(integrity());
        }
    } else {
        return Err(integrity());
    }
    Ok(())
}
impl Engine {
    pub(crate) fn initialize_actor_lifecycle(&self, version: i64) -> Result<()> {
        let present:i64=self.conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('recorded_actor_migrations','recorded_actor_replay_fences','recorded_actor_observations')",[],|r|r.get(0))?;
        if (version < 27 && present != 0) || (version >= 27 && present != 3) {
            return Err(integrity());
        }
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS recorded_actor_migrations(adapter TEXT PRIMARY KEY REFERENCES recorded_actor_definitions(adapter),destination_adapter TEXT UNIQUE NOT NULL REFERENCES recorded_actor_definitions(adapter),principal TEXT NOT NULL,nonce TEXT NOT NULL,receipt_id TEXT UNIQUE NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,UNIQUE(principal,nonce));
CREATE TABLE IF NOT EXISTS recorded_actor_replay_fences(adapter TEXT PRIMARY KEY REFERENCES recorded_actor_definitions(adapter),source_adapter TEXT NOT NULL REFERENCES recorded_actor_definitions(adapter),through_sequence INTEGER NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS recorded_actor_observations(adapter TEXT NOT NULL REFERENCES recorded_actor_definitions(adapter),event_id TEXT NOT NULL,nonce TEXT NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(adapter,event_id),UNIQUE(adapter,nonce));")?;
        Ok(())
    }
    fn actor_migration_cell(&self, adapter: &str) -> Result<Option<(String, String)>> {
        self.read_budget.request()?;
        let row:Option<(Option<String>,String)>=self.conn.query_row("SELECT CASE WHEN length(CAST(body AS BLOB))<=?2 THEN body END,substr(digest,1,129) FROM recorded_actor_migrations WHERE adapter=?1",params![adapter,RECORD_LIMIT as i64],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        row.map(|(body, digest)| {
            let body = body.ok_or_else(|| err("E_BUDGET", "actor migration exceeds limit"))?;
            self.read_budget.charge(body.len())?;
            Ok((body, digest))
        })
        .transpose()
    }
    fn actor_migration_basic(&self, adapter: &str) -> Result<Option<MigrationRecord>> {
        let Some((body, digest)) = self.actor_migration_cell(adapter)? else {
            return Ok(None);
        };
        let record: MigrationRecord = serde_json::from_str(&body).map_err(|_| integrity())?;
        validate_record(&record, &digest)?;
        let row:(String,String,String,String)=self.conn.query_row("SELECT destination_adapter,principal,nonce,receipt_id FROM recorded_actor_migrations WHERE adapter=?1",[adapter],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        if row
            != (
                record.request.destination.manifest.id.clone(),
                record.principal.clone(),
                record.request.nonce.clone(),
                record.receipt.receipt_id.clone(),
            )
            || self.actor_definition(adapter)? != record.source
            || self.actor_definition(&record.request.destination.manifest.id)?
                != record.request.destination
            || self.dispatch_manifest(adapter)?.1 != "removed"
            || self.dispatch_manifest(adapter)?.2 != record.before_checkpoint
            || self.actor_state(adapter)?.as_ref() != Some(&record.before)
        {
            return Err(integrity());
        }
        if let ProjectionMigrationKind::Rollback { restore_from } = &record.request.disposition {
            if restore_from == adapter
                || self.dispatch_manifest(restore_from)?.1 != "removed"
                || self.actor_state(restore_from)?.as_ref() != Some(&record.restored)
                || self.dispatch_manifest(restore_from)?.2 != record.restored_checkpoint
            {
                return Err(integrity());
            }
            let mut expected = self.actor_definition(restore_from)?;
            expected.manifest.id = record.request.destination.manifest.id.clone();
            if expected != record.request.destination {
                return Err(integrity());
            }
        }
        Ok(Some(record))
    }
    fn actor_migration(&self, adapter: &str) -> Result<Option<MigrationRecord>> {
        let Some(initial) = self.actor_migration_basic(adapter)? else {
            return Ok(None);
        };
        let mut pending = vec![adapter.to_string()];
        let mut seen = HashSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            if seen.len() > 16 {
                return Err(err("E_BUDGET", "actor migration lineage exceeds limit"));
            }
            let record = self.actor_migration_basic(&id)?.ok_or_else(integrity)?;
            if self.actor_fence_cell(&id)? != record.before_fence
                || self.actor_fence_cell(&record.request.destination.manifest.id)?
                    != record.after_fence
            {
                return Err(integrity());
            }
            if let Some(fence) = &record.before_fence {
                let origin = self.actor_fence_origin(fence)?;
                if origin == id {
                    return Err(integrity());
                }
                let prior = self.actor_migration_basic(&origin)?.ok_or_else(integrity)?;
                if prior.after_fence.as_ref() != Some(fence) {
                    return Err(integrity());
                }
                pending.push(origin);
            }
            if let ProjectionMigrationKind::Rollback { restore_from } = &record.request.disposition
            {
                let mut next = restore_from.clone();
                let mut path = HashSet::new();
                for _ in 0..16 {
                    if next == id {
                        break;
                    }
                    if !path.insert(next.clone()) {
                        return Err(integrity());
                    }
                    let link = self.actor_migration_basic(&next)?.ok_or_else(integrity)?;
                    pending.push(next);
                    next = link.request.destination.manifest.id;
                }
                if next != id {
                    return Err(integrity());
                }
            }
        }
        Ok(Some(initial))
    }
    fn actor_fence_origin(&self, fence: &Fence) -> Result<String> {
        self.read_budget.request()?;
        self.conn
            .query_row(
                "SELECT substr(adapter,1,513) FROM recorded_actor_migrations WHERE receipt_id=?1",
                [&fence.migration_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(integrity)
    }
    fn actor_fence_cell(&self, adapter: &str) -> Result<Option<Fence>> {
        self.read_budget.request()?;
        let row:Option<(String,i64,Option<String>,String)>=self.conn.query_row("SELECT substr(source_adapter,1,513),through_sequence,CASE WHEN length(CAST(body AS BLOB))<=4096 THEN body END,substr(digest,1,129) FROM recorded_actor_replay_fences WHERE adapter=?1",[adapter],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((source, through, body, digest)) = row else {
            return Ok(None);
        };
        let body = body.ok_or_else(|| err("E_BUDGET", "actor replay fence exceeds limit"))?;
        self.read_budget.charge(body.len())?;
        let fence: Fence = serde_json::from_str(&body).map_err(|_| integrity())?;
        if fence.adapter != adapter
            || fence.source != source
            || fence.source == adapter
            || fence.through != through
            || through < 0
            || retention::hash(&fence)? != digest
        {
            return Err(integrity());
        }
        Ok(Some(fence))
    }
    pub(crate) fn actor_fence(&self, adapter: &str) -> Result<Option<Fence>> {
        let Some(fence) = self.actor_fence_cell(adapter)? else {
            return Ok(None);
        };
        let origin = self.actor_fence_origin(&fence)?;
        let migration = self.actor_migration(&origin)?.ok_or_else(integrity)?;
        if migration.after_fence.as_ref() != Some(&fence) {
            return Err(integrity());
        }
        Ok(Some(fence))
    }
    pub(crate) fn require_actor_new_computation(&self, adapter: &str, event: &str) -> Result<()> {
        if let Some(fence) = self.actor_fence(adapter)? {
            let sequence: i64 = self
                .conn
                .query_row(
                    "SELECT sequence FROM events WHERE event_id=?1",
                    [event],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or_else(|| err("E_UNAVAILABLE", "actor occurrence unavailable"))?;
            if sequence <= fence.through {
                return Err(err(
                    "E_ACTOR_REPLAY",
                    "observe the actual recorded outcome before new computation or effects",
                ));
            }
        }
        Ok(())
    }
    pub fn recorded_actor_migration_inputs_for(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<RecordedActorMigrationInputs> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.actor_migration_inputs(adapter, host)
    }
    fn actor_migration_inputs(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<RecordedActorMigrationInputs> {
        let definition = self.actor_definition_for(adapter, host)?;
        self.require_recorded_actor_ready(adapter)?;
        let state = self.actor_state(adapter)?.ok_or_else(integrity)?;
        let mut out = RecordedActorMigrationInputs {
            source_adapter: adapter.into(),
            definition_digest: retention::hash(&definition)?,
            state_digest: retention::hash(&state)?,
            current_inputs: self.actor_inputs(adapter, host)?,
            checkpoint_binding: String::new(),
        };
        out.checkpoint_binding = binding(&out)?;
        Ok(out)
    }
    fn actor_restore_record(&self, current: &str, restore_from: &str) -> Result<MigrationRecord> {
        let restored = self
            .actor_migration(restore_from)?
            .ok_or_else(|| err("E_ACTOR_RESTORE", "recorded prior pair unavailable"))?;
        let mut next = restored.request.destination.manifest.id.clone();
        let mut seen = HashSet::new();
        for _ in 0..16 {
            if next == current {
                return Ok(restored);
            }
            if !seen.insert(next.clone()) {
                return Err(integrity());
            }
            next = self
                .actor_migration(&next)?
                .ok_or_else(|| err("E_ACTOR_RESTORE", "prior pair is outside actor lineage"))?
                .request
                .destination
                .manifest
                .id;
        }
        Err(err("E_BUDGET", "actor restoration lineage exceeds limit"))
    }
    pub fn migrate_recorded_actor_for(
        &self,
        request: &RecordedActorMigration,
        host: &HostContext,
    ) -> Result<RecordedActorMigrationReceipt> {
        self.migrate_recorded_actor_boundary(request, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn migrate_recorded_actor_test_before_commit(
        &self,
        request: &RecordedActorMigration,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<RecordedActorMigrationReceipt> {
        self.migrate_recorded_actor_boundary(request, host, before_commit)
    }
    fn migrate_recorded_actor_boundary(
        &self,
        request: &RecordedActorMigration,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<RecordedActorMigrationReceipt> {
        let _budget = self.read_budget.enter();
        json_size(request, 3 * 1024 * 1024)?;
        if !valid_id(&request.nonce) {
            return Err(err("E_ACTOR_MIGRATION", "invalid actor migration nonce"));
        }
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_write_scope()?;
        let source = &request.inputs.source_adapter;
        self.require_adapter_host(source, host)?;
        self.reject_governed_effect_adapter(source)?;
        if let Some(record) = self.actor_migration(source)? {
            if record.request != *request || record.principal != host.principal {
                return Err(err(
                    "E_RECEIPT_CONFLICT",
                    "actor migration identity already binds another transfer",
                ));
            }
            for pin in record
                .before
                .inputs
                .input_snapshots
                .iter()
                .chain(record.after.inputs.input_snapshots.iter())
                .chain(record.request.inputs.current_inputs.input_snapshots.iter())
            {
                self.retention_whole(pin, host)?;
            }
            let mut receipt = record.receipt;
            receipt.duplicate = true;
            tx.commit()?;
            return Ok(receipt);
        }
        let actual = self.actor_migration_inputs(source, host)?;
        if actual != request.inputs {
            return Err(err("E_CONFLICT", "actor state, input or policy changed"));
        }
        let definition = self.actor_definition(source)?;
        let (_, lifecycle, checkpoint) = self.dispatch_manifest(source)?;
        if !matches!(lifecycle.as_str(), "paused" | "installed") {
            return Err(err(
                "E_ACTOR_PENDING",
                "pause actor before compatible version transfer",
            ));
        }
        let pending:i64=self.conn.query_row("SELECT (SELECT count(*) FROM dispatch_pending WHERE adapter=?1)+(SELECT count(*) FROM governance_delivery_pending WHERE adapter=?1)+(SELECT count(*) FROM effect_intents WHERE adapter=?1 AND state IN ('pending','unknown'))",[source],|r|r.get(0))?;
        if pending != 0 {
            return Err(err(
                "E_ACTOR_PENDING",
                "resolve all delivery and effect work before transfer",
            ));
        }
        recorded_actors::validate_definition(&request.destination)?;
        if source == &request.destination.manifest.id
            || !compatible(&definition, &request.destination)
        {
            return Err(err(
                "E_ACTOR_COMPATIBILITY",
                "actor owner, scope or state/event ABI is incompatible",
            ));
        }
        let occupied:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM dispatch_adapters WHERE id=?1) OR EXISTS(SELECT 1 FROM recorded_actor_migrations WHERE principal=?2 AND nonce=?3)",params![request.destination.manifest.id,host.principal,request.nonce],|r|r.get(0))?;
        if occupied {
            return Err(err(
                "E_RECEIPT_CONFLICT",
                "actor destination or migration nonce is already reserved",
            ));
        }
        let before = self.actor_state(source)?.ok_or_else(integrity)?;
        let before_fence = self.actor_fence(source)?;
        let (restored, restored_checkpoint, restored_from) = match &request.disposition {
            ProjectionMigrationKind::Upgrade => (before.clone(), checkpoint, None),
            ProjectionMigrationKind::Rollback { restore_from } => {
                let prior = self.actor_restore_record(source, restore_from)?;
                let mut expected = prior.source.clone();
                expected.manifest.id = request.destination.manifest.id.clone();
                if request.destination != expected
                    || prior.before.inputs.epoch != actual.current_inputs.epoch
                {
                    return Err(err(
                        "E_ACTOR_RESTORE",
                        "actual prior artifact/state pair is incompatible or expired",
                    ));
                }
                (
                    prior.before,
                    prior.before_checkpoint,
                    Some(restore_from.clone()),
                )
            }
        };
        for pin in restored
            .inputs
            .input_snapshots
            .iter()
            .chain(before.inputs.input_snapshots.iter())
        {
            self.retention_whole(pin, host)?;
        }
        let after = rebound(&restored, &request.destination)?;
        let receipt = RecordedActorMigrationReceipt {
            source_adapter: source.clone(),
            destination_adapter: request.destination.manifest.id.clone(),
            receipt_id: receipt_id(request, &host.principal)?,
            state_digest: retention::hash(&after)?,
            definition_digest: retention::hash(&request.destination)?,
            duplicate: false,
        };
        let after_fence = match &request.disposition {
            ProjectionMigrationKind::Upgrade => before_fence.as_ref().map(|f| Fence {
                adapter: request.destination.manifest.id.clone(),
                source: f.source.clone(),
                through: f.through,
                migration_id: receipt.receipt_id.clone(),
            }),
            ProjectionMigrationKind::Rollback { .. } => Some(Fence {
                adapter: request.destination.manifest.id.clone(),
                source: source.clone(),
                through: checkpoint.max(before_fence.as_ref().map_or(0, |f| f.through)),
                migration_id: receipt.receipt_id.clone(),
            }),
        };
        let record = MigrationRecord {
            request: request.clone(),
            principal: host.principal.clone(),
            source: definition,
            before,
            before_checkpoint: checkpoint,
            before_fence,
            restored_from,
            restored,
            restored_checkpoint,
            after: after.clone(),
            after_fence: after_fence.clone(),
            receipt: receipt.clone(),
        };
        let body = serde_json::to_string(&record)?;
        if body.len() > RECORD_LIMIT {
            return Err(err("E_BUDGET", "actor migration record exceeds limit"));
        }
        let (count,used):(i64,i64)=self.conn.query_row("SELECT count(*),coalesce(sum(length(CAST(body AS BLOB))),0) FROM recorded_actor_migrations WHERE principal=?1",[&host.principal],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if count >= 128
            || used
                .checked_add(body.len() as i64)
                .is_none_or(|n| n > 64 * 1024 * 1024)
        {
            return Err(err("E_BUDGET", "actor migration quota exceeded"));
        }
        self.install_recorded_actor_in_transaction(&request.destination, host)?;
        self.put_actor_state(&after, restored_checkpoint)?;
        self.conn.execute(
            "UPDATE recorded_actor_definitions SET initialized=1 WHERE adapter=?1",
            [&request.destination.manifest.id],
        )?;
        self.conn.execute(
            "UPDATE dispatch_adapters SET state='paused',checkpoint=?2 WHERE id=?1",
            params![request.destination.manifest.id, restored_checkpoint],
        )?;
        self.conn.execute(
            "UPDATE dispatch_adapters SET state='removed' WHERE id=?1",
            [source],
        )?;
        self.conn.execute(
            "INSERT INTO recorded_actor_migrations VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                source,
                request.destination.manifest.id,
                host.principal,
                request.nonce,
                receipt.receipt_id,
                body,
                retention::hash(&record)?
            ],
        )?;
        if let Some(fence) = after_fence {
            self.conn.execute(
                "INSERT INTO recorded_actor_replay_fences VALUES (?1,?2,?3,?4,?5)",
                params![
                    fence.adapter,
                    fence.source,
                    fence.through,
                    serde_json::to_string(&fence)?,
                    retention::hash(&fence)?
                ],
            )?;
        }
        before_commit();
        tx.commit()?;
        Ok(receipt)
    }
}
