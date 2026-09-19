//! Durable principal-scoped live values. Full recomputation is the correctness oracle.
use super::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewClock {
    Fixed,
    Tick,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ViewDefinition {
    pub id: String,
    pub expression: GraphExpression,
    pub clock: ViewClock,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViewSnapshot {
    pub generation: u64,
    pub tick: Option<i64>,
    pub current: bool,
    pub result: QueryResult,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViewChange {
    pub generation: u64,
    pub tick: Option<i64>,
    pub added_nodes: Vec<String>,
    pub removed_nodes: Vec<String>,
    pub changed_nodes: Vec<String>,
    pub added_edges: Vec<String>,
    pub removed_edges: Vec<String>,
    pub changed_edges: Vec<String>,
    pub result: QueryResult,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewFreshness {
    RequireCurrent,
    AllowStale,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct HeadDependency {
    graph: String,
    branch: String,
    revision: Option<String>,
}
struct ViewRecord {
    definition: ViewDefinition,
    tick: Option<i64>,
    generation: i64,
    result: QueryResult,
    dependencies: Vec<HeadDependency>,
}
impl Engine {
    pub(crate) fn initialize_views(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS live_views(id TEXT NOT NULL,principal TEXT NOT NULL,definition TEXT NOT NULL,tick INTEGER,generation INTEGER NOT NULL,result TEXT NOT NULL,dependencies TEXT NOT NULL,PRIMARY KEY(id,principal));
CREATE TABLE IF NOT EXISTS live_view_changes(id TEXT NOT NULL,principal TEXT NOT NULL,generation INTEGER NOT NULL,transition TEXT NOT NULL,PRIMARY KEY(id,principal));
CREATE TABLE IF NOT EXISTS view_change_authorization(id TEXT NOT NULL,principal TEXT NOT NULL,prior_result TEXT NOT NULL,PRIMARY KEY(id,principal));
CREATE TABLE IF NOT EXISTS view_dependencies(id TEXT NOT NULL,principal TEXT NOT NULL,graph_id TEXT NOT NULL,branch_id TEXT NOT NULL,PRIMARY KEY(id,principal,graph_id,branch_id));
CREATE INDEX IF NOT EXISTS view_dependency_graph ON view_dependencies(graph_id,branch_id);")?;
        Ok(())
    }
    /// Registration authorizes only a local materialization under the exact host principal.
    pub fn register_view(
        &mut self,
        definition: &ViewDefinition,
        tick: Option<i64>,
        host: &HostContext,
    ) -> Result<ViewSnapshot> {
        let _read_scope = self.read_budget.enter();
        if !valid_id(&definition.id) || !valid_id(&host.principal) {
            return Err(err("E_VIEW", "view and principal IDs required"));
        }
        json_size(definition, 1024 * 1024)?;
        let tx = self.conn.unchecked_transaction()?;
        let prior: Option<String> = self
            .conn
            .query_row(
                "SELECT definition FROM live_views WHERE id=?1 AND principal=?2",
                params![definition.id, host.principal],
                |r| r.get(0),
            )
            .optional()?;
        let encoded = serde_json::to_string(definition)?;
        if let Some(prior) = prior {
            if prior != encoded {
                return Err(err(
                    "E_VIEW_CONFLICT",
                    "view definition is immutable; register a new ID",
                ));
            }
            tx.commit()?;
            return self.read_view(&definition.id, tick, ViewFreshness::RequireCurrent, host);
        }
        let (result, dependencies) = self.compute_view(definition, tick, host)?;
        let change = change_between(None, &result, 1, tick);
        self.conn.execute(
            "INSERT INTO live_views VALUES (?1,?2,?3,?4,1,?5,?6)",
            params![
                definition.id,
                host.principal,
                encoded,
                tick,
                serde_json::to_string(&result)?,
                serde_json::to_string(&dependencies)?
            ],
        )?;
        self.record_view_change(
            &definition.id,
            &host.principal,
            &change,
            &dependencies,
            &result,
        )?;
        tx.commit()?;
        Ok(ViewSnapshot {
            generation: 1,
            tick,
            current: result.coverage == Coverage::Complete,
            result,
        })
    }
    fn load_view(&self, id: &str, host: &HostContext) -> Result<ViewRecord> {
        type Row = (String, Option<i64>, i64, String, String);
        let row:Option<Row>=self.conn.query_row("SELECT definition,tick,generation,result,dependencies FROM live_views WHERE id=?1 AND principal=?2",params![id,host.principal],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let (definition, tick, generation, result, dependencies) =
            row.ok_or_else(|| err("E_UNAVAILABLE", "view unavailable"))?;
        Ok(ViewRecord {
            definition: serde_json::from_str(&definition)?,
            tick,
            generation,
            result: serde_json::from_str(&result)?,
            dependencies: serde_json::from_str(&dependencies)?,
        })
    }
    fn compute_view(
        &self,
        definition: &ViewDefinition,
        tick: Option<i64>,
        host: &HostContext,
    ) -> Result<(QueryResult, Vec<HeadDependency>)> {
        validate_expression_profile(&definition.expression, VERSION)?;
        let expression = clock_expression(
            &definition.expression,
            &definition.clock,
            tick,
            0,
            &mut 1000,
        )?;
        validate_expression_profile(&expression, VERSION)?;
        let mut dependencies = BTreeMap::new();
        collect_heads(&expression, &mut dependencies);
        let result = self.expression(&expression, &BTreeMap::new(), host, 0, &mut 1000)?;
        // Observe authorized live metadata dependencies from exactly the snapshots used by evaluation.
        for reference in &result.input_snapshots {
            if let Some(data) = self.load(&reference.graph_id, &reference.revision)? {
                let (visible, _) = self.authorized(data, host)?;
                for attachment in visible.attachments {
                    if let MetadataValue::LiveGraph {
                        graph_id,
                        branch_id,
                    } = attachment.value
                    {
                        dependencies.insert((graph_id, branch_id), ());
                    }
                }
            }
        }
        if dependencies.len() > 1000 {
            return Err(err("E_BUDGET", "view dependency budget exceeded"));
        }
        let dependencies = dependencies
            .into_keys()
            .map(|(graph, branch)| {
                Ok(HeadDependency {
                    revision: self.head(&graph, &branch)?,
                    graph,
                    branch,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((result, dependencies))
    }
    fn view_current(&self, record: &ViewRecord, tick: Option<i64>) -> Result<bool> {
        validate_tick(&record.definition.clock, tick)?;
        if record.tick != tick || record.result.coverage == Coverage::Partial {
            return Ok(false);
        }
        for dependency in &record.dependencies {
            if self.head(&dependency.graph, &dependency.branch)? != dependency.revision {
                return Ok(false);
            }
        }
        Ok(true)
    }
    /// Stale data is returned only with an explicit opt-in and `current=false`.
    pub fn read_view(
        &self,
        id: &str,
        tick: Option<i64>,
        freshness: ViewFreshness,
        host: &HostContext,
    ) -> Result<ViewSnapshot> {
        let _read_scope = self.read_budget.enter();
        let tx = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let record = self.load_view(id, host)?;
        self.require_current_result_authority(&record.result, host)?;
        let current = self.view_current(&record, tick)?;
        if !current && freshness == ViewFreshness::RequireCurrent {
            return Err(err("E_FRESHNESS", "view requires explicit refresh"));
        }
        let result = ViewSnapshot {
            generation: record.generation as u64,
            tick: record.tick,
            current,
            result: record.result,
        };
        if let Some(tx) = tx {
            tx.commit()?
        }
        Ok(result)
    }
    /// Explicit tick/recompute in one snapshot and one durable cache transaction; no pure wall-clock read.
    pub fn refresh_view(
        &mut self,
        id: &str,
        tick: Option<i64>,
        host: &HostContext,
    ) -> Result<ViewSnapshot> {
        let _read_scope = self.read_budget.enter();
        let tx = self.conn.unchecked_transaction()?;
        let old = self.load_view(id, host)?;
        validate_tick(&old.definition.clock, tick)?;
        if old.tick.zip(tick).is_some_and(|(old, new)| new < old) {
            return Err(err(
                "E_CLOCK",
                "view tick cannot move backwards; register a separate historical view",
            ));
        }
        let (result, dependencies) = self.compute_view(&old.definition, tick, host)?;
        let changed = old.result != result || old.tick != tick;
        let generation = old
            .generation
            .checked_add(i64::from(changed))
            .ok_or_else(|| err("E_BUDGET", "view generation exhausted"))?;
        self.conn.execute("UPDATE live_views SET tick=?3,generation=?4,result=?5,dependencies=?6 WHERE id=?1 AND principal=?2",params![id,host.principal,tick,generation,serde_json::to_string(&result)?,serde_json::to_string(&dependencies)?])?;
        if changed {
            let change = change_between(Some(&old.result), &result, generation as u64, tick);
            self.record_view_change(id, &host.principal, &change, &dependencies, &old.result)?;
        }
        tx.commit()?;
        Ok(ViewSnapshot {
            generation: generation as u64,
            tick,
            current: result.coverage == Coverage::Complete,
            result,
        })
    }
    fn record_view_change(
        &self,
        id: &str,
        principal: &str,
        change: &ViewChange,
        dependencies: &[HeadDependency],
        previous: &QueryResult,
    ) -> Result<()> {
        json_size(change, MATERIALIZED_LIMIT)?;
        json_size(previous, MATERIALIZED_LIMIT)?;
        self.conn.execute("INSERT INTO view_change_authorization VALUES (?1,?2,?3) ON CONFLICT(id,principal) DO UPDATE SET prior_result=excluded.prior_result", params![id,principal,serde_json::to_string(previous)?])?;
        self.conn.execute("INSERT INTO live_view_changes VALUES (?1,?2,?3,?4) ON CONFLICT(id,principal) DO UPDATE SET generation=excluded.generation,transition=excluded.transition",params![id,principal,i64::try_from(change.generation).map_err(|_|err("E_BUDGET","view generation out of range"))?,serde_json::to_string(change)?])?;
        self.conn.execute(
            "DELETE FROM view_dependencies WHERE id=?1 AND principal=?2",
            params![id, principal],
        )?;
        for dependency in dependencies {
            self.conn.execute(
                "INSERT INTO view_dependencies VALUES (?1,?2,?3,?4)",
                params![id, principal, dependency.graph, dependency.branch],
            )?;
        }
        Ok(())
    }
    /// Revalidate stored values before returning them; stale data is never stale authority.
    pub(crate) fn require_current_result_authority(
        &self,
        value: &QueryResult,
        host: &HostContext,
    ) -> Result<()> {
        for reference in &value.input_snapshots {
            if !self.identity_reference_allowed(&reference.graph_id, &reference.revision, host)? {
                return Err(err(
                    "E_UNAVAILABLE",
                    "stored result unavailable under current authority",
                ));
            }
        }
        for data in
            std::iter::once(&value.graph).chain(value.metadata_graphs.iter().map(|g| &g.graph))
        {
            let (authorized, incomplete) = self.authorized(data.clone(), host)?;
            if incomplete || &authorized != data {
                return Err(err(
                    "E_UNAVAILABLE",
                    "stored result unavailable under current authority",
                ));
            }
        }
        Ok(())
    }
    /// One retained transition per view. Gaps explicitly require snapshot resynchronization.
    pub fn view_changes(
        &self,
        id: &str,
        after_generation: u64,
        host: &HostContext,
    ) -> Result<Option<ViewChange>> {
        let _read_scope = self.read_budget.enter();
        let tx = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let record = self.load_view(id, host)?;
        self.require_current_result_authority(&record.result, host)?;
        let current = record.generation as u64;
        if after_generation > current {
            return Err(err("E_VIEW_CURSOR", "cursor is newer than this view"));
        }
        if after_generation == current {
            if let Some(tx) = tx {
                tx.commit()?;
            }
            return Ok(None);
        }
        if current - after_generation > 1 {
            return Err(err(
                "E_REPLAY_WINDOW",
                "view transition expired; read the current snapshot",
            ));
        }
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(MATERIALIZED_LIMIT) as i64;
        let prior:Option<Option<String>>=self.conn.query_row("SELECT CASE WHEN length(CAST(prior_result AS BLOB))<=?3 THEN prior_result END FROM view_change_authorization WHERE id=?1 AND principal=?2",params![id,host.principal,limit],|r|r.get(0)).optional()?;
        let prior = prior
            .ok_or_else(|| {
                err(
                    "E_REPLAY_WINDOW",
                    "transition authorization unavailable; resynchronize from snapshot",
                )
            })?
            .ok_or_else(|| err("E_BUDGET", "view authorization exceeds read budget"))?;
        self.read_budget.charge(prior.len())?;
        self.require_current_result_authority(&serde_json::from_str(&prior)?, host)?;
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(MATERIALIZED_LIMIT) as i64;
        let json: Option<String> = self.conn.query_row(
            "SELECT CASE WHEN length(CAST(transition AS BLOB))<=?3 THEN transition END FROM live_view_changes WHERE id=?1 AND principal=?2",
            params![id, host.principal,limit],
            |r| r.get(0),
        )?;
        let json = json.ok_or_else(|| err("E_BUDGET", "view transition exceeds read budget"))?;
        self.read_budget.charge(json.len())?;
        let value: ViewChange = serde_json::from_str(&json)?;
        self.require_current_result_authority(&value.result, host)?;
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok(Some(value))
    }
}
fn validate_tick(clock: &ViewClock, tick: Option<i64>) -> Result<()> {
    if matches!(
        (clock, tick),
        (ViewClock::Fixed, Some(_)) | (ViewClock::Tick, None)
    ) {
        return Err(err(
            "E_CLOCK",
            "fixed views reject ticks; clocked views require an explicit tick",
        ));
    }
    Ok(())
}
fn clock_expression(
    expression: &GraphExpression,
    clock: &ViewClock,
    tick: Option<i64>,
    depth: usize,
    budget: &mut usize,
) -> Result<GraphExpression> {
    validate_tick(clock, tick)?;
    if depth > 32 || *budget == 0 {
        return Err(err("E_BUDGET", "view expression budget exceeded"));
    }
    *budget -= 1;
    let mut value = expression.clone();
    match &mut value {
        GraphExpression::ResolveIdentity { selection } => {
            if let Some(t) = tick {
                selection.valid_at = t;
            }
        }
        GraphExpression::Cluster { selection } => {
            if let Some(t) = tick {
                selection.valid_at = t;
            }
        }
        GraphExpression::Counterparts { input, selection } => {
            **input = clock_expression(input, clock, tick, depth + 1, budget)?;
            if let Some(t) = tick {
                selection.valid_at = t;
            }
        }
        GraphExpression::Geometry {
            operation,
            valid_at,
        } => {
            for input in operation.inputs_mut() {
                *input = clock_expression(input, clock, tick, depth + 1, budget)?;
            }
            if let Some(t) = tick {
                *valid_at = t;
            }
        }
        GraphExpression::Reference { .. } => {
            return Err(err(
                "E_BINDING",
                "standalone views cannot reference program-local bindings",
            ))
        }
        GraphExpression::Query { query } => {
            if *clock == ViewClock::Tick {
                query.valid_at = tick
            }
        }
        GraphExpression::Union { left, right }
        | GraphExpression::Diff {
            before: left,
            after: right,
        }
        | GraphExpression::Join { left, right, .. } => {
            **left = clock_expression(left, clock, tick, depth + 1, budget)?;
            **right = clock_expression(right, clock, tick, depth + 1, budget)?;
        }
        GraphExpression::Filter {
            input, valid_at, ..
        } => {
            **input = clock_expression(input, clock, tick, depth + 1, budget)?;
            if *clock == ViewClock::Tick {
                *valid_at = tick
            }
        }
        GraphExpression::Support {
            input, valid_at, ..
        } => {
            **input = clock_expression(input, clock, tick, depth + 1, budget)?;
            if let Some(t) = tick {
                *valid_at = t
            }
        }
        GraphExpression::Metadata { input, .. }
        | GraphExpression::Project { input, .. }
        | GraphExpression::Reason { input, .. }
        | GraphExpression::TypedContext { input, .. }
        | GraphExpression::Context { input, .. }
        | GraphExpression::Explain { input } => {
            **input = clock_expression(input, clock, tick, depth + 1, budget)?;
        }
    }
    if *clock == ViewClock::Tick && matches!(expression, GraphExpression::Metadata { .. }) {
        return Ok(GraphExpression::Filter {
            input: Box::new(value),
            predicate: None,
            valid_at: tick,
        });
    }
    Ok(value)
}
fn collect_heads(expression: &GraphExpression, heads: &mut BTreeMap<(String, String), ()>) {
    match expression {
        GraphExpression::Geometry { operation, .. } => {
            for input in operation.inputs() {
                collect_heads(input, heads);
            }
        }
        GraphExpression::Query { query } => {
            if query.revision.is_none() {
                heads.insert((query.graph_id.clone(), query.branch_id.clone()), ());
            }
        }
        GraphExpression::Union { left, right }
        | GraphExpression::Diff {
            before: left,
            after: right,
        }
        | GraphExpression::Join { left, right, .. } => {
            collect_heads(left, heads);
            collect_heads(right, heads)
        }
        GraphExpression::Filter { input, .. }
        | GraphExpression::Support { input, .. }
        | GraphExpression::Metadata { input, .. }
        | GraphExpression::Project { input, .. }
        | GraphExpression::Reason { input, .. }
        | GraphExpression::TypedContext { input, .. }
        | GraphExpression::Context { input, .. }
        | GraphExpression::Explain { input }
        | GraphExpression::Counterparts { input, .. } => collect_heads(input, heads),
        GraphExpression::Reference { .. }
        | GraphExpression::ResolveIdentity { .. }
        | GraphExpression::Cluster { .. } => {}
    }
}
fn changes<T: PartialEq>(
    before: impl Iterator<Item = (String, T)>,
    after: impl Iterator<Item = (String, T)>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let a: BTreeMap<_, _> = before.collect();
    let b: BTreeMap<_, _> = after.collect();
    (
        b.keys().filter(|k| !a.contains_key(*k)).cloned().collect(),
        a.keys().filter(|k| !b.contains_key(*k)).cloned().collect(),
        b.iter()
            .filter(|(k, v)| a.get(*k).is_some_and(|old| old != *v))
            .map(|(k, _)| k.clone())
            .collect(),
    )
}
fn change_between(
    old: Option<&QueryResult>,
    new: &QueryResult,
    generation: u64,
    tick: Option<i64>,
) -> ViewChange {
    let empty = GraphData::default();
    let old = old.map_or(&empty, |r| &r.graph);
    let (added_nodes, removed_nodes, changed_nodes) = changes(
        old.nodes.iter().map(|n| (n.id.clone(), n)),
        new.graph.nodes.iter().map(|n| (n.id.clone(), n)),
    );
    let (added_edges, removed_edges, changed_edges) = changes(
        old.edges.iter().map(|e| (e.id.clone(), e)),
        new.graph.edges.iter().map(|e| (e.id.clone(), e)),
    );
    ViewChange {
        generation,
        tick,
        added_nodes,
        removed_nodes,
        changed_nodes,
        added_edges,
        removed_edges,
        changed_edges,
        result: new.clone(),
    }
}
