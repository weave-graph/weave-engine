//! Trusted-host durable dispatch. Registration never executes an artifact.
use super::*;
use serde::{Deserialize, Serialize};
type EffectRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionScope {
    pub graph_id: String,
    pub branch_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterManifest {
    pub id: String,
    pub version: String,
    pub artifact_digest: String,
    pub config_revision: String,
    pub principal: String,
    pub subscriptions: Vec<SubscriptionScope>,
    pub output_graphs: Vec<String>,
    pub effect_destinations: Vec<String>,
    pub max_attempts: u32,
    pub lease_ms: i64,
    pub max_pending_events: u32,
    pub projection_replay: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DispatchEnvelope {
    pub specversion: String,
    pub id: String,
    pub source: String,
    pub event_type: String,
    pub graph: GraphRef,
    pub branch_id: String,
    pub lease: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HandlerReceipt {
    pub duplicate: bool,
    pub results: Vec<CommandResult>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectIntent {
    pub id: String,
    pub adapter: String,
    pub event_id: String,
    pub destination: String,
    pub idempotency_key: String,
    pub payload: serde_json::Value,
    pub state: String,
    pub response: Option<serde_json::Value>,
}
impl Engine {
    pub(crate) fn initialize_dispatch(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS dispatch_adapters(id TEXT PRIMARY KEY,manifest TEXT NOT NULL,state TEXT NOT NULL,checkpoint INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS dispatch_pending(adapter TEXT PRIMARY KEY REFERENCES dispatch_adapters(id),event_id TEXT NOT NULL REFERENCES events(event_id),lease TEXT NOT NULL,expires INTEGER NOT NULL,attempts INTEGER NOT NULL,next_at INTEGER NOT NULL,status TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS handler_receipts(adapter TEXT NOT NULL,event_id TEXT NOT NULL,request_hash TEXT NOT NULL,results TEXT NOT NULL,PRIMARY KEY(adapter,event_id));
CREATE TABLE IF NOT EXISTS effect_intents(id TEXT PRIMARY KEY,adapter TEXT NOT NULL,event_id TEXT NOT NULL,destination TEXT NOT NULL,idempotency_key TEXT NOT NULL,payload TEXT NOT NULL,state TEXT NOT NULL,response TEXT,UNIQUE(adapter,idempotency_key));
CREATE TABLE IF NOT EXISTS engine_identity(id INTEGER PRIMARY KEY CHECK(id=1),source TEXT NOT NULL);
INSERT OR IGNORE INTO engine_identity VALUES (1,'urn:weave:replica:' || lower(hex(randomblob(16))));")?;
        Ok(())
    }
    /// Trusted installation pins a manifest. It does not load or execute its artifact.
    pub fn install_adapter(
        &self,
        manifest: &AdapterManifest,
        authority: &HostContext,
    ) -> Result<()> {
        if manifest.principal != authority.principal
            || !valid_id(&manifest.id)
            || !valid_id(&manifest.version)
            || !valid_id(&manifest.config_revision)
            || !manifest.artifact_digest.starts_with("sha256:")
            || manifest.artifact_digest.len() != 71
            || !manifest.artifact_digest[7..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            || manifest.subscriptions.is_empty()
            || manifest.subscriptions.len() > 100
            || manifest.output_graphs.len() > 100
            || manifest.effect_destinations.len() > 100
            || !(1..=20).contains(&manifest.max_attempts)
            || !(100..=3_600_000).contains(&manifest.lease_ms)
            || !(1..=100_000).contains(&manifest.max_pending_events)
            || manifest
                .subscriptions
                .iter()
                .any(|s| !valid_id(&s.graph_id) || !valid_id(&s.branch_id))
            || manifest
                .output_graphs
                .iter()
                .any(|g| !authority.writable_graphs.contains(g))
            || manifest.effect_destinations.iter().any(|d| !valid_id(d))
        {
            return Err(err(
                "E_MANIFEST",
                "invalid or unauthorized adapter manifest",
            ));
        }
        json_size(manifest, 256 * 1024)?;
        let json = serde_json::to_string(manifest)?;
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT manifest FROM dispatch_adapters WHERE id=?1",
                [&manifest.id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            if existing != json {
                return Err(err("E_ADAPTER_VERSION","use a new adapter identity for changed artifact/configuration; checkpoints cannot be reused"));
            }
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO dispatch_adapters(id,manifest,state) VALUES (?1,?2,'installed')",
            params![manifest.id, json],
        )?;
        Ok(())
    }
    pub(crate) fn dispatch_manifest(&self, id: &str) -> Result<(AdapterManifest, String, i64)> {
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(256 * 1024);
        let row: (Option<String>, String, i64) = self.conn.query_row(
            "SELECT CASE WHEN length(CAST(manifest AS BLOB))<=?2 THEN manifest END,substr(state,1,32),checkpoint FROM dispatch_adapters WHERE id=?1",
            params![id, limit as i64], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
            .optional()?.ok_or_else(|| err("E_ADAPTER", "adapter unavailable"))?;
        let encoded = row
            .0
            .ok_or_else(|| err("E_BUDGET", "adapter manifest exceeds read budget"))?;
        self.read_budget.charge(encoded.len())?;
        Ok((serde_json::from_str(&encoded)?, row.1, row.2))
    }
    /// Trusted lifecycle management; removed identities cannot silently restart.
    pub fn set_adapter_state(&self, id: &str, state: &str) -> Result<()> {
        let transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let _clock_scope = self.operation_write_scope()?;
        let (_, current, _) = self.dispatch_manifest(id)?;
        if !["running", "paused", "draining", "removed"].contains(&state) || current == "removed" {
            return Err(err("E_LIFECYCLE", "invalid lifecycle transition"));
        }
        if state == "removed" {
            let pending: i64 = self.conn.query_row(
                "SELECT (SELECT COUNT(*) FROM dispatch_pending WHERE adapter=?1)+(SELECT COUNT(*) FROM governance_delivery_pending WHERE adapter=?1)",
                [id],
                |r| r.get(0))?;
            if pending > 0 {
                return Err(err(
                    "E_LIFECYCLE",
                    "drain or explicitly resolve pending delivery before removal",
                ));
            }
        }
        if state == "paused" || state == "removed" {
            self.invalidate_governance_leases(id)?;
        }
        self.conn.execute(
            "UPDATE dispatch_adapters SET state=?2 WHERE id=?1",
            params![id, state],
        )?;
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        Ok(())
    }
    pub(crate) fn scoped_event(
        &self,
        manifest: &AdapterManifest,
        id: &str,
    ) -> Result<Option<(String, String, String, i64)>> {
        let event: Option<(String, String, String, i64)> = self
            .conn
            .query_row(
                "SELECT graph_id,branch_id,revision,sequence FROM events WHERE event_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((graph, branch, revision, sequence)) = event else {
            return Ok(None);
        };
        if !manifest
            .subscriptions
            .iter()
            .any(|s| s.graph_id == graph && s.branch_id == branch)
        {
            return Ok(None);
        }
        let host = HostContext::new(&manifest.principal, manifest.output_graphs.clone());
        if !self.protected_reference_allowed(&graph, &revision, &host)? {
            return Ok(None);
        }
        let Some(data) = self.load(&graph, &revision)? else {
            return Ok(None);
        };
        let (visible, incomplete) = self.authorized(data.clone(), &host)?;
        if !whole_graph_visible(&data, visible, incomplete) {
            return Ok(None);
        }
        Ok(Some((graph, branch, revision, sequence)))
    }
    /// At most one in-flight event per adapter. Global offsets and skipped private events are not exposed.
    /// Old explicit-time native calls must be migrated rather than silently ignored.
    /// ```compile_fail
    /// let mut engine = weave_engine::Engine::memory().unwrap();
    /// let _ = engine.poll_adapter("adapter", 123);
    /// ```
    pub fn poll_adapter(&mut self, id: &str) -> Result<Option<DispatchEnvelope>> {
        self.reject_governed_effect_adapter(id)?;
        let tx = self.conn.unchecked_transaction()?;
        let _clock_scope = self.operation_write_scope()?;
        let now_ms = self.operation_time()?;
        let (manifest, state, mut checkpoint) = self.dispatch_manifest(id)?;
        if state != "running" && state != "draining" {
            return Err(err("E_PAUSED", "adapter is not running"));
        }
        let pending:Option<(String,String,i64,u32,i64,String)>=self.conn.query_row("SELECT event_id,lease,expires,attempts,next_at,status FROM dispatch_pending WHERE adapter=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
        let (event_id, attempts) = if let Some((
            event,
            _lease,
            expires,
            attempts,
            next_at,
            status,
        )) = pending
        {
            if status == "dead_letter" || now_ms < next_at || now_ms < expires {
                return Ok(None);
            }
            if attempts >= manifest.max_attempts {
                self.conn.execute(
                    "UPDATE dispatch_pending SET status='dead_letter' WHERE adapter=?1",
                    [id],
                )?;
                tx.commit()?;
                return Ok(None);
            }
            (event, attempts + 1)
        } else {
            if state == "draining" {
                tx.commit()?;
                return Ok(None);
            }
            // Scan a bounded page; private occurrences advance only the private durable checkpoint.
            let mut stmt=self.conn.prepare("SELECT event_id,sequence FROM events WHERE sequence>?1 ORDER BY sequence LIMIT 1001")?;
            let rows = stmt
                .query_map([checkpoint], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut selected = None;
            let mut visible_backlog = 0u32;
            for (event, seq) in rows {
                if self.scoped_event(&manifest, &event)?.is_some() {
                    visible_backlog += 1;
                    if selected.is_none() {
                        selected = Some(event)
                    }
                } else if selected.is_none() {
                    checkpoint = seq
                }
            }
            if visible_backlog > manifest.max_pending_events {
                self.conn.execute(
                    "UPDATE dispatch_adapters SET state='paused' WHERE id=?1",
                    [id],
                )?;
                tx.commit()?;
                return Err(err(
                    "E_BACKPRESSURE",
                    "adapter backlog requires host review",
                ));
            }
            self.conn.execute(
                "UPDATE dispatch_adapters SET checkpoint=?2 WHERE id=?1",
                params![id, checkpoint],
            )?;
            let Some(event) = selected else {
                tx.commit()?;
                return Ok(None);
            };
            (event, 1)
        };
        let Some((graph, branch, revision, _)) = self.scoped_event(&manifest, &event_id)? else {
            return Err(err("E_UNAVAILABLE", "delivery is unavailable"));
        };
        let lease: String = self
            .conn
            .query_row("SELECT lower(hex(randomblob(24)))", [], |r| r.get(0))?;
        let expires = now_ms
            .checked_add(manifest.lease_ms)
            .ok_or_else(|| err("E_CLOCK", "dispatcher clock overflow"))?;
        self.conn.execute("INSERT INTO dispatch_pending VALUES (?1,?2,?3,?4,?5,0,'leased') ON CONFLICT(adapter) DO UPDATE SET lease=excluded.lease,expires=excluded.expires,attempts=excluded.attempts,status=excluded.status",params![id,event_id,lease,expires,attempts])?;
        let source: String =
            self.conn
                .query_row("SELECT source FROM engine_identity WHERE id=1", [], |r| {
                    r.get(0)
                })?;
        tx.commit()?;
        Ok(Some(DispatchEnvelope {
            specversion: "1.0".into(),
            event_type: if event_id.starts_with("accept:") {
                "graph.accepted".into()
            } else {
                "graph.committed".into()
            },
            id: event_id,
            source,
            graph: GraphRef {
                graph_id: graph,
                revision,
            },
            branch_id: branch,
            lease,
        }))
    }
    /// Atomic local graph writes, output events, receipt and checkpoint. No external action runs here.
    pub fn complete_handler(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        program: &Program,
    ) -> Result<HandlerReceipt> {
        self.complete_handler_boundary(adapter, event, lease, program, || {})
    }
    /// Explicit test observer, absent from ordinary builds; used for process-kill recovery evidence.
    #[cfg(feature = "recovery-testing")]
    pub fn complete_handler_test_before_commit(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        program: &Program,
        before_commit: impl FnOnce(),
    ) -> Result<HandlerReceipt> {
        self.complete_handler_boundary(adapter, event, lease, program, before_commit)
    }
    fn complete_handler_boundary(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        program: &Program,
        before_commit: impl FnOnce(),
    ) -> Result<HandlerReceipt> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _clock_scope = self.operation_write_scope()?;
            self.reject_governed_effect_adapter(adapter)?;
            if self.is_compiled_handler(adapter)? {
                return Err(err(
                    "E_HANDLER_BOUND",
                    "compiled handlers require prepared completion",
                ));
            }
            let result = self.complete_handler_in_transaction(adapter, event, lease, program)?;
            before_commit();
            Ok(result)
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
    pub(crate) fn complete_handler_in_transaction(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        program: &Program,
    ) -> Result<HandlerReceipt> {
        if self.conn.is_autocommit() {
            return Err(err(
                "E_TRANSACTION",
                "handler completion requires transaction",
            ));
        }
        let _clock_scope = self.operation_write_scope()?;
        json_size(program, 16 * 1024 * 1024)?;
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(program)?));
        let (manifest, state, _) = self.dispatch_manifest(adapter)?;
        if state != "running" && state != "draining" {
            return Err(err("E_PAUSED", "adapter is not running"));
        }
        // Replays remain subject to current event authority, even after their lease
        // has been acknowledged. The handler may also have read unrelated graphs.
        let (_, _, _, sequence) = self
            .scoped_event(&manifest, event)?
            .ok_or_else(|| err("E_UNAVAILABLE", "delivery is unavailable"))?;
        let host = HostContext::new(&manifest.principal, manifest.output_graphs.clone());
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(MATERIALIZED_LIMIT + 4096);
        let prior: Option<(String, Option<String>)> = self.conn.query_row(
                "SELECT substr(request_hash,1,65),CASE WHEN length(CAST(results AS BLOB))<=?3 THEN results ELSE NULL END FROM handler_receipts WHERE adapter=?1 AND event_id=?2",
                params![adapter,event,limit as i64], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((prior, results)) = prior {
            if prior != hash {
                return Err(err(
                    "E_RECEIPT_CONFLICT",
                    "processed occurrence has different commands",
                ));
            }
            let results = results
                .ok_or_else(|| err("E_BUDGET", "stored handler receipt exceeds read budget"))?;
            self.read_budget.charge(results.len())?;
            let results: Vec<CommandResult> = serde_json::from_str(&results)?;
            for result in &results {
                if let CommandResult::Queried { result } = result {
                    self.require_current_result_authority(result, &host)?;
                }
            }
            return Ok(HandlerReceipt {
                duplicate: true,
                results,
            });
        }
        self.check_lease(adapter, event, lease)?;
        // Until declassification exists, derived handler output stays within the installed principal.
        for command in &program.commands {
            let datas: Vec<&GraphData> = match command {
                Command::Commit { data, .. } => vec![data],
                Command::CommitBatch { commits, .. } => commits.iter().map(|c| &c.data).collect(),
                _ => vec![],
            };
            for data in datas {
                if data
                    .nodes
                    .iter()
                    .any(|n| n.readers != [manifest.principal.clone()])
                    || data
                        .edges
                        .iter()
                        .any(|e| e.readers != [manifest.principal.clone()])
                    || data
                        .structural_edges
                        .iter()
                        .any(|e| e.readers != [manifest.principal.clone()])
                    || data
                        .assertions
                        .iter()
                        .any(|a| a.readers != [manifest.principal.clone()])
                    || data
                        .attachments
                        .iter()
                        .any(|a| a.readers != [manifest.principal.clone()])
                {
                    return Err(err(
                        "E_EGRESS",
                        "handler output must retain its installed principal restriction",
                    ));
                }
            }
        }
        let results = self.execute(program, &host)?;
        let json = serde_json::to_string(&results)?;
        self.conn.execute(
            "INSERT INTO handler_receipts VALUES (?1,?2,?3,?4)",
            params![adapter, event, hash, json],
        )?;
        self.conn.execute(
            "UPDATE dispatch_adapters SET checkpoint=?2 WHERE id=?1",
            params![adapter, sequence],
        )?;
        self.conn
            .execute("DELETE FROM dispatch_pending WHERE adapter=?1", [adapter])?;
        Ok(HandlerReceipt {
            duplicate: false,
            results,
        })
    }
    pub(crate) fn check_lease(&self, adapter: &str, event: &str, lease: &str) -> Result<()> {
        let matches:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM dispatch_pending WHERE adapter=?1 AND event_id=?2 AND lease=?3 AND status='leased' AND expires>?4)",params![adapter,event,lease,self.operation_time()?],|r|r.get(0))?;
        if !matches {
            return Err(err("E_LEASE", "delivery lease is not current"));
        }
        Ok(())
    }
    pub fn fail_handler(&self, adapter: &str, event: &str, lease: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let _clock_scope = self.operation_write_scope()?;
        let now_ms = self.operation_time()?;
        self.check_lease(adapter, event, lease)?;
        let (manifest, _, _) = self.dispatch_manifest(adapter)?;
        let attempts: u32 = self.conn.query_row(
            "SELECT attempts FROM dispatch_pending WHERE adapter=?1",
            [adapter],
            |r| r.get(0),
        )?;
        let jitter: i64 = self
            .conn
            .query_row("SELECT abs(random() % 251)", [], |r| r.get(0))?;
        let delay = (1000i64 << attempts.min(16)) + jitter;
        let next = now_ms
            .checked_add(delay)
            .ok_or_else(|| err("E_CLOCK", "retry clock overflow"))?;
        self.conn.execute(
            "UPDATE dispatch_pending SET expires=0,next_at=?2,status=?3 WHERE adapter=?1",
            params![
                adapter,
                next,
                if attempts >= manifest.max_attempts {
                    "dead_letter"
                } else {
                    "retry"
                }
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn replay_handler_dead_letter(&self, adapter: &str) -> Result<()> {
        self.conn.execute("UPDATE dispatch_pending SET attempts=0,expires=0,next_at=0,status='retry' WHERE adapter=?1 AND status='dead_letter'",[adapter])?;
        Ok(())
    }
    /// Durable intent only. The host must explicitly dispatch it; projection replay cannot request effects.
    pub fn request_effect(
        &self,
        adapter: &str,
        event: &str,
        lease: &str,
        destination: &str,
        key: &str,
        payload: serde_json::Value,
    ) -> Result<EffectIntent> {
        self.reject_governed_effect_adapter(adapter)?;
        let tx = self.conn.unchecked_transaction()?;
        let _clock_scope = self.operation_write_scope()?;
        let (manifest, state, _) = self.dispatch_manifest(adapter)?;
        if state != "running"
            || manifest.projection_replay
            || !manifest
                .effect_destinations
                .iter()
                .any(|d| d == destination)
            || !valid_id(key)
        {
            return Err(err(
                "E_EFFECT_AUTHORITY",
                "effect is not authorized for this adapter run",
            ));
        }
        self.check_lease(adapter, event, lease)?;
        if self.scoped_event(&manifest, event)?.is_none() {
            return Err(err("E_UNAVAILABLE", "delivery unavailable"));
        }
        json_size(&payload, 1024 * 1024)?;
        let id = format!(
            "effect:{:x}",
            Sha256::digest(serde_json::to_vec(&(adapter, key))?)
        );
        let old = self.effect_intent(&id)?;
        if let Some(old) = old {
            if old.event_id != event || old.destination != destination || old.payload != payload {
                return Err(err(
                    "E_EFFECT_CONFLICT",
                    "idempotency key already binds another intent",
                ));
            }
            return Ok(old);
        }
        self.conn.execute(
            "INSERT INTO effect_intents VALUES (?1,?2,?3,?4,?5,?6,'pending',NULL)",
            params![
                id,
                adapter,
                event,
                destination,
                key,
                serde_json::to_string(&payload)?
            ],
        )?;
        tx.commit()?;
        self.effect_intent(&id)?
            .ok_or_else(|| err("E_EFFECT", "intent unavailable"))
    }
    pub fn effect_intent(&self, id: &str) -> Result<Option<EffectIntent>> {
        self.reject_governed_effect_intent(id)?;
        let raw:Option<EffectRow>=self.conn.query_row("SELECT adapter,event_id,destination,idempotency_key,payload,state,response FROM effect_intents WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?;
        raw.map(
            |(adapter, event_id, destination, idempotency_key, payload, state, response)| {
                Ok(EffectIntent {
                    id: id.into(),
                    adapter,
                    event_id,
                    destination,
                    idempotency_key,
                    payload: serde_json::from_str(&payload)?,
                    state,
                    response: response.map(|r| serde_json::from_str(&r)).transpose()?,
                })
            },
        )
        .transpose()
    }
    /// Persist unknown BEFORE the caller attempts I/O. Unknown never automatically retries.
    pub fn begin_effect_dispatch(&self, id: &str) -> Result<EffectIntent> {
        let tx = self.conn.unchecked_transaction()?;
        let _clock_scope = self.operation_write_scope()?;
        let intent = self
            .effect_intent(id)?
            .ok_or_else(|| err("E_EFFECT", "intent unavailable"))?;
        let (manifest, state, _) = self.dispatch_manifest(&intent.adapter)?;
        if state != "running"
            || manifest.projection_replay
            || !manifest.effect_destinations.contains(&intent.destination)
        {
            return Err(err(
                "E_EFFECT_AUTHORITY",
                "adapter effect authority inactive",
            ));
        }
        if self.scoped_event(&manifest, &intent.event_id)?.is_none() {
            return Err(err("E_UNAVAILABLE", "effect source is unavailable"));
        }
        if intent.state != "pending" {
            return Err(err(
                "E_EFFECT_UNKNOWN",
                "intent already attempted; reconcile before further action",
            ));
        }
        self.conn.execute(
            "UPDATE effect_intents SET state='unknown' WHERE id=?1",
            [id],
        )?;
        tx.commit()?;
        self.effect_intent(id)?
            .ok_or_else(|| err("E_EFFECT", "intent unavailable"))
    }
    /// Trusted broker reconciliation, never a plan command. Failed/confirmed outcomes are terminal.
    pub fn reconcile_effect(
        &self,
        id: &str,
        outcome: &str,
        response: serde_json::Value,
    ) -> Result<()> {
        self.reject_governed_effect_intent(id)?;
        if !["confirmed", "failed"].contains(&outcome) {
            return Err(err("E_EFFECT", "invalid reconciliation outcome"));
        }
        json_size(&response, 1024 * 1024)?;
        let changed = self.conn.execute(
            "UPDATE effect_intents SET state=?2,response=?3 WHERE id=?1 AND state='unknown'",
            params![id, outcome, serde_json::to_string(&response)?],
        )?;
        if changed != 1 {
            return Err(err("E_EFFECT", "only unknown effects can be reconciled"));
        }
        Ok(())
    }
}
