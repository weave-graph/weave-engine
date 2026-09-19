//! Native reference runtime. The portable wire/model crate is `weave-contract`.
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use weave_contract::*;

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
}
impl Engine {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }
    pub fn memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }
    fn from_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
 CREATE TABLE IF NOT EXISTS revisions(revision TEXT PRIMARY KEY,graph_id TEXT NOT NULL,branch_id TEXT NOT NULL,parent TEXT,recorded_at INTEGER NOT NULL,data TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS heads(graph_id TEXT NOT NULL,branch_id TEXT NOT NULL,revision TEXT NOT NULL REFERENCES revisions(revision),PRIMARY KEY(graph_id,branch_id));
 CREATE TABLE IF NOT EXISTS events(sequence INTEGER PRIMARY KEY AUTOINCREMENT,event_id TEXT UNIQUE NOT NULL,graph_id TEXT NOT NULL,branch_id TEXT NOT NULL,revision TEXT NOT NULL REFERENCES revisions(revision),actor TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS adapters(id TEXT PRIMARY KEY,graph_id TEXT NOT NULL,paused INTEGER NOT NULL DEFAULT 0);
 CREATE TABLE IF NOT EXISTS deliveries(adapter TEXT NOT NULL REFERENCES adapters(id),event_id TEXT NOT NULL REFERENCES events(event_id),attempts INTEGER NOT NULL,status TEXT NOT NULL,PRIMARY KEY(adapter,event_id));
 CREATE TABLE IF NOT EXISTS effects(adapter TEXT NOT NULL,event_id TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(adapter,event_id));")?;
        Ok(Self { conn })
    }
    pub fn execute(&mut self, program: &Program, host: &HostContext) -> Result<Vec<CommandResult>> {
        if program.version != VERSION {
            return Err(err("E_VERSION", "unsupported contract version"));
        }
        if program.commands.len() > 1000 {
            return Err(err("E_BUDGET", "at most 1000 commands"));
        }
        // One program transaction: a later rejection cannot leave earlier graph changes or events.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            let mut out = Vec::new();
            for command in &program.commands {
                out.push(match command {
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
                        CommandResult::Committed { revision, event_id }
                    }
                    Command::Query { query } => CommandResult::Queried {
                        result: self.query(query, host)?,
                    },
                });
            }
            Ok(out)
        })();
        match result {
            Ok(out) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(out)
            }
            Err(e) => {
                self.conn.execute_batch("ROLLBACK")?;
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
    ) -> Result<(String, String)> {
        if !host.writable_graphs.contains(graph) {
            return Err(err(
                "E_FORBIDDEN",
                "host has not granted graph write authority",
            ));
        }
        if graph.is_empty() || branch.is_empty() || host.principal.is_empty() {
            return Err(err("E_ID", "identifiers must not be empty"));
        }
        validate_graph(data)?;
        let head = self.head(graph, branch)?;
        if head.as_deref() != expected {
            return Err(err(
                "E_CONFLICT",
                "expected head differs from current branch head",
            ));
        }
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
        Ok((revision, event_id))
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
        let data: Option<String> = self
            .conn
            .query_row(
                "SELECT data FROM revisions WHERE graph_id=?1 AND revision=?2",
                params![graph, revision],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }
    pub fn query(&self, query: &QueryPlan, host: &HostContext) -> Result<QueryResult> {
        if query.max_depth > 32 {
            return Err(err("E_BUDGET", "metadata depth cannot exceed 32"));
        }
        let revision = match &query.revision {
            Some(rev) => rev.clone(),
            None => self
                .head(&query.graph_id, &query.branch_id)?
                .ok_or_else(|| err("E_UNAVAILABLE", "graph unavailable"))?,
        };
        let data = self
            .load(&query.graph_id, &revision)?
            .ok_or_else(|| err("E_UNAVAILABLE", "graph unavailable"))?;
        let (mut graph, incomplete_derivation) = self.authorized(data, host)?;
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
        let mut result = QueryResult {
            version: VERSION.into(),
            graph,
            snapshots: BTreeMap::from([(query.graph_id.clone(), revision.clone())]),
            coverage: Coverage::Complete,
            diagnostics: Vec::new(),
            provenance: Vec::new(),
            metadata_graphs: Vec::new(),
        };

        if incomplete_derivation {
            partial(
                &mut result,
                "E_DERIVATION_UNAVAILABLE",
                "some derivation dependencies unavailable",
            );
        }
        result.provenance = result
            .graph
            .edges
            .iter()
            .map(|edge| AssertionRef {
                graph_id: query.graph_id.clone(),
                revision: revision.clone(),
                assertion_id: edge.id.clone(),
            })
            .collect();
        if query.include_metadata {
            let mut seen = HashSet::from([(query.graph_id.clone(), revision)]);
            let mut pending: std::collections::VecDeque<_> =
                refs(&result.graph).into_iter().map(|r| (r, 1)).collect();
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
                match self.load(&reference.graph_id, &reference.revision)? {
                    None => partial(
                        &mut result,
                        "E_DEPENDENCY_UNAVAILABLE",
                        "metadata dependency unavailable",
                    ),
                    Some(data) => {
                        let was_empty = data.nodes.is_empty() && data.edges.is_empty();
                        let (data, incomplete_derivation) = self.authorized(data, host)?;
                        if incomplete_derivation {
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
                        pending.extend(refs(&data).into_iter().map(|r| (r, depth + 1)));
                        result.metadata_graphs.push(ResolvedGraph {
                            reference,
                            graph: data,
                        });
                    }
                }
            }
        }
        Ok(result)
    }
    fn authorized(&self, data: GraphData, host: &HostContext) -> Result<(GraphData, bool)> {
        let mut data = visible(data, &host.principal);
        let mut edges = Vec::new();
        let mut incomplete = false;
        for edge in data.edges {
            let mut visiting = HashSet::new();
            let mut budget = 1000usize;
            if self.premises_visible(&edge, host, &mut visiting, &mut budget, 0)? {
                edges.push(edge);
            } else {
                incomplete = true;
            }
        }
        data.edges = edges;
        Ok((data, incomplete))
    }
    fn premises_visible(
        &self,
        edge: &Edge,
        host: &HostContext,
        visiting: &mut HashSet<(String, String, String)>,
        budget: &mut usize,
        depth: u32,
    ) -> Result<bool> {
        if depth > 32 {
            return Ok(false);
        }
        for reference in &edge.derived_from {
            if *budget == 0 {
                return Ok(false);
            }
            *budget -= 1;
            let key = (
                reference.graph_id.clone(),
                reference.revision.clone(),
                reference.assertion_id.clone(),
            );
            if !visiting.insert(key.clone()) {
                return Ok(false);
            }
            let Some(source) = self.load(&reference.graph_id, &reference.revision)? else {
                return Ok(false);
            };
            let source = visible(source, &host.principal);
            let Some(premise) = source.edges.iter().find(|e| e.id == reference.assertion_id) else {
                return Ok(false);
            };
            if !self.premises_visible(premise, host, visiting, budget, depth + 1)? {
                return Ok(false);
            }
            visiting.remove(&key);
        }
        Ok(true)
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
                event_type: "graph.committed".into(),
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
    data
}
fn allowed(readers: &[String], principal: &str) -> bool {
    readers.is_empty() || readers.iter().any(|r| r == principal)
}
fn refs(data: &GraphData) -> Vec<GraphRef> {
    data.nodes
        .iter()
        .flat_map(|n| n.metadata.clone())
        .chain(data.edges.iter().flat_map(|e| e.metadata.clone()))
        .collect()
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
    if data.nodes.len() > 100_000 || data.edges.len() > 100_000 {
        return Err(err("E_BUDGET", "graph exceeds 100000 nodes or edges"));
    }
    let mut ids = HashSet::new();
    for n in &data.nodes {
        if n.id.is_empty()
            || n.entity_id.is_empty()
            || n.space_id.is_empty()
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
        if e.id.is_empty() || e.predicate.is_empty() || !edge_ids.insert(&e.id) {
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
    }
    for reference in refs(data) {
        if reference.graph_id.is_empty() || reference.revision.is_empty() {
            return Err(err(
                "E_REFERENCE",
                "metadata reference must pin graph and revision",
            ));
        }
    }
    Ok(())
}
