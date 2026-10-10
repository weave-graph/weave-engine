//! Explicit trusted storage administration. Logical identities and causal anchors survive erasure.
use super::*;
use rusqlite::types::ValueRef;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};

const KNOWN_TABLES: &[&str] = &[
    "adapters",
    "delivery_cancellations",
    "projection_rebuild_requests",
    "projection_migrations",
    "admission_epochs",
    "admission_policy",
    "admission_receipts",
    "assertion_structures",
    "compiled_handlers",
    "compiled_migrations",
    "compiled_rebuild_receipts",
    "compiled_replay_states",
    "deliveries",
    "dispatch_adapters",
    "dispatch_pending",
    "edge_structures",
    "effect_intents",
    "effects",
    "engine_identity",
    "events",
    "governance_approval_nonces",
    "governance_approvals",
    "governance_decisions",
    "governance_delivery_pending",
    "governance_delivery_receipts",
    "governance_events",
    "governance_exposure_decisions",
    "governance_graphs",
    "governance_policies",
    "governance_proposals",
    "governance_receipts",
    "governance_subscriptions",
    "governance_views",
    "governed_effect_bindings",
    "governed_effect_context",
    "governed_effect_receipts",
    "handler_preparations",
    "handler_receipts",
    "head_observations",
    "heads",
    "identity_candidates",
    "identity_decisions",
    "identity_mapping_heads",
    "identity_memberships",
    "identity_policies",
    "identity_receipts",
    "integration_receipts",
    "isolated_proposals",
    "live_view_changes",
    "live_views",
    "mount_events",
    "mount_receipts",
    "mounts",
    "recorded_actor_definitions",
    "recorded_actor_states",
    "recorded_actor_receipts",
    "retention_adapter_states",
    "retention_policy",
    "retention_projection_receipts",
    "retention_retired_branches",
    "retention_roots",
    "retention_stateful_adapters",
    "retention_tombstones",
    "retention_view_epochs",
    "revision_integrity",
    "revisions",
    "schema_registry",
    "snapshot_manifests",
    "view_change_authorization",
    "view_dependencies",
    "view_schedule_cursors",
    "view_schedules",
    "view_selection",
    "view_sources",
];
pub(crate) type AnchorRow = (String, String, Option<String>, String, String, i64, i64);

// Encoded kernel cells are distinguished from legal arbitrary identifier/literal strings.
const JSON_COLUMNS: &[&str] = &[
    "body",
    "data",
    "definition",
    "dependencies",
    "manifest",
    "policy",
    "payload",
    "response",
    "result",
    "results",
    "refs",
    "schema",
    "processed_manifest",
    "preparation",
    "registration",
    "receipt",
    "request",
    "context",
    "binding",
    "capsule",
    "guards",
    "descriptor",
    "template",
    "transition",
    "prior_result",
    "spec",
    "authorization",
];
const MAX_REVISIONS: usize = 3000;
const MAX_ROWS: usize = 100_000;
const MAX_VALUES: usize = 1_000_000;
const CELL_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RetentionPolicy {
    pub history_before_ms: i64,
    /// Trusted local event coordinate. Never disclose this through a principal endpoint.
    pub replay_through_sequence: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RetentionPlan {
    pub generation: i64,
    pub policy: RetentionPolicy,
    pub retained: Vec<GraphRef>,
    pub collect: Vec<GraphRef>,
    pub payload_bytes: u64,
    pub digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RetentionReceipt {
    pub generation: i64,
    pub plan_digest: String,
    pub collected_payloads: usize,
    pub payload_bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecordedAvailability {
    pub observer: String,
    pub graph_id: String,
    pub branch_id: String,
    /// Local observation horizon; not remote history or query completeness.
    pub retained_from_ms: i64,
    pub observed_until_ms: i64,
}

fn failure() -> Error {
    err("E_RETENTION_INTEGRITY", "retention state unavailable")
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
pub(crate) fn hash(value: &impl Serialize) -> Result<String> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}
pub(crate) fn state(conn: &Connection) -> Result<(i64, RetentionPolicy)> {
    let (generation, encoded, digest, epoch): (i64, String, String, String) = conn.query_row(
        "SELECT generation,substr(policy,1,4097),substr(digest,1,129),substr(epoch,1,49) FROM retention_policy WHERE id=1",
        [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?,r.get(3)?)),
    )?;
    let policy: RetentionPolicy = serde_json::from_str(&encoded).map_err(|_| failure())?;
    if generation < 0
        || policy.history_before_ms < 0
        || policy.replay_through_sequence < 0
        || encoded.len() > 4096
        || epoch.len() != 48
        || !epoch.bytes().all(|c| c.is_ascii_hexdigit())
        || hash(&(generation, &policy, &epoch))? != digest
    {
        return Err(failure());
    }
    Ok((generation, policy))
}

struct References<'a> {
    revisions: &'a BTreeMap<String, (GraphRef, i64, bool)>,
    events: &'a BTreeMap<String, String>,
    roots: BTreeSet<String>,
    work: usize,
}
impl References<'_> {
    fn string(&mut self, text: &str, depth: usize) -> Result<()> {
        if self.revisions.contains_key(text) {
            self.roots.insert(text.into());
        }
        if let Some(revision) = self.events.get(text) {
            self.roots.insert(revision.clone());
        }
        // Some durable receipts contain an original JSON response as a string.
        if matches!(text.trim_start().chars().next(), Some('{' | '[' | '"')) {
            if depth >= 32 {
                return Err(err(
                    "E_BUDGET",
                    "retention embedded response nesting exceeded",
                ));
            }
            match serde_json::from_str::<serde_json::Value>(text) {
                Ok(value) => self.value(&value, depth + 1)?,
                Err(_) if depth == 0 => return Err(failure()),
                Err(_) => {} // Plain graph identifiers and literal strings are not encoded JSON.
            }
        }
        Ok(())
    }
    fn value(&mut self, value: &serde_json::Value, depth: usize) -> Result<()> {
        self.work = self
            .work
            .checked_sub(1)
            .ok_or_else(|| err("E_BUDGET", "retention value budget exceeded"))?;
        if depth > 128 {
            return Err(err("E_BUDGET", "retention nesting budget exceeded"));
        }
        match value {
            serde_json::Value::String(s) => self.string(s, depth)?,
            serde_json::Value::Array(a) => {
                for v in a {
                    self.value(v, depth + 1)?;
                }
            }
            serde_json::Value::Object(o) => {
                for (k, v) in o {
                    self.string(k, depth)?;
                    self.value(v, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl Engine {
    /// Disclose only this authorized branch's local observation horizon.
    pub fn recorded_availability_for(
        &self,
        graph: &str,
        branch: &str,
        host: &HostContext,
    ) -> Result<RecordedAvailability> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        let current = self.recorded_checkpoint_for(graph, branch, host)?;
        self.read_budget.request()?;
        let first: i64 = self.conn.query_row(
            "SELECT min(recorded_at_ms) FROM head_observations WHERE graph_id=?1 AND branch_id=?2",
            params![graph, branch],
            |r| r.get(0),
        )?;
        Ok(RecordedAvailability {
            observer: current.observer,
            graph_id: graph.into(),
            branch_id: branch.into(),
            retained_from_ms: self.retention_history_floor()?.max(first),
            observed_until_ms: self.operation_time()?,
        })
    }
    pub(crate) fn initialize_retention(&self, old_version: i64) -> Result<()> {
        if old_version < 22 && self.conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('retention_policy','retention_roots','retention_tombstones','retention_adapter_states','retention_projection_receipts','retention_view_epochs','retention_retired_branches','retention_stateful_adapters')", [], |r| r.get::<_,i64>(0),
        )? != 0 { return Err(failure()); }
        if old_version >= 22 && self.conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('retention_policy','retention_roots','retention_tombstones','retention_adapter_states','retention_projection_receipts','retention_view_epochs','retention_retired_branches','retention_stateful_adapters')", [], |r| r.get::<_, i64>(0),
        )? != 8 {
            return Err(failure());
        }
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS retention_policy(id INTEGER PRIMARY KEY CHECK(id=1),generation INTEGER NOT NULL,policy TEXT NOT NULL,digest TEXT NOT NULL,epoch TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS retention_roots(principal TEXT NOT NULL,id TEXT NOT NULL,refs TEXT NOT NULL,PRIMARY KEY(principal,id));
CREATE TABLE IF NOT EXISTS retention_tombstones(revision TEXT PRIMARY KEY REFERENCES revisions(revision),graph_id TEXT NOT NULL,branch_id TEXT NOT NULL,parent TEXT,content_digest TEXT NOT NULL,payload_digest TEXT NOT NULL,erased_at_ms INTEGER NOT NULL,generation INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS retention_stateful_adapters(adapter TEXT PRIMARY KEY REFERENCES dispatch_adapters(id));
CREATE TABLE IF NOT EXISTS retention_adapter_states(adapter TEXT PRIMARY KEY REFERENCES dispatch_adapters(id),epoch TEXT NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,checkpoint INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS retention_projection_receipts(adapter TEXT NOT NULL,event_id TEXT NOT NULL,request_digest TEXT NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(adapter,event_id));
CREATE TABLE IF NOT EXISTS retention_view_epochs(principal TEXT PRIMARY KEY,epoch TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS retention_retired_branches(graph_id TEXT NOT NULL,branch_id TEXT NOT NULL,revision TEXT NOT NULL,released_at_ms INTEGER NOT NULL,PRIMARY KEY(graph_id,branch_id));")?;
        let policy = RetentionPolicy::default();
        if old_version < 22 {
            let epoch: String =
                self.conn
                    .query_row("SELECT lower(hex(randomblob(24)))", [], |r| r.get(0))?;
            self.conn.execute(
                "INSERT INTO retention_policy VALUES (1,0,?1,?2,?3)",
                params![
                    serde_json::to_string(&policy)?,
                    hash(&(0_i64, &policy, &epoch))?,
                    epoch
                ],
            )?;
            self.conn.execute(
                "INSERT INTO retention_view_epochs SELECT principal,?1 FROM view_schedule_cursors",
                [epoch],
            )?;
        }
        state(&self.conn)?;
        Ok(())
    }
    pub(crate) fn retention_history_floor(&self) -> Result<i64> {
        Ok(state(&self.conn)?.1.history_before_ms)
    }
    pub(crate) fn retention_replay_epoch(&self) -> Result<String> {
        state(&self.conn)?;
        Ok(self
            .conn
            .query_row("SELECT epoch FROM retention_policy WHERE id=1", [], |r| {
                r.get(0)
            })?)
    }
    /// Pin genuine whole-authorized inputs. A retention pin does not confer read authority.
    pub fn retain_snapshots_for(
        &self,
        id: &str,
        references: &[GraphRef],
        host: &HostContext,
    ) -> Result<()> {
        if !valid_id(id)
            || !valid_id(&host.principal)
            || references.is_empty()
            || references.len() > 1000
        {
            return Err(err("E_RETENTION_ROOT", "invalid retention root"));
        }
        let _budget = self.read_budget.enter();
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_write_scope()?;
        let mut refs = references.to_vec();
        refs.sort_by(|a, b| (&a.graph_id, &a.revision).cmp(&(&b.graph_id, &b.revision)));
        refs.dedup();
        for reference in &refs {
            self.retention_whole(reference, host)?;
        }
        let body = serde_json::to_string(&refs)?;
        let old: Option<String> = self
            .conn
            .query_row(
                "SELECT refs FROM retention_roots WHERE principal=?1 AND id=?2",
                params![host.principal, id],
                |r| r.get(0),
            )
            .optional()?;
        if old.as_ref().is_some_and(|old| old != &body) {
            return Err(err(
                "E_RETENTION_ROOT",
                "root identity already pins other inputs",
            ));
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO retention_roots VALUES (?1,?2,?3)",
            params![host.principal, id, body],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Release only this session owner's pin; no graph or foreign pin is deleted.
    pub fn release_retention_root_for(&self, id: &str, host: &HostContext) -> Result<()> {
        if !valid_id(id) || !valid_id(&host.principal) {
            return Err(err("E_RETENTION_ROOT", "invalid root identity"));
        }
        self.conn.execute(
            "DELETE FROM retention_roots WHERE principal=?1 AND id=?2",
            params![host.principal, id],
        )?;
        Ok(())
    }
    pub(crate) fn retention_whole(&self, reference: &GraphRef, host: &HostContext) -> Result<()> {
        let query: QueryPlan = serde_json::from_value(
            serde_json::json!({"graph_id":reference.graph_id,"revision":reference.revision}),
        )?;
        let result = self.query(&query, host)?;
        self.require_current_result_authority(&result, host)?;
        let data = self
            .load(&reference.graph_id, &reference.revision)?
            .ok_or_else(|| err("E_UNAVAILABLE", "snapshot unavailable"))?;
        let (visible, incomplete) = self.authorized(data.clone(), host)?;
        if !whole_graph_visible(&data, visible, incomplete) {
            return Err(err("E_UNAVAILABLE", "snapshot unavailable"));
        }
        Ok(())
    }
    /// Explicit branch-root retirement. Other branches, pins, receipts and references still retain it.
    pub fn release_branch_for(
        &self,
        graph: &str,
        branch: &str,
        expected: &str,
        host: &HostContext,
    ) -> Result<()> {
        identity_acceptance::require_external_graph(graph)?;
        if ![graph, branch, expected, &host.principal]
            .iter()
            .all(|s| valid_id(s))
            || !host.writable_graphs.contains(graph)
        {
            return Err(err("E_FORBIDDEN", "branch retirement unavailable"));
        }
        let _budget = self.read_budget.enter();
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_write_scope()?;
        let actual = self
            .head(graph, branch)?
            .ok_or_else(|| err("E_UNAVAILABLE", "branch unavailable"))?;
        self.retention_whole(
            &GraphRef {
                graph_id: graph.into(),
                revision: actual.clone(),
            },
            host,
        )?;
        if actual != expected {
            return Err(err("E_CONFLICT", "branch root changed"));
        }
        self.conn.execute(
            "INSERT INTO retention_retired_branches VALUES (?1,?2,?3,?4)",
            params![graph, branch, expected, self.operation_time()?],
        )?;
        self.conn.execute(
            "DELETE FROM heads WHERE graph_id=?1 AND branch_id=?2 AND revision=?3",
            params![graph, branch, expected],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Out-of-band trusted administrator preview; never a language or remote capability operation.
    pub fn plan_retention(&self, policy: &RetentionPolicy) -> Result<RetentionPlan> {
        let _budget = self.read_budget.enter();
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.retention_plan(policy)
    }
    fn retention_plan(&self, policy: &RetentionPolicy) -> Result<RetentionPlan> {
        let (generation, previous) = state(&self.conn)?;
        let last: i64 =
            self.conn
                .query_row("SELECT coalesce(max(sequence),0) FROM events", [], |r| {
                    r.get(0)
                })?;
        if policy.history_before_ms < previous.history_before_ms
            || policy.history_before_ms > self.operation_time()?
            || policy.replay_through_sequence < previous.replay_through_sequence
            || policy.replay_through_sequence > last
        {
            return Err(err(
                "E_RETENTION_POLICY",
                "retention frontiers must advance within actual local history",
            ));
        }
        let mut revisions = BTreeMap::new();
        let mut statement = self.conn.prepare("SELECT revision,graph_id,recorded_at,length(CAST(data AS BLOB))>0 FROM revisions ORDER BY revision LIMIT ?1")?;
        let rows = statement.query_map([(MAX_REVISIONS + 1) as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, bool>(3)?,
            ))
        })?;
        for row in rows {
            let (revision, graph_id, time, present) = row?;
            if revisions.len() == MAX_REVISIONS {
                return Err(err(
                    "E_BUDGET",
                    "retention revision inventory limit exceeded",
                ));
            }
            if !valid_id(&revision) || !valid_id(&graph_id) || time < 0 {
                return Err(failure());
            }
            if !present {
                match self.load(&graph_id, &revision) {
                    Err(error) if error.code == "E_UNAVAILABLE" => {}
                    Err(error) => return Err(error),
                    Ok(_) => return Err(failure()),
                }
            }
            revisions.insert(
                revision.clone(),
                (GraphRef { graph_id, revision }, time, present),
            );
        }
        let mut events = BTreeMap::new();
        let mut roots = BTreeSet::new();
        let mut statement = self
            .conn
            .prepare("SELECT event_id,revision,sequence FROM events ORDER BY sequence LIMIT ?1")?;
        for (index, row) in statement
            .query_map([(MAX_ROWS + 1) as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .enumerate()
        {
            if index == MAX_ROWS {
                return Err(err("E_BUDGET", "retention event inventory limit exceeded"));
            }
            let (event, revision, sequence) = row?;
            if sequence > policy.replay_through_sequence {
                roots.insert(revision.clone());
            }
            events.insert(event, revision);
        }
        let mut references = References {
            revisions: &revisions,
            events: &events,
            roots,
            work: MAX_VALUES,
        };
        let tables = self.conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?.query_map([],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut row_budget = MAX_ROWS;
        for table in tables {
            if !KNOWN_TABLES.contains(&table.as_str()) {
                return Err(err(
                    "E_RETENTION_SCHEMA",
                    "unknown registry requires an explicit retention profile",
                ));
            }
            if [
                "revisions",
                "events",
                "snapshot_manifests",
                "revision_integrity",
                "retention_tombstones",
                "retention_policy",
                "retention_retired_branches",
            ]
            .contains(&table.as_str())
            {
                continue;
            }
            let columns = self
                .conn
                .prepare(&format!("PRAGMA table_info({})", quote(&table)))?
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if columns.is_empty() || columns.len() > 128 {
                return Err(failure());
            }
            let expressions = columns
                .iter()
                .flat_map(|c| {
                    let c = quote(c);
                    [
                        format!("typeof({c})"),
                        format!("CASE WHEN length(CAST({c} AS BLOB))<=?1 THEN {c} ELSE NULL END"),
                    ]
                })
                .collect::<Vec<_>>()
                .join(",");
            let condition = if table == "head_observations" {
                " WHERE recorded_at_ms>=?2 OR rowid IN (SELECT max(o.rowid) FROM head_observations o JOIN heads h ON h.graph_id=o.graph_id AND h.branch_id=o.branch_id WHERE o.recorded_at_ms<=?2 GROUP BY o.graph_id,o.branch_id)"
            } else {
                ""
            };
            let sql = format!(
                "SELECT {expressions} FROM {}{condition} LIMIT {}",
                quote(&table),
                row_budget + 1
            );
            self.read_budget.request()?;
            let mut statement = self.conn.prepare(&sql)?;
            let mut rows = if table == "head_observations" {
                statement.query(params![CELL_BYTES as i64, policy.history_before_ms])?
            } else {
                statement.query([CELL_BYTES as i64])?
            };
            while let Some(row) = rows.next()? {
                row_budget = row_budget
                    .checked_sub(1)
                    .ok_or_else(|| err("E_BUDGET", "retention registry row budget exceeded"))?;
                for (index, column) in columns.iter().enumerate() {
                    let kind: String = row.get(2 * index)?;
                    match row.get_ref(2 * index + 1)? {
                        ValueRef::Text(text) => {
                            self.read_budget.charge(text.len())?;
                            let encoded = (JSON_COLUMNS.contains(&column.as_str())
                                && !(table == "effects" && column == "payload"))
                                || column.contains("json")
                                || (table == "view_selection" && column == "state");
                            let text = std::str::from_utf8(text).map_err(|_| failure())?;
                            if encoded {
                                let value: serde_json::Value =
                                    serde_json::from_str(text).map_err(|_| failure())?;
                                if column == "body"
                                    && matches!(
                                        table.as_str(),
                                        "retention_adapter_states"
                                            | "recorded_actor_definitions"
                                            | "recorded_actor_states"
                                            | "recorded_actor_receipts"
                                            | "retention_projection_receipts"
                                            | "delivery_cancellations"
                                            | "projection_migrations"
                                            | "compiled_migrations"
                                            | "compiled_rebuild_receipts"
                                            | "compiled_replay_states"
                                    )
                                {
                                    let field = |name: &str| -> Result<String> {
                                        let index = columns
                                            .iter()
                                            .position(|c| c == name)
                                            .ok_or_else(failure)?;
                                        Ok(row.get(2 * index + 1)?)
                                    };
                                    let adapter = if matches!(
                                        table.as_str(),
                                        "projection_migrations" | "compiled_migrations"
                                    ) {
                                        field("source_adapter")?
                                    } else {
                                        field("adapter")?
                                    };
                                    if matches!(
                                        table.as_str(),
                                        "recorded_actor_definitions"
                                            | "recorded_actor_states"
                                            | "recorded_actor_receipts"
                                    ) {
                                        recorded_actors::validate_retained_actor(
                                            self,
                                            &table,
                                            &adapter,
                                            &if table == "recorded_actor_receipts" {
                                                field("event_id")?
                                            } else {
                                                String::new()
                                            },
                                            &field("digest")?,
                                            &value,
                                        )?;
                                    } else if matches!(
                                        table.as_str(),
                                        "compiled_rebuild_receipts" | "compiled_replay_states"
                                    ) {
                                        compiled_rebuild::validate_retained_rebuild(
                                            self,
                                            &table,
                                            &adapter,
                                            &if table == "compiled_rebuild_receipts" {
                                                field("nonce")?
                                            } else {
                                                String::new()
                                            },
                                            &field("digest")?,
                                            &value,
                                        )?;
                                    } else if table == "compiled_migrations" {
                                        compiled_lifecycle::validate_retained_migration(
                                            self,
                                            &adapter,
                                            &field("destination_adapter")?,
                                            &field("principal")?,
                                            &field("nonce")?,
                                            &field("digest")?,
                                            &value,
                                        )?;
                                    } else if table == "projection_migrations" {
                                        projection_migration::validate_retained_migration(
                                            &adapter,
                                            &field("destination_adapter")?,
                                            &field("principal")?,
                                            &field("nonce")?,
                                            &field("digest")?,
                                            &value,
                                        )?;
                                    } else if table == "delivery_cancellations" {
                                        let event = field("event_id")?;
                                        let revision = events.get(&event).ok_or_else(|| {
                                            err(
                                                "E_LIFECYCLE_INTEGRITY",
                                                "cancellation occurrence unavailable",
                                            )
                                        })?;
                                        let source =
                                            &revisions.get(revision).ok_or_else(failure)?.0;
                                        adapter_lifecycle::validate_retained_cancellation(
                                            &adapter,
                                            &event,
                                            &field("nonce")?,
                                            source,
                                            &field("digest")?,
                                            &value,
                                        )?;
                                    } else if table == "retention_adapter_states" {
                                        self.projection_state(&adapter)?.ok_or_else(failure)?;
                                    } else {
                                        projection_rebase::validate_retained_receipt(
                                            &adapter,
                                            &field("request_digest")?,
                                            &field("digest")?,
                                            &value,
                                        )?;
                                    }
                                }
                                references.value(&value, 1)?;
                            } else {
                                references.string(text, 1)?;
                            }
                        }
                        ValueRef::Null if kind != "null" => {
                            return Err(err("E_BUDGET", "retention registry cell exceeds budget"))
                        }
                        ValueRef::Blob(_) => {
                            return Err(err(
                                "E_RETENTION_SCHEMA",
                                "opaque registry blocks require an explicit retention profile",
                            ))
                        }
                        _ => {}
                    }
                }
            }
        }
        let mut queue = VecDeque::from_iter(references.roots.iter().cloned());
        let mut retained = BTreeSet::new();
        while let Some(revision) = queue.pop_front() {
            if !retained.insert(revision.clone()) {
                continue;
            }
            let (reference, _, present) = revisions.get(&revision).ok_or_else(failure)?;
            if !present {
                return Err(err(
                    "E_RETENTION_UNAVAILABLE",
                    "a retained root requires expired payload",
                ));
            }
            let data = self
                .load(&reference.graph_id, &revision)?
                .ok_or_else(failure)?;
            for reference in refs(&data) {
                if revisions.contains_key(&reference.revision) {
                    queue.push_back(reference.revision);
                }
            }
            // Conservative exact-ID tracing also covers every proof carrier and literal pointer.
            // False positives retain storage; they cannot erase an unexamined reference.
            let mut nested = References {
                revisions: &revisions,
                events: &events,
                roots: BTreeSet::new(),
                work: references.work,
            };
            nested.value(&serde_json::to_value(&data)?, 0)?;
            references.work = nested.work;
            queue.extend(nested.roots);
            // Any member needed for an atomic logical snapshot retains all members, not ancestor payloads.
            if revision.starts_with("logical:") {
                self.read_budget.request()?;
                let encoded: String = self.conn.query_row("SELECT m.manifest FROM snapshot_manifests m JOIN revision_integrity i ON i.manifest_id=m.id WHERE i.revision=?1",[&revision],|r|r.get(0))?;
                self.read_budget.charge(encoded.len())?;
                let manifest: SnapshotManifest = serde_json::from_str(&encoded)?;
                for member in manifest.members {
                    queue.push_back(member.revision);
                }
            }
        }
        let mut collect = Vec::new();
        let mut payload_bytes = 0;
        for (revision, (reference, time, present)) in &revisions {
            if *present && *time < policy.history_before_ms && !retained.contains(revision) {
                self.load(&reference.graph_id, revision)?
                    .ok_or_else(failure)?;
                let bytes: i64 = self.conn.query_row(
                    "SELECT length(CAST(data AS BLOB)) FROM revisions WHERE revision=?1",
                    [revision],
                    |r| r.get(0),
                )?;
                payload_bytes += u64::try_from(bytes).map_err(|_| failure())?;
                collect.push(reference.clone());
            }
        }
        let mut plan = RetentionPlan {
            generation,
            policy: policy.clone(),
            retained: retained.iter().map(|r| revisions[r].0.clone()).collect(),
            collect,
            payload_bytes,
            digest: String::new(),
        };
        plan.digest = hash(&plan)?;
        Ok(plan)
    }
    /// Recheck the complete root graph inside the erasure transaction. No implicit vacuum or external deletion.
    pub fn compact_retention(&self, plan: &RetentionPlan) -> Result<RetentionReceipt> {
        self.compact_retention_boundary(plan, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn compact_retention_test_before_commit(
        &self,
        plan: &RetentionPlan,
        before: impl FnOnce(),
    ) -> Result<RetentionReceipt> {
        self.compact_retention_boundary(plan, before)
    }
    fn compact_retention_boundary(
        &self,
        plan: &RetentionPlan,
        before: impl FnOnce(),
    ) -> Result<RetentionReceipt> {
        let _budget = self.read_budget.enter();
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_write_scope()?;
        if &self.retention_plan(&plan.policy)? != plan {
            return Err(err("E_CONFLICT", "retention roots changed since preview"));
        }
        let (_, current_policy) = state(&self.conn)?;
        if plan.collect.is_empty() && current_policy == plan.policy {
            tx.commit()?;
            return Ok(RetentionReceipt {
                generation: plan.generation,
                plan_digest: plan.digest.clone(),
                collected_payloads: 0,
                payload_bytes: 0,
            });
        }
        let generation = plan.generation.checked_add(1).ok_or_else(failure)?;
        let epoch: String = if current_policy == plan.policy {
            self.conn
                .query_row("SELECT epoch FROM retention_policy WHERE id=1", [], |r| {
                    r.get(0)
                })?
        } else {
            self.conn
                .query_row("SELECT lower(hex(randomblob(24)))", [], |r| r.get(0))?
        };
        let time = self.operation_time()?;
        for reference in &plan.collect {
            self.read_budget.request()?;
            let (branch, parent, body): (String, Option<String>, String) = self.conn.query_row(
                "SELECT branch_id,parent,data FROM revisions WHERE revision=?1 AND graph_id=?2",
                params![reference.revision, reference.graph_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            self.read_budget.charge(body.len())?;
            let data: GraphData = serde_json::from_str(&body)?;
            let content = snapshot::content_digest(&reference.graph_id, &branch, &parent, &data)?;
            let payload = format!("sha256:{:x}", Sha256::digest(body.as_bytes()));
            self.conn.execute(
                "INSERT INTO retention_tombstones VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    reference.revision,
                    reference.graph_id,
                    branch,
                    parent,
                    content,
                    payload,
                    time,
                    generation
                ],
            )?;
            self.conn.execute(
                "UPDATE revisions SET data='' WHERE revision=?1",
                [&reference.revision],
            )?;
        }
        self.conn.execute(
            "UPDATE retention_policy SET generation=?1,policy=?2,digest=?3,epoch=?4 WHERE id=1",
            params![
                generation,
                serde_json::to_string(&plan.policy)?,
                hash(&(generation, &plan.policy, &epoch))?,
                epoch
            ],
        )?;
        before();
        tx.commit()?;
        Ok(RetentionReceipt {
            generation,
            plan_digest: plan.digest.clone(),
            collected_payloads: plan.collect.len(),
            payload_bytes: plan.payload_bytes,
        })
    }
}
