//! Kernel-bound local handler causality. Native host writes remain trusted roots.
use super::*;
use serde::{Deserialize, Serialize};
const DEFAULT_DEPTH: u32 = 16;
const MAX_DEPTH: u32 = 64;
const CELL_LIMIT: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CausalDispatchPolicy {
    pub max_depth: u32,
}
impl Default for CausalDispatchPolicy {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_DEPTH,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CausalOrigin {
    LocalRoot,
    LegacyBoundary,
    Handler,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Causation {
    origin: CausalOrigin,
    event: String,
    sequence: i64,
    parent: Option<String>,
    root: String,
    depth: u32,
    adapter: Option<String>,
    registration_digest: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Circuit {
    event: String,
    policy: CausalDispatchPolicy,
    lineage_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PendingDispatchStatus {
    pub attempts: u32,
    pub state: String,
    pub retry_at_ms: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AdapterLagStatus {
    pub adapter: String,
    pub lifecycle: String,
    pub visible_backlog_lower_bound: usize,
    pub backlog_truncated: bool,
    pub pending: Option<PendingDispatchStatus>,
    pub visible_unknown_effects: usize,
    pub checkpoint_expired: bool,
    pub circuit_open: bool,
}
fn integrity() -> Error {
    err(
        "E_CAUSAL_INTEGRITY",
        "local causal dispatch state unavailable",
    )
}
fn valid_policy(policy: &CausalDispatchPolicy) -> Result<()> {
    if !(1..=MAX_DEPTH).contains(&policy.max_depth) {
        return Err(err(
            "E_CAUSAL_POLICY",
            "causal depth must be between 1 and 64",
        ));
    }
    Ok(())
}
fn output_events(results: &[CommandResult]) -> Vec<&str> {
    results
        .iter()
        .flat_map(|r| match r {
            CommandResult::Committed { event_id, .. } => vec![event_id.as_str()],
            CommandResult::BatchCommitted { commits, .. } => commits
                .iter()
                .filter_map(|c| c.event_id.as_deref())
                .collect(),
            _ => vec![],
        })
        .collect()
}
pub(crate) fn validate_retained_causality(
    engine: &Engine,
    table: &str,
    id: &str,
    digest: &str,
    value: &serde_json::Value,
) -> Result<()> {
    if table == "event_causation" {
        let record: Causation = serde_json::from_value(value.clone()).map_err(|_| integrity())?;
        if retention::hash(&record)? != digest || engine.causal_lineage(id)? != record {
            return Err(integrity());
        }
    } else if table == "dispatch_causal_policies" {
        let policy: CausalDispatchPolicy =
            serde_json::from_value(value.clone()).map_err(|_| integrity())?;
        if retention::hash(&(id, &policy))? != digest || engine.causal_policy(id)? != policy {
            return Err(integrity());
        }
    } else {
        let circuit: Circuit = serde_json::from_value(value.clone()).map_err(|_| integrity())?;
        if retention::hash(&(id, &circuit))? != digest
            || engine.causal_circuit(id)?.as_ref() != Some(&circuit)
        {
            return Err(integrity());
        }
    }
    Ok(())
}
impl Engine {
    pub(crate) fn initialize_causal_dispatch(&self, version: i64) -> Result<()> {
        let present:i64=self.conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('event_causation','dispatch_causal_policies','dispatch_circuits')",[],|r|r.get(0))?;
        if (version < 29 && present != 0) || (version >= 29 && present != 3) {
            return Err(integrity());
        }
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS event_causation(event_id TEXT PRIMARY KEY REFERENCES events(event_id),body TEXT NOT NULL,digest TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS dispatch_causal_policies(adapter TEXT PRIMARY KEY REFERENCES dispatch_adapters(id),body TEXT NOT NULL,digest TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS dispatch_circuits(adapter TEXT PRIMARY KEY REFERENCES dispatch_adapters(id),event_id TEXT NOT NULL REFERENCES events(event_id),body TEXT NOT NULL,digest TEXT NOT NULL);")?;
        if version < 29 {
            let mut statement = self.conn.prepare(
                "SELECT substr(event_id,1,1025) FROM events ORDER BY sequence LIMIT 100001",
            )?;
            let events = statement
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if events.len() > 100000 || events.iter().any(|event| event.len() > 1024) {
                return Err(err(
                    "E_BUDGET",
                    "legacy causal event inventory exceeds limit",
                ));
            }
            for event in events {
                self.record_causal_root(&event, CausalOrigin::LegacyBoundary)?;
            }
            let mut statement = self
                .conn
                .prepare("SELECT substr(id,1,513) FROM dispatch_adapters ORDER BY id LIMIT 4097")?;
            let adapters = statement
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if adapters.len() > 4096 || adapters.iter().any(|adapter| !valid_id(adapter)) {
                return Err(err(
                    "E_BUDGET",
                    "legacy causal adapter inventory exceeds limit",
                ));
            }
            for adapter in adapters {
                self.install_default_causal_policy(&adapter)?;
            }
        }
        let missing:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM events e LEFT JOIN event_causation c ON c.event_id=e.event_id WHERE c.event_id IS NULL) OR EXISTS(SELECT 1 FROM dispatch_adapters a LEFT JOIN dispatch_causal_policies p ON p.adapter=a.id WHERE p.adapter IS NULL)",[],|r|r.get(0))?;
        if missing {
            return Err(integrity());
        }
        Ok(())
    }
    pub(crate) fn record_local_causal_root(&self, event: &str) -> Result<()> {
        self.record_causal_root(event, CausalOrigin::LocalRoot)
    }
    fn record_causal_root(&self, event: &str, origin: CausalOrigin) -> Result<()> {
        let sequence: i64 = self.conn.query_row(
            "SELECT sequence FROM events WHERE event_id=?1",
            [event],
            |r| r.get(0),
        )?;
        let record = Causation {
            origin,
            event: event.into(),
            sequence,
            parent: None,
            root: event.into(),
            depth: 0,
            adapter: None,
            registration_digest: None,
        };
        self.conn.execute(
            "INSERT INTO event_causation VALUES (?1,?2,?3)",
            params![
                event,
                serde_json::to_string(&record)?,
                retention::hash(&record)?
            ],
        )?;
        Ok(())
    }
    pub(crate) fn install_default_causal_policy(&self, adapter: &str) -> Result<()> {
        let count: i64 =
            self.conn
                .query_row("SELECT count(*) FROM dispatch_causal_policies", [], |r| {
                    r.get(0)
                })?;
        if count >= 4096 {
            return Err(err(
                "E_BACKPRESSURE",
                "native causal adapter capacity exceeded",
            ));
        }
        let policy = CausalDispatchPolicy::default();
        self.conn.execute(
            "INSERT INTO dispatch_causal_policies VALUES (?1,?2,?3)",
            params![
                adapter,
                serde_json::to_string(&policy)?,
                retention::hash(&(adapter, &policy))?
            ],
        )?;
        Ok(())
    }
    fn causal_cell(&self, table: &str, column: &str, id: &str) -> Result<Option<(String, String)>> {
        self.read_budget.request()?;
        let query=format!("SELECT CASE WHEN length(CAST(body AS BLOB))<=?2 THEN body END,substr(digest,1,129) FROM {table} WHERE {column}=?1");
        let row: Option<(Option<String>, String)> = self
            .conn
            .query_row(&query, params![id, CELL_LIMIT as i64], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        row.map(|(body, digest)| {
            let body = body.ok_or_else(|| err("E_BUDGET", "causal record exceeds limit"))?;
            self.read_budget.charge(body.len())?;
            Ok((body, digest))
        })
        .transpose()
    }
    pub(crate) fn verify_installed_causal_policy(&self, adapter: &str) -> Result<()> {
        self.causal_policy(adapter).map(|_| ())
    }
    fn causal_policy(&self, adapter: &str) -> Result<CausalDispatchPolicy> {
        let (body, digest) = self
            .causal_cell("dispatch_causal_policies", "adapter", adapter)?
            .ok_or_else(integrity)?;
        let policy: CausalDispatchPolicy = serde_json::from_str(&body).map_err(|_| integrity())?;
        valid_policy(&policy).map_err(|_| integrity())?;
        if retention::hash(&(adapter, &policy))? != digest {
            return Err(integrity());
        }
        Ok(policy)
    }
    fn causal_lineage(&self, event: &str) -> Result<Causation> {
        let _budget = self.read_budget.enter();
        let mut next = event.to_string();
        let mut child: Option<Causation> = None;
        let mut first = None;
        let mut seen = HashSet::new();
        for _ in 0..=MAX_DEPTH {
            if !seen.insert(next.clone()) {
                return Err(integrity());
            }
            let (body, digest) = self
                .causal_cell("event_causation", "event_id", &next)?
                .ok_or_else(integrity)?;
            let record: Causation = serde_json::from_str(&body).map_err(|_| integrity())?;
            let sequence: Option<i64> = self
                .conn
                .query_row(
                    "SELECT sequence FROM events WHERE event_id=?1",
                    [&next],
                    |r| r.get(0),
                )
                .optional()?;
            if record.event != next
                || sequence != Some(record.sequence)
                || record.sequence <= 0
                || record.depth > MAX_DEPTH
                || retention::hash(&record)? != digest
            {
                return Err(integrity());
            }
            if let Some(child) = &child {
                if child.sequence <= record.sequence
                    || child.depth != record.depth + 1
                    || child.root != record.root
                {
                    return Err(integrity());
                }
            }
            if first.is_none() {
                first = Some(record.clone());
            }
            match (&record.parent, &record.adapter, &record.registration_digest) {
                (None, None, None)
                    if record.depth == 0
                        && record.root == record.event
                        && matches!(
                            record.origin,
                            CausalOrigin::LocalRoot | CausalOrigin::LegacyBoundary
                        ) =>
                {
                    return first.ok_or_else(integrity)
                }
                (Some(parent), Some(adapter), Some(binding))
                    if record.depth > 0 && record.origin == CausalOrigin::Handler =>
                {
                    let (manifest, _, _) = self.dispatch_manifest(adapter)?;
                    if retention::hash(&manifest)? != *binding {
                        return Err(integrity());
                    }
                    let (graph,branch,revision,actor): (String,String,String,String) = self.conn.query_row(
                        "SELECT graph_id,branch_id,revision,actor FROM events WHERE event_id=?1",
                        [&record.event],
                        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
                    )?;
                    let (parent_graph, parent_branch): (String, String) = self.conn.query_row(
                        "SELECT graph_id,branch_id FROM events WHERE event_id=?1",
                        [parent],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )?;
                    if !manifest.output_graphs.contains(&graph)
                        || actor != manifest.principal
                        || !manifest.subscriptions.contains(&SubscriptionScope {
                            graph_id: parent_graph,
                            branch_id: parent_branch,
                        })
                    {
                        return Err(integrity());
                    }
                    self.read_budget.request()?;
                    let receipt:Option<Option<String>>=self.conn.query_row("SELECT CASE WHEN length(CAST(results AS BLOB))<=?3 THEN results END FROM handler_receipts WHERE adapter=?1 AND event_id=?2",params![adapter,parent,MATERIALIZED_LIMIT as i64],|r|r.get(0)).optional()?;
                    let receipt = receipt.flatten().ok_or_else(integrity)?;
                    self.read_budget.charge(receipt.len())?;
                    let results: Vec<CommandResult> =
                        serde_json::from_str(&receipt).map_err(|_| integrity())?;
                    let matches = results.iter().any(|r| match r {
                        CommandResult::Committed {
                            event_id,
                            revision: output,
                        } => event_id == &record.event && output == &revision,
                        CommandResult::BatchCommitted { commits, .. } => commits.iter().any(|c| {
                            c.event_id.as_ref() == Some(&record.event)
                                && c.revision == revision
                                && c.graph_id == graph
                                && c.branch_id == branch
                        }),
                        _ => false,
                    });
                    if !matches {
                        return Err(integrity());
                    }
                    next = parent.clone();
                    child = Some(record);
                }
                _ => return Err(integrity()),
            }
        }
        Err(integrity())
    }
    pub(crate) fn require_causal_work(&self, adapter: &str, event: &str) -> Result<()> {
        let record = self.causal_lineage(event)?;
        let policy = self.causal_policy(adapter)?;
        if record.depth >= policy.max_depth || self.causal_circuit(adapter)?.is_some() {
            return Err(err("E_CIRCUIT_OPEN", "causal work requires owner review"));
        }
        Ok(())
    }
    fn causal_circuit(&self, adapter: &str) -> Result<Option<Circuit>> {
        let _budget = self.read_budget.enter();
        let Some((body, digest)) = self.causal_cell("dispatch_circuits", "adapter", adapter)?
        else {
            return Ok(None);
        };
        let circuit: Circuit = serde_json::from_str(&body).map_err(|_| integrity())?;
        let event: String = self.conn.query_row(
            "SELECT event_id FROM dispatch_circuits WHERE adapter=?1",
            [adapter],
            |r| r.get(0),
        )?;
        valid_policy(&circuit.policy).map_err(|_| integrity())?;
        let lineage = self.causal_lineage(&event)?;
        if lineage.depth < circuit.policy.max_depth
            || circuit.policy != self.causal_policy(adapter)?
            || event != circuit.event
            || retention::hash(&(adapter, &circuit))? != digest
            || retention::hash(&lineage)? != circuit.lineage_digest
        {
            return Err(integrity());
        }
        Ok(Some(circuit))
    }
    pub(crate) fn suspend_causal_delivery(&self, adapter: &str, event: &str) -> Result<bool> {
        let record = self.causal_lineage(event)?;
        let policy = self.causal_policy(adapter)?;
        if self.causal_circuit(adapter)?.is_none() && record.depth < policy.max_depth {
            return Ok(false);
        }
        let circuit = Circuit {
            event: event.into(),
            policy,
            lineage_digest: retention::hash(&record)?,
        };
        self.conn.execute(
            "INSERT INTO dispatch_circuits VALUES (?1,?2,?3,?4) ON CONFLICT(adapter) DO NOTHING",
            params![
                adapter,
                event,
                serde_json::to_string(&circuit)?,
                retention::hash(&(adapter, &circuit))?
            ],
        )?;
        self.conn.execute(
            "UPDATE dispatch_adapters SET state='paused' WHERE id=?1",
            [adapter],
        )?;
        Ok(true)
    }
    pub(crate) fn bind_handler_causation(
        &self,
        adapter: &str,
        parent: &str,
        frontier: i64,
        results: &[CommandResult],
    ) -> Result<()> {
        let _budget = self.read_budget.enter();
        let source = self.causal_lineage(parent)?;
        let (manifest, _, _) = self.dispatch_manifest(adapter)?;
        let outputs = output_events(results);
        let mut statement = self
            .conn
            .prepare("SELECT event_id FROM events WHERE sequence>?1 ORDER BY sequence LIMIT 257")?;
        let events = statement
            .query_map([frontier], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if events.len() > 256 || (!events.is_empty() && source.depth >= MAX_DEPTH) {
            return Err(err("E_BUDGET", "local causal output budget exceeded"));
        }
        for event in events {
            if !outputs.contains(&event.as_str()) {
                return Err(integrity());
            }
            let previous = self.causal_lineage(&event)?;
            if previous.depth != 0
                || previous.parent.is_some()
                || previous.sequence <= source.sequence
            {
                return Err(integrity());
            }
            let record = Causation {
                origin: CausalOrigin::Handler,
                event: event.clone(),
                sequence: previous.sequence,
                parent: Some(parent.into()),
                root: source.root.clone(),
                depth: source.depth + 1,
                adapter: Some(adapter.into()),
                registration_digest: Some(retention::hash(&manifest)?),
            };
            self.conn.execute(
                "UPDATE event_causation SET body=?2,digest=?3 WHERE event_id=?1",
                params![
                    &event,
                    serde_json::to_string(&record)?,
                    retention::hash(&record)?
                ],
            )?;
        }
        Ok(())
    }
    pub fn set_causal_dispatch_policy_for(
        &self,
        adapter: &str,
        policy: &CausalDispatchPolicy,
        host: &HostContext,
    ) -> Result<()> {
        let _budget = self.read_budget.enter();
        valid_policy(policy)?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let _clock = self.operation_write_scope()?;
        self.require_adapter_host(adapter, host)?;
        self.reject_governed_effect_adapter(adapter)?;
        let (_, state, _) = self.dispatch_manifest(adapter)?;
        let unresolved:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM dispatch_pending WHERE adapter=?1) OR EXISTS(SELECT 1 FROM effect_intents WHERE adapter=?1 AND state IN ('pending','unknown'))",[adapter],|r|r.get(0))?;
        self.causal_policy(adapter)?;
        self.causal_circuit(adapter)?;
        if !matches!(state.as_str(), "installed" | "paused") || unresolved {
            return Err(err(
                "E_CAUSAL_POLICY",
                "pause and drain actual work before changing causal policy",
            ));
        }
        self.conn.execute(
            "UPDATE dispatch_causal_policies SET body=?2,digest=?3 WHERE adapter=?1",
            params![
                adapter,
                serde_json::to_string(policy)?,
                retention::hash(&(adapter, policy))?
            ],
        )?;
        self.conn
            .execute("DELETE FROM dispatch_circuits WHERE adapter=?1", [adapter])?;
        tx.commit()?;
        Ok(())
    }
    pub fn adapter_lag_status_for(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<AdapterLagStatus> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.require_adapter_host(adapter, host)?;
        self.reject_governed_effect_adapter(adapter)?;
        let (manifest, lifecycle, checkpoint) = self.dispatch_manifest(adapter)?;
        let checkpoint_expired = match self.require_replay_checkpoint(adapter) {
            Ok(()) => false,
            Err(e) if e.code == "E_CHECKPOINT_EXPIRED" => true,
            Err(e) => return Err(e),
        };
        let scopes = manifest
            .subscriptions
            .iter()
            .map(|_| "(graph_id=? AND branch_id=?)")
            .collect::<Vec<_>>()
            .join(" OR ");
        let query=format!("SELECT event_id FROM events WHERE sequence>? AND ({scopes}) ORDER BY sequence LIMIT 4097");
        let mut parameters = vec![rusqlite::types::Value::Integer(checkpoint)];
        for scope in &manifest.subscriptions {
            parameters.push(rusqlite::types::Value::Text(scope.graph_id.clone()));
            parameters.push(rusqlite::types::Value::Text(scope.branch_id.clone()));
        }
        let mut statement = self.conn.prepare(&query)?;
        let events = statement
            .query_map(rusqlite::params_from_iter(parameters), |r| {
                r.get::<_, String>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut visible = 0;
        let mut truncated = false;
        for (index, event) in events.iter().enumerate() {
            if index >= 4096 {
                return Err(err("E_BUDGET", "scoped lag scan exceeded limit"));
            }
            if self.scoped_event(&manifest, event)?.is_some() {
                visible += 1;
                if visible > 256 {
                    visible = 256;
                    truncated = true;
                    break;
                }
            }
        }
        let row: Option<(String, u32, String, i64)> = self
            .conn
            .query_row(
                "SELECT event_id,attempts,status,next_at FROM dispatch_pending WHERE adapter=?1",
                [adapter],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let pending = if let Some((event, attempts, state, retry_at_ms)) = row {
            if self.scoped_event(&manifest, &event)?.is_some() {
                Some(PendingDispatchStatus {
                    attempts,
                    state,
                    retry_at_ms,
                })
            } else {
                None
            }
        } else {
            None
        };
        let mut statement = self.conn.prepare(
            "SELECT event_id FROM effect_intents WHERE adapter=?1 AND state='unknown' LIMIT 129",
        )?;
        let unknown = statement
            .query_map([adapter], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if unknown.len() > 128 {
            return Err(err(
                "E_BUDGET",
                "scoped unknown-effect count exceeded limit",
            ));
        }
        let mut visible_unknown_effects = 0;
        for event in unknown {
            if self.scoped_event(&manifest, &event)?.is_some() {
                visible_unknown_effects += 1;
            }
        }
        let circuit_open = if let Some(circuit) = self.causal_circuit(adapter)? {
            self.scoped_event(&manifest, &circuit.event)?.is_some()
        } else {
            false
        };
        Ok(AdapterLagStatus {
            adapter: adapter.into(),
            lifecycle,
            visible_backlog_lower_bound: visible,
            backlog_truncated: truncated,
            pending,
            visible_unknown_effects,
            checkpoint_expired,
            circuit_open,
        })
    }
}
