//! Trusted compiled projection handlers. No caller Program or QueryResult is authority.
use super::*;
use serde::{Deserialize, Serialize};
const REGISTRATION_LIMIT: usize = 2 * 1024 * 1024;

/// Trusted host mapping of an inert artifact slot to one writable destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HandlerOutputBinding {
    pub slot: String,
    pub graph_id: String,
    pub branch_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreparedHandlerReceipt {
    pub duplicate: bool,
    pub preparation_id: String,
    pub definition_digest: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    slot: String,
    graph_id: String,
    branch_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    template: CompiledHandlerTemplate,
    output: Output,
    manifest: AdapterManifest,
}
fn digest(domain: &str, value: &impl Serialize) -> Result<String> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&(domain, value))?)
    ))
}
fn invalid() -> Error {
    err(
        "E_HANDLER_INTEGRITY",
        "compiled handler binding unavailable",
    )
}
impl Engine {
    pub(crate) fn initialize_compiled_handlers(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS compiled_handlers(adapter TEXT PRIMARY KEY REFERENCES dispatch_adapters(id),principal TEXT NOT NULL,registration TEXT NOT NULL,binding_digest TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS handler_preparations(adapter TEXT NOT NULL REFERENCES compiled_handlers(adapter),event_id TEXT NOT NULL,principal TEXT NOT NULL,preparation_id TEXT UNIQUE NOT NULL,body TEXT NOT NULL,body_digest TEXT NOT NULL,PRIMARY KEY(adapter,event_id));")?;
        Ok(())
    }
    pub(crate) fn is_compiled_handler(&self, adapter: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM compiled_handlers WHERE adapter=?1)",
            [adapter],
            |r| r.get(0),
        )?)
    }
    /// Installation is explicit trusted-host authority, never a serialized command.
    pub fn install_compiled_handler(
        &self,
        manifest: &AdapterManifest,
        template: &CompiledHandlerTemplate,
        output: &HandlerOutputBinding,
        authority: &HostContext,
    ) -> Result<()> {
        handler_registration::validate_handler_template(template)
            .map_err(|d| err(&d.code, &d.message))?;
        if !valid_id(&authority.principal)
            || !valid_id(&output.graph_id)
            || !valid_id(&output.branch_id)
            || output.slot != template.output_slot
            || manifest.principal != authority.principal
            || manifest.artifact_digest != template.definition_digest
            || manifest.subscriptions
                != [SubscriptionScope {
                    graph_id: template.input.graph_id.clone(),
                    branch_id: template.input.branch_id.clone(),
                }]
            || manifest.output_graphs != [output.graph_id.clone()]
            || !manifest.effect_destinations.is_empty()
            || !manifest.projection_replay
            || !authority.writable_graphs.contains(&output.graph_id)
            || (output.graph_id == template.input.graph_id
                && output.branch_id == template.input.branch_id)
        {
            return Err(err(
                "E_HANDLER_INSTALL",
                "invalid or unauthorized handler installation",
            ));
        }
        identity_acceptance::require_external_graph(&output.graph_id)?;
        json_size(manifest, 256 * 1024)?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let _clock = self.operation_write_scope()?;
        let registration = Registration {
            template: template.clone(),
            output: Output {
                slot: output.slot.clone(),
                graph_id: output.graph_id.clone(),
                branch_id: output.branch_id.clone(),
            },
            manifest: manifest.clone(),
        };
        json_size(&registration, REGISTRATION_LIMIT)?;
        let encoded = serde_json::to_string(&registration)?;
        if self.is_compiled_handler(&manifest.id)? {
            let prior = self.handler_registration(&manifest.id)?;
            if prior != registration {
                return Err(err(
                    "E_ADAPTER_VERSION",
                    "compiled installation is immutable",
                ));
            }
            tx.commit()?;
            return Ok(());
        }
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM dispatch_adapters WHERE id=?1)",
            [&manifest.id],
            |r| r.get(0),
        )?;
        if exists {
            return Err(err(
                "E_HANDLER_INSTALL",
                "legacy adapter cannot become compiled",
            ));
        }
        let (count, bytes): (i64, i64) = self.conn.query_row("SELECT COUNT(*),COALESCE(SUM(length(CAST(registration AS BLOB))),0) FROM compiled_handlers WHERE principal=?1", [&authority.principal], |r| Ok((r.get(0)?,r.get(1)?)))?;
        if count >= 128 || bytes.saturating_add(encoded.len() as i64) > 16 * 1024 * 1024 {
            return Err(err("E_BUDGET", "compiled registration quota exceeded"));
        }
        self.install_adapter(manifest, authority)?;
        self.conn.execute(
            "INSERT INTO compiled_handlers VALUES (?1,?2,?3,?4)",
            params![
                manifest.id,
                authority.principal,
                encoded,
                digest("weave-handler-registration-binding/1", &registration)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    fn handler_registration(&self, adapter: &str) -> Result<Registration> {
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(REGISTRATION_LIMIT);
        let (encoded, expected, principal): (Option<String>, String, String) = self.conn.query_row("SELECT CASE WHEN length(CAST(registration AS BLOB))<=?2 THEN registration END,substr(binding_digest,1,72),substr(principal,1,513) FROM compiled_handlers WHERE adapter=?1", params![adapter,limit as i64], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or_else(invalid)?;
        let encoded =
            encoded.ok_or_else(|| err("E_BUDGET", "compiled registration exceeds read budget"))?;
        self.read_budget.charge(encoded.len())?;
        let registration: Registration = serde_json::from_str(&encoded)?;
        handler_registration::validate_handler_template(&registration.template)
            .map_err(|_| invalid())?;
        let (manifest, _, _) = self.dispatch_manifest(adapter)?;
        if expected != digest("weave-handler-registration-binding/1", &registration)?
            || registration.manifest != manifest
            || manifest.id != adapter
            || manifest.principal != principal
            || manifest.artifact_digest != registration.template.definition_digest
        {
            return Err(invalid());
        }
        Ok(registration)
    }
}

const PREPARATION_LIMIT: usize = 20 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preparation {
    format: String,
    adapter: String,
    event: String,
    binding_digest: String,
    source: GraphRef,
    closure: Vec<GraphRef>,
    prepared_at_ms: i64,
    coverage: Coverage,
    diagnostics: Vec<Diagnostic>,
    program: Program,
}
fn unavailable() -> Error {
    err("E_HANDLER_INPUT", "handler input unavailable")
}
fn canonical_pins(pins: &mut Vec<GraphRef>) {
    pins.sort_by(|a, b| (&a.graph_id, &a.revision).cmp(&(&b.graph_id, &b.revision)));
    pins.dedup();
}
fn no_live(data: &GraphData) -> Result<()> {
    if data
        .attachments
        .iter()
        .any(|a| matches!(a.value, MetadataValue::LiveGraph { .. }))
    {
        return Err(err(
            "E_HANDLER_LIVE",
            "compiled input must pin all metadata",
        ));
    }
    Ok(())
}
impl Engine {
    fn handler_input(
        &self,
        registration: &Registration,
        event: &str,
    ) -> Result<(QueryResult, Vec<GraphRef>)> {
        let manifest = &registration.manifest;
        let host = HostContext::new(&manifest.principal, manifest.output_graphs.clone());
        let (graph_id, branch_id, revision, _) = self
            .scoped_event(manifest, event)?
            .ok_or_else(unavailable)?;
        let raw = self.load(&graph_id, &revision)?.ok_or_else(unavailable)?;
        no_live(&raw)?; // Never resolve a detectable live root just to reject it later.
        let input = self.query(
            &QueryPlan {
                graph_id: graph_id.clone(),
                revision: Some(revision.clone()),
                branch_id,
                predicate: None,
                from: None,
                to: None,
                valid_at: None,
                include_metadata: registration.template.input.metadata_depth > 0,
                max_depth: registration.template.input.metadata_depth,
            },
            &host,
        )?;
        if input.coverage != Coverage::Complete {
            return Err(unavailable());
        }
        if input.metadata_graphs.len() >= 1000 {
            return Err(err("E_BUDGET", "handler input closure limit"));
        }
        let mut closure = vec![GraphRef { graph_id, revision }];
        closure.extend(input.metadata_graphs.iter().map(|g| g.reference.clone()));
        canonical_pins(&mut closure);
        for reference in &closure {
            if !self.protected_reference_allowed(&reference.graph_id, &reference.revision, &host)? {
                return Err(unavailable());
            }
            let raw = self
                .load(&reference.graph_id, &reference.revision)?
                .ok_or_else(unavailable)?;
            no_live(&raw)?;
            let (authorized, partial) = self.authorized(raw.clone(), &host)?;
            if !whole_graph_visible(&raw, authorized, partial) {
                return Err(unavailable());
            }
        }
        Ok((input, closure))
    }
    fn handler_running(&self, registration: &Registration, event: &str) -> Result<()> {
        let (_, state, _) = self.dispatch_manifest(&registration.manifest.id)?;
        if state != "running" && state != "draining" {
            return Err(err("E_PAUSED", "adapter is not running"));
        }
        if state == "draining" {
            let pending: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM dispatch_pending WHERE adapter=?1 AND event_id=?2)",
                params![registration.manifest.id, event],
                |r| r.get(0),
            )?;
            if !pending {
                return Err(err(
                    "E_PAUSED",
                    "draining adapter requires pending occurrence",
                ));
            }
        }
        Ok(())
    }
    fn handler_completed(&self, adapter: &str, event: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM handler_receipts WHERE adapter=?1 AND event_id=?2)",
            params![adapter, event],
            |r| r.get(0),
        )?)
    }
    fn stored_preparation(
        &self,
        adapter: &str,
        event: &str,
    ) -> Result<Option<(String, Preparation)>> {
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(PREPARATION_LIMIT);
        type Row = (String, Option<String>, String, String);
        let row:Option<Row>=self.conn.query_row("SELECT substr(preparation_id,1,129),CASE WHEN length(CAST(body AS BLOB))<=?3 THEN body END,substr(body_digest,1,72),substr(principal,1,513) FROM handler_preparations WHERE adapter=?1 AND event_id=?2",params![adapter,event,limit as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((id, body, expected, principal)) = row else {
            return Ok(None);
        };
        let body = body.ok_or_else(|| err("E_BUDGET", "preparation exceeds read budget"))?;
        self.read_budget.charge(body.len())?;
        let preparation: Preparation = serde_json::from_str(&body)?;
        let registration = self.handler_registration(adapter)?;
        if digest("weave-handler-preparation-body/1", &preparation)? != expected
            || preparation.adapter != adapter
            || preparation.event != event
            || principal != registration.manifest.principal
            || !id.starts_with("preparation:")
            || id.len() != 60
        {
            return Err(invalid());
        }
        Ok(Some((id, preparation)))
    }
    fn validate_preparation(
        &self,
        registration: &Registration,
        preparation: &Preparation,
    ) -> Result<()> {
        if preparation.format != "weave-handler-preparation/1"
            || preparation.binding_digest
                != digest("weave-handler-registration-binding/1", registration)?
            || preparation.prepared_at_ms < 0
            || ![VERSION, "0.18.0"].contains(&preparation.program.version.as_str())
        {
            return Err(invalid());
        }
        let (input, closure) = self.handler_input(registration, &preparation.event)?;
        let source = GraphRef {
            graph_id: registration.template.input.graph_id.clone(),
            revision: input
                .snapshots
                .get(&registration.template.input.graph_id)
                .cloned()
                .ok_or_else(invalid)?,
        };
        if preparation.source != source
            || preparation.closure != closure
            || preparation.program.source_revisions != registration.template.source_revisions
        {
            return Err(invalid());
        }
        let [Command::Commit {
            graph_id,
            branch_id,
            data,
            ..
        }] = preparation.program.commands.as_slice()
        else {
            return Err(invalid());
        };
        if preparation.program.version != VERSION
            && weave_contract::carrier_profile::requires_v019(data)
        {
            return Err(invalid());
        }
        if graph_id != &registration.output.graph_id || branch_id != &registration.output.branch_id
        {
            return Err(invalid());
        }
        json_size(&preparation.program, 16 * 1024 * 1024)?;
        verify_output_gates(data, &closure)?;
        let host = HostContext::new(
            &registration.manifest.principal,
            registration.manifest.output_graphs.clone(),
        );
        let (authorized, partial) = self.authorized(data.clone(), &host)?;
        if !whole_graph_visible(data, authorized, partial) {
            return Err(unavailable());
        }
        verify_attribution(data, registration, preparation)?;
        Ok(())
    }
    pub fn prepare_compiled_handler(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
    ) -> Result<PreparedHandlerReceipt> {
        self.prepare_handler_boundary(adapter, event, lease, None, || {})
    }
    /// Prepare under the embedding session's durable principal/output authority.
    pub fn prepare_compiled_handler_for(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        host: &HostContext,
    ) -> Result<PreparedHandlerReceipt> {
        self.prepare_handler_boundary(adapter, event, lease, Some(host), || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn prepare_compiled_handler_test_before_commit(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        before_commit: impl FnOnce(),
    ) -> Result<PreparedHandlerReceipt> {
        self.prepare_handler_boundary(adapter, event, lease, None, before_commit)
    }
    fn prepare_handler_boundary(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        host: Option<&HostContext>,
        before_commit: impl FnOnce(),
    ) -> Result<PreparedHandlerReceipt> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _clock = self.operation_write_scope()?;
            if let Some(host) = host {
                self.require_adapter_host(adapter, host)?;
            }
            let registration = self.handler_registration(adapter)?;
            self.handler_running(&registration, event)?;
            if let Some((id, preparation)) = self.stored_preparation(adapter, event)? {
                self.validate_preparation(&registration, &preparation)?;
                if !self.handler_completed(adapter, event)? {
                    self.check_lease(adapter, event, lease)?;
                }
                return Ok(PreparedHandlerReceipt {
                    duplicate: true,
                    preparation_id: id,
                    definition_digest: registration.template.definition_digest,
                });
            }
            self.check_lease(adapter, event, lease)?;
            let (mut input, closure) = self.handler_input(&registration, event)?;
            let source = GraphRef {
                graph_id: registration.template.input.graph_id.clone(),
                revision: input
                    .snapshots
                    .get(&registration.template.input.graph_id)
                    .cloned()
                    .ok_or_else(invalid)?,
            };
            let gate = GraphInfluence {
                derivations: Vec::new(),
                snapshots: closure.clone(),
                ..Default::default()
            };
            let inherited = weave_contract::influence::input_influence(&input.graph)
                .map_err(|d| err(&d.code, &d.message))?;
            input.graph.influence =
                weave_contract::influence::merge(inherited.as_ref(), Some(&gate))
                    .map_err(|d| err(&d.code, &d.message))?;
            merge_sources(
                &mut input.source_revisions,
                &registration.template.source_revisions,
            )?;
            let mut materialized = json_size(&input, MATERIALIZED_LIMIT)?;
            let mut object_count =
                input.graph.nodes.len() + input.graph.edges.len() + input.graph.attachments.len();
            let mut values = BTreeMap::from([(
                handler_registration::HANDLER_EVENT_BINDING.to_string(),
                input,
            )]);
            let host = HostContext::new(
                &registration.manifest.principal,
                registration.manifest.output_graphs.clone(),
            );
            let mut work = 1000;
            for binding in &registration.template.recipe.bindings {
                charge_references(&binding.value, &values, &mut materialized)?;
                let mut result = self.expression(&binding.value, &values, &host, 0, &mut work)?;
                merge_sources(
                    &mut result.source_revisions,
                    &registration.template.source_revisions,
                )?;
                materialized +=
                    json_size(&result, MATERIALIZED_LIMIT.saturating_sub(materialized))?;
                object_count += result.graph.nodes.len()
                    + result.graph.edges.len()
                    + result.graph.attachments.len();
                if object_count > 200_000 {
                    return Err(err("E_BUDGET", "handler bound-object budget exceeded"));
                }
                values.insert(binding.name.clone(), result);
            }
            let mut result = values
                .remove(&registration.template.recipe.output)
                .ok_or_else(invalid)?;
            result.graph.influence =
                weave_contract::influence::merge(result.graph.influence.as_ref(), Some(&gate))
                    .map_err(|d| err(&d.code, &d.message))?;
            result.source_revisions =
                algebra::merge_source_revisions(&result.source_revisions, &[])
                    .map_err(|d| err(&d.code, &d.message))?;
            if result.source_revisions != registration.template.source_revisions {
                return Err(invalid());
            }
            if result.diagnostics.len() > 256 {
                return Err(err("E_BUDGET", "handler diagnostic count exceeded"));
            }
            json_size(&result.diagnostics, 64 * 1024)?;
            let coverage = result.coverage.clone();
            let diagnostics = result.diagnostics.clone();
            let data = owned_output(result, &registration, event)?;
            let expected_head = self.head(
                &registration.output.graph_id,
                &registration.output.branch_id,
            )?;
            if let Some(revision) = &expected_head {
                if !self.protected_reference_allowed(
                    &registration.output.graph_id,
                    revision,
                    &host,
                )? {
                    return Err(unavailable());
                }
                let raw = self
                    .load(&registration.output.graph_id, revision)?
                    .ok_or_else(unavailable)?;
                let (authorized, partial) = self.authorized(raw.clone(), &host)?;
                if !whole_graph_visible(&raw, authorized, partial) {
                    return Err(unavailable());
                }
            }
            let program = Program {
                version: VERSION.into(),
                source_revisions: registration.template.source_revisions.clone(),
                commands: vec![Command::Commit {
                    graph_id: registration.output.graph_id.clone(),
                    branch_id: registration.output.branch_id.clone(),
                    expected_head,
                    data,
                }],
            };
            json_size(&program, 16 * 1024 * 1024)?;
            let preparation = Preparation {
                format: "weave-handler-preparation/1".into(),
                adapter: adapter.into(),
                event: event.into(),
                binding_digest: digest("weave-handler-registration-binding/1", &registration)?,
                source,
                closure,
                prepared_at_ms: self.operation_time()?,
                coverage,
                diagnostics,
                program,
            };
            self.validate_preparation(&registration, &preparation)?;
            json_size(&preparation, PREPARATION_LIMIT)?;
            let body = serde_json::to_string(&preparation)?;
            let (count,bytes):(i64,i64)=self.conn.query_row("SELECT COUNT(*),COALESCE(SUM(length(CAST(body AS BLOB))),0) FROM handler_preparations WHERE principal=?1",[&registration.manifest.principal],|r|Ok((r.get(0)?,r.get(1)?)))?;
            if count >= 256 || bytes.saturating_add(body.len() as i64) > 64 * 1024 * 1024 {
                return Err(err(
                    "E_BACKPRESSURE",
                    "retained handler preparation quota exceeded",
                ));
            }
            let id: String = self.conn.query_row(
                "SELECT 'preparation:' || lower(hex(randomblob(24)))",
                [],
                |r| r.get(0),
            )?;
            self.conn.execute(
                "INSERT INTO handler_preparations VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    adapter,
                    event,
                    registration.manifest.principal,
                    id,
                    body,
                    digest("weave-handler-preparation-body/1", &preparation)?
                ],
            )?;
            before_commit();
            Ok(PreparedHandlerReceipt {
                duplicate: false,
                preparation_id: id,
                definition_digest: registration.template.definition_digest,
            })
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
    pub fn complete_prepared_handler(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        preparation_id: &str,
    ) -> Result<HandlerReceipt> {
        self.complete_prepared_boundary(adapter, event, lease, preparation_id, None, || {})
    }
    /// Complete under current durable host authority, including historical retries.
    pub fn complete_prepared_handler_for(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        preparation_id: &str,
        host: &HostContext,
    ) -> Result<HandlerReceipt> {
        self.complete_prepared_boundary(adapter, event, lease, preparation_id, Some(host), || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn complete_prepared_handler_test_before_commit(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        preparation_id: &str,
        before_commit: impl FnOnce(),
    ) -> Result<HandlerReceipt> {
        self.complete_prepared_boundary(adapter, event, lease, preparation_id, None, before_commit)
    }
    fn complete_prepared_boundary(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        preparation_id: &str,
        host: Option<&HostContext>,
        before_commit: impl FnOnce(),
    ) -> Result<HandlerReceipt> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _clock = self.operation_write_scope()?;
            if let Some(host) = host {
                self.require_adapter_host(adapter, host)?;
            }
            let registration = self.handler_registration(adapter)?;
            self.handler_running(&registration, event)?;
            let (id, preparation) = self
                .stored_preparation(adapter, event)?
                .ok_or_else(invalid)?;
            if id != preparation_id {
                return Err(invalid());
            }
            self.validate_preparation(&registration, &preparation)?;
            let receipt =
                self.complete_handler_in_transaction(adapter, event, lease, &preparation.program)?;
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

fn charge_references(
    expression: &GraphExpression,
    values: &BTreeMap<String, QueryResult>,
    used: &mut usize,
) -> Result<()> {
    use GraphExpression::*;
    match expression {
        Reference { name } => {
            let value = values.get(name).ok_or_else(invalid)?;
            *used += json_size(value, MATERIALIZED_LIMIT.saturating_sub(*used))?;
        }
        Window { input, .. }
        | Filter { input, .. }
        | Project { input, .. }
        | Support { input, .. }
        | Metadata { input, .. }
        | Reason { input, .. }
        | Context { input, .. }
        | Explain { input }
        | Counterparts { input, .. } => charge_references(input, values, used)?,
        Sequence { left, right, .. } | Union { left, right } | Join { left, right, .. } => {
            charge_references(left, values, used)?;
            charge_references(right, values, used)?;
        }
        Diff { before, after } => {
            charge_references(before, values, used)?;
            charge_references(after, values, used)?;
        }
        Geometry { operation, .. } => {
            for input in operation.inputs() {
                charge_references(input, values, used)?;
            }
        }
        Query { .. }
        | TypedContext { .. }
        | AcceptedGraph { .. }
        | CurrentView { .. }
        | ResolveIdentity { .. }
        | Cluster { .. } => return Err(invalid()),
    }
    Ok(())
}
fn local_id(namespace: &str, kind: &str, id: &str) -> Result<String> {
    Ok(format!(
        "handler:{:x}",
        Sha256::digest(serde_json::to_vec(&(
            "weave-handler-object/1",
            namespace,
            kind,
            id
        ))?)
    ))
}
fn owned_output(
    mut result: QueryResult,
    registration: &Registration,
    event: &str,
) -> Result<GraphData> {
    if result.graph.profile != GraphProfile::Legacy
        || !result.graph.structural_edges.is_empty()
        || !result.graph.assertions.is_empty()
    {
        return Err(err(
            "E_HANDLER_PROFILE",
            "handler requires materialized legacy output",
        ));
    }
    json_size(&result, MATERIALIZED_LIMIT)?;
    no_live(&result.graph)?;
    identity_acceptance::require_external_schema(&result.graph)?;
    let namespace = digest(
        "weave-handler-output-namespace/1",
        &(&registration.template.definition_digest, event),
    )?;
    let mut proof_bytes = json_size(&result, MATERIALIZED_LIMIT)?;
    for node in &mut result.graph.nodes {
        if let Some(origins) = result.node_origins.get(&node.id) {
            proof_bytes += json_size(origins, MATERIALIZED_LIMIT.saturating_sub(proof_bytes))?;
            node.derived_nodes.extend(origins.iter().cloned());
            node.derived_nodes.sort_by(|a, b| {
                weave_contract::influence::node_key(a).cmp(&weave_contract::influence::node_key(b))
            });
            node.derived_nodes.dedup();
        }
    }
    for edge in &mut result.graph.edges {
        let origins = result
            .edge_origins
            .get(&edge.id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        proof_bytes += json_size(
            &(origins, &edge.derivations),
            MATERIALIZED_LIMIT.saturating_sub(proof_bytes),
        )?;
        if edge.derivations.is_empty() {
            // Preserve an original record's exact assertion and existing AND dependencies.
            let mut sources = origins.to_vec();
            sources.extend(edge.derived_from.iter().cloned());
            sources.sort_by(|a, b| {
                weave_contract::influence::assertion_key(a)
                    .cmp(&weave_contract::influence::assertion_key(b))
            });
            sources.dedup();
            edge.derivations =
                algebra::edge_alternatives(edge, &sources).map_err(|d| err(&d.code, &d.message))?;
        }
        // Existing OR alternatives remain alternatives. The flat vector is their index.
        edge.derived_from = edge
            .derivations
            .iter()
            .flat_map(|g| g.premises.iter().cloned())
            .collect();
        edge.derived_from.sort_by(|a, b| {
            weave_contract::influence::assertion_key(a)
                .cmp(&weave_contract::influence::assertion_key(b))
        });
        edge.derived_from.dedup();
    }
    for attachment in &mut result.graph.attachments {
        if let Some(origins) = result.attachment_origins.get(&attachment.id) {
            proof_bytes += json_size(origins, MATERIALIZED_LIMIT.saturating_sub(proof_bytes))?;
            attachment.derived_from.extend(origins.iter().cloned());
            attachment.derived_from.sort_by(|a, b| {
                weave_contract::influence::assertion_key(a)
                    .cmp(&weave_contract::influence::assertion_key(b))
            });
            attachment.derived_from.dedup();
        }
    }
    let nodes: BTreeMap<String, String> = result
        .graph
        .nodes
        .iter()
        .map(|n| Ok((n.id.clone(), local_id(&namespace, "node", &n.id)?)))
        .collect::<Result<_>>()?;
    let edges: BTreeMap<String, String> = result
        .graph
        .edges
        .iter()
        .map(|e| Ok((e.id.clone(), local_id(&namespace, "edge", &e.id)?)))
        .collect::<Result<_>>()?;
    let mut generated = HashSet::new();
    for n in &mut result.graph.nodes {
        n.id = nodes.get(&n.id).cloned().ok_or_else(invalid)?;
        n.readers = vec![registration.manifest.principal.clone()];
        if !generated.insert(n.id.clone()) {
            return Err(invalid());
        }
    }
    for e in &mut result.graph.edges {
        e.id = edges.get(&e.id).cloned().ok_or_else(invalid)?;
        e.from = nodes.get(&e.from).cloned().ok_or_else(invalid)?;
        e.to = nodes.get(&e.to).cloned().ok_or_else(invalid)?;
        e.readers = vec![registration.manifest.principal.clone()];
        if !generated.insert(e.id.clone()) {
            return Err(invalid());
        }
    }
    for a in &mut result.graph.attachments {
        a.id = local_id(&namespace, "attachment", &a.id)?;
        match &mut a.host {
            MetadataHost::Node { id } => *id = nodes.get(id).cloned().ok_or_else(invalid)?,
            MetadataHost::Edge { id } => *id = edges.get(id).cloned().ok_or_else(invalid)?,
            MetadataHost::Assertion { .. } => return Err(invalid()),
            MetadataHost::Graph | MetadataHost::Entity { .. } => {}
        }
        a.readers = vec![registration.manifest.principal.clone()];
        if !generated.insert(a.id.clone()) {
            return Err(invalid());
        }
    }
    let attribution = MetadataAttachment {
        derivations: Vec::new(),
        id: local_id(&namespace, "attribution", "sources")?,
        host: MetadataHost::Graph,
        key: "weave.handler.attribution".into(),
        value: MetadataValue::Literal {
            value: serde_json::json!({"format":"weave-handler-attribution/1","template_digest":registration.template.definition_digest,"source_revisions":result.source_revisions,"coverage":result.coverage,"diagnostics":result.diagnostics}),
        },
        valid_time: Interval {
            start: 0,
            end: None,
        },
        origin: None,
        readers: vec![registration.manifest.principal.clone()],
        required: false,
        schema_revision: None,
        context: result
            .selected_context
            .as_ref()
            .and_then(ContextSelection::reference)
            .cloned(),
        derived_from: vec![],
        derived_nodes: vec![],
        derived_snapshots: vec![],
    };
    if !generated.insert(attribution.id.clone()) {
        return Err(invalid());
    }
    result.graph.attachments.push(attribution);
    // This is now a newly owned snapshot. No changed object is labeled as an original record.
    result.node_origins.clear();
    result.edge_origins.clear();
    result.attachment_origins.clear();
    result.metadata_graphs.clear();
    context_typing::protect_result_generated(&mut result).map_err(|d| err(&d.code, &d.message))?;
    weave_contract::influence::protect_generated_result(&mut result, MATERIALIZED_LIMIT)
        .map_err(|d| err(&d.code, &d.message))?;
    let data = result.graph;
    check_output_capacity(&data)?;
    validate_graph(&data)?;
    json_size(&data, 16 * 1024 * 1024)?;
    Ok(data)
}
fn check_output_capacity(data: &GraphData) -> Result<()> {
    let excessive =
        data.nodes.iter().any(|n| {
            n.derived_from.len() + n.derived_nodes.len() + n.derived_snapshots.len() > 999
        }) || data.edges.iter().any(|e| {
            e.derived_from.len() + e.derived_nodes.len() + e.derived_snapshots.len() > 999
        }) || data.attachments.iter().any(|a| {
            a.derived_from.len()
                + a.derived_nodes.len()
                + a.derived_snapshots.len()
                + usize::from(a.origin.is_some())
                > 999
        }) || data
            .influence
            .as_ref()
            .is_some_and(|i| i.assertions.len() + i.nodes.len() + i.snapshots.len() > 999);
    if excessive {
        return Err(err(
            "E_BUDGET",
            "handler output leaves no materialization proof capacity",
        ));
    }
    Ok(())
}
fn verify_output_gates(data: &GraphData, closure: &[GraphRef]) -> Result<()> {
    validate_graph(data)?;
    check_output_capacity(data)?;
    no_live(data)?;
    let Some(influence) = &data.influence else {
        return Err(invalid());
    };
    let includes = |pins: &[GraphRef]| closure.iter().all(|p| pins.contains(p));
    if !includes(&influence.snapshots)
        || data.nodes.iter().any(|n| !includes(&n.derived_snapshots))
        || data.edges.iter().any(|e| !includes(&e.derived_snapshots))
        || data
            .attachments
            .iter()
            .any(|a| !includes(&a.derived_snapshots))
    {
        return Err(invalid());
    }
    Ok(())
}
fn verify_attribution(
    data: &GraphData,
    registration: &Registration,
    preparation: &Preparation,
) -> Result<()> {
    json_size(&preparation.diagnostics, 64 * 1024)?;
    if preparation.diagnostics.len() > 256 {
        return Err(invalid());
    }
    let namespace = digest(
        "weave-handler-output-namespace/1",
        &(&registration.template.definition_digest, &preparation.event),
    )?;
    let id = local_id(&namespace, "attribution", "sources")?;
    let expected = serde_json::json!({"format":"weave-handler-attribution/1","template_digest":registration.template.definition_digest,"source_revisions":preparation.program.source_revisions,"coverage":preparation.coverage,"diagnostics":preparation.diagnostics});
    let Some(attachment) = data.attachments.iter().find(|a| a.id == id) else {
        return Err(invalid());
    };
    if attachment.host != MetadataHost::Graph
        || attachment.key != "weave.handler.attribution"
        || attachment.value != (MetadataValue::Literal { value: expected })
    {
        return Err(invalid());
    }
    let correct = |readers: &[String]| readers == [registration.manifest.principal.as_str()];
    if data.nodes.iter().any(|n| !correct(&n.readers))
        || data.edges.iter().any(|e| !correct(&e.readers))
        || data.attachments.iter().any(|a| !correct(&a.readers))
    {
        return Err(invalid());
    }
    Ok(())
}
