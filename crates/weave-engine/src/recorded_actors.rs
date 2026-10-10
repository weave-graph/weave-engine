//! Recorded native actors: trusted computation, kernel-bound state and effects.
use super::*;
use serde::{Deserialize, Serialize};

const STATE_LIMIT: usize = 1024 * 1024;
const RECORD_LIMIT: usize = 8 * STATE_LIMIT;
const DEFINITION_LIMIT: usize = 2 * STATE_LIMIT;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedActorDefinition {
    pub manifest: AdapterManifest,
    pub event_schema: String,
    /// Declared compatible opaque host-state ABI. Default omission preserves store26 hashes.
    #[serde(
        default = "default_state_protocol",
        skip_serializing_if = "is_default_state_protocol"
    )]
    pub state_protocol: String,
    pub metadata_depth: u32,
    /// Actual stored artifact bytes. Hash verification is not execution attestation.
    pub artifact: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedActorPrimary {
    pub scope: SubscriptionScope,
    pub graph: GraphRef,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedActorInputs {
    pub adapter: String,
    pub registration_digest: String,
    /// Current state compare-and-swap for explicit initialization/reconstruction.
    pub prior_state_digest: Option<String>,
    pub epoch: String,
    pub primary_inputs: Vec<RecordedActorPrimary>,
    pub input_snapshots: Vec<GraphRef>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedToolResult {
    pub name: String,
    pub media_type: String,
    pub value: serde_json::Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedToolArtifact {
    pub result: RecordedToolResult,
    pub digest: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedActorState {
    pub inputs: RecordedActorInputs,
    pub state_revision: String,
    pub state: serde_json::Value,
    pub artifacts: Vec<RecordedToolArtifact>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedActorBootstrap {
    pub inputs: RecordedActorInputs,
    pub state_revision: String,
    pub state: serde_json::Value,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedActorRunInputs {
    pub event: String,
    pub primary_input: GraphRef,
    pub state_digest: String,
    pub registration_digest: String,
    pub input_snapshots: Vec<GraphRef>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedActorCompletion {
    pub adapter: String,
    pub event: String,
    pub lease: String,
    pub prior_state_digest: String,
    pub state_revision: String,
    pub state: serde_json::Value,
    pub input_snapshots: Vec<GraphRef>,
    pub tool_results: Vec<RecordedToolResult>,
    pub program: Program,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedActorReceipt {
    pub receipt_id: String,
    pub state_digest: String,
    pub artifacts: Vec<RecordedToolArtifact>,
    pub effects: Vec<EffectIntent>,
    pub handler: HandlerReceipt,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub(crate) definition: RecordedActorDefinition,
    pub(crate) request_digest: String,
    pub(crate) program_digest: String,
    pub(crate) before: RecordedActorState,
    pub(crate) after: RecordedActorState,
    pub(crate) receipt: RecordedActorReceipt,
    pub(crate) checkpoint: i64,
}
pub(crate) struct CompletionToken {
    adapter: String,
    event: String,
}
impl CompletionToken {
    pub(crate) fn observed(adapter: &str, event: &str) -> Self {
        Self {
            adapter: adapter.into(),
            event: event.into(),
        }
    }
    pub(crate) fn authorizes(&self, adapter: &str, event: &str) -> bool {
        self.adapter == adapter && self.event == event
    }
}
pub(crate) fn default_state_protocol() -> String {
    "weave-recorded-opaque-state/1".into()
}
fn is_default_state_protocol(value: &str) -> bool {
    value == "weave-recorded-opaque-state/1"
}
fn integrity() -> Error {
    err("E_ACTOR_INTEGRITY", "recorded actor binding unavailable")
}
fn canonical_pins(pins: &mut Vec<GraphRef>) {
    pins.sort_by(|a, b| (&a.graph_id, &a.revision).cmp(&(&b.graph_id, &b.revision)));
    pins.dedup();
}
pub(crate) fn validate_definition(definition: &RecordedActorDefinition) -> Result<()> {
    if definition.event_schema != VERSION
        || !valid_id(&definition.state_protocol)
        || definition.manifest.projection_replay
        || definition.metadata_depth > 8
        || definition.artifact.is_empty()
        || definition.artifact.len() > 256 * 1024
        || definition.manifest.artifact_digest
            != format!("sha256:{:x}", Sha256::digest(&definition.artifact))
    {
        return Err(err(
            "E_ACTOR_DEFINITION",
            "invalid recorded actor definition",
        ));
    }
    json_size(definition, DEFINITION_LIMIT)?;
    Ok(())
}
fn artifacts(results: &[RecordedToolResult]) -> Result<Vec<RecordedToolArtifact>> {
    if results.len() > 32 {
        return Err(err("E_BUDGET", "recorded tool count limit exceeded"));
    }
    let mut names = HashSet::new();
    let mut out = Vec::new();
    for result in results {
        if !valid_id(&result.name)
            || !names.insert(&result.name)
            || result.media_type.is_empty()
            || result.media_type.len() > 128
            || !result.media_type.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(err("E_ACTOR_ARTIFACT", "invalid recorded tool artifact"));
        }
        out.push(RecordedToolArtifact {
            result: result.clone(),
            digest: retention::hash(&("weave-recorded-tool/1", result))?,
        });
    }
    json_size(&out, 2 * STATE_LIMIT)?;
    Ok(out)
}
fn completion_digest(request: &RecordedActorCompletion) -> Result<String> {
    // Renewed leases identify a worker, not a different computation.
    retention::hash(&(
        "weave-recorded-actor-completion/1",
        &request.adapter,
        &request.event,
        &request.prior_state_digest,
        &request.state_revision,
        &request.state,
        &request.input_snapshots,
        &request.tool_results,
        &request.program,
    ))
}
pub(crate) fn validate_state(
    state: &RecordedActorState,
    definition: &RecordedActorDefinition,
) -> Result<()> {
    if state.inputs.adapter != definition.manifest.id
        || state.inputs.registration_digest != retention::hash(definition)?
        || !valid_id(&state.state_revision)
        || state.inputs.epoch.len() != 48
        || !state.inputs.epoch.bytes().all(|b| b.is_ascii_hexdigit())
        || state.inputs.primary_inputs.len() != definition.manifest.subscriptions.len()
        || state
            .inputs
            .primary_inputs
            .iter()
            .zip(&definition.manifest.subscriptions)
            .any(|(p, s)| {
                p.scope != *s
                    || p.graph.graph_id != s.graph_id
                    || !state.inputs.input_snapshots.contains(&p.graph)
            })
        || state.inputs.input_snapshots.len() > 1000
        || artifacts(
            &state
                .artifacts
                .iter()
                .map(|a| a.result.clone())
                .collect::<Vec<_>>(),
        )? != state.artifacts
    {
        return Err(integrity());
    }
    let mut canonical = state.inputs.input_snapshots.clone();
    canonical_pins(&mut canonical);
    if canonical != state.inputs.input_snapshots {
        return Err(integrity());
    }
    json_size(state, 4 * STATE_LIMIT)?;
    Ok(())
}
fn validate_record(record: &Record, adapter: &str, digest: &str) -> Result<()> {
    validate_definition(&record.definition).map_err(|_| integrity())?;
    validate_state(&record.before, &record.definition)?;
    validate_state(&record.after, &record.definition)?;
    if retention::hash(record)? != digest
        || record.checkpoint < 0
        || record.definition.manifest.id != adapter
        || record.receipt.handler.duplicate
        || record.receipt.receipt_id
            != retention::hash(&("weave-recorded-actor-receipt/1", &record.request_digest))?
        || record.receipt.state_digest != retention::hash(&record.after)?
        || record.receipt.artifacts != record.after.artifacts
        || record.receipt.effects.iter().any(|e| {
            e.adapter != adapter
                || !matches!(e.state.as_str(), "confirmed" | "failed")
                || e.response.is_none()
        })
    {
        return Err(integrity());
    }
    Ok(())
}
pub(crate) fn validate_retained_actor(
    engine: &Engine,
    table: &str,
    adapter: &str,
    event: &str,
    digest: &str,
    value: &serde_json::Value,
) -> Result<()> {
    if table == "recorded_actor_definitions" {
        let definition: RecordedActorDefinition =
            serde_json::from_value(value.clone()).map_err(|_| integrity())?;
        if retention::hash(&definition)? != digest
            || engine.actor_definition(adapter)? != definition
        {
            return Err(integrity());
        }
    } else if table == "recorded_actor_states" {
        let state: RecordedActorState =
            serde_json::from_value(value.clone()).map_err(|_| integrity())?;
        if engine.actor_state(adapter)?.as_ref() != Some(&state) {
            return Err(integrity());
        }
    } else {
        let record: Record = serde_json::from_value(value.clone()).map_err(|_| integrity())?;
        validate_record(&record, adapter, digest)?;
        if engine.actor_record(adapter, event)?.as_ref() != Some(&record)
            || record.receipt.effects.iter().any(|e| e.event_id != event)
        {
            return Err(integrity());
        }
    }
    Ok(())
}
impl Engine {
    pub(crate) fn initialize_recorded_actors(&self, version: i64) -> Result<()> {
        let present: i64 = self.conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('recorded_actor_definitions','recorded_actor_states','recorded_actor_receipts')", [], |r| r.get(0))?;
        if (version < 26 && present != 0) || (version >= 26 && present != 3) {
            return Err(integrity());
        }
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS recorded_actor_definitions(adapter TEXT PRIMARY KEY REFERENCES dispatch_adapters(id),body TEXT NOT NULL,digest TEXT NOT NULL,initialized INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS recorded_actor_states(adapter TEXT PRIMARY KEY REFERENCES recorded_actor_definitions(adapter),body TEXT NOT NULL,digest TEXT NOT NULL,checkpoint INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS recorded_actor_receipts(adapter TEXT NOT NULL REFERENCES recorded_actor_definitions(adapter),event_id TEXT NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(adapter,event_id));")?;
        Ok(())
    }
    pub(crate) fn is_recorded_actor(&self, adapter: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM recorded_actor_definitions WHERE adapter=?1)",
            [adapter],
            |r| r.get(0),
        )?)
    }
    fn actor_cell(
        &self,
        table: &str,
        adapter: &str,
        event: Option<&str>,
        limit: usize,
    ) -> Result<Option<(String, String)>> {
        self.read_budget.request()?;
        let sql = format!("SELECT CASE WHEN length(CAST(body AS BLOB))<=?2 THEN body END,substr(digest,1,129) FROM {table} WHERE adapter=?1{}", if event.is_some() { " AND event_id=?3" } else { "" });
        let row: Option<(Option<String>, String)> = if let Some(event) = event {
            self.conn
                .query_row(&sql, params![adapter, limit as i64, event], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?
        } else {
            self.conn
                .query_row(&sql, params![adapter, limit as i64], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?
        };
        row.map(|(body, digest)| {
            let body = body.ok_or_else(|| err("E_BUDGET", "recorded actor cell exceeds limit"))?;
            self.read_budget.charge(body.len())?;
            Ok((body, digest))
        })
        .transpose()
    }
    pub(crate) fn actor_definition(&self, adapter: &str) -> Result<RecordedActorDefinition> {
        let (body, digest) = self
            .actor_cell(
                "recorded_actor_definitions",
                adapter,
                None,
                DEFINITION_LIMIT,
            )?
            .ok_or_else(integrity)?;
        let definition: RecordedActorDefinition =
            serde_json::from_str(&body).map_err(|_| integrity())?;
        validate_definition(&definition).map_err(|_| integrity())?;
        let (manifest, _, _) = self.dispatch_manifest(adapter)?;
        let (initialized,has_state):(i64,bool) = self.conn.query_row("SELECT initialized,EXISTS(SELECT 1 FROM recorded_actor_states WHERE adapter=?1) FROM recorded_actor_definitions WHERE adapter=?1", [adapter], |r| Ok((r.get(0)?,r.get(1)?)))?;
        if !matches!(initialized, 0 | 1)
            || (initialized == 1) != has_state
            || definition.manifest != manifest
            || manifest.id != adapter
            || retention::hash(&definition)? != digest
        {
            return Err(integrity());
        }
        Ok(definition)
    }
    pub(crate) fn actor_definition_for(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<RecordedActorDefinition> {
        self.require_adapter_host(adapter, host)?;
        self.reject_governed_effect_adapter(adapter)?;
        let definition = self.actor_definition(adapter)?;
        if self.is_compiled_handler(adapter)? || self.dispatch_manifest(adapter)?.1 == "removed" {
            return Err(err("E_ACTOR_MODE", "active recorded actor required"));
        }
        Ok(definition)
    }
    /// Trusted native installation records actual artifact bytes; it executes no code.
    pub fn install_recorded_actor_for(
        &self,
        definition: &RecordedActorDefinition,
        host: &HostContext,
    ) -> Result<()> {
        let _budget = self.read_budget.enter();
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_write_scope()?;
        self.install_recorded_actor_in_transaction(definition, host)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn install_recorded_actor_in_transaction(
        &self,
        definition: &RecordedActorDefinition,
        host: &HostContext,
    ) -> Result<()> {
        if self.conn.is_autocommit() {
            return Err(err(
                "E_TRANSACTION",
                "actor installation requires transaction",
            ));
        }
        validate_definition(definition)?;
        let id = &definition.manifest.id;
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM dispatch_adapters WHERE id=?1)",
            [id],
            |r| r.get(0),
        )?;
        if exists {
            self.require_adapter_host(id, host)?;
            if !self.is_recorded_actor(id)? || self.actor_definition(id)? != *definition {
                return Err(err(
                    "E_ACTOR_MODE",
                    "use a new identity for recorded actor installation",
                ));
            }
            return Ok(());
        }
        let count: i64 =
            self.conn
                .query_row("SELECT count(*) FROM recorded_actor_definitions", [], |r| {
                    r.get(0)
                })?;
        if count >= 128 {
            return Err(err(
                "E_BUDGET",
                "recorded actor registration limit exceeded",
            ));
        }
        self.install_adapter(&definition.manifest, host)?;
        self.conn.execute(
            "INSERT INTO recorded_actor_definitions VALUES (?1,?2,?3,0)",
            params![
                id,
                serde_json::to_string(definition)?,
                retention::hash(definition)?
            ],
        )?;
        Ok(())
    }
    pub(crate) fn actor_closure(
        &self,
        definition: &RecordedActorDefinition,
        reference: &GraphRef,
        branch: &str,
        host: &HostContext,
    ) -> Result<Vec<GraphRef>> {
        self.retention_whole(reference, host)?;
        let raw = self
            .load(&reference.graph_id, &reference.revision)?
            .ok_or_else(|| err("E_UNAVAILABLE", "actor input unavailable"))?;
        if raw
            .attachments
            .iter()
            .any(|a| matches!(a.value, MetadataValue::LiveGraph { .. }))
        {
            return Err(err(
                "E_ACTOR_INPUT",
                "recorded actors require pinned metadata",
            ));
        }
        let input = self.query(
            &QueryPlan {
                graph_id: reference.graph_id.clone(),
                revision: Some(reference.revision.clone()),
                branch_id: branch.into(),
                predicate: None,
                from: None,
                to: None,
                valid_at: None,
                include_metadata: definition.metadata_depth > 0,
                max_depth: definition.metadata_depth,
            },
            host,
        )?;
        if input.coverage != Coverage::Complete || input.metadata_graphs.len() >= 1000 {
            return Err(err(
                "E_UNAVAILABLE",
                "whole actor metadata input unavailable",
            ));
        }
        let mut pins = vec![reference.clone()];
        pins.extend(input.metadata_graphs.iter().map(|g| g.reference.clone()));
        canonical_pins(&mut pins);
        for pin in &pins {
            self.retention_whole(pin, host)?;
            let graph = self
                .load(&pin.graph_id, &pin.revision)?
                .ok_or_else(integrity)?;
            if graph
                .attachments
                .iter()
                .any(|a| matches!(a.value, MetadataValue::LiveGraph { .. }))
            {
                return Err(err(
                    "E_ACTOR_INPUT",
                    "recorded actors require pinned metadata",
                ));
            }
        }
        Ok(pins)
    }
    pub(crate) fn actor_inputs(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<RecordedActorInputs> {
        let definition = self.actor_definition_for(adapter, host)?;
        let mut primary_inputs = Vec::new();
        let mut input_snapshots = Vec::new();
        for scope in &definition.manifest.subscriptions {
            let graph = GraphRef {
                graph_id: scope.graph_id.clone(),
                revision: self
                    .head(&scope.graph_id, &scope.branch_id)?
                    .ok_or_else(|| err("E_UNAVAILABLE", "actor primary input unavailable"))?,
            };
            input_snapshots.extend(self.actor_closure(
                &definition,
                &graph,
                &scope.branch_id,
                host,
            )?);
            primary_inputs.push(RecordedActorPrimary {
                scope: scope.clone(),
                graph,
            });
        }
        canonical_pins(&mut input_snapshots);
        if input_snapshots.len() > 1000 {
            return Err(err("E_BUDGET", "recorded actor input limit exceeded"));
        }
        Ok(RecordedActorInputs {
            adapter: adapter.into(),
            registration_digest: retention::hash(&definition)?,
            prior_state_digest: self
                .actor_state(adapter)?
                .as_ref()
                .map(retention::hash)
                .transpose()?,
            epoch: self.retention_replay_epoch()?,
            primary_inputs,
            input_snapshots,
        })
    }
    pub fn recorded_actor_inputs_for(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<RecordedActorInputs> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.actor_inputs(adapter, host)
    }
    pub(crate) fn actor_state(&self, adapter: &str) -> Result<Option<RecordedActorState>> {
        let Some((body, digest)) =
            self.actor_cell("recorded_actor_states", adapter, None, 4 * STATE_LIMIT)?
        else {
            if self.is_recorded_actor(adapter)? {
                self.actor_definition(adapter)?;
            }
            return Ok(None);
        };
        let state: RecordedActorState = serde_json::from_str(&body).map_err(|_| integrity())?;
        validate_state(&state, &self.actor_definition(adapter)?)?;
        let checkpoint: i64 = self.conn.query_row(
            "SELECT checkpoint FROM recorded_actor_states WHERE adapter=?1",
            [adapter],
            |r| r.get(0),
        )?;
        if checkpoint < 0
            || checkpoint != self.dispatch_manifest(adapter)?.2
            || digest != retention::hash(&(&state, checkpoint))?
        {
            return Err(integrity());
        }
        Ok(Some(state))
    }
    pub(crate) fn put_actor_state(
        &self,
        state: &RecordedActorState,
        checkpoint: i64,
    ) -> Result<()> {
        validate_state(state, &self.actor_definition(&state.inputs.adapter)?)?;
        self.conn.execute("INSERT INTO recorded_actor_states VALUES (?1,?2,?3,?4) ON CONFLICT(adapter) DO UPDATE SET body=excluded.body,digest=excluded.digest,checkpoint=excluded.checkpoint", params![state.inputs.adapter, serde_json::to_string(state)?, retention::hash(&(state, checkpoint))?, checkpoint])?;
        Ok(())
    }
    pub(crate) fn advance_recorded_actor_checkpoint(
        &self,
        adapter: &str,
        checkpoint: i64,
    ) -> Result<()> {
        if let Some(state) = self.actor_state(adapter)? {
            self.put_actor_state(&state, checkpoint)?;
        }
        Ok(())
    }
    pub(crate) fn require_recorded_actor_effect(&self, adapter: &str, event: &str) -> Result<()> {
        if self.is_recorded_actor(adapter)? {
            self.require_recorded_actor_ready(adapter)?;
            self.require_actor_new_computation(adapter, event)?;
        }
        Ok(())
    }
    pub(crate) fn require_recorded_actor_ready(&self, adapter: &str) -> Result<()> {
        let state = self.actor_state(adapter)?.ok_or_else(|| {
            err(
                "E_ACTOR_STATE",
                "initialize recorded actor state explicitly",
            )
        })?;
        let rebuild: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM projection_rebuild_requests WHERE adapter=?1)",
            [adapter],
            |r| r.get(0),
        )?;
        if rebuild || state.inputs.epoch != self.retention_replay_epoch()? {
            return Err(err(
                "E_CHECKPOINT_EXPIRED",
                "explicit recorded actor reconstruction required",
            ));
        }
        let definition = self.actor_definition(adapter)?;
        let host = HostContext::new(
            &definition.manifest.principal,
            definition.manifest.output_graphs.clone(),
        );
        for pin in &state.inputs.input_snapshots {
            self.retention_whole(pin, &host)?;
        }
        Ok(())
    }
    pub fn recorded_actor_state_for(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<RecordedActorState> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.actor_definition_for(adapter, host)?;
        self.require_recorded_actor_ready(adapter)?;
        self.actor_state(adapter)?.ok_or_else(integrity)
    }
    /// Explicit trusted current-snapshot initialization/reconstruction, with no effect execution.
    pub fn bootstrap_recorded_actor_for(
        &self,
        request: &RecordedActorBootstrap,
        host: &HostContext,
    ) -> Result<String> {
        self.bootstrap_recorded_actor_boundary(request, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn bootstrap_recorded_actor_test_before_commit(
        &self,
        request: &RecordedActorBootstrap,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<String> {
        self.bootstrap_recorded_actor_boundary(request, host, before_commit)
    }
    fn bootstrap_recorded_actor_boundary(
        &self,
        request: &RecordedActorBootstrap,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<String> {
        let _budget = self.read_budget.enter();
        json_size(request, STATE_LIMIT)?;
        if !valid_id(&request.state_revision) {
            return Err(err("E_ACTOR_STATE", "invalid actor state revision"));
        }
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_write_scope()?;
        let actual = self.actor_inputs(&request.inputs.adapter, host)?;
        if actual != request.inputs {
            return Err(err("E_CONFLICT", "actor input or policy changed"));
        }
        let (_, lifecycle, _) = self.dispatch_manifest(&actual.adapter)?;
        if !matches!(lifecycle.as_str(), "installed" | "paused") {
            return Err(err(
                "E_ACTOR_PENDING",
                "pause actor before snapshot initialization",
            ));
        }
        let pending: i64 = self.conn.query_row("SELECT (SELECT count(*) FROM dispatch_pending WHERE adapter=?1)+(SELECT count(*) FROM governance_delivery_pending WHERE adapter=?1)+(SELECT count(*) FROM effect_intents WHERE adapter=?1 AND state IN ('pending','unknown'))", [&actual.adapter], |r| r.get(0))?;
        if pending != 0 {
            return Err(err(
                "E_ACTOR_PENDING",
                "resolve in-flight work before actor initialization",
            ));
        }
        if self.is_recorded_actor(&actual.adapter)? {
            self.actor_state(&actual.adapter)?;
        }
        let checkpoint: i64 =
            self.conn
                .query_row("SELECT coalesce(max(sequence),0) FROM events", [], |r| {
                    r.get(0)
                })?;
        let state = RecordedActorState {
            inputs: actual,
            state_revision: request.state_revision.clone(),
            state: request.state.clone(),
            artifacts: vec![],
        };
        self.put_actor_state(&state, checkpoint)?;
        self.conn.execute(
            "UPDATE recorded_actor_definitions SET initialized=1 WHERE adapter=?1",
            [&state.inputs.adapter],
        )?;
        self.conn.execute(
            "UPDATE dispatch_adapters SET checkpoint=?2 WHERE id=?1",
            params![state.inputs.adapter, checkpoint],
        )?;
        self.conn.execute(
            "DELETE FROM projection_rebuild_requests WHERE adapter=?1",
            [&state.inputs.adapter],
        )?;
        let digest = retention::hash(&state)?;
        before_commit();
        tx.commit()?;
        Ok(digest)
    }
    /// Current authorized occurrence/state for a new trusted host computation.
    /// A retained host journal can recover an existing computation without calling a tool again.
    pub fn recorded_actor_run_inputs_for(
        &self,
        adapter: &str,
        event: &str,
        lease: &str,
        host: &HostContext,
    ) -> Result<RecordedActorRunInputs> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        let definition = self.actor_definition_for(adapter, host)?;
        let (_, lifecycle, _) = self.dispatch_manifest(adapter)?;
        if !matches!(lifecycle.as_str(), "running" | "draining") {
            return Err(err("E_PAUSED", "actor is not running"));
        }
        self.require_recorded_actor_ready(adapter)?;
        self.check_lease(adapter, event, lease)?;
        self.require_uncanceled_delivery(adapter, event)?;
        self.require_actor_new_computation(adapter, event)?;
        let (graph_id, branch_id, revision, _) = self
            .scoped_event(&definition.manifest, event)?
            .ok_or_else(|| err("E_UNAVAILABLE", "actor occurrence unavailable"))?;
        self.require_causal_work(adapter, event)?;
        let primary_input = GraphRef { graph_id, revision };
        let input_snapshots = self.actor_closure(&definition, &primary_input, &branch_id, host)?;
        let state = self.actor_state(adapter)?.ok_or_else(integrity)?;
        Ok(RecordedActorRunInputs {
            event: event.into(),
            primary_input,
            state_digest: retention::hash(&state)?,
            registration_digest: retention::hash(&definition)?,
            input_snapshots,
        })
    }
    /// Read an actual historical completion under current whole authority, including retired actors.
    /// This never executes a tool, effect or graph command and does not require replay readiness.
    pub fn recorded_actor_receipt_for(
        &self,
        adapter: &str,
        event: &str,
        host: &HostContext,
    ) -> Result<RecordedActorReceipt> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.require_adapter_host(adapter, host)?;
        self.reject_governed_effect_adapter(adapter)?;
        let definition = self.actor_definition(adapter)?;
        self.scoped_event(&definition.manifest, event)?
            .ok_or_else(|| err("E_UNAVAILABLE", "recorded actor occurrence unavailable"))?;
        let current = self.actor_state(adapter)?.ok_or_else(integrity)?;
        let record = self
            .actor_record(adapter, event)?
            .ok_or_else(|| err("E_UNAVAILABLE", "recorded actor receipt unavailable"))?;
        for pin in current
            .inputs
            .input_snapshots
            .iter()
            .chain(record.before.inputs.input_snapshots.iter())
            .chain(record.after.inputs.input_snapshots.iter())
        {
            self.retention_whole(pin, host)?;
        }
        Ok(record.receipt)
    }
    pub(crate) fn actor_record(&self, adapter: &str, event: &str) -> Result<Option<Record>> {
        let Some((body, digest)) = self.actor_cell(
            "recorded_actor_receipts",
            adapter,
            Some(event),
            RECORD_LIMIT,
        )?
        else {
            return Ok(None);
        };
        let record: Record = serde_json::from_str(&body).map_err(|_| integrity())?;
        validate_record(&record, adapter, &digest)?;
        if self.actor_definition(adapter)? != record.definition
            || record.receipt.effects.iter().any(|e| e.event_id != event)
        {
            return Err(integrity());
        }
        self.read_budget.request()?;
        let row: Option<(String, Option<String>)> = self.conn.query_row("SELECT substr(request_hash,1,129),CASE WHEN length(CAST(results AS BLOB))<=?3 THEN results END FROM handler_receipts WHERE adapter=?1 AND event_id=?2", params![adapter,event,RECORD_LIMIT as i64], |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (program_digest, results) = row.ok_or_else(integrity)?;
        let results =
            results.ok_or_else(|| err("E_BUDGET", "actor handler receipt exceeds limit"))?;
        self.read_budget.charge(results.len())?;
        let actual: Vec<CommandResult> = serde_json::from_str(&results).map_err(|_| integrity())?;
        let (graph_id, branch_id, revision, sequence): (String, String, String, i64) = self
            .conn
            .query_row(
                "SELECT graph_id,branch_id,revision,sequence FROM events WHERE event_id=?1",
                [event],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
            .ok_or_else(integrity)?;
        if !record
            .definition
            .manifest
            .subscriptions
            .contains(&SubscriptionScope {
                graph_id: graph_id.clone(),
                branch_id,
            })
            || !record
                .after
                .inputs
                .input_snapshots
                .contains(&GraphRef { graph_id, revision })
            || record.checkpoint != sequence
            || program_digest != record.program_digest
            || actual != record.receipt.handler.results
            || self.actor_effects(adapter, event)? != record.receipt.effects
        {
            return Err(integrity());
        }
        Ok(Some(record))
    }
    fn actor_effects(&self, adapter: &str, event: &str) -> Result<Vec<EffectIntent>> {
        self.read_budget.request()?;
        let mut statement = self.conn.prepare("SELECT substr(id,1,513) FROM effect_intents WHERE adapter=?1 AND event_id=?2 ORDER BY id LIMIT 33")?;
        let ids = statement
            .query_map(params![adapter, event], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if ids.len() > 32 {
            return Err(err("E_BUDGET", "recorded actor effect count exceeded"));
        }
        let mut effects = Vec::new();
        for id in ids {
            self.read_budget.request()?;
            let size: i64 = self.conn.query_row("SELECT length(CAST(payload AS BLOB))+coalesce(length(CAST(response AS BLOB)),0) FROM effect_intents WHERE id=?1", [&id], |r| r.get(0))?;
            if size > 2 * STATE_LIMIT as i64 {
                return Err(err("E_BUDGET", "recorded effect exceeds limit"));
            }
            self.read_budget.charge(size as usize)?;
            let effect = self.effect_intent(&id)?.ok_or_else(integrity)?;
            if effect.adapter != adapter
                || effect.event_id != event
                || effect.id
                    != format!(
                        "effect:{:x}",
                        Sha256::digest(serde_json::to_vec(&(adapter, &effect.idempotency_key))?)
                    )
                || !self
                    .actor_definition(adapter)?
                    .manifest
                    .effect_destinations
                    .contains(&effect.destination)
            {
                return Err(integrity());
            }
            if !matches!(effect.state.as_str(), "confirmed" | "failed") || effect.response.is_none()
            {
                return Err(err(
                    "E_ACTOR_EFFECT_PENDING",
                    "reconcile every occurrence effect before actor completion",
                ));
            }
            effects.push(effect);
        }
        json_size(&effects, 2 * STATE_LIMIT)?;
        Ok(effects)
    }
    pub(crate) fn recorded_actor_effect_quota(
        &self,
        adapter: &str,
        event: &str,
        payload: &serde_json::Value,
    ) -> Result<()> {
        if !self.is_recorded_actor(adapter)? {
            return Ok(());
        }
        let (count,total):(i64,i64) = self.conn.query_row("SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))+coalesce(length(CAST(response AS BLOB)),0)),0) FROM effect_intents WHERE adapter=?1", [adapter], |r|Ok((r.get(0)?,r.get(1)?)))?;
        let occurrence: i64 = self.conn.query_row(
            "SELECT count(*) FROM effect_intents WHERE adapter=?1 AND event_id=?2",
            params![adapter, event],
            |r| r.get(0),
        )?;
        let bytes = serde_json::to_vec(payload)?.len() as i64;
        if count >= 4096
            || occurrence >= 32
            || total
                .checked_add(bytes)
                .is_none_or(|n| n > 64 * STATE_LIMIT as i64)
        {
            return Err(err("E_BUDGET", "recorded actor effect quota exceeded"));
        }
        Ok(())
    }
    fn actor_after(
        &self,
        request: &RecordedActorCompletion,
        before: &RecordedActorState,
        results: &[CommandResult],
        event_pins: &[GraphRef],
        host: &HostContext,
    ) -> Result<RecordedActorState> {
        let mut pins = before.inputs.input_snapshots.clone();
        pins.extend(request.input_snapshots.iter().cloned());
        pins.extend(event_pins.iter().cloned());
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
                | CommandResult::Unchanged { revision } => revisions.push(revision),
                CommandResult::BatchCommitted { commits, .. }
                | CommandResult::BatchUnchanged { commits } => {
                    pins.extend(commits.iter().map(|c| GraphRef {
                        graph_id: c.graph_id.clone(),
                        revision: c.revision.clone(),
                    }))
                }
            }
        }
        for value in values {
            self.require_current_result_authority(value, host)?;
            pins.extend(value.input_snapshots.iter().cloned());
            pins.extend(value.snapshots.iter().map(|(g, r)| GraphRef {
                graph_id: g.clone(),
                revision: r.clone(),
            }));
            pins.extend(value.metadata_graphs.iter().map(|g| g.reference.clone()));
            pins.extend(value.recorded_observations.iter().map(|o| o.graph.clone()));
            for witness in &value.accepted_observations {
                pins.push(witness.occurrence.clone());
                pins.push(witness.source.clone());
            }
        }
        for revision in revisions {
            let graph_id = self.conn.query_row(
                "SELECT graph_id FROM revisions WHERE revision=?1",
                [revision],
                |r| r.get(0),
            )?;
            pins.push(GraphRef {
                graph_id,
                revision: revision.clone(),
            });
        }
        canonical_pins(&mut pins);
        if pins.len() > 1000 {
            return Err(err("E_BUDGET", "recorded actor pin limit exceeded"));
        }
        for pin in &pins {
            self.retention_whole(pin, host)?;
        }
        let state = RecordedActorState {
            inputs: RecordedActorInputs {
                input_snapshots: pins,
                ..before.inputs.clone()
            },
            state_revision: request.state_revision.clone(),
            state: request.state.clone(),
            artifacts: artifacts(&request.tool_results)?,
        };
        validate_state(&state, &self.actor_definition(&request.adapter)?)?;
        Ok(state)
    }
    /// Trusted host computation is recorded; the kernel commits actual effects, artifacts,
    /// outputs, state and its private delivery coordinate atomically.
    pub fn complete_recorded_actor_for(
        &mut self,
        request: &RecordedActorCompletion,
        host: &HostContext,
    ) -> Result<RecordedActorReceipt> {
        self.complete_recorded_actor_boundary(request, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn complete_recorded_actor_test_before_commit(
        &mut self,
        request: &RecordedActorCompletion,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<RecordedActorReceipt> {
        self.complete_recorded_actor_boundary(request, host, before_commit)
    }
    fn complete_recorded_actor_boundary(
        &mut self,
        request: &RecordedActorCompletion,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<RecordedActorReceipt> {
        let _budget = self.read_budget.enter();
        json_size(request, 4 * STATE_LIMIT)?;
        if !valid_id(&request.state_revision) || request.input_snapshots.len() > 1000 {
            return Err(err("E_ACTOR_STATE", "invalid actor state or pins"));
        }
        artifacts(&request.tool_results)?;
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _clock = self.operation_write_scope()?;
            self.require_adapter_host(&request.adapter, host)?;
            self.reject_governed_effect_adapter(&request.adapter)?;
            let definition = self.actor_definition(&request.adapter)?;
            self.require_uncanceled_delivery(&request.adapter, &request.event)?;
            let (graph_id, branch_id, revision, _) = self
                .scoped_event(&definition.manifest, &request.event)?
                .ok_or_else(|| err("E_UNAVAILABLE", "actor occurrence unavailable"))?;
            let event_pins = self.actor_closure(
                &definition,
                &GraphRef { graph_id, revision },
                &branch_id,
                host,
            )?;
            let current = self
                .actor_state(&request.adapter)?
                .ok_or_else(|| err("E_ACTOR_STATE", "actor state unavailable"))?;
            for pin in &current.inputs.input_snapshots {
                self.retention_whole(pin, host)?;
            }
            let digest = completion_digest(request)?;
            let token = CompletionToken {
                adapter: request.adapter.clone(),
                event: request.event.clone(),
            };
            if let Some(record) = self.actor_record(&request.adapter, &request.event)? {
                if record.request_digest != digest
                    || retention::hash(&record.before)? != request.prior_state_digest
                {
                    return Err(err(
                        "E_RECEIPT_CONFLICT",
                        "recorded occurrence has different computation",
                    ));
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
                if self.actor_effects(&request.adapter, &request.event)? != record.receipt.effects {
                    return Err(integrity());
                }
                // The typed loader already bound the actual handler journal and terminal
                // effects. Historical exact retries remain readable after retirement.
                let mut handler = record.receipt.handler.clone();
                handler.duplicate = true;
                if record.program_digest
                    != format!(
                        "{:x}",
                        Sha256::digest(serde_json::to_vec(&request.program)?)
                    )
                    || self.actor_after(
                        request,
                        &record.before,
                        &handler.results,
                        &event_pins,
                        host,
                    )? != record.after
                {
                    return Err(integrity());
                }
                let mut receipt = record.receipt;
                receipt.handler.duplicate = true;
                before_commit();
                return Ok(receipt);
            }
            self.actor_definition_for(&request.adapter, host)?;
            self.require_recorded_actor_ready(&request.adapter)?;
            self.check_lease(&request.adapter, &request.event, &request.lease)?;
            self.require_actor_new_computation(&request.adapter, &request.event)?;
            if retention::hash(&current)? != request.prior_state_digest {
                return Err(err("E_CONFLICT", "actor state changed"));
            }
            let effects = self.actor_effects(&request.adapter, &request.event)?;
            let (count, used): (i64, i64) = self.conn.query_row("SELECT count(*),coalesce(sum(length(CAST(body AS BLOB))),0) FROM recorded_actor_receipts WHERE adapter=?1", [&request.adapter], |r| Ok((r.get(0)?, r.get(1)?)))?;
            if count >= 128 {
                return Err(err("E_BUDGET", "recorded actor receipt count exceeded"));
            }
            let handler = self.complete_handler_with_actor(
                &request.adapter,
                &request.event,
                &request.lease,
                &request.program,
                &token,
            )?;
            if handler.duplicate {
                return Err(integrity());
            }
            let after = self.actor_after(request, &current, &handler.results, &event_pins, host)?;
            let checkpoint = self.dispatch_manifest(&request.adapter)?.2;
            let receipt = RecordedActorReceipt {
                receipt_id: retention::hash(&("weave-recorded-actor-receipt/1", &digest))?,
                state_digest: retention::hash(&after)?,
                artifacts: after.artifacts.clone(),
                effects,
                handler,
            };
            let record = Record {
                definition,
                request_digest: digest,
                program_digest: format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&request.program)?)
                ),
                before: current,
                after: after.clone(),
                receipt: receipt.clone(),
                checkpoint,
            };
            let body = serde_json::to_string(&record)?;
            if body.len() > RECORD_LIMIT
                || used
                    .checked_add(body.len() as i64)
                    .is_none_or(|n| n > 64 * STATE_LIMIT as i64)
            {
                return Err(err(
                    "E_BUDGET",
                    "recorded actor receipt byte limit exceeded",
                ));
            }
            self.put_actor_state(&after, checkpoint)?;
            self.conn.execute(
                "INSERT INTO recorded_actor_receipts VALUES (?1,?2,?3,?4)",
                params![
                    request.adapter,
                    request.event,
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
