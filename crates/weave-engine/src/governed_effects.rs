//! Explicit trusted-host execution grants for genuine accepted graph occurrences.
use super::*;
use serde::{Deserialize, Serialize};
const REGISTRATION_LIMIT: usize = 128 * 1024;
const PAYLOAD_LIMIT: usize = 1024 * 1024;
const CONTEXT_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectEncoder {
    CanonicalGraphV1,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectStart {
    AfterInstallation,
    ReplayHistory,
}
/// Trusted native configuration; no serialized Program can install it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedEffectGrant {
    pub id: String,
    pub revision: String,
    pub principal: String,
    pub view_id: String,
    pub source: SubscriptionScope,
    pub request_schema: GraphSchema,
    pub destination_id: String,
    pub destination_principal: String,
    pub encoder: EffectEncoder,
    pub execution_id: String,
    pub start: EffectStart,
    pub not_before_ms: i64,
    pub expires_at_ms: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GovernedEffectDisposition {
    Intent { intent_id: String },
    PolicyChange,
    SupersededPublication,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedEffectReceipt {
    pub duplicate: bool,
    pub ordinal: u64,
    pub disposition: GovernedEffectDisposition,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GovernedEffectStatus {
    pub intent_id: String,
    pub attempt_id: Option<String>,
    pub state: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GovernedEffectDispatch {
    pub intent_id: String,
    pub attempt_id: String,
    pub destination_id: String,
    pub idempotency_key: String,
    pub payload: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciledOutcome {
    Confirmed,
    Failed,
}
/// Evidence supplied by the explicitly trusted broker, not remote attestation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SinkEvidence {
    pub receipt_id: String,
    pub response_digest: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    grant: GovernedEffectGrant,
    manifest: AdapterManifest,
}
fn unavailable() -> Error {
    err("E_GOV_EFFECT_UNAVAILABLE", "governed effect unavailable")
}
fn digest(domain: &str, value: &impl Serialize) -> Result<String> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&(domain, value))?)
    ))
}
/// Bind this digest into the installed AdapterManifest.artifact_digest.
pub fn governed_effect_grant_digest(grant: &GovernedEffectGrant) -> Result<String> {
    json_size(grant, 64 * 1024)?;
    digest("weave-governed-effect-grant/1", grant)
}
fn check_registration(reg: &Registration) -> Result<()> {
    let g = &reg.grant;
    let m = &reg.manifest;
    for id in [
        &g.id,
        &g.revision,
        &g.principal,
        &g.view_id,
        &g.source.graph_id,
        &g.source.branch_id,
        &g.destination_id,
        &g.destination_principal,
        &g.execution_id,
    ] {
        if !valid_id(id) {
            return Err(unavailable());
        }
    }
    if g.principal != g.destination_principal
        || g.principal != m.principal
        || m.subscriptions != [g.source.clone()]
        || !m.output_graphs.is_empty()
        || m.effect_destinations != [g.destination_id.clone()]
        || m.projection_replay
        || m.artifact_digest != governed_effect_grant_digest(g)?
        || g.not_before_ms < 0
        || g.expires_at_ms <= g.not_before_ms
    {
        return Err(unavailable());
    }
    let empty = GraphData {
        schema: Some(g.request_schema.clone()),
        ..GraphData::default()
    };
    if !weave_contract::validate_schema_graph(&empty).is_empty() {
        return Err(unavailable());
    }
    json_size(m, 64 * 1024)?;
    Ok(())
}
impl Engine {
    pub(crate) fn initialize_governed_effects(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS governed_effect_bindings(adapter TEXT PRIMARY KEY REFERENCES dispatch_adapters(id),principal TEXT NOT NULL,destination_id TEXT NOT NULL,execution_id TEXT NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,revoked INTEGER NOT NULL DEFAULT 0,UNIQUE(principal,destination_id,execution_id));
CREATE TABLE IF NOT EXISTS governed_effect_receipts(adapter TEXT NOT NULL REFERENCES governed_effect_bindings(adapter),event_id TEXT NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(adapter,event_id));
CREATE TABLE IF NOT EXISTS governed_effect_context(intent_id TEXT PRIMARY KEY REFERENCES effect_intents(id),adapter TEXT NOT NULL REFERENCES governed_effect_bindings(adapter),principal TEXT NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,attempt_id TEXT,reconciliation_digest TEXT);")?;
        Ok(())
    }
    pub(crate) fn is_governed_effect(&self, adapter: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM governed_effect_bindings WHERE adapter=?1)",
            [adapter],
            |r| r.get(0),
        )?)
    }
    pub(crate) fn reject_governed_effect_adapter(&self, adapter: &str) -> Result<()> {
        if self.is_governed_effect(adapter)? {
            return Err(err(
                "E_EFFECT_BOUND",
                "governed effects require their bound API",
            ));
        }
        Ok(())
    }
    pub(crate) fn reject_governed_effect_intent(&self, id: &str) -> Result<()> {
        // The adapter binding also protects an intent whose auxiliary context is missing.
        let bound: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM effect_intents i JOIN governed_effect_bindings b ON b.adapter=i.adapter WHERE i.id=?1) OR EXISTS(SELECT 1 FROM governed_effect_context WHERE intent_id=?1)", [id], |r|r.get(0))?;
        if bound {
            return Err(err(
                "E_EFFECT_BOUND",
                "governed effects require their bound API",
            ));
        }
        Ok(())
    }
    fn effect_registration(&self, adapter: &str) -> Result<(Registration, bool)> {
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(REGISTRATION_LIMIT);
        type Row = (Option<String>, String, String, String, String, bool);
        let row: Row = self.conn.query_row("SELECT CASE WHEN length(CAST(body AS BLOB))<=?2 THEN body END,substr(digest,1,72),substr(principal,1,513),substr(destination_id,1,513),substr(execution_id,1,513),revoked FROM governed_effect_bindings WHERE adapter=?1", params![adapter,limit as i64], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?.ok_or_else(unavailable)?;
        let body = row
            .0
            .ok_or_else(|| err("E_BUDGET", "effect registration read bound"))?;
        self.read_budget.charge(body.len())?;
        let reg: Registration = serde_json::from_str(&body).map_err(|_| unavailable())?;
        check_registration(&reg)?;
        if reg.manifest.id != adapter
            || row.1 != digest("weave-governed-effect-registration/1", &reg)?
            || row.2 != reg.grant.principal
            || row.3 != reg.grant.destination_id
            || row.4 != reg.grant.execution_id
            || self.dispatch_manifest(adapter)?.0 != reg.manifest
        {
            return Err(unavailable());
        }
        Ok((reg, row.5))
    }
    fn active_effect_registration(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<Registration> {
        let (reg, revoked) = self.effect_registration(adapter)?;
        let now = self.operation_time()?;
        if revoked
            || reg.grant.principal != host.principal
            || now < reg.grant.not_before_ms
            || now >= reg.grant.expires_at_ms
        {
            return Err(unavailable());
        }
        Ok(reg)
    }
    pub(crate) fn guard_governed_effect_poll(
        &self,
        adapter: &str,
        view: &str,
        host: &HostContext,
    ) -> Result<()> {
        if self.is_governed_effect(adapter)?
            && self
                .active_effect_registration(adapter, host)?
                .grant
                .view_id
                != view
        {
            return Err(unavailable());
        }
        Ok(())
    }
    pub fn install_governed_effect(
        &self,
        manifest: &AdapterManifest,
        grant: &GovernedEffectGrant,
        host: &HostContext,
    ) -> Result<()> {
        json_size(grant, 64 * 1024)?;
        json_size(manifest, 64 * 1024)?;
        let reg = Registration {
            grant: grant.clone(),
            manifest: manifest.clone(),
        };
        check_registration(&reg)?;
        if host.principal != grant.principal {
            return Err(unavailable());
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let _clock = self.operation_write_scope()?;
        let now = self.operation_time()?;
        if now < grant.not_before_ms || now >= grant.expires_at_ms {
            return Err(unavailable());
        }
        self.inspect_governance_head(&grant.view_id, host)?;
        if self.is_governed_effect(&manifest.id)? {
            let prior = self.active_effect_registration(&manifest.id, host)?;
            let active: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM governance_subscriptions WHERE adapter=?1 AND view_id=?2 AND state='active')",params![manifest.id,grant.view_id],|r|r.get(0))?;
            if prior != reg || !active || self.dispatch_manifest(&manifest.id)?.1 == "removed" {
                return Err(unavailable());
            }
            tx.commit()?;
            return Ok(());
        }
        let occupied: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM dispatch_adapters WHERE id=?1) OR EXISTS(SELECT 1 FROM governed_effect_bindings WHERE principal=?2 AND destination_id=?3 AND execution_id=?4)",params![manifest.id,grant.principal,grant.destination_id,grant.execution_id],|r|r.get(0))?;
        if occupied {
            return Err(err(
                "E_EFFECT_IDENTITY",
                "adapter or execution namespace already reserved",
            ));
        }
        let body = serde_json::to_string(&reg)?;
        let (count,bytes):(i64,i64)=self.conn.query_row("SELECT COUNT(*),COALESCE(SUM(length(CAST(body AS BLOB))),0) FROM governed_effect_bindings WHERE principal=?1",[&host.principal],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if count >= 128 || bytes.saturating_add(body.len() as i64) > 16 * 1024 * 1024 {
            return Err(err("E_BUDGET", "effect registration quota"));
        }
        self.install_adapter(manifest, host)?;
        // Binding is installed only after the existing private subscription is created,
        // within this same outer writer transaction. Public subscription then rejects it.
        self.subscribe_governance(&manifest.id, &grant.view_id, host)?;
        if grant.start == EffectStart::AfterInstallation {
            self.conn.execute("UPDATE governance_subscriptions SET checkpoint=(SELECT COALESCE(MAX(sequence),0) FROM governance_events WHERE view_id=?2) WHERE adapter=?1 AND view_id=?2",params![manifest.id,grant.view_id])?;
        }
        self.conn.execute(
            "INSERT INTO governed_effect_bindings VALUES (?1,?2,?3,?4,?5,?6,0)",
            params![
                manifest.id,
                grant.principal,
                grant.destination_id,
                grant.execution_id,
                body,
                digest("weave-governed-effect-registration/1", &reg)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn revoke_governed_effect_grant(&self, adapter: &str, host: &HostContext) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let _clock = self.operation_write_scope()?;
        let (reg, revoked) = self.effect_registration(adapter)?;
        if reg.grant.principal != host.principal {
            return Err(unavailable());
        }
        if !revoked {
            self.conn.execute(
                "UPDATE governed_effect_bindings SET revoked=1 WHERE adapter=?1",
                [adapter],
            )?;
            self.conn.execute("UPDATE governance_subscriptions SET state='canceled',epoch=epoch+1 WHERE adapter=?1",[adapter])?;
            self.conn.execute(
                "DELETE FROM governance_delivery_pending WHERE adapter=?1",
                [adapter],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    format: String,
    source: GraphRef,
    publication: GraphRef,
    graph: GraphData,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentContext {
    adapter: String,
    event: String,
    registration: String,
    source: GraphRef,
    publication: GraphRef,
    closure: Vec<GraphRef>,
    payload_digest: String,
    idempotency_key: String,
    created_at_ms: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptBody {
    event: String,
    registration: String,
    ordinal: u64,
    disposition: GovernedEffectDisposition,
}
struct StoredIntent {
    context: IntentContext,
    payload: String,
    state: String,
    attempt: Option<String>,
    reconciliation: Option<String>,
}
struct AuthorizedRequest {
    payload: String,
    source: GraphRef,
    publication: GraphRef,
    closure: Vec<GraphRef>,
    current: bool,
}
fn canonical_pins(pins: &mut [GraphRef]) {
    pins.sort_by(|a, b| (&a.graph_id, &a.revision).cmp(&(&b.graph_id, &b.revision)));
}
fn registration_digest(reg: &Registration) -> Result<String> {
    digest("weave-governed-effect-registration/1", reg)
}
fn normalize_unavailable(error: Error) -> Error {
    if ["E_BUDGET", "E_CLOCK", "E_STORAGE"].contains(&error.code.as_str()) {
        error
    } else {
        unavailable()
    }
}
impl Engine {
    fn effect_active(&self, adapter: &str, host: &HostContext) -> Result<Registration> {
        let reg = self.active_effect_registration(adapter, host)?;
        let state = self.dispatch_manifest(adapter)?.1;
        if state != "running" && state != "draining" {
            return Err(unavailable());
        }
        let active:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM governance_subscriptions WHERE adapter=?1 AND view_id=?2 AND state='active')",params![adapter,reg.grant.view_id],|r|r.get(0))?;
        if !active {
            return Err(unavailable());
        }
        Ok(reg)
    }
    fn effect_event(
        &self,
        reg: &Registration,
        event: &str,
        host: &HostContext,
    ) -> Result<GovernanceEvent> {
        self.governance_delivery_event(&reg.grant.view_id, event, host)
            .map_err(normalize_unavailable)?
            .ok_or_else(unavailable)
    }
    fn effect_request(
        &self,
        reg: &Registration,
        event: &GovernanceEvent,
        host: &HostContext,
    ) -> Result<AuthorizedRequest> {
        let p = self
            .effect_publication(&reg.grant.view_id, &event.decision_id, host)
            .map_err(normalize_unavailable)?;
        if p.source.graph_id != reg.grant.source.graph_id || p.branch != reg.grant.source.branch_id
        {
            return Err(unavailable());
        }
        let raw = self
            .load(&p.source.graph_id, &p.source.revision)?
            .ok_or_else(unavailable)?;
        if raw.schema.as_ref() != Some(&reg.grant.request_schema) {
            return Err(unavailable());
        }
        // Charge serialized size before cloning/encoding the original graph body.
        let payload = Payload {
            format: "weave-governed-graph-effect/1".into(),
            source: p.source.clone(),
            publication: p.reference.clone(),
            graph: raw,
        };
        json_size(&payload, PAYLOAD_LIMIT)?;
        let closure = self
            .effect_closure(&[p.source.clone(), p.reference.clone()], host)
            .map_err(normalize_unavailable)?;
        Ok(AuthorizedRequest {
            payload: serde_json::to_string(&payload)?,
            source: p.source,
            publication: p.reference,
            closure,
            current: p.current,
        })
    }
    fn effect_closure(&self, roots: &[GraphRef], host: &HostContext) -> Result<Vec<GraphRef>> {
        let mut queued = HashSet::new();
        let mut pending = Vec::new();
        for root in roots {
            if queued.insert((root.graph_id.clone(), root.revision.clone())) {
                pending.push(root.clone());
            }
        }
        let mut work = 0;
        let mut result = Vec::new();
        while let Some(reference) = pending.pop() {
            if !self.protected_reference_allowed(&reference.graph_id, &reference.revision, host)? {
                return Err(unavailable());
            }
            let raw = self
                .load(&reference.graph_id, &reference.revision)?
                .ok_or_else(unavailable)?;
            if raw.attachments.iter().any(|a| {
                matches!(
                    a.value,
                    MetadataValue::LiveGraph { .. } | MetadataValue::Object { .. }
                )
            }) {
                return Err(unavailable());
            }
            let (visible, partial) = self.authorized(raw.clone(), host)?;
            if !whole_graph_visible(&raw, visible, partial) {
                return Err(unavailable());
            }
            // This is the complete semantic visitor. Revision ancestry is not an input
            // dependency; unlike export, this profile emits no history/manifest records.
            let record = CapsuleRevision {
                graph_id: reference.graph_id.clone(),
                branch_id: String::new(),
                revision: reference.revision.clone(),
                parent: None,
                data: raw,
            };
            capsule_export::visit_dependencies(&record, &mut work, |graph, revision| {
                if !queued.contains(&(graph.to_owned(), revision.to_owned())) {
                    if queued.len() >= 1000 {
                        return Err(err("E_BUDGET", "effect closure bound"));
                    }
                    queued.insert((graph.to_owned(), revision.to_owned()));
                    pending.push(GraphRef {
                        graph_id: graph.into(),
                        revision: revision.into(),
                    });
                }
                Ok(())
            })?;
            result.push(reference);
        }
        canonical_pins(&mut result);
        Ok(result)
    }
    fn stored_effect_receipt(
        &self,
        adapter: &str,
        event: &str,
        reg: &Registration,
    ) -> Result<Option<ReceiptBody>> {
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(8192);
        let row:Option<(Option<String>,String)>=self.conn.query_row("SELECT CASE WHEN length(CAST(body AS BLOB))<=?3 THEN body END,substr(digest,1,72) FROM governed_effect_receipts WHERE adapter=?1 AND event_id=?2",params![adapter,event,limit as i64],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((body, hash)) = row else {
            return Ok(None);
        };
        let body = body.ok_or_else(|| err("E_BUDGET", "effect receipt read bound"))?;
        self.read_budget.charge(body.len())?;
        let receipt: ReceiptBody = serde_json::from_str(&body).map_err(|_| unavailable())?;
        if receipt.event != event
            || receipt.registration != registration_digest(reg)?
            || hash != digest("weave-governed-effect-receipt/1", &receipt)?
        {
            return Err(unavailable());
        }
        Ok(Some(receipt))
    }
    fn stored_governed_intent(
        &self,
        id: &str,
        host: &HostContext,
    ) -> Result<(Registration, StoredIntent)> {
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(CONTEXT_LIMIT);
        type Row = (
            Option<String>,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
        );
        let row:Row=self.conn.query_row("SELECT CASE WHEN length(CAST(body AS BLOB))<=?2 THEN body END,substr(digest,1,72),substr(adapter,1,513),substr(principal,1,513),substr(attempt_id,1,129),substr(reconciliation_digest,1,72) FROM governed_effect_context WHERE intent_id=?1",params![id,limit as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?.ok_or_else(unavailable)?;
        if row.3 != host.principal {
            return Err(unavailable());
        }
        let body = row
            .0
            .ok_or_else(|| err("E_BUDGET", "effect context read bound"))?;
        self.read_budget.charge(body.len())?;
        let ctx: IntentContext = serde_json::from_str(&body).map_err(|_| unavailable())?;
        let reg = self.effect_registration(&row.2)?.0;
        if ctx.adapter != row.2
            || reg.grant.principal != host.principal
            || ctx.registration != registration_digest(&reg)?
            || row.1 != digest("weave-governed-effect-context/1", &(id, &ctx))?
        {
            return Err(unavailable());
        }
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(PAYLOAD_LIMIT);
        type IntentRow = (
            String,
            String,
            String,
            String,
            Option<String>,
            String,
            Option<String>,
        );
        let r:IntentRow=self.conn.query_row("SELECT substr(adapter,1,513),substr(event_id,1,513),substr(destination,1,513),substr(idempotency_key,1,513),CASE WHEN length(CAST(payload AS BLOB))<=?2 THEN payload END,substr(state,1,32),CASE WHEN response IS NULL OR length(CAST(response AS BLOB))<=65536 THEN response ELSE 'oversized' END FROM effect_intents WHERE id=?1",params![id,limit as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?.ok_or_else(unavailable)?;
        let payload =
            r.4.ok_or_else(|| err("E_BUDGET", "effect payload read bound"))?;
        self.read_budget
            .charge(payload.len() + r.6.as_ref().map_or(0, String::len))?;
        if r.0 != ctx.adapter
            || r.1 != ctx.event
            || r.2 != reg.grant.destination_id
            || r.3 != ctx.idempotency_key
            || digest("weave-governed-effect-payload/1", &payload)? != ctx.payload_digest
            || !["pending", "unknown", "confirmed", "failed", "canceled"].contains(&r.5.as_str())
        {
            return Err(unavailable());
        }
        if matches!(r.5.as_str(), "pending" | "canceled") {
            if row.4.is_some() || row.5.is_some() || r.6.is_some() {
                return Err(unavailable());
            }
        } else if row.4.as_ref().is_none_or(|id| !valid_id(id)) {
            return Err(unavailable());
        }
        if matches!(r.5.as_str(), "confirmed" | "failed") {
            let response = r.6.as_ref().ok_or_else(unavailable)?;
            if row.5.as_ref()
                != Some(&digest(
                    "weave-governed-effect-reconciliation/1",
                    &(id, &row.4, &r.5, response),
                )?)
            {
                return Err(unavailable());
            }
        } else if row.5.is_some() || r.6.is_some() {
            return Err(unavailable());
        }
        Ok((
            reg,
            StoredIntent {
                context: ctx,
                payload,
                state: r.5,
                attempt: row.4,
                reconciliation: row.5,
            },
        ))
    }
    fn authorize_stored_effect(
        &self,
        reg: &Registration,
        stored: &StoredIntent,
        host: &HostContext,
    ) -> Result<()> {
        if self.effect_active(&reg.manifest.id, host)? != *reg {
            return Err(unavailable());
        }
        let event = self.effect_event(reg, &stored.context.event, host)?;
        if event.event_type != "view.accepted" {
            return Err(unavailable());
        }
        let fresh = self.effect_request(reg, &event, host)?;
        if !fresh.current
            || fresh.source != stored.context.source
            || fresh.publication != stored.context.publication
            || fresh.closure != stored.context.closure
            || fresh.payload != stored.payload
        {
            return Err(unavailable());
        }
        Ok(())
    }
    fn effect_quota(&self, principal: &str, additional: usize, new_receipt: bool) -> Result<()> {
        let (count,bytes):(i64,i64)=self.conn.query_row("SELECT (SELECT COUNT(*) FROM governed_effect_receipts r JOIN governed_effect_bindings b ON b.adapter=r.adapter WHERE b.principal=?1),(SELECT COALESCE(SUM(length(CAST(c.body AS BLOB))+length(CAST(i.payload AS BLOB))+COALESCE(length(CAST(i.response AS BLOB)),0)),0) FROM governed_effect_context c JOIN effect_intents i ON i.id=c.intent_id WHERE c.principal=?1)+(SELECT COALESCE(SUM(length(CAST(r.body AS BLOB))),0) FROM governed_effect_receipts r JOIN governed_effect_bindings b ON b.adapter=r.adapter WHERE b.principal=?1)",[principal],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if (new_receipt && count >= 256)
            || bytes.saturating_add(additional as i64) > 64 * 1024 * 1024
        {
            return Err(err("E_BUDGET", "governed effect retained quota"));
        }
        Ok(())
    }
    pub fn enqueue_governed_effect(
        &self,
        adapter: &str,
        event: &str,
        lease: &str,
        host: &HostContext,
    ) -> Result<GovernedEffectReceipt> {
        self.enqueue_governed_effect_observed(adapter, event, lease, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn enqueue_governed_effect_test_before_commit(
        &self,
        adapter: &str,
        event: &str,
        lease: &str,
        host: &HostContext,
        hook: impl FnOnce(),
    ) -> Result<GovernedEffectReceipt> {
        self.enqueue_governed_effect_observed(adapter, event, lease, host, hook)
    }
    fn enqueue_governed_effect_observed(
        &self,
        adapter: &str,
        event_id: &str,
        lease: &str,
        host: &HostContext,
        hook: impl FnOnce(),
    ) -> Result<GovernedEffectReceipt> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let _clock = self.operation_write_scope()?;
        let reg = self.effect_active(adapter, host)?;
        let event = self.effect_event(&reg, event_id, host)?;
        let request = if event.event_type == "view.accepted" {
            Some(self.effect_request(&reg, &event, host)?)
        } else {
            None
        };
        let prior = self.stored_effect_receipt(adapter, event_id, &reg)?;
        if let Some(receipt) = prior {
            match &receipt.disposition {
                GovernedEffectDisposition::Intent { intent_id } => {
                    let (bound, stored) = self.stored_governed_intent(intent_id, host)?;
                    if bound != reg || stored.context.event != event_id {
                        return Err(unavailable());
                    }
                    self.authorize_stored_effect(&reg, &stored, host)?;
                }
                GovernedEffectDisposition::PolicyChange if request.is_none() => {}
                GovernedEffectDisposition::SupersededPublication
                    if request.as_ref().is_some_and(|r| !r.current) => {}
                _ => return Err(unavailable()),
            }
            let ack = self.acknowledge_governance_in_transaction(
                adapter,
                &reg.grant.view_id,
                event_id,
                lease,
                host,
            )?;
            if !ack.duplicate || ack.ordinal != receipt.ordinal {
                return Err(unavailable());
            }
            hook();
            tx.commit()?;
            return Ok(GovernedEffectReceipt {
                duplicate: true,
                ordinal: receipt.ordinal,
                disposition: receipt.disposition,
            });
        }
        let disposition = match request {
            None => GovernedEffectDisposition::PolicyChange,
            Some(request) if !request.current => GovernedEffectDisposition::SupersededPublication,
            Some(request) => {
                let intent_id: String = self.conn.query_row(
                    "SELECT 'governed-effect:' || lower(hex(randomblob(24)))",
                    [],
                    |r| r.get(0),
                )?;
                let replica: String = self.conn.query_row(
                    "SELECT substr(source,1,513) FROM engine_identity WHERE id=1",
                    [],
                    |r| r.get(0),
                )?;
                let idempotency_key = digest(
                    "weave-governed-effect-key/1",
                    &(
                        &reg.grant.principal,
                        &reg.grant.destination_id,
                        &reg.grant.execution_id,
                        &replica,
                        event_id,
                    ),
                )?;
                let context = IntentContext {
                    adapter: adapter.into(),
                    event: event_id.into(),
                    registration: registration_digest(&reg)?,
                    source: request.source,
                    publication: request.publication,
                    closure: request.closure,
                    payload_digest: digest("weave-governed-effect-payload/1", &request.payload)?,
                    idempotency_key: idempotency_key.clone(),
                    created_at_ms: self.operation_time()?,
                };
                json_size(&context, CONTEXT_LIMIT)?;
                let body = serde_json::to_string(&context)?;
                self.effect_quota(
                    &host.principal,
                    body.len() + request.payload.len() + 8192,
                    true,
                )?;
                self.conn.execute(
                    "INSERT INTO effect_intents VALUES (?1,?2,?3,?4,?5,?6,'pending',NULL)",
                    params![
                        intent_id,
                        adapter,
                        event_id,
                        reg.grant.destination_id,
                        idempotency_key,
                        request.payload
                    ],
                )?;
                self.conn.execute(
                    "INSERT INTO governed_effect_context VALUES (?1,?2,?3,?4,?5,NULL,NULL)",
                    params![
                        intent_id,
                        adapter,
                        host.principal,
                        body,
                        digest("weave-governed-effect-context/1", &(&intent_id, &context))?
                    ],
                )?;
                GovernedEffectDisposition::Intent { intent_id }
            }
        };
        self.effect_quota(&host.principal, 8192, true)?;
        let ack = self.acknowledge_governance_in_transaction(
            adapter,
            &reg.grant.view_id,
            event_id,
            lease,
            host,
        )?;
        if ack.duplicate {
            return Err(unavailable());
        }
        let receipt = ReceiptBody {
            event: event_id.into(),
            registration: registration_digest(&reg)?,
            ordinal: ack.ordinal,
            disposition: disposition.clone(),
        };
        self.conn.execute(
            "INSERT INTO governed_effect_receipts VALUES (?1,?2,?3,?4)",
            params![
                adapter,
                event_id,
                serde_json::to_string(&receipt)?,
                digest("weave-governed-effect-receipt/1", &receipt)?
            ],
        )?;
        hook();
        tx.commit()?;
        Ok(GovernedEffectReceipt {
            duplicate: false,
            ordinal: ack.ordinal,
            disposition,
        })
    }
    pub fn read_governed_effect(
        &self,
        intent: &str,
        host: &HostContext,
    ) -> Result<GovernedEffectStatus> {
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_scope()?;
        let (reg, stored) = self.stored_governed_intent(intent, host)?;
        self.authorize_stored_effect(&reg, &stored, host)?;
        let status = GovernedEffectStatus {
            intent_id: intent.into(),
            attempt_id: stored.attempt,
            state: stored.state,
        };
        tx.commit()?;
        Ok(status)
    }
    pub fn begin_governed_effect(
        &self,
        intent: &str,
        host: &HostContext,
    ) -> Result<GovernedEffectDispatch> {
        self.begin_governed_effect_observed(intent, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn begin_governed_effect_test_before_commit(
        &self,
        intent: &str,
        host: &HostContext,
        hook: impl FnOnce(),
    ) -> Result<GovernedEffectDispatch> {
        self.begin_governed_effect_observed(intent, host, hook)
    }
    fn begin_governed_effect_observed(
        &self,
        intent: &str,
        host: &HostContext,
        hook: impl FnOnce(),
    ) -> Result<GovernedEffectDispatch> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let _clock = self.operation_write_scope()?;
        let (reg, stored) = self.stored_governed_intent(intent, host)?;
        self.authorize_stored_effect(&reg, &stored, host)?;
        if stored.state != "pending" {
            return Err(err(
                "E_EFFECT_UNKNOWN",
                "intent already attempted or terminal",
            ));
        }
        let attempt: String = self.conn.query_row(
            "SELECT 'effect-attempt:' || lower(hex(randomblob(24)))",
            [],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "UPDATE governed_effect_context SET attempt_id=?2 WHERE intent_id=?1",
            params![intent, attempt],
        )?;
        self.conn.execute(
            "UPDATE effect_intents SET state='unknown' WHERE id=?1 AND state='pending'",
            [intent],
        )?;
        let ticket = GovernedEffectDispatch {
            intent_id: intent.into(),
            attempt_id: attempt,
            destination_id: reg.grant.destination_id,
            idempotency_key: stored.context.idempotency_key,
            payload: stored.payload.into_bytes(),
        };
        hook();
        tx.commit()?;
        Ok(ticket)
    }
    pub fn cancel_governed_effect(&self, intent: &str, host: &HostContext) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let _clock = self.operation_write_scope()?;
        let (_, stored) = self.stored_governed_intent(intent, host)?;
        if stored.state != "pending" && stored.state != "canceled" {
            return Err(err(
                "E_EFFECT_UNKNOWN",
                "only pending effects can be canceled",
            ));
        }
        self.conn.execute(
            "UPDATE effect_intents SET state='canceled' WHERE id=?1",
            [intent],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn reconcile_governed_effect(
        &self,
        intent: &str,
        attempt: &str,
        outcome: ReconciledOutcome,
        evidence: &SinkEvidence,
        host: &HostContext,
    ) -> Result<()> {
        self.reconcile_governed_effect_observed(intent, attempt, outcome, evidence, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn reconcile_governed_effect_test_before_commit(
        &self,
        intent: &str,
        attempt: &str,
        outcome: ReconciledOutcome,
        evidence: &SinkEvidence,
        host: &HostContext,
        hook: impl FnOnce(),
    ) -> Result<()> {
        self.reconcile_governed_effect_observed(intent, attempt, outcome, evidence, host, hook)
    }
    fn reconcile_governed_effect_observed(
        &self,
        intent: &str,
        attempt: &str,
        outcome: ReconciledOutcome,
        evidence: &SinkEvidence,
        host: &HostContext,
        hook: impl FnOnce(),
    ) -> Result<()> {
        json_size(evidence, 64 * 1024)?;
        if !valid_id(&evidence.receipt_id)
            || evidence.response_digest.len() != 71
            || !evidence.response_digest.starts_with("sha256:")
            || !evidence.response_digest[7..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err(unavailable());
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let _clock = self.operation_write_scope()?;
        let (_, stored) = self.stored_governed_intent(intent, host)?;
        if stored.attempt.as_deref() != Some(attempt) {
            return Err(unavailable());
        }
        let state = match outcome {
            ReconciledOutcome::Confirmed => "confirmed",
            ReconciledOutcome::Failed => "failed",
        };
        let response = serde_json::to_string(evidence)?;
        let hash = digest(
            "weave-governed-effect-reconciliation/1",
            &(intent, Some(attempt), state, &response),
        )?;
        if stored.state != "unknown" {
            if stored.state != state || stored.reconciliation.as_deref() != Some(&hash) {
                return Err(err("E_EFFECT_CONFLICT", "reconciliation is immutable"));
            }
        } else {
            self.effect_quota(&host.principal, response.len(), false)?;
            self.conn.execute(
                "UPDATE effect_intents SET state=?2,response=?3 WHERE id=?1",
                params![intent, state, response],
            )?;
            self.conn.execute(
                "UPDATE governed_effect_context SET reconciliation_digest=?2 WHERE intent_id=?1",
                params![intent, hash],
            )?;
        }
        hook();
        tx.commit()?;
        Ok(())
    }
}
