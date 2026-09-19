//! Native reference runtime. The portable wire/model crate is `weave-contract`.
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use weave_contract::*;
mod capsule;
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
        if program.version != VERSION
            && program.version != "0.2.0"
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
        if program.version != VERSION
            && program
                .commands
                .iter()
                .any(|c| matches!(c, Command::Bind { .. } | Command::Evaluate { .. }))
        {
            return Err(err("E_VERSION", "graph expressions require contract 0.3.0"));
        }
        if program.commands.len() > 1000 {
            return Err(err("E_BUDGET", "at most 1000 commands"));
        }
        // One program transaction: a later rejection cannot leave earlier graph changes or events.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            let mut out = Vec::new();
            let mut values = BTreeMap::new();
            let mut value_size = 0usize;
            let mut materialized_bytes = 0usize;
            for command in &program.commands {
                let command_result = match command {
                    Command::Bind { name, value } => {
                        if !valid_id(name) || values.contains_key(name) {
                            return Err(err(
                                "E_BINDING",
                                "graph value name must be nonempty and unique",
                            ));
                        }
                        let result = self.expression(value, &values, host, 0, &mut 1000)?;
                        value_size += result.graph.nodes.len() + result.graph.edges.len();
                        if value_size > 200_000 {
                            return Err(err("E_BUDGET", "bound graph value budget exceeded"));
                        }
                        materialized_bytes += json_size(
                            &result,
                            MATERIALIZED_LIMIT.saturating_sub(materialized_bytes),
                        )?;
                        values.insert(name.clone(), result.clone());
                        CommandResult::Queried { result }
                    }
                    Command::Evaluate { value } => CommandResult::Queried {
                        result: self.expression(value, &values, host, 0, &mut 1000)?,
                    },

                    Command::Join {
                        left,
                        right,
                        output_predicate,
                        match_on: JoinMatch::EntitySpaceToFrom,
                    } => CommandResult::Queried {
                        result: self.join(left, right, output_predicate, host)?,
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
                        CommandResult::Committed { revision, event_id }
                    }
                    Command::Query { query } => CommandResult::Queried {
                        result: self.query(query, host)?,
                    },
                };
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
        if !valid_id(graph) || !valid_id(branch) || !valid_id(&host.principal) {
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
            input_snapshots: vec![GraphRef {
                graph_id: query.graph_id.clone(),
                revision: revision.clone(),
            }],
            coverage: Coverage::Complete,
            diagnostics: Vec::new(),
            provenance: Vec::new(),
            edge_origins: BTreeMap::new(),
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
        result.edge_origins = result
            .provenance
            .iter()
            .map(|r| (r.assertion_id.clone(), vec![r.clone()]))
            .collect();
        let mut query_bytes = json_size(&result, MATERIALIZED_LIMIT)?;
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
                        query_bytes +=
                            json_size(&data, MATERIALIZED_LIMIT.saturating_sub(query_bytes))?;
                        result.input_snapshots.push(reference.clone());
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
    /// Identity-key path join. Pure over two pinned, authorized graph views.
    pub fn join(
        &self,
        left: &QueryPlan,
        right: &QueryPlan,
        predicate: &str,
        host: &HostContext,
    ) -> Result<QueryResult> {
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
        let pairs = l
            .graph
            .edges
            .len()
            .checked_mul(r.graph.edges.len())
            .ok_or_else(|| err("E_BUDGET", "join pair budget exceeded"))?;
        if pairs > 1_000_000 {
            return Err(err("E_BUDGET", "join pair budget exceeds 1000000"));
        }
        let ln: BTreeMap<_, _> = l.graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let rn: BTreeMap<_, _> = r.graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let mut nodes = BTreeMap::new();
        let mut edges = Vec::new();
        let mut provenance = Vec::new();
        let mut edge_origins = BTreeMap::new();
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
                // Results are scoped to the evaluating principal until a release policy exists.
                source.readers = vec![host.principal.clone()];
                target.readers = vec![host.principal.clone()];
                let edge = Edge {
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
                };
                nodes.insert(source.id.clone(), source);
                nodes.insert(target.id.clone(), target);
                edge_origins.insert(edge.id.clone(), premises.clone());
                edges.push(edge);
                if edges.len() > 100_000 {
                    return Err(err("E_BUDGET", "join output edge budget exceeded"));
                }
                for reference in premises {
                    if !provenance.contains(&reference) {
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
        Ok(QueryResult {
            version: VERSION.into(),
            graph: GraphData {
                nodes: nodes.into_values().collect(),
                edges,
            },
            snapshots,
            input_snapshots,
            coverage,
            diagnostics,
            provenance,
            edge_origins,
            metadata_graphs,
        })
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

const MATERIALIZED_LIMIT: usize = 32 * 1024 * 1024;
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
