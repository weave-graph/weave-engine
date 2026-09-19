//! Native reference runtime. The portable wire/model crate is `weave-contract`.
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use weave_contract::*;
mod admission;
mod identity_acceptance;
pub use identity_acceptance::{
    IdentityCandidate, IdentityDecisionReceipt, IdentityDecisionRequest, IdentityPolicy,
};
mod assertions;
mod read_budget;
pub use admission::{Admitted, ProposalReceipt};
mod capsule;
use assertions::{assertion_edge, materialize, validate_explicit};
mod dispatch;
mod views;
pub use dispatch::{
    AdapterManifest, DispatchEnvelope, EffectIntent, HandlerReceipt, SubscriptionScope,
};
pub use views::{ViewChange, ViewClock, ViewDefinition, ViewFreshness, ViewSnapshot};
mod clustering;
mod geometry;
pub use weave_contract::{ClusterRequest, IdentityPolicyRef, IdentityResolve};
mod metadata;
mod snapshot;
mod typed;
pub use capsule::{Capsule, CapsuleRevision};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Error {
    pub code: String,
    pub message: String,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        err("E_STORAGE", &e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        err("E_FORMAT", &e.to_string())
    }
}
fn err(code: &str, message: &str) -> Error {
    Error {
        code: code.into(),
        message: message.into(),
    }
}
pub type Result<T> = std::result::Result<T, Error>;
/// Authority supplied by the trusted embedding host, never deserialized from plans.
#[derive(Debug, Clone)]
pub struct HostContext {
    pub principal: String,
    pub writable_graphs: HashSet<String>,
}
impl HostContext {
    pub fn new(principal: impl Into<String>, graphs: impl IntoIterator<Item = String>) -> Self {
        Self {
            principal: principal.into(),
            writable_graphs: graphs.into_iter().collect(),
        }
    }
}
pub struct Engine {
    conn: Connection,
    read_budget: read_budget::ReadBudget,
}
impl Engine {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }
    pub fn memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }
    fn from_connection(conn: Connection) -> Result<Self> {
        Self::from_connection_boundary(conn, || {})
    }
    /// Test-only interruption after schema/backfill SQL, before its outer COMMIT.
    #[cfg(feature = "recovery-testing")]
    pub fn open_test_before_schema_commit(
        path: impl AsRef<Path>,
        before_commit: impl FnOnce(),
    ) -> Result<Self> {
        Self::from_connection_boundary(Connection::open(path)?, before_commit)
    }
    fn from_connection_boundary(conn: Connection, before_commit: impl FnOnce()) -> Result<Self> {
        let version = conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?;
        if !(0..=7).contains(&version) {
            return Err(err(
                "E_STORAGE_VERSION",
                "database schema version is unsupported",
            ));
        }
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;")?;
        let engine = Self {
            conn,
            read_budget: read_budget::ReadBudget::default(),
        };
        let initialization = rusqlite::Transaction::new_unchecked(
            &engine.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        engine.conn.execute_batch("
 CREATE TABLE IF NOT EXISTS revisions(revision TEXT PRIMARY KEY,graph_id TEXT NOT NULL,branch_id TEXT NOT NULL,parent TEXT,recorded_at INTEGER NOT NULL,data TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS heads(graph_id TEXT NOT NULL,branch_id TEXT NOT NULL,revision TEXT NOT NULL REFERENCES revisions(revision),PRIMARY KEY(graph_id,branch_id));
 CREATE TABLE IF NOT EXISTS events(sequence INTEGER PRIMARY KEY AUTOINCREMENT,event_id TEXT UNIQUE NOT NULL,graph_id TEXT NOT NULL,branch_id TEXT NOT NULL,revision TEXT NOT NULL REFERENCES revisions(revision),actor TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS adapters(id TEXT PRIMARY KEY,graph_id TEXT NOT NULL,paused INTEGER NOT NULL DEFAULT 0);
 CREATE TABLE IF NOT EXISTS deliveries(adapter TEXT NOT NULL REFERENCES adapters(id),event_id TEXT NOT NULL REFERENCES events(event_id),attempts INTEGER NOT NULL,status TEXT NOT NULL,PRIMARY KEY(adapter,event_id));
 CREATE TABLE IF NOT EXISTS schema_registry(id TEXT NOT NULL,revision TEXT NOT NULL,descriptor TEXT NOT NULL,PRIMARY KEY(id,revision));
 CREATE TABLE IF NOT EXISTS snapshot_manifests(id TEXT PRIMARY KEY,batch_id TEXT UNIQUE NOT NULL,manifest TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS revision_integrity(revision TEXT PRIMARY KEY REFERENCES revisions(revision),content_digest TEXT NOT NULL,manifest_id TEXT NOT NULL REFERENCES snapshot_manifests(id));
 CREATE TABLE IF NOT EXISTS assertion_structures(graph_id TEXT NOT NULL,assertion_id TEXT NOT NULL,edge_id TEXT NOT NULL,source TEXT NOT NULL,PRIMARY KEY(graph_id,assertion_id));
 CREATE TABLE IF NOT EXISTS edge_structures(graph_id TEXT NOT NULL,edge_id TEXT NOT NULL,from_id TEXT NOT NULL,to_id TEXT NOT NULL,predicate TEXT NOT NULL,PRIMARY KEY(graph_id,edge_id));
 CREATE TABLE IF NOT EXISTS effects(adapter TEXT NOT NULL,event_id TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(adapter,event_id));")?;
        // Backfill structural identity from legacy immutable snapshots on first upgrade.
        if engine
            .conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?
            < 6
        {
            {
                let mut statement = engine
                    .conn
                    .prepare("SELECT substr(graph_id,1,513),substr(revision,1,513) FROM revisions ORDER BY rowid")?;
                let rows = statement
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
                for row in rows {
                    let (graph, revision) = row?;
                    let data = engine.load(&graph, &revision)?.ok_or_else(|| {
                        err("E_INTEGRITY", "legacy revision unavailable during backfill")
                    })?;
                    engine.validate_structures(&graph, &data)?;
                    engine.record_structures(&graph, &data)?;
                }
            }
            engine.conn.pragma_update(None, "user_version", 6)?;
        }
        engine.initialize_dispatch()?;
        engine.initialize_views()?;
        engine.initialize_admission()?;
        engine.initialize_identity()?;
        engine.conn.pragma_update(None, "user_version", 7)?;
        before_commit();
        initialization.commit()?;
        Ok(engine)
    }
    pub fn execute(&mut self, program: &Program, host: &HostContext) -> Result<Vec<CommandResult>> {
        let _read_scope = self.read_budget.enter();
        if ![
            VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0", "0.7.0", "0.6.0", "0.5.0",
            "0.4.0", "0.3.0",
        ]
        .contains(&program.version.as_str())
            && program.version != "0.2.0"
            && program.version != "0.3.0"
            && program.version != LEGACY_VERSION
        {
            return Err(err("E_VERSION", "unsupported contract version"));
        }
        if program.version == LEGACY_VERSION
            && program
                .commands
                .iter()
                .any(|c| matches!(c, Command::Join { .. }))
        {
            return Err(err("E_VERSION", "join requires contract 0.2.0"));
        }
        if ![
            VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0", "0.7.0", "0.6.0", "0.5.0",
            "0.4.0", "0.3.0",
        ]
        .contains(&program.version.as_str())
            && program
                .commands
                .iter()
                .any(|c| matches!(c, Command::Bind { .. } | Command::Evaluate { .. }))
        {
            return Err(err("E_VERSION", "graph expressions require contract 0.3.0"));
        }
        if ![VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0", "0.7.0", "0.6.0", "0.5.0", "0.4.0"].contains(&program.version.as_str()) && program.commands.iter().any(|command| matches!(command,Command::Commit { data,.. } if data.schema.is_some() || !data.attachments.is_empty() || data.nodes.iter().any(|n|n.type_id.is_some()) || data.edges.iter().any(|e|e.type_id.is_some()))) { return Err(err("E_VERSION","schemas and named attachments require contract 0.4.0")); }
        if ![
            VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0", "0.7.0", "0.6.0", "0.5.0",
        ]
        .contains(&program.version.as_str())
            && program.commands.iter().any(|c| match c {
                Command::Commit { data, .. } => {
                    data.edges.iter().any(|e| !e.derivations.is_empty())
                }
                Command::CommitBatch { commits, .. } => commits
                    .iter()
                    .any(|c| c.data.edges.iter().any(|e| !e.derivations.is_empty())),
                _ => false,
            })
        {
            return Err(err(
                "E_VERSION",
                "derivation alternatives require contract 0.5.0",
            ));
        }
        if ![
            VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0", "0.7.0", "0.6.0",
        ]
        .contains(&program.version.as_str())
            && program.commands.iter().any(|c| match c {
                Command::Commit { data, .. } => requires_explicit_profile(data),
                Command::CommitBatch { commits, .. } => {
                    commits.iter().any(|c| requires_explicit_profile(&c.data))
                }
                _ => false,
            })
        {
            return Err(err(
                "E_VERSION",
                "explicit assertions require contract 0.6.0",
            ));
        }
        if ![VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0"]
            .contains(&program.version.as_str())
            && program.commands.iter().any(|c| match c {
                Command::Commit { data, .. } => {
                    data.attachments.iter().any(|a| a.context.is_some())
                        || data.nodes.iter().any(|n| n.context_scope.is_some())
                }
                Command::CommitBatch { commits, .. } => commits.iter().any(|c| {
                    c.data.attachments.iter().any(|a| a.context.is_some())
                        || c.data.nodes.iter().any(|n| n.context_scope.is_some())
                }),
                _ => false,
            })
        {
            return Err(err(
                "E_VERSION",
                "attachment/node context requires contract 0.8.0",
            ));
        }
        if ![VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0"].contains(&program.version.as_str())
            && program.commands.iter().any(|c| match c {
                Command::Commit { data, .. } => {
                    has_float_schema(data) || data.nodes.iter().any(|n| !n.derived_from.is_empty())
                }
                Command::CommitBatch { commits, .. } => commits.iter().any(|c| {
                    has_float_schema(&c.data)
                        || c.data.nodes.iter().any(|n| !n.derived_from.is_empty())
                }),
                _ => false,
            })
        {
            return Err(err(
                "E_VERSION",
                "Float schemas and node dependencies require contract 0.9.0",
            ));
        }
        if ![VERSION, "0.12.0", "0.11.0"].contains(&program.version.as_str())
            && program.commands.iter().any(|c| match c {
                Command::Commit { data, .. } => {
                    data.nodes.iter().any(|n| !n.derived_nodes.is_empty())
                }
                Command::CommitBatch { commits, .. } => commits
                    .iter()
                    .any(|c| c.data.nodes.iter().any(|n| !n.derived_nodes.is_empty())),
                _ => false,
            })
        {
            return Err(err(
                "E_VERSION",
                "node source influences require contract 0.11.0",
            ));
        }
        if ![VERSION, "0.12.0"].contains(&program.version.as_str())
            && program.commands.iter().any(|command| match command {
                Command::Commit { data, .. } => has_exact_schema(data),
                Command::CommitBatch { commits, .. } => {
                    commits.iter().any(|c| has_exact_schema(&c.data))
                }
                _ => false,
            })
        {
            return Err(err(
                "E_VERSION",
                "exact Decimal and Quantity schemas require contract 0.12.0",
            ));
        }
        for command in &program.commands {
            if let Command::Bind { value, .. } | Command::Evaluate { value } = command {
                validate_expression_profile(value, &program.version)?;
            }
        }
        if !program.source_revisions.is_empty()
            && ![
                VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0", "0.7.0", "0.6.0",
            ]
            .contains(&program.version.as_str())
        {
            return Err(err(
                "E_VERSION",
                "source revision manifests require contract 0.6.0",
            ));
        }
        json_size(&program.source_revisions, 1024 * 1024)?;
        let mut source_labels = BTreeMap::new();
        for source in &program.source_revisions {
            if !valid_id(&source.name) || !valid_id(&source.revision) || !valid_id(&source.digest) {
                return Err(err(
                    "E_SOURCE_REVISION",
                    "source revision fields must be bounded identifiers",
                ));
            }
            if source_labels
                .insert((&source.name, &source.revision), &source.digest)
                .is_some_and(|prior| prior != &source.digest)
            {
                return Err(err(
                    "E_SOURCE_REVISION",
                    "conflicting source revision labels",
                ));
            }
        }
        if program.source_revisions.len() > 1000 {
            return Err(err("E_BUDGET", "source manifest exceeds budget"));
        }
        if program.commands.len() > 1000 {
            return Err(err("E_BUDGET", "at most 1000 commands"));
        }
        // One program transaction: a later rejection cannot leave earlier graph changes or events.
        self.conn.execute_batch("SAVEPOINT weave_program")?;
        let result = (|| {
            let mut out = Vec::new();
            let mut values = BTreeMap::new();
            let mut value_size = 0usize;
            let mut materialized_bytes = 0usize;
            for command in &program.commands {
                let mut command_result = match command {
                    Command::CommitBatch { batch_id, commits } => {
                        if ![
                            VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0", "0.7.0",
                            "0.6.0", "0.5.0", "0.4.0",
                        ]
                        .contains(&program.version.as_str())
                        {
                            return Err(err(
                                "E_VERSION",
                                "snapshot batches require contract 0.4.0",
                            ));
                        }
                        let (manifest_id, commits) = self.commit_batch(batch_id, commits, host)?;
                        match manifest_id {
                            Some(manifest_id) => CommandResult::BatchCommitted {
                                manifest_id,
                                commits,
                            },
                            None => CommandResult::BatchUnchanged { commits },
                        }
                    }
                    Command::Bind { name, value } => {
                        if !valid_id(name) || values.contains_key(name) {
                            return Err(err(
                                "E_BINDING",
                                "graph value name must be nonempty and unique",
                            ));
                        }
                        let mut result = self.expression(value, &values, host, 0, &mut 1000)?;
                        merge_sources(&mut result.source_revisions, &program.source_revisions)?;
                        value_size += result.graph.nodes.len() + result.graph.edges.len();
                        if value_size > 200_000 {
                            return Err(err("E_BUDGET", "bound graph value budget exceeded"));
                        }
                        materialized_bytes += json_size(
                            &result,
                            MATERIALIZED_LIMIT.saturating_sub(materialized_bytes),
                        )?;
                        values.insert(name.clone(), result.clone());
                        CommandResult::Queried {
                            result: Box::new(result),
                        }
                    }
                    Command::Evaluate { value } => CommandResult::Queried {
                        result: Box::new(self.expression(value, &values, host, 0, &mut 1000)?),
                    },

                    Command::Join {
                        left,
                        right,
                        output_predicate,
                        match_on: JoinMatch::EntitySpaceToFrom,
                    } => CommandResult::Queried {
                        result: Box::new(self.join(left, right, output_predicate, host)?),
                    },
                    Command::Commit {
                        graph_id,
                        branch_id,
                        expected_head,
                        data,
                    } => {
                        let (revision, event_id) = self.commit_inner(
                            graph_id,
                            branch_id,
                            expected_head.as_deref(),
                            data,
                            host,
                        )?;
                        match event_id {
                            Some(event_id) => CommandResult::Committed { revision, event_id },
                            None => CommandResult::Unchanged { revision },
                        }
                    }
                    Command::Query { query } => CommandResult::Queried {
                        result: Box::new(self.query(query, host)?),
                    },
                };
                if let CommandResult::Queried { result } = &mut command_result {
                    merge_sources(&mut result.source_revisions, &program.source_revisions)?;
                    if let Command::Bind { name, .. } = command {
                        if let Some(bound) = values.get_mut(name) {
                            bound.source_revisions = result.source_revisions.clone();
                        }
                    }
                }
                materialized_bytes += json_size(
                    &command_result,
                    MATERIALIZED_LIMIT.saturating_sub(materialized_bytes),
                )?;
                out.push(command_result);
            }
            Ok(out)
        })();
        match result {
            Ok(out) => {
                self.conn.execute_batch("RELEASE weave_program")?;
                Ok(out)
            }
            Err(e) => {
                self.conn
                    .execute_batch("ROLLBACK TO weave_program; RELEASE weave_program")?;
                Err(e)
            }
        }
    }
    fn commit_inner(
        &self,
        graph: &str,
        branch: &str,
        expected: Option<&str>,
        data: &GraphData,
        host: &HostContext,
    ) -> Result<(String, Option<String>)> {
        identity_acceptance::require_external_graph(graph)?;
        identity_acceptance::require_external_schema(data)?;
        self.commit_storage_inner(graph, branch, expected, data, host)
    }
    fn commit_storage_inner(
        &self,
        graph: &str,
        branch: &str,
        expected: Option<&str>,
        data: &GraphData,
        host: &HostContext,
    ) -> Result<(String, Option<String>)> {
        if !host.writable_graphs.contains(graph) {
            return Err(err(
                "E_FORBIDDEN",
                "host has not granted graph write authority",
            ));
        }
        if !valid_id(graph) || !valid_id(branch) || !valid_id(&host.principal) {
            return Err(err("E_ID", "identifiers must not be empty"));
        }
        validate_graph(data)?;
        self.validate_structures(graph, data)?;
        let head = self.head(graph, branch)?;
        if head.as_deref() != expected {
            return Err(err(
                "E_CONFLICT",
                "expected head differs from current branch head",
            ));
        }
        if let Some(revision) = &head {
            if self.load(graph, revision)?.as_ref() == Some(data) {
                return Ok((revision.clone(), None));
            }
        }
        json_size(data, 16 * 1024 * 1024)?;
        let json = serde_json::to_string(data)?;
        let bytes = serde_json::to_vec(&("weave-revision-v0.1", graph, branch, &head, data))?;
        let revision = format!("sha256:{:x}", Sha256::digest(bytes));
        let event_id = format!("commit:{revision}");
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| err("E_CLOCK", "clock before epoch"))?
            .as_millis();
        let time = i64::try_from(time).map_err(|_| err("E_CLOCK", "clock out of range"))?;
        self.conn.execute(
            "INSERT INTO revisions VALUES (?1,?2,?3,?4,?5,?6)",
            params![revision, graph, branch, head, time, json],
        )?;
        self.conn.execute("INSERT INTO heads VALUES (?1,?2,?3) ON CONFLICT(graph_id,branch_id) DO UPDATE SET revision=excluded.revision",params![graph,branch,revision])?;
        self.conn.execute("INSERT INTO events(event_id,graph_id,branch_id,revision,actor) VALUES (?1,?2,?3,?4,?5)",params![event_id,graph,branch,revision,host.principal])?;
        self.validate_required_metadata(data, host)?;
        self.record_structures(graph, data)?;
        Ok((revision, Some(event_id)))
    }
    pub fn head(&self, graph: &str, branch: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT revision FROM heads WHERE graph_id=?1 AND branch_id=?2",
                params![graph, branch],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn recorded_at(&self, revision: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT recorded_at FROM revisions WHERE revision=?1",
            [revision],
            |r| r.get(0),
        )?)
    }
    fn load(&self, graph: &str, revision: &str) -> Result<Option<GraphData>> {
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(16 * 1024 * 1024);
        let row: Option<(String, Option<String>, Option<String>)> = self
            .conn
            .query_row(
                "SELECT substr(branch_id,1,513),substr(parent,1,513),CASE WHEN length(CAST(data AS BLOB))<=?3 THEN data ELSE NULL END FROM revisions WHERE graph_id=?1 AND revision=?2",
                params![graph, revision, limit as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((branch, parent, encoded)) = row else {
            return Ok(None);
        };
        let encoded = encoded.ok_or_else(|| {
            if limit < 16 * 1024 * 1024 {
                err("E_BUDGET", "cumulative graph-read byte budget exceeded")
            } else {
                err("E_INTEGRITY", "stored revision exceeds format bounds")
            }
        })?;
        self.read_budget.charge(encoded.len())?;
        if !valid_id(&branch) || parent.as_ref().is_some_and(|p| !valid_id(p)) {
            return Err(err("E_INTEGRITY", "stored revision exceeds format bounds"));
        }
        let data: GraphData = serde_json::from_str(&encoded)
            .map_err(|_| err("E_INTEGRITY", "stored revision is malformed"))?;
        let digest = snapshot::content_digest(graph, &branch, &parent, &data)?;
        if revision.starts_with("logical:") {
            let limit = self.read_budget.remaining().min(16 * 1024 * 1024);
            let integrity: Option<(String, String, Option<String>)> = self.conn.query_row(
                "SELECT substr(i.content_digest,1,129),substr(i.manifest_id,1,129),CASE WHEN length(CAST(m.manifest AS BLOB))<=?2 THEN m.manifest ELSE NULL END FROM revision_integrity i JOIN snapshot_manifests m ON m.id=i.manifest_id WHERE i.revision=?1",
                params![revision,limit as i64], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            let (stored_digest, id, manifest) = integrity
                .ok_or_else(|| err("E_INTEGRITY", "stored logical revision lacks its manifest"))?;
            let manifest = manifest.ok_or_else(|| {
                if limit < 16 * 1024 * 1024 {
                    err("E_BUDGET", "cumulative graph-read byte budget exceeded")
                } else {
                    err("E_INTEGRITY", "stored manifest exceeds format bounds")
                }
            })?;
            self.read_budget.charge(manifest.len())?;
            let manifest: SnapshotManifest = serde_json::from_str(&manifest)
                .map_err(|_| err("E_INTEGRITY", "stored manifest is malformed"))?;
            let hash = format!(
                "manifest:{:x}",
                Sha256::digest(serde_json::to_vec(&("weave-manifest-v0.4", &manifest))?)
            );
            let member = manifest
                .members
                .iter()
                .find(|m| m.graph_id == graph && m.revision == revision);
            if digest != stored_digest
                || hash != id
                || !member.is_some_and(|m| {
                    m.branch_id == branch && m.parent == parent && m.content_digest == digest
                })
                || revision != format!("logical:{}:{graph}", manifest.batch_id)
                || manifest.members.is_empty()
                || manifest.members.len() > 100
                || manifest
                    .members
                    .windows(2)
                    .any(|m| m[0].graph_id >= m[1].graph_id)
            {
                return Err(err(
                    "E_INTEGRITY",
                    "stored logical revision does not match its manifest",
                ));
            }
        } else if digest != revision {
            return Err(err("E_INTEGRITY", "stored revision digest mismatch"));
        }
        Ok(Some(data))
    }
    pub fn query(&self, query: &QueryPlan, host: &HostContext) -> Result<QueryResult> {
        let _read_scope = self.read_budget.enter();
        if !valid_id(&query.graph_id)
            || !valid_id(&query.branch_id)
            || !valid_id(&host.principal)
            || query.revision.as_ref().is_some_and(|v| !valid_id(v))
        {
            return Err(err("E_ID", "identifiers require 1 to 512 UTF-8 bytes"));
        }
        if query.max_depth > 32 {
            return Err(err("E_BUDGET", "metadata depth cannot exceed 32"));
        }
        let read_transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let revision = match &query.revision {
            Some(rev) => rev.clone(),
            None => self
                .head(&query.graph_id, &query.branch_id)?
                .ok_or_else(|| err("E_UNAVAILABLE", "graph unavailable"))?,
        };
        if !self.identity_reference_allowed(&query.graph_id, &revision, host)? {
            return Err(err("E_UNAVAILABLE", "graph unavailable"));
        }
        let data = self
            .load(&query.graph_id, &revision)?
            .ok_or_else(|| err("E_UNAVAILABLE", "graph unavailable"))?;
        let (graph, incomplete_derivation) = self.authorized(data, host)?;
        let (mut graph, materialized_origins) = materialize(graph, &query.graph_id, &revision)?;
        graph.edges.retain(|e| {
            query.predicate.as_ref().is_none_or(|p| p == &e.predicate)
                && query.from.as_ref().is_none_or(|p| p == &e.from)
                && query.to.as_ref().is_none_or(|p| p == &e.to)
                && query.valid_at.is_none_or(|t| e.valid_time.contains(t))
        });
        if query.predicate.is_some()
            || query.from.is_some()
            || query.to.is_some()
            || query.valid_at.is_some()
        {
            let ids: HashSet<_> = graph
                .edges
                .iter()
                .flat_map(|e| [e.from.clone(), e.to.clone()])
                .collect();
            graph.nodes.retain(|n| ids.contains(&n.id));
        }
        graph
            .attachments
            .retain(|a| query.valid_at.is_none_or(|t| a.valid_time.contains(t)));
        prune_attachments(&mut graph);
        let mut result = QueryResult {
            selected_context: None,
            source_revisions: vec![],
            version: VERSION.into(),
            graph,
            snapshots: BTreeMap::from([(query.graph_id.clone(), revision.clone())]),
            input_snapshots: vec![GraphRef {
                graph_id: query.graph_id.clone(),
                revision: revision.clone(),
            }],
            coverage: Coverage::Complete,
            diagnostics: Vec::new(),
            provenance: Vec::new(),
            edge_origins: BTreeMap::new(),
            node_origins: BTreeMap::new(),
            attachment_origins: materialized_origins,
            metadata_graphs: Vec::new(),
        };

        if identity_acceptance::reserved(&query.graph_id) {
            partial(
                &mut result,
                "E_IDENTITY_SCOPE",
                "identity results cover only authorized and available membership records",
            );
        } else if incomplete_derivation {
            partial(
                &mut result,
                "E_DERIVATION_UNAVAILABLE",
                "some derivation dependencies unavailable",
            );
        }
        let mut query_bytes = json_size(&result, MATERIALIZED_LIMIT)?;
        for node in &mut result.graph.nodes {
            let origins = vec![NodeRef {
                graph_id: query.graph_id.clone(),
                revision: revision.clone(),
                node_id: node.id.clone(),
            }];
            query_bytes += json_size(
                &(&node.id, &origins),
                MATERIALIZED_LIMIT.saturating_sub(query_bytes),
            )?;
            if !node.derived_nodes.contains(&origins[0]) {
                query_bytes +=
                    json_size(&origins[0], MATERIALIZED_LIMIT.saturating_sub(query_bytes))?;
                node.derived_nodes.push(origins[0].clone());
            }
            result.node_origins.insert(node.id.clone(), origins);
        }
        for attachment in &result.graph.attachments {
            let references = result
                .attachment_origins
                .get(&attachment.id)
                .cloned()
                .unwrap_or_else(|| {
                    vec![AssertionRef {
                        graph_id: query.graph_id.clone(),
                        revision: revision.clone(),
                        assertion_id: attachment.id.clone(),
                    }]
                });
            query_bytes += json_size(
                &(&attachment.id, &references),
                MATERIALIZED_LIMIT.saturating_sub(query_bytes),
            )?;
            result
                .attachment_origins
                .insert(attachment.id.clone(), references);
        }
        for edge in &result.graph.edges {
            let reference = AssertionRef {
                graph_id: query.graph_id.clone(),
                revision: revision.clone(),
                assertion_id: edge.id.clone(),
            };
            query_bytes += json_size(&reference, MATERIALIZED_LIMIT.saturating_sub(query_bytes))?;
            query_bytes += json_size(
                &(&edge.id, [&reference]),
                MATERIALIZED_LIMIT.saturating_sub(query_bytes),
            )?;
            result.provenance.push(reference.clone());
            result.edge_origins.insert(edge.id.clone(), vec![reference]);
        }
        let mut live_pins = BTreeMap::new();
        let (initial_refs, unavailable_live) = self.query_refs(&result.graph, &mut live_pins)?;
        metadata::pin_live_attachments(&mut result.graph, &live_pins);
        if unavailable_live {
            partial(
                &mut result,
                "E_DEPENDENCY_UNAVAILABLE",
                "metadata dependency unavailable",
            );
        }
        for ((graph_id, _), revision) in &live_pins {
            if let Some(revision) = revision {
                let reference = GraphRef {
                    graph_id: graph_id.clone(),
                    revision: revision.clone(),
                };
                if !result.input_snapshots.contains(&reference) {
                    result.input_snapshots.push(reference);
                }
            }
        }
        if query.include_metadata {
            let mut seen = HashSet::from([(query.graph_id.clone(), revision)]);
            let mut pending: std::collections::VecDeque<_> =
                initial_refs.into_iter().map(|r| (r, 1)).collect();
            let mut visited = 0;
            while let Some((reference, depth)) = pending.pop_front() {
                if !seen.insert((reference.graph_id.clone(), reference.revision.clone())) {
                    continue;
                }
                visited += 1;
                if depth > query.max_depth || visited > 1000 {
                    partial(&mut result, "E_BUDGET", "metadata traversal budget reached");
                    continue;
                }
                match if self.identity_reference_allowed(
                    &reference.graph_id,
                    &reference.revision,
                    host,
                )? {
                    self.load(&reference.graph_id, &reference.revision)?
                } else {
                    None
                } {
                    None => partial(
                        &mut result,
                        "E_DEPENDENCY_UNAVAILABLE",
                        "metadata dependency unavailable",
                    ),
                    Some(data) => {
                        let was_empty = data.nodes.is_empty() && data.edges.is_empty();
                        let (data, incomplete_derivation) = self.authorized(data, host)?;
                        let (mut data, attachment_origins) =
                            materialize(data, &reference.graph_id, &reference.revision)?;
                        if identity_acceptance::reserved(&reference.graph_id) {
                            partial(
                                &mut result,
                                "E_IDENTITY_SCOPE",
                                "identity results cover only authorized and available membership records",
                            );
                        } else if incomplete_derivation {
                            partial(
                                &mut result,
                                "E_DERIVATION_UNAVAILABLE",
                                "some derivation dependencies unavailable",
                            );
                        }
                        if !was_empty && data.nodes.is_empty() && data.edges.is_empty() {
                            partial(
                                &mut result,
                                "E_DEPENDENCY_UNAVAILABLE",
                                "metadata dependency unavailable",
                            );
                            continue;
                        }
                        let (references, unavailable_live) =
                            self.query_refs(&data, &mut live_pins)?;
                        if unavailable_live {
                            partial(
                                &mut result,
                                "E_DEPENDENCY_UNAVAILABLE",
                                "metadata dependency unavailable",
                            );
                        }
                        metadata::pin_live_attachments(&mut data, &live_pins);
                        pending.extend(references.into_iter().map(|r| (r, depth + 1)));
                        query_bytes +=
                            json_size(&data, MATERIALIZED_LIMIT.saturating_sub(query_bytes))?;
                        if !result.input_snapshots.contains(&reference) {
                            result.input_snapshots.push(reference.clone());
                        }
                        result.metadata_graphs.push(ResolvedGraph {
                            attachment_origins,
                            reference,
                            graph: data,
                        });
                    }
                }
            }
        }
        json_size(&result, MATERIALIZED_LIMIT)?;
        if let Some(transaction) = read_transaction {
            transaction.commit()?;
        }
        Ok(result)
    }
    /// Identity-key path join. Pure over two pinned, authorized graph views.
    pub fn join(
        &self,
        left: &QueryPlan,
        right: &QueryPlan,
        predicate: &str,
        host: &HostContext,
    ) -> Result<QueryResult> {
        let _read_scope = self.read_budget.enter();
        if !valid_id(predicate) {
            return Err(err("E_ID", "join output predicate required"));
        }
        let transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let l = self.query(left, host)?;
        let r = self.query(right, host)?;
        let result = Self::join_values(l, r, predicate, host)?;
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        Ok(result)
    }
    fn join_values(
        l: QueryResult,
        r: QueryResult,
        predicate: &str,
        host: &HostContext,
    ) -> Result<QueryResult> {
        if !valid_id(predicate) {
            return Err(err("E_ID", "join output predicate required"));
        }
        let mut schema = typed::joined_schema(&l.graph, &r.graph, predicate)?;
        let pairs = l
            .graph
            .edges
            .len()
            .checked_mul(r.graph.edges.len())
            .ok_or_else(|| err("E_BUDGET", "join pair budget exceeded"))?;
        if pairs > 1_000_000 {
            return Err(err("E_BUDGET", "join pair budget exceeds 1000000"));
        }
        // Bound construction itself, not only the final result retained by execute.
        let mut join_bytes = 512usize;
        for size in [
            json_size(&l.metadata_graphs, MATERIALIZED_LIMIT)?,
            json_size(&r.metadata_graphs, MATERIALIZED_LIMIT)?,
            json_size(&l.input_snapshots, MATERIALIZED_LIMIT)?,
            json_size(&r.input_snapshots, MATERIALIZED_LIMIT)?,
        ] {
            join_bytes = join_bytes
                .checked_add(size)
                .ok_or_else(|| err("E_BUDGET", "join byte budget exceeded"))?;
            if join_bytes > MATERIALIZED_LIMIT {
                return Err(err("E_BUDGET", "join byte budget exceeded"));
            }
        }
        let selected_context =
            context::compatible_context(l.selected_context.as_ref(), r.selected_context.as_ref())
                .map_err(|d| err(&d.code, &d.message))?;
        let ln: BTreeMap<_, _> = l.graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let rn: BTreeMap<_, _> = r.graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let mut nodes = BTreeMap::new();
        let mut edges = Vec::new();
        let mut provenance = Vec::new();
        let mut edge_origins = BTreeMap::new();
        let mut node_origins = BTreeMap::new();
        let mut attachments = BTreeMap::new();
        let mut attachment_origins = BTreeMap::new();
        for le in &l.graph.edges {
            for re in &r.graph.edges {
                let a = ln[le.to.as_str()];
                let b = rn[re.from.as_str()];
                if a.entity_id != b.entity_id || a.space_id != b.space_id {
                    continue;
                }
                // Negative support is not a positive path premise.
                if le.polarity != Polarity::Positive || re.polarity != Polarity::Positive {
                    continue;
                }
                let start = le.valid_time.start.max(re.valid_time.start);
                let end = match (le.valid_time.end, re.valid_time.end) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, None) => a,
                    (None, b) => b,
                };
                if end.is_some_and(|end| start >= end) {
                    continue;
                }
                context::ensure_consumable(
                    l.selected_context.as_ref(),
                    le.assertion_context.as_ref(),
                )
                .map_err(|d| err(&d.code, &d.message))?;
                context::ensure_consumable(
                    r.selected_context.as_ref(),
                    re.assertion_context.as_ref(),
                )
                .map_err(|d| err(&d.code, &d.message))?;
                for node in [
                    ln[le.from.as_str()],
                    ln[le.to.as_str()],
                    rn[re.from.as_str()],
                    rn[re.to.as_str()],
                ] {
                    if let Some(scope) = &node.context_scope {
                        context::compatible_context(selected_context.as_ref(), Some(scope))
                            .map_err(|d| err(&d.code, &d.message))?;
                    }
                }
                let mut premises = l
                    .edge_origins
                    .get(&le.id)
                    .cloned()
                    .ok_or_else(|| err("E_PROVENANCE", "missing left edge origin"))?;
                for reference in r
                    .edge_origins
                    .get(&re.id)
                    .ok_or_else(|| err("E_PROVENANCE", "missing right edge origin"))?
                {
                    if !premises.contains(reference) {
                        premises.push(reference.clone());
                    }
                }
                let derivations = algebra::combine_derivations(
                    le,
                    l.edge_origins.get(&le.id).expect("checked origin"),
                    re,
                    r.edge_origins.get(&re.id).expect("checked origin"),
                    "weave:join",
                    BTreeMap::from([(
                        "predicate".into(),
                        serde_json::Value::String(predicate.into()),
                    )]),
                    &l.input_snapshots
                        .iter()
                        .chain(&r.input_snapshots)
                        .cloned()
                        .collect::<Vec<_>>(),
                    &algebra_context(host),
                )
                .map_err(|d| err(&d.code, &d.message))?;
                premises.clear();
                for reference in derivations.iter().flat_map(|d| &d.premises) {
                    if !premises.contains(reference) {
                        premises.push(reference.clone());
                    }
                }
                let mut source = ln[le.from.as_str()].clone();
                let mut target = rn[re.to.as_str()].clone();
                source.id = format!(
                    "join-node:{:x}",
                    Sha256::digest(serde_json::to_vec(&(
                        &l.input_snapshots,
                        source.id.as_str()
                    ))?)
                );
                target.id = format!(
                    "join-node:{:x}",
                    Sha256::digest(serde_json::to_vec(&(
                        &r.input_snapshots,
                        target.id.as_str()
                    ))?)
                );
                for (id, input, old_id) in [(&source.id, &l, &le.from), (&target.id, &r, &re.to)] {
                    if !node_origins.contains_key(id) {
                        let origins = input
                            .node_origins
                            .get(old_id)
                            .cloned()
                            .ok_or_else(|| err("E_PROVENANCE", "missing node origin"))?;
                        join_bytes += json_size(
                            &(id, &origins),
                            MATERIALIZED_LIMIT.saturating_sub(join_bytes),
                        )?;
                        node_origins.insert(id.clone(), origins);
                    }
                }
                // Results are scoped to the evaluating principal until a release policy exists.
                source.readers = vec![host.principal.clone()];
                target.readers = vec![host.principal.clone()];
                let type_id = if let Some(output) = &mut schema {
                    typed::type_join_node(
                        &mut source,
                        l.graph.schema.as_ref().expect("both schemas validated"),
                        output,
                        &mut join_bytes,
                    )?;
                    typed::type_join_node(
                        &mut target,
                        r.graph.schema.as_ref().expect("both schemas validated"),
                        output,
                        &mut join_bytes,
                    )?;
                    Some(typed::type_join_edge(
                        &source,
                        &target,
                        output,
                        &mut join_bytes,
                    )?)
                } else {
                    None
                };
                let edge = Edge {
                    assertion_source: None,
                    assertion_context: selected_context
                        .as_ref()
                        .and_then(|c| c.reference().cloned()),
                    structural_ref: None,
                    assertion_properties: BTreeMap::new(),
                    type_id,
                    id: format!(
                        "join-edge:{:x}",
                        Sha256::digest(serde_json::to_vec(&(
                            "weave-join-v0.2",
                            predicate,
                            &premises,
                            start,
                            end
                        ))?)
                    ),
                    predicate: predicate.into(),
                    from: source.id.clone(),
                    to: target.id.clone(),
                    valid_time: Interval { start, end },
                    polarity: Polarity::Positive,
                    properties: BTreeMap::new(),
                    metadata: vec![],
                    readers: vec![host.principal.clone()],
                    derived_from: premises.clone(),
                    derivations,
                };
                if !nodes.contains_key(&source.id) {
                    join_bytes +=
                        json_size(&source, MATERIALIZED_LIMIT.saturating_sub(join_bytes))?;
                }
                if !nodes.contains_key(&target.id) && target.id != source.id {
                    join_bytes +=
                        json_size(&target, MATERIALIZED_LIMIT.saturating_sub(join_bytes))?;
                }
                join_bytes += json_size(&edge, MATERIALIZED_LIMIT.saturating_sub(join_bytes))?;
                join_bytes += json_size(
                    &(&edge.id, &premises),
                    MATERIALIZED_LIMIT.saturating_sub(join_bytes),
                )?;
                metadata::carry_node_attachments(
                    &l,
                    ln[le.from.as_str()],
                    &source,
                    host,
                    &mut attachments,
                    &mut attachment_origins,
                    &mut join_bytes,
                )?;
                metadata::carry_node_attachments(
                    &r,
                    rn[re.to.as_str()],
                    &target,
                    host,
                    &mut attachments,
                    &mut attachment_origins,
                    &mut join_bytes,
                )?;
                nodes.insert(source.id.clone(), source);
                nodes.insert(target.id.clone(), target);
                edge_origins.insert(edge.id.clone(), premises.clone());
                edges.push(edge);
                if edges.len() > 100_000 {
                    return Err(err("E_BUDGET", "join output edge budget exceeded"));
                }
                for reference in premises {
                    if !provenance.contains(&reference) {
                        join_bytes +=
                            json_size(&reference, MATERIALIZED_LIMIT.saturating_sub(join_bytes))?;
                        provenance.push(reference);
                    }
                }
            }
        }
        let mut input_snapshots = l.input_snapshots.clone();
        for reference in r.input_snapshots {
            if !input_snapshots.contains(&reference) {
                input_snapshots.push(reference);
            }
        }
        let mut metadata_graphs = l.metadata_graphs;
        for resolved in r.metadata_graphs {
            if !metadata_graphs
                .iter()
                .any(|x| x.reference == resolved.reference)
            {
                metadata_graphs.push(resolved);
            }
        }
        let mut diagnostics = l.diagnostics;
        for diagnostic in r.diagnostics {
            if !diagnostics.contains(&diagnostic) {
                diagnostics.push(diagnostic);
            }
        }
        let coverage = if l.coverage == Coverage::Partial || r.coverage == Coverage::Partial {
            Coverage::Partial
        } else {
            Coverage::Complete
        };
        // Legacy map only retains unambiguous graph IDs. The full vector is authoritative.
        let mut snapshots = BTreeMap::new();
        for reference in &input_snapshots {
            if !input_snapshots
                .iter()
                .any(|x| x.graph_id == reference.graph_id && x.revision != reference.revision)
            {
                snapshots.insert(reference.graph_id.clone(), reference.revision.clone());
            }
        }
        if let Some(schema) = &mut schema {
            schema.id = format!(
                "derived-schema:{:x}",
                Sha256::digest(serde_json::to_vec(&(
                    &schema.id,
                    &schema.nodes,
                    &schema.edges
                ))?)
            );
        }
        let mut result = QueryResult {
            selected_context,
            source_revisions: algebra::merge_source_revisions(
                &l.source_revisions,
                &r.source_revisions,
            )
            .map_err(|d| err(&d.code, &d.message))?,
            version: VERSION.into(),
            graph: GraphData {
                profile: GraphProfile::Legacy,
                structural_edges: vec![],
                assertions: vec![],
                schema,
                attachments: attachments.into_values().collect(),
                nodes: nodes.into_values().collect(),
                edges,
            },
            snapshots,
            input_snapshots,
            coverage,
            diagnostics,
            provenance,
            edge_origins,
            node_origins,
            attachment_origins,
            metadata_graphs,
        };
        merge_sources(&mut result.source_revisions, &[])?;
        validate_graph(&result.graph)?;
        json_size(&result, MATERIALIZED_LIMIT)?;
        Ok(result)
    }
    fn expression(
        &self,
        expression: &GraphExpression,
        values: &BTreeMap<String, QueryResult>,
        host: &HostContext,
        depth: u32,
        budget: &mut usize,
    ) -> Result<QueryResult> {
        if depth > 32 || *budget == 0 {
            return Err(err("E_BUDGET", "graph expression budget exceeded"));
        }
        *budget -= 1;
        match expression {
            GraphExpression::ResolveIdentity { selection } => {
                self.resolve_identity(selection, host)
            }
            GraphExpression::Cluster { selection } => self.cluster_navigation(selection, host),
            GraphExpression::Counterparts { input, selection } => {
                let input = self.expression(input, values, host, depth + 1, budget)?;
                counterpart::select(input, selection, &algebra_context(host))
                    .map_err(|d| err(&d.code, &d.message))
            }
            GraphExpression::Geometry {
                operation,
                valid_at,
            } => self.geometry(operation, *valid_at, values, host, depth + 1, budget),
            GraphExpression::Explain { input } => {
                let input = self.expression(input, values, host, depth + 1, budget)?;
                identity::explain(&input, &algebra_context(host))
                    .map_err(|d| err(&d.code, &d.message))
            }
            GraphExpression::Context { input, selection } => {
                let input = self.expression(input, values, host, depth + 1, budget)?;
                context::select(input, selection, &algebra_context(host))
                    .map_err(|d| err(&d.code, &d.message))
            }
            GraphExpression::Reason { input, rules: set } => {
                let input = self.expression(input, values, host, depth + 1, budget)?;
                rules::reason(
                    input,
                    set,
                    &RuleBudget {
                        max_steps: 100_000,
                        max_rounds: 128,
                        max_facts: 10_000,
                        max_derivations: 128,
                    },
                    &algebra_context(host),
                )
                .map_err(|d| err(&d.code, &d.message))
            }
            GraphExpression::Union { left, right } => {
                let l = self.expression(left, values, host, depth + 1, budget)?;
                let r = self.expression(right, values, host, depth + 1, budget)?;
                algebra::union(l, r, &algebra_context(host)).map_err(|d| err(&d.code, &d.message))
            }
            GraphExpression::Diff { before, after } => {
                let l = self.expression(before, values, host, depth + 1, budget)?;
                let r = self.expression(after, values, host, depth + 1, budget)?;
                algebra::diff(l, r, &algebra_context(host)).map_err(|d| err(&d.code, &d.message))
            }
            GraphExpression::Project {
                input,
                node_ids,
                edge_ids,
            } => {
                let input = self.expression(input, values, host, depth + 1, budget)?;
                algebra::project(input, node_ids, edge_ids, &algebra_context(host))
                    .map_err(|d| err(&d.code, &d.message))
            }
            GraphExpression::Support {
                input,
                predicate,
                from,
                to,
                valid_at,
            } => {
                let input = self.expression(input, values, host, depth + 1, budget)?;
                algebra::support(
                    input,
                    predicate,
                    from,
                    to,
                    *valid_at,
                    &algebra_context(host),
                )
                .map_err(|d| err(&d.code, &d.message))
            }
            GraphExpression::Metadata {
                input,
                host: attachment_host,
                key,
            } => {
                let input = self.expression(input, values, host, depth + 1, budget)?;
                self.metadata_value(input, attachment_host, key, host)
            }
            GraphExpression::Query { query } => self.query(query, host),
            GraphExpression::Reference { name } => values
                .get(name)
                .cloned()
                .ok_or_else(|| err("E_BINDING", "graph value is not bound")),
            GraphExpression::Join {
                left,
                right,
                output_predicate,
                match_on: JoinMatch::EntitySpaceToFrom,
            } => {
                let l = self.expression(left, values, host, depth + 1, budget)?;
                let r = self.expression(right, values, host, depth + 1, budget)?;
                Self::join_values(l, r, output_predicate, host)
            }
            GraphExpression::Filter {
                input,
                predicate,
                valid_at,
            } => {
                let mut value = self.expression(input, values, host, depth + 1, budget)?;
                value.graph.edges.retain(|e| {
                    predicate.as_ref().is_none_or(|p| p == &e.predicate)
                        && valid_at.is_none_or(|t| e.valid_time.contains(t))
                });
                if predicate.is_some() || valid_at.is_some() {
                    let nodes: HashSet<_> = value
                        .graph
                        .edges
                        .iter()
                        .flat_map(|e| [e.from.clone(), e.to.clone()])
                        .collect();
                    value.graph.nodes.retain(|n| nodes.contains(&n.id));
                }
                value
                    .node_origins
                    .retain(|id, _| value.graph.nodes.iter().any(|n| &n.id == id));
                prune_attachments(&mut value.graph);
                value
                    .attachment_origins
                    .retain(|id, _| value.graph.attachments.iter().any(|a| &a.id == id));
                let edges: HashSet<_> = value.graph.edges.iter().map(|e| e.id.as_str()).collect();
                value
                    .edge_origins
                    .retain(|id, _| edges.contains(id.as_str()));
                value.provenance = Vec::new();
                for reference in value.edge_origins.values().flatten() {
                    if !value.provenance.contains(reference) {
                        value.provenance.push(reference.clone());
                    }
                }
                Ok(value)
            }
        }
    }
    fn authorized_nodes(&self, data: GraphData, host: &HostContext) -> Result<(GraphData, bool)> {
        let mut data = visible(data, &host.principal);
        let mut incomplete = false;
        let mut nodes = Vec::new();
        for node in std::mem::take(&mut data.nodes) {
            let mut visiting = HashSet::new();
            let mut budget = 1000;
            if self.premises_visible(&node.derived_from, host, &mut visiting, &mut budget, 0)?
                && self.node_refs_visible(
                    &node.derived_nodes,
                    host,
                    &mut visiting,
                    &mut budget,
                    0,
                )?
            {
                nodes.push(node);
            } else {
                incomplete = true;
            }
        }
        data.nodes = nodes;
        Ok((visible(data, &host.principal), incomplete))
    }
    fn authorized(&self, data: GraphData, host: &HostContext) -> Result<(GraphData, bool)> {
        let (mut data, mut incomplete) = self.authorized_nodes(data, host)?;
        let mut edges = Vec::new();
        for mut edge in data.edges {
            if self.authorize_edge_groups(&mut edge, host, &mut incomplete)? {
                edges.push(edge);
            }
        }
        let mut claims = Vec::new();
        for mut assertion in data.assertions {
            let structure = data
                .structural_edges
                .iter()
                .find(|e| e.id == assertion.edge_id)
                .ok_or_else(|| err("E_ASSERTION", "structural edge unavailable"))?;
            let mut edge = assertion_edge(&assertion, structure, None);
            if self.authorize_edge_groups(&mut edge, host, &mut incomplete)? {
                assertion.derived_from = edge.derived_from;
                assertion.derivations = edge.derivations;
                claims.push(assertion);
            }
        }
        data.assertions = claims;
        data.edges = edges;
        prune_attachments(&mut data);
        let mut attachments = Vec::new();
        for attachment in data.attachments {
            let allowed = match &attachment.origin {
                Some(reference) => self.premises_visible(
                    std::slice::from_ref(reference),
                    host,
                    &mut HashSet::new(),
                    &mut 1000,
                    0,
                )?,
                None => true,
            };
            if allowed {
                attachments.push(attachment);
            } else {
                incomplete = true;
            }
        }
        data.attachments = attachments;
        Ok((data, incomplete))
    }
    fn authorize_edge_groups(
        &self,
        edge: &mut Edge,
        host: &HostContext,
        incomplete: &mut bool,
    ) -> Result<bool> {
        let mut budget = 1000;
        if edge.derivations.is_empty() {
            let allowed = self.premises_visible(
                &edge.derived_from,
                host,
                &mut HashSet::new(),
                &mut budget,
                0,
            )?;
            *incomplete |= !allowed;
            return Ok(allowed);
        }
        let mut groups = Vec::new();
        for mut group in std::mem::take(&mut edge.derivations) {
            if self.premises_visible(&group.premises, host, &mut HashSet::new(), &mut budget, 0)? {
                group.input_snapshots.retain(|r| {
                    group
                        .premises
                        .iter()
                        .any(|p| p.graph_id == r.graph_id && p.revision == r.revision)
                });
                groups.push(group);
            } else {
                *incomplete = true;
            }
        }
        edge.derived_from = Vec::new();
        for p in groups.iter().flat_map(|g| &g.premises) {
            if !edge.derived_from.contains(p) {
                edge.derived_from.push(p.clone());
            }
        }
        edge.derivations = groups;
        Ok(!edge.derivations.is_empty())
    }
    fn node_refs_visible(
        &self,
        references: &[NodeRef],
        host: &HostContext,
        visiting: &mut HashSet<(u8, String, String, String)>,
        budget: &mut usize,
        depth: u32,
    ) -> Result<bool> {
        if depth > 32 {
            return Ok(false);
        }
        for reference in references {
            if *budget == 0 {
                return Ok(false);
            }
            *budget -= 1;
            let key = (
                1,
                reference.graph_id.clone(),
                reference.revision.clone(),
                reference.node_id.clone(),
            );
            if !visiting.insert(key.clone()) {
                return Ok(false);
            }
            if !self.identity_reference_allowed(&reference.graph_id, &reference.revision, host)? {
                return Ok(false);
            }
            let Some(source) = self.load(&reference.graph_id, &reference.revision)? else {
                return Ok(false);
            };
            let source = visible(source, &host.principal);
            let Some(node) = source.nodes.iter().find(|n| n.id == reference.node_id) else {
                return Ok(false);
            };
            let permitted =
                self.premises_visible(&node.derived_from, host, visiting, budget, depth + 1)?
                    && self.node_refs_visible(
                        &node.derived_nodes,
                        host,
                        visiting,
                        budget,
                        depth + 1,
                    )?;
            visiting.remove(&key);
            if !permitted {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn premises_visible(
        &self,
        references: &[AssertionRef],
        host: &HostContext,
        visiting: &mut HashSet<(u8, String, String, String)>,
        budget: &mut usize,
        depth: u32,
    ) -> Result<bool> {
        if depth > 32 {
            return Ok(false);
        }
        for reference in references {
            if *budget == 0 {
                return Ok(false);
            }
            *budget -= 1;
            let key = (
                0,
                reference.graph_id.clone(),
                reference.revision.clone(),
                reference.assertion_id.clone(),
            );
            if !visiting.insert(key.clone()) {
                return Ok(false);
            }
            if !self.identity_reference_allowed(&reference.graph_id, &reference.revision, host)? {
                return Ok(false);
            }
            let Some(source) = self.load(&reference.graph_id, &reference.revision)? else {
                return Ok(false);
            };
            let source = visible(source, &host.principal);
            let mut endpoint_ids = Vec::new();
            if let Some(e) = source.edges.iter().find(|e| e.id == reference.assertion_id) {
                endpoint_ids.extend([e.from.as_str(), e.to.as_str()]);
            } else if let Some(a) = source
                .assertions
                .iter()
                .find(|a| a.id == reference.assertion_id)
            {
                if let Some(e) = source.structural_edges.iter().find(|e| e.id == a.edge_id) {
                    endpoint_ids.extend([e.from.as_str(), e.to.as_str()]);
                }
            } else if let Some(a) = source
                .attachments
                .iter()
                .find(|a| a.id == reference.assertion_id)
            {
                match &a.host {
                    MetadataHost::Node { id } => endpoint_ids.push(id.as_str()),
                    MetadataHost::Entity { id } => endpoint_ids.extend(
                        source
                            .nodes
                            .iter()
                            .filter(|n| &n.entity_id == id)
                            .map(|n| n.id.as_str()),
                    ),
                    MetadataHost::Edge { id } => {
                        if let Some(e) = source.edges.iter().find(|e| &e.id == id) {
                            endpoint_ids.extend([e.from.as_str(), e.to.as_str()]);
                        }
                        if let Some(e) = source.structural_edges.iter().find(|e| &e.id == id) {
                            endpoint_ids.extend([e.from.as_str(), e.to.as_str()]);
                        }
                    }
                    MetadataHost::Assertion { id } => {
                        if let Some(a) = source.assertions.iter().find(|a| &a.id == id) {
                            if let Some(e) =
                                source.structural_edges.iter().find(|e| e.id == a.edge_id)
                            {
                                endpoint_ids.extend([e.from.as_str(), e.to.as_str()]);
                            }
                        }
                    }
                    MetadataHost::Graph => {}
                }
            }
            for node in source
                .nodes
                .iter()
                .filter(|n| endpoint_ids.contains(&n.id.as_str()))
            {
                if !self.premises_visible(&node.derived_from, host, visiting, budget, depth + 1)?
                    || !self.node_refs_visible(
                        &node.derived_nodes,
                        host,
                        visiting,
                        budget,
                        depth + 1,
                    )?
                {
                    return Ok(false);
                }
            }
            let permitted = if let Some(premise) =
                source.edges.iter().find(|e| e.id == reference.assertion_id)
            {
                self.edge_dependencies_visible(premise, host, visiting, budget, depth + 1)?
            } else if let Some(assertion) = source
                .assertions
                .iter()
                .find(|a| a.id == reference.assertion_id)
            {
                let structure = source
                    .structural_edges
                    .iter()
                    .find(|e| e.id == assertion.edge_id)
                    .ok_or_else(|| err("E_ASSERTION", "missing structural edge"))?;
                self.edge_dependencies_visible(
                    &assertion_edge(assertion, structure, None),
                    host,
                    visiting,
                    budget,
                    depth + 1,
                )?
            } else if let Some(attachment) = source
                .attachments
                .iter()
                .find(|a| a.id == reference.assertion_id)
            {
                let own = self.premises_visible(
                    &attachment.origin.iter().cloned().collect::<Vec<_>>(),
                    host,
                    visiting,
                    budget,
                    depth + 1,
                )?;
                let host_visible = if let MetadataHost::Assertion { id } = &attachment.host {
                    if let Some(assertion) = source.assertions.iter().find(|a| &a.id == id) {
                        let structure = source
                            .structural_edges
                            .iter()
                            .find(|e| e.id == assertion.edge_id)
                            .ok_or_else(|| err("E_ASSERTION", "missing structural edge"))?;
                        self.edge_dependencies_visible(
                            &assertion_edge(assertion, structure, None),
                            host,
                            visiting,
                            budget,
                            depth + 1,
                        )?
                    } else {
                        false
                    }
                } else if let MetadataHost::Edge { id } = &attachment.host {
                    match source.edges.iter().find(|e| &e.id == id) {
                        Some(edge) => {
                            self.edge_dependencies_visible(edge, host, visiting, budget, depth + 1)?
                        }
                        None => source.structural_edges.iter().any(|e| &e.id == id),
                    }
                } else {
                    true
                };
                own && host_visible
            } else {
                false
            };
            if !permitted {
                return Ok(false);
            }
            visiting.remove(&key);
        }
        Ok(true)
    }
    fn edge_dependencies_visible(
        &self,
        edge: &Edge,
        host: &HostContext,
        visiting: &mut HashSet<(u8, String, String, String)>,
        budget: &mut usize,
        depth: u32,
    ) -> Result<bool> {
        if edge.derivations.is_empty() {
            return self.premises_visible(&edge.derived_from, host, visiting, budget, depth);
        }
        for group in &edge.derivations {
            if self.premises_visible(&group.premises, host, &mut visiting.clone(), budget, depth)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// Trusted host administration only; this is deliberately not a plan operation.
    pub fn register_adapter(&self, id: &str, graph: &str) -> Result<()> {
        if id.is_empty() || graph.is_empty() {
            return Err(err("E_ID", "adapter identifiers must not be empty"));
        }
        self.conn.execute(
            "INSERT INTO adapters(id,graph_id) VALUES (?1,?2)",
            params![id, graph],
        )?;
        Ok(())
    }
    pub fn pause_adapter(&self, id: &str, paused: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE adapters SET paused=?2 WHERE id=?1",
            params![id, paused],
        )?;
        Ok(())
    }
    /// Durable reference adapter records one logical audit effect per event. No external action.
    pub fn deliver(
        &mut self,
        adapter: &str,
        event_id: &str,
        simulate_failure: bool,
    ) -> Result<String> {
        let tx = self.conn.transaction()?;
        let (graph, paused): (String, bool) = tx.query_row(
            "SELECT graph_id,paused FROM adapters WHERE id=?1",
            [adapter],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if paused {
            return Err(err("E_PAUSED", "adapter paused"));
        }
        let event_graph: String = tx.query_row(
            "SELECT graph_id FROM events WHERE event_id=?1",
            [event_id],
            |r| r.get(0),
        )?;
        if graph != event_graph {
            return Err(err("E_FORBIDDEN", "event outside adapter graph scope"));
        }
        let existing: Option<(u32, String)> = tx
            .query_row(
                "SELECT attempts,status FROM deliveries WHERE adapter=?1 AND event_id=?2",
                params![adapter, event_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((_, status)) = &existing {
            if status == "delivered" || status == "dead_letter" {
                return Ok(status.clone());
            }
        }
        let attempts = existing.map_or(1, |(n, _)| n + 1);
        let status = if simulate_failure {
            if attempts >= 3 {
                "dead_letter"
            } else {
                "retry"
            }
        } else {
            "delivered"
        };
        tx.execute("INSERT INTO deliveries VALUES (?1,?2,?3,?4) ON CONFLICT(adapter,event_id) DO UPDATE SET attempts=excluded.attempts,status=excluded.status",params![adapter,event_id,attempts,status])?;
        if !simulate_failure {
            tx.execute(
                "INSERT OR IGNORE INTO effects VALUES (?1,?2,?3)",
                params![adapter, event_id, "audit recorded"],
            )?;
        }
        tx.commit()?;
        Ok(status.into())
    }
    /// Explicit admin replay only resets dead letters; completed effects stay deduplicated.
    pub fn replay_dead_letter(&self, adapter: &str, event_id: &str) -> Result<()> {
        self.conn.execute("UPDATE deliveries SET attempts=0,status='retry' WHERE adapter=?1 AND event_id=?2 AND status='dead_letter'",params![adapter,event_id])?;
        Ok(())
    }
    pub fn event_count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))?
            as u64)
    }
    pub fn effect_count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM effects", [], |r| r.get::<_, i64>(0))?
            as u64)
    }
    /// Trusted host event log, never an unprivileged query endpoint.
    pub fn events(&self) -> Result<Vec<Event>> {
        let mut stmt=self.conn.prepare("SELECT event_id,graph_id,branch_id,revision,sequence,actor FROM events ORDER BY sequence")?;
        let rows = stmt.query_map([], |r| {
            Ok(Event {
                version: VERSION.into(),
                event_type: {
                    let id: String = r.get(0)?;
                    if id.starts_with("accept:") {
                        "graph.accepted".into()
                    } else {
                        "graph.committed".into()
                    }
                },
                event_id: r.get(0)?,
                graph_id: r.get(1)?,
                branch_id: r.get(2)?,
                revision: r.get(3)?,
                sequence: u64::try_from(r.get::<_, i64>(4)?)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(4, -1))?,
                actor: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}
fn visible(mut data: GraphData, principal: &str) -> GraphData {
    data.nodes.retain(|n| allowed(&n.readers, principal));
    let nodes: HashSet<_> = data.nodes.iter().map(|n| n.id.as_str()).collect();
    data.edges.retain(|e| {
        allowed(&e.readers, principal)
            && nodes.contains(e.from.as_str())
            && nodes.contains(e.to.as_str())
    });
    data.structural_edges.retain(|e| {
        allowed(&e.readers, principal)
            && nodes.contains(e.from.as_str())
            && nodes.contains(e.to.as_str())
    });
    let structures: HashSet<_> = data
        .structural_edges
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    data.assertions
        .retain(|a| allowed(&a.readers, principal) && structures.contains(a.edge_id.as_str()));
    data.attachments.retain(|a| allowed(&a.readers, principal));
    prune_attachments(&mut data);
    data
}
fn allowed(readers: &[String], principal: &str) -> bool {
    readers.is_empty() || readers.iter().any(|r| r == principal)
}
fn refs(data: &GraphData) -> Vec<GraphRef> {
    data.nodes
        .iter()
        .flat_map(|n| n.metadata.clone())
        .chain(data.nodes.iter().flat_map(|n| {
            n.derived_nodes.iter().map(|p| GraphRef {
                graph_id: p.graph_id.clone(),
                revision: p.revision.clone(),
            })
        }))
        .chain(data.nodes.iter().flat_map(|n| {
            n.derived_from.iter().map(|p| GraphRef {
                graph_id: p.graph_id.clone(),
                revision: p.revision.clone(),
            })
        }))
        .chain(data.nodes.iter().filter_map(|n| {
            n.context_scope
                .as_ref()
                .and_then(ContextSelection::reference)
                .cloned()
        }))
        .chain(data.edges.iter().flat_map(|e| e.metadata.clone()))
        .chain(
            data.edges
                .iter()
                .filter_map(|e| e.assertion_context.clone()),
        )
        .chain(
            data.structural_edges
                .iter()
                .flat_map(|e| e.metadata.clone()),
        )
        .chain(
            data.assertions
                .iter()
                .flat_map(|a| a.metadata.iter().chain(a.context.iter()).cloned()),
        )
        .chain(data.attachments.iter().filter_map(|a| a.context.clone()))
        .chain(data.attachments.iter().filter_map(|a| match &a.value {
            MetadataValue::Graph { reference } => Some(reference.clone()),
            _ => None,
        }))
        .collect()
}
fn prune_attachments(data: &mut GraphData) {
    let nodes: HashSet<_> = data.nodes.iter().map(|n| n.id.as_str()).collect();
    let entities: HashSet<_> = data.nodes.iter().map(|n| n.entity_id.as_str()).collect();
    let edges: HashSet<_> = data
        .edges
        .iter()
        .map(|e| e.id.as_str())
        .chain(data.structural_edges.iter().map(|e| e.id.as_str()))
        .collect();
    let assertions: HashSet<_> = data.assertions.iter().map(|a| a.id.as_str()).collect();
    data.attachments.retain(|a| match &a.host {
        MetadataHost::Graph => true,
        MetadataHost::Node { id } => nodes.contains(id.as_str()),
        MetadataHost::Edge { id } => edges.contains(id.as_str()),
        MetadataHost::Assertion { id } => assertions.contains(id.as_str()),
        MetadataHost::Entity { id } => entities.contains(id.as_str()),
    });
}

fn partial(result: &mut QueryResult, code: &str, message: &str) {
    result.coverage = Coverage::Partial;
    if !result.diagnostics.iter().any(|d| d.code == code) {
        result.diagnostics.push(Diagnostic {
            code: code.into(),
            message: message.into(),
        });
    }
}
fn validate_graph(data: &GraphData) -> Result<()> {
    for node in &data.nodes {
        if node
            .derived_from
            .len()
            .saturating_add(node.derived_nodes.len())
            > 1000
            || node
                .derived_nodes
                .iter()
                .any(|p| !valid_id(&p.graph_id) || !valid_id(&p.revision) || !valid_id(&p.node_id))
            || node.derived_from.iter().any(|p| {
                !valid_id(&p.graph_id) || !valid_id(&p.revision) || !valid_id(&p.assertion_id)
            })
        {
            return Err(err(
                "E_PROVENANCE",
                "node dependencies must be bounded pinned assertions",
            ));
        }
    }
    for reference in refs(data) {
        if !valid_id(&reference.graph_id) || !valid_id(&reference.revision) {
            return Err(err(
                "E_REFERENCE",
                "metadata/context reference must pin bounded graph and revision IDs",
            ));
        }
    }
    if data.profile == GraphProfile::Explicit {
        if let Some(d) = validate_schema_graph(data).first() {
            return Err(err(&d.code, &d.message));
        }
        return validate_explicit(data);
    }
    if !data.structural_edges.is_empty() || !data.assertions.is_empty() {
        return Err(err(
            "E_PROFILE",
            "explicit records require explicit graph profile",
        ));
    }
    if let Some(diagnostic) = validate_schema_graph(data).first() {
        return Err(err(&diagnostic.code, &diagnostic.message));
    }
    if data.nodes.len() > 100_000 || data.edges.len() > 100_000 {
        return Err(err("E_BUDGET", "graph exceeds 100000 nodes or edges"));
    }
    let mut ids = HashSet::new();
    for n in &data.nodes {
        if !valid_id(&n.id)
            || !valid_id(&n.entity_id)
            || !valid_id(&n.space_id)
            || !ids.insert(n.id.as_str())
        {
            return Err(err(
                "E_ID",
                "node IDs must be nonempty and unique; entity and space required",
            ));
        }
    }
    let mut edge_ids = HashSet::new();
    for e in &data.edges {
        if !valid_id(&e.id) || !valid_id(&e.predicate) || !edge_ids.insert(&e.id) {
            return Err(err(
                "E_ID",
                "edge IDs and predicates required; edge IDs unique",
            ));
        }
        if !ids.contains(e.from.as_str()) || !ids.contains(e.to.as_str()) {
            return Err(err("E_ENDPOINT", "edge endpoints must exist in graph"));
        }
        if !e.valid_time.valid() {
            return Err(err("E_INTERVAL", "valid time end must exceed start"));
        }
        if e.derivations.len() > 128
            || e.derivations
                .iter()
                .any(|d| !valid_id(&d.operator) || d.premises.is_empty() || d.premises.len() > 1000)
        {
            return Err(err(
                "E_DERIVATION",
                "derivation groups need an operator and bounded nonempty premises",
            ));
        }
        if !e.derivations.is_empty() {
            let grouped: HashSet<_> = e
                .derivations
                .iter()
                .flat_map(|g| {
                    g.premises
                        .iter()
                        .map(|p| (&p.graph_id, &p.revision, &p.assertion_id))
                })
                .collect();
            let flattened: HashSet<_> = e
                .derived_from
                .iter()
                .map(|p| (&p.graph_id, &p.revision, &p.assertion_id))
                .collect();
            if grouped != flattened {
                return Err(err(
                    "E_DERIVATION",
                    "flat compatibility index must equal derivation premise union",
                ));
            }
        }
    }
    for reader in data
        .nodes
        .iter()
        .flat_map(|n| &n.readers)
        .chain(data.edges.iter().flat_map(|e| &e.readers))
    {
        if !valid_id(reader) {
            return Err(err("E_ID", "reader identifiers require 1 to 512 bytes"));
        }
    }
    for reference in data.edges.iter().flat_map(|e| &e.derived_from) {
        if !valid_id(&reference.graph_id)
            || !valid_id(&reference.revision)
            || !valid_id(&reference.assertion_id)
        {
            return Err(err("E_ID", "provenance identifiers require 1 to 512 bytes"));
        }
    }
    let mut attachment_ids = HashSet::new();
    for a in &data.attachments {
        if !valid_id(&a.id)
            || !valid_id(&a.key)
            || !a.valid_time.valid()
            || !attachment_ids.insert(&a.id)
            || edge_ids.contains(&a.id)
        {
            return Err(err(
                "E_ATTACHMENT",
                "attachment identity, key and interval required",
            ));
        }
        let host = match &a.host {
            MetadataHost::Graph => true,
            MetadataHost::Node { id } => data.nodes.iter().any(|n| &n.id == id),
            MetadataHost::Edge { id } => data.edges.iter().any(|e| &e.id == id),
            MetadataHost::Assertion { .. } => false,
            MetadataHost::Entity { id } => data.nodes.iter().any(|n| &n.entity_id == id),
        };
        if !host || a.readers.iter().any(|r| !valid_id(r)) {
            return Err(err("E_ATTACHMENT", "attachment host or readers invalid"));
        }
        if let MetadataValue::LiveGraph {
            graph_id,
            branch_id,
        } = &a.value
        {
            if !valid_id(graph_id) || !valid_id(branch_id) {
                return Err(err("E_REFERENCE", "live graph reference invalid"));
            }
        }
    }
    for reference in refs(data) {
        if !valid_id(&reference.graph_id) || !valid_id(&reference.revision) {
            return Err(err(
                "E_REFERENCE",
                "metadata reference must pin graph and revision",
            ));
        }
    }
    Ok(())
}

pub const MATERIALIZED_LIMIT: usize = 32 * 1024 * 1024;
/// Count serialized bytes without allocating a second serialization buffer.
fn json_size(value: &impl serde::Serialize, limit: usize) -> Result<usize> {
    struct Counter {
        size: usize,
        limit: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.size = self
                .size
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("size overflow"))?;
            if self.size > self.limit {
                return Err(std::io::Error::other("materialization limit"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { size: 0, limit };
    serde_json::to_writer(&mut counter, value).map_err(|_| {
        err(
            "E_BUDGET",
            "serialized materialization byte budget exceeded",
        )
    })?;
    Ok(counter.size)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 512
}

fn validate_expression_profile(expression: &GraphExpression, version: &str) -> Result<()> {
    let mut pending = vec![(expression, 0usize)];
    let mut count = 0;
    while let Some((expression, depth)) = pending.pop() {
        count += 1;
        if count > 1000 || depth > 32 {
            return Err(err("E_BUDGET", "expression structure exceeds budget"));
        }
        match expression {
            GraphExpression::ResolveIdentity { .. } | GraphExpression::Cluster { .. } => {
                if version != VERSION {
                    return Err(err(
                        "E_VERSION",
                        "native pinned services require contract 0.13.0",
                    ));
                }
            }
            GraphExpression::Counterparts { input, .. } => {
                if ![VERSION, "0.12.0", "0.11.0", "0.10.0"].contains(&version) {
                    return Err(err(
                        "E_VERSION",
                        "counterpart selection requires contract 0.10.0",
                    ));
                }
                pending.push((input, depth + 1));
            }
            GraphExpression::Geometry { operation, .. } => {
                if ![VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0"].contains(&version) {
                    return Err(err("E_VERSION", "geometry requires contract 0.9.0"));
                }
                pending.extend(
                    operation
                        .inputs()
                        .into_iter()
                        .map(|input| (input, depth + 1)),
                );
            }
            GraphExpression::Explain { input } => {
                if ![VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0"].contains(&version) {
                    return Err(err(
                        "E_VERSION",
                        "explain expression requires contract 0.9.0",
                    ));
                }
                pending.push((input, depth + 1));
            }
            GraphExpression::Context { input, .. } => {
                if ![VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0"].contains(&version) {
                    return Err(err(
                        "E_VERSION",
                        "context selection requires contract 0.8.0",
                    ));
                }
                pending.push((input, depth + 1));
            }
            GraphExpression::Reason { input, .. } => {
                if ![
                    VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0", "0.7.0",
                ]
                .contains(&version)
                {
                    return Err(err("E_VERSION", "finite rules require contract 0.7.0"));
                }
                pending.push((input, depth + 1));
            }
            GraphExpression::Union { left, right }
            | GraphExpression::Diff {
                before: left,
                after: right,
            } => {
                if ![
                    VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0", "0.7.0", "0.6.0",
                    "0.5.0",
                ]
                .contains(&version)
                {
                    return Err(err("E_VERSION", "graph algebra requires contract 0.5.0"));
                }
                pending.push((left, depth + 1));
                pending.push((right, depth + 1));
            }
            GraphExpression::Project { input, .. } | GraphExpression::Support { input, .. } => {
                if ![
                    VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0", "0.7.0", "0.6.0",
                    "0.5.0",
                ]
                .contains(&version)
                {
                    return Err(err("E_VERSION", "graph algebra requires contract 0.5.0"));
                }
                pending.push((input, depth + 1));
            }
            GraphExpression::Metadata { input, host, .. } => {
                if matches!(host, MetadataHost::Assertion { .. })
                    && ![
                        VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0", "0.7.0", "0.6.0",
                    ]
                    .contains(&version)
                {
                    return Err(err(
                        "E_VERSION",
                        "assertion metadata hosts require contract 0.6.0",
                    ));
                }
                if ![
                    VERSION, "0.12.0", "0.11.0", "0.10.0", "0.9.0", "0.8.0", "0.7.0", "0.6.0",
                    "0.5.0", "0.4.0",
                ]
                .contains(&version)
                {
                    return Err(err(
                        "E_VERSION",
                        "metadata expressions require contract 0.4.0",
                    ));
                }
                pending.push((input, depth + 1));
            }
            GraphExpression::Filter { input, .. } => pending.push((input, depth + 1)),
            GraphExpression::Join { left, right, .. } => {
                pending.push((left, depth + 1));
                pending.push((right, depth + 1));
            }
            _ => {}
        }
    }
    Ok(())
}

fn algebra_context(host: &HostContext) -> AlgebraContext {
    AlgebraContext {
        principal: host.principal.clone(),
        max_objects: 100_000,
        max_output_bytes: MATERIALIZED_LIMIT,
    }
}

fn requires_explicit_profile(data: &GraphData) -> bool {
    data.profile == GraphProfile::Explicit
        || !data.structural_edges.is_empty()
        || !data.assertions.is_empty()
        || data.edges.iter().any(|e| {
            e.structural_ref.is_some()
                || e.assertion_source.is_some()
                || e.assertion_context.is_some()
                || !e.assertion_properties.is_empty()
        })
        || data
            .attachments
            .iter()
            .any(|a| matches!(a.host, MetadataHost::Assertion { .. }))
}

fn merge_sources(target: &mut Vec<SourceRevision>, sources: &[SourceRevision]) -> Result<()> {
    let mut labels = BTreeMap::new();
    for source in target.iter().chain(sources) {
        if labels
            .insert((&source.name, &source.revision), &source.digest)
            .is_some_and(|prior| prior != &source.digest)
        {
            return Err(err(
                "E_SOURCE_REVISION",
                "source revision label has conflicting content identity",
            ));
        }
    }
    for source in sources {
        if target.iter().any(|prior| {
            prior.name == source.name
                && prior.revision == source.revision
                && prior.digest != source.digest
        }) {
            return Err(err(
                "E_SOURCE_REVISION",
                "source revision label has conflicting content identity",
            ));
        }
        if !target.contains(source) {
            target.push(source.clone());
        }
    }
    Ok(())
}

fn has_exact_schema(data: &GraphData) -> bool {
    data.schema.as_ref().is_some_and(|schema| {
        schema
            .nodes
            .values()
            .flat_map(|n| n.properties.values())
            .chain(schema.edges.values().flat_map(|e| e.properties.values()))
            .any(|p| matches!(p.value_type, ScalarType::Decimal | ScalarType::Quantity(_)))
    })
}
fn has_float_schema(data: &GraphData) -> bool {
    data.schema.as_ref().is_some_and(|s| {
        s.nodes
            .values()
            .flat_map(|n| n.properties.values())
            .chain(s.edges.values().flat_map(|e| e.properties.values()))
            .any(|p| p.value_type == ScalarType::Float)
    })
}
