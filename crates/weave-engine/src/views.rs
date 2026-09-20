//! Durable principal-scoped live values. Full recomputation is the correctness oracle.
use super::*;
use serde::{Deserialize, Serialize};
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
CREATE TABLE IF NOT EXISTS view_selection(id TEXT NOT NULL,principal TEXT NOT NULL,state TEXT,digest TEXT,PRIMARY KEY(id,principal));
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
        self.register_view_inner(definition, tick, host, None)
    }
    pub(crate) fn register_view_inner(
        &self,
        definition: &ViewDefinition,
        tick: Option<i64>,
        host: &HostContext,
        template: Option<&CompiledViewTemplate>,
    ) -> Result<ViewSnapshot> {
        let _read_scope = self.read_budget.enter();
        if !valid_id(&definition.id) || !valid_id(&host.principal) {
            return Err(err("E_VIEW", "view and principal IDs required"));
        }
        json_size(definition, 1024 * 1024)?;
        let tx = self.conn.unchecked_transaction()?;
        let _clock_scope = self.operation_write_scope()?;
        self.read_budget.request()?;
        let definition_limit = self.read_budget.remaining().min(1024 * 1024) as i64;
        let prior: Option<Option<String>> = self.conn.query_row(
            "SELECT CASE WHEN length(CAST(definition AS BLOB))<=?3 THEN definition END FROM live_views WHERE id=?1 AND principal=?2",
            params![definition.id, host.principal, definition_limit], |r| r.get(0)
        ).optional()?;
        let encoded = serde_json::to_string(definition)?;
        validate_expression_profile(&definition.expression, VERSION)?;
        clock_expression(
            &definition.expression,
            &definition.clock,
            tick,
            0,
            &mut 1000,
        )?;
        if let Some(prior) = prior {
            let prior =
                prior.ok_or_else(|| err("E_BUDGET", "view definition exceeds read budget"))?;
            self.read_budget.charge(prior.len())?;
            let stored = self.compiled_view_template(definition, host)?;
            if prior != encoded || stored.as_ref() != template {
                return Err(err(
                    "E_VIEW_CONFLICT",
                    "view definition is immutable; register a new ID",
                ));
            }
            let result =
                self.read_view(&definition.id, tick, ViewFreshness::RequireCurrent, host)?;
            tx.commit()?;
            return Ok(result);
        }
        let sources = template.map_or(&[][..], |t| t.source_revisions.as_slice());
        let (result, dependencies) =
            self.compute_view_with_sources(definition, tick, host, sources)?;
        let change = change_between(None, &result, 1, tick);
        self.conn.execute(
            "INSERT INTO live_views(id,principal,definition,tick,generation,result,dependencies,source_digest) VALUES (?1,?2,?3,?4,1,?5,?6,?7)",
            params![
                definition.id,
                host.principal,
                encoded,
                tick,
                serde_json::to_string(&result)?,
                serde_json::to_string(&dependencies)?,
                template.map(|t|&t.definition_digest)
            ],
        )?;
        if let Some(template) = template {
            self.conn.execute(
                "INSERT INTO view_sources VALUES (?1,?2,?3,?4)",
                params![
                    definition.id,
                    host.principal,
                    serde_json::to_string(template)?,
                    template.definition_digest
                ],
            )?;
        }
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
    /// Opt into private membership caching. This grants no query or write authority.
    /// Ordinary synchronous views remain available when the enrollment quota is full.
    pub fn enroll_incremental_view(&mut self, id: &str, host: &HostContext) -> Result<()> {
        let _read_scope = self.read_budget.enter();
        let tx = self.conn.unchecked_transaction()?;
        let _clock_scope = self.operation_write_scope()?;
        let record = self.load_view(id, host)?;
        self.require_current_result_authority(&record.result, host)?;
        let enrolled: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM view_selection WHERE id=?1 AND principal=?2)",
            params![id, host.principal],
            |r| r.get(0),
        )?;
        if !enrolled {
            let count: i64 = self.conn.query_row(
                "SELECT count(*) FROM view_selection WHERE principal=?1",
                [&host.principal],
                |r| r.get(0),
            )?;
            if count >= 256 {
                return Err(err(
                    "E_BUDGET",
                    "incremental view enrollment quota exceeded",
                ));
            }
            self.conn.execute(
                "INSERT INTO view_selection(id,principal) VALUES (?1,?2)",
                params![id, host.principal],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn view_schedule_definition(
        &self,
        id: &str,
        host: &HostContext,
    ) -> Result<(ViewDefinition, Option<i64>)> {
        self.read_budget.request()?;
        let row: Option<(Option<String>,Option<i64>)> = self.conn.query_row("SELECT CASE WHEN length(CAST(definition AS BLOB))<=?3 THEN definition END,tick FROM live_views WHERE id=?1 AND principal=?2",params![id,host.principal,self.read_budget.remaining().min(1024*1024) as i64],|r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (definition, tick) = row.ok_or_else(|| err("E_UNAVAILABLE", "view unavailable"))?;
        let definition =
            definition.ok_or_else(|| err("E_BUDGET", "view definition exceeds read budget"))?;
        self.read_budget.charge(definition.len())?;
        Ok((serde_json::from_str(&definition)?, tick))
    }
    fn selection_state(
        &self,
        id: &str,
        host: &HostContext,
    ) -> Result<(bool, Option<selection::State>)> {
        self.read_budget.request()?;
        let row: Option<(Option<String>, Option<String>)> = self.conn.query_row(
            "SELECT CASE WHEN length(CAST(state AS BLOB))<=?3 THEN state END,substr(digest,1,65) FROM view_selection WHERE id=?1 AND principal=?2",
            params![id,host.principal,self.read_budget.remaining().min(selection::STATE_LIMIT) as i64],
            |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((Some(encoded), Some(digest))) = row.as_ref() else {
            return Ok((row.is_some(), None));
        };
        self.read_budget.charge(encoded.len())?;
        if format!("{:x}", Sha256::digest(encoded.as_bytes())) != *digest {
            return Ok((true, None));
        }
        let state = serde_json::from_str::<selection::State>(encoded)
            .ok()
            .filter(selection::State::valid);
        Ok((true, state))
    }
    fn store_selection(
        &self,
        id: &str,
        host: &HostContext,
        state: Option<&selection::State>,
    ) -> Result<()> {
        let encoded = match state {
            Some(state) => {
                json_size(state, selection::STATE_LIMIT)?;
                Some(serde_json::to_string(state)?)
            }
            None => None,
        };
        let other: i64 = self.conn.query_row("SELECT (SELECT coalesce(sum(length(CAST(state AS BLOB))),0) FROM view_selection WHERE principal=?1 AND id<>?2)+(SELECT coalesce(sum(length(CAST(processed_manifest AS BLOB))),0) FROM view_schedules WHERE principal=?1)", params![host.principal,id], |r| r.get(0))?;
        if usize::try_from(other)
            .unwrap_or(usize::MAX)
            .saturating_add(encoded.as_ref().map_or(0, String::len))
            > 256 * 1024 * 1024
        {
            return Err(err("E_BUDGET", "incremental view state quota exceeded"));
        }
        let digest = encoded
            .as_ref()
            .map(|s| format!("{:x}", Sha256::digest(s.as_bytes())));
        self.conn.execute(
            "UPDATE view_selection SET state=?3,digest=?4 WHERE id=?1 AND principal=?2",
            params![id, host.principal, encoded, digest],
        )?;
        Ok(())
    }
    fn compute_selection_view(
        &self,
        definition: &ViewDefinition,
        tick: Option<i64>,
        host: &HostContext,
        work: &mut ViewSelectionWork,
    ) -> Result<(QueryResult, Vec<HeadDependency>)> {
        let (enrolled, prior) = self.selection_state(&definition.id, host)?;
        if !enrolled {
            return self.compute_view(definition, tick, host);
        }
        validate_expression_profile(&definition.expression, VERSION)?;
        let expression = clock_expression(
            &definition.expression,
            &definition.clock,
            tick,
            0,
            &mut 1000,
        )?;
        let mut tentative = None;
        if let Some(plan) = selection::Plan::parse(&expression) {
            let query = plan.query;
            if !identity_acceptance::reserved(&query.graph_id)
                && !governance_graph::reserved(&query.graph_id)
                && valid_id(&query.graph_id)
                && valid_id(&query.branch_id)
                && valid_id(&host.principal)
                && query.revision.as_ref().is_none_or(|r| valid_id(r))
                && query.max_depth <= 32
            {
                let revision = match &query.revision {
                    Some(r) => Some(r.clone()),
                    None => self.head(&query.graph_id, &query.branch_id)?,
                };
                if let Some(revision) = revision {
                    if !self.protected_reference_allowed(&query.graph_id, &revision, host)? {
                        return Err(err("E_UNAVAILABLE", "graph unavailable"));
                    }
                    if let Some(raw) = self.load(&query.graph_id, &revision)? {
                        if selection::eligible(&raw) {
                            let definition_hash =
                                self.view_definition_fingerprint(definition, host)?;
                            let reusable = prior
                                .as_ref()
                                .map(|s| s.reusable(&definition_hash, &host.principal, &raw))
                                .transpose()?
                                .unwrap_or(false);
                            match selection::evaluate(
                                &plan,
                                &definition_hash,
                                &revision,
                                raw,
                                &host.principal,
                                prior.as_ref(),
                                work,
                            ) {
                                Ok((mut result, state)) => {
                                    self.attach_view_sources(definition, host, &mut result)?;
                                    let dependencies = if query.revision.is_none() {
                                        vec![HeadDependency {
                                            graph: query.graph_id.clone(),
                                            branch: query.branch_id.clone(),
                                            revision: Some(revision),
                                        }]
                                    } else {
                                        vec![]
                                    };
                                    tentative = Some((result, state, dependencies, reusable));
                                }
                                Err(error) if error.code == "E_BUDGET" => {}
                                Err(error) => return Err(error),
                            }
                        }
                    }
                }
            }
        }
        if let Some((result, state, dependencies, reusable)) = tentative {
            if !reusable {
                // Cold/corrupt state is rebuilt only after the authoritative full oracle agrees.
                work.fallback_runs += 1;
                let (oracle, oracle_dependencies) = self.compute_view(definition, tick, host)?;
                if result != oracle || dependencies != oracle_dependencies {
                    work.oracle_disagreements += 1;
                    self.store_selection(&definition.id, host, None)?;
                    return Ok((oracle, oracle_dependencies));
                }
            }
            self.store_selection(&definition.id, host, Some(&state))?;
            return Ok((result, dependencies));
        }
        work.fallback_runs += 1;
        let result = self.compute_view(definition, tick, host)?;
        self.store_selection(&definition.id, host, None)?;
        Ok(result)
    }
    fn load_view(&self, id: &str, host: &HostContext) -> Result<ViewRecord> {
        self.read_budget.request()?;
        type Row = (
            Option<String>,
            Option<i64>,
            i64,
            Option<String>,
            Option<String>,
        );
        // Each conditional shares a total-byte guard: oversized persisted values never
        // cross SQLite's result boundary into a Rust String before the limit check.
        let row: Option<Row> = self
            .conn
            .query_row(
                "WITH candidate AS (
                SELECT definition,tick,generation,result,dependencies,
                    length(CAST(definition AS BLOB)) AS d,
                    length(CAST(result AS BLOB)) AS r,
                    length(CAST(dependencies AS BLOB)) AS p
                FROM live_views WHERE id=?1 AND principal=?2
             ) SELECT
                CASE WHEN d<=1048576 AND r<=?4 AND p<=4194304 AND d+r+p<=?3 THEN definition END,
                tick,generation,
                CASE WHEN d<=1048576 AND r<=?4 AND p<=4194304 AND d+r+p<=?3 THEN result END,
                CASE WHEN d<=1048576 AND r<=?4 AND p<=4194304 AND d+r+p<=?3 THEN dependencies END
             FROM candidate",
                params![
                    id,
                    host.principal,
                    self.read_budget.remaining() as i64,
                    MATERIALIZED_LIMIT as i64
                ],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (definition, tick, generation, result, dependencies) =
            row.ok_or_else(|| err("E_UNAVAILABLE", "view unavailable"))?;
        let (Some(definition), Some(result), Some(dependencies)) =
            (definition, result, dependencies)
        else {
            return Err(err("E_BUDGET", "stored view exceeds read budget"));
        };
        self.read_budget
            .charge(definition.len() + result.len() + dependencies.len())?;
        if generation < 1 {
            return Err(err("E_INTEGRITY", "invalid stored view generation"));
        }
        let dependencies: Vec<HeadDependency> = serde_json::from_str(&dependencies)?;
        if dependencies.len() > 1000 {
            return Err(err("E_BUDGET", "stored view dependency count exceeded"));
        }
        let definition: ViewDefinition = serde_json::from_str(&definition)?;
        let result: QueryResult = serde_json::from_str(&result)?;
        if let Some(template) = self.compiled_view_template(&definition, host)? {
            let merged = algebra::merge_source_revisions(
                &result.source_revisions,
                &template.source_revisions,
            )
            .map_err(|d| err(&d.code, &d.message))?;
            if merged != result.source_revisions {
                return Err(err("E_INTEGRITY", "stored view source manifest mismatch"));
            }
        }
        Ok(ViewRecord {
            definition,
            tick,
            generation,
            result,
            dependencies,
        })
    }
    fn compute_view(
        &self,
        definition: &ViewDefinition,
        tick: Option<i64>,
        host: &HostContext,
    ) -> Result<(QueryResult, Vec<HeadDependency>)> {
        let template = self.compiled_view_template(definition, host)?;
        self.compute_view_with_sources(
            definition,
            tick,
            host,
            template
                .as_ref()
                .map_or(&[][..], |t| t.source_revisions.as_slice()),
        )
    }
    fn compute_view_with_sources(
        &self,
        definition: &ViewDefinition,
        tick: Option<i64>,
        host: &HostContext,
        sources: &[SourceRevision],
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
        let mut result = self.expression(&expression, &BTreeMap::new(), host, 0, &mut 1000)?;
        merge_sources(&mut result.source_revisions, sources)?;
        json_size(&result, MATERIALIZED_LIMIT)?;
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
    fn pending_tick_satisfied(
        &self,
        id: &str,
        tick: Option<i64>,
        host: &HostContext,
    ) -> Result<bool> {
        let requested: Option<Option<i64>> = self.conn.query_row("SELECT requested_tick FROM view_schedules WHERE id=?1 AND principal=?2 AND pending=1",params![id,host.principal],|r|r.get(0)).optional()?;
        Ok(requested
            .flatten()
            .zip(tick)
            .is_none_or(|(requested, processed)| requested <= processed))
    }
    fn view_current(
        &self,
        record: &ViewRecord,
        tick: Option<i64>,
        host: &HostContext,
    ) -> Result<bool> {
        validate_tick(&record.definition.clock, tick)?;
        if record.tick != tick || record.result.coverage == Coverage::Partial {
            return Ok(false);
        }
        if !self.pending_tick_satisfied(&record.definition.id, record.tick, host)? {
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
        let _clock_scope = self.operation_scope()?;
        let record = self.load_view(id, host)?;
        self.require_current_result_authority(&record.result, host)?;
        let current = self.view_current(&record, tick, host)?;
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
    /// Explicit fact-time tick/recompute; current authority uses the separate trusted operation clock.
    pub fn refresh_view(
        &mut self,
        id: &str,
        tick: Option<i64>,
        host: &HostContext,
    ) -> Result<ViewSnapshot> {
        self.refresh_view_with_work(id, tick, host)
            .map(|(snapshot, _)| snapshot)
    }
    /// Membership work counts are trusted-host diagnostics, never graph output.
    pub fn refresh_view_with_work(
        &mut self,
        id: &str,
        tick: Option<i64>,
        host: &HostContext,
    ) -> Result<(ViewSnapshot, ViewSelectionWork)> {
        self.refresh_view_work_inner(id, tick, host)
    }
    pub(crate) fn refresh_view_work_inner(
        &self,
        id: &str,
        tick: Option<i64>,
        host: &HostContext,
    ) -> Result<(ViewSnapshot, ViewSelectionWork)> {
        let mut work = ViewSelectionWork::default();
        let _read_scope = self.read_budget.enter();
        let tx = self.optional_read_transaction()?;
        let _clock_scope = self.operation_write_scope()?;
        let old = self.load_view(id, host)?;
        validate_tick(&old.definition.clock, tick)?;
        if old.tick.zip(tick).is_some_and(|(old, new)| new < old) {
            return Err(err(
                "E_CLOCK",
                "view tick cannot move backwards; register a separate historical view",
            ));
        }
        let (result, dependencies) =
            self.compute_selection_view(&old.definition, tick, host, &mut work)?;
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
        let current =
            result.coverage == Coverage::Complete && self.pending_tick_satisfied(id, tick, host)?;
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok((
            ViewSnapshot {
                generation: generation as u64,
                tick,
                current,
                result,
            },
            work,
        ))
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
            if !self.protected_reference_allowed(&reference.graph_id, &reference.revision, host)? {
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
        let _clock_scope = self.operation_scope()?;
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
        GraphExpression::AcceptedGraph { .. } | GraphExpression::CurrentView { .. } => {
            return Err(err(
                "E_VIEW_DEPENDENCY",
                "governed or cached view reads cannot be registered as view dependencies",
            ))
        }
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
        GraphExpression::AcceptedGraph { .. }
        | GraphExpression::CurrentView { .. }
        | GraphExpression::Reference { .. }
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
