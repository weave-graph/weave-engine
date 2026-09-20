//! Private bounded owner queues. One drained view is one atomic SQL/clock operation.
use super::*;
use serde::Serialize;

/// Trusted host scheduling limits; no serialized plan can install them.
#[derive(Debug, Clone, Copy)]
pub struct ViewScanBudget {
    pub max_events: usize,
    pub max_views: usize,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewScanProgress {
    pub events_completed: usize,
    pub view_notifications: usize,
}
#[derive(Debug, Clone)]
pub struct ViewWorkOutcome {
    pub view_id: String,
    pub generation: u64,
    pub current: bool,
    pub work: ViewSelectionWork,
}
#[derive(Serialize)]
struct ProcessedManifest<'a> {
    evaluator: &'static str,
    definition: String,
    tick: Option<i64>,
    generation: u64,
    input_snapshots: &'a [GraphRef],
    live_heads: Vec<(String, String, Option<String>)>,
    schema: String,
    complete: bool,
}
impl Engine {
    pub(crate) fn initialize_view_schedule(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS view_schedule_cursors(principal TEXT PRIMARY KEY,sequence INTEGER NOT NULL,page_id TEXT,ticket INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS view_schedules(id TEXT NOT NULL,principal TEXT NOT NULL,requested_tick INTEGER,pending INTEGER NOT NULL,requested_sequence INTEGER NOT NULL,age INTEGER NOT NULL,processed_manifest TEXT,attempts INTEGER NOT NULL DEFAULT 0,last_error TEXT,PRIMARY KEY(id,principal));
CREATE INDEX IF NOT EXISTS view_schedule_pending ON view_schedules(principal,pending,age,id);")?;
        Ok(())
    }
    /// Enroll an existing incremental-cache view for explicit host-driven scheduling.
    /// Enrollment immediately queues a refresh, closing the registration/scanner race.
    pub fn enable_view_schedule(&mut self, id: &str, host: &HostContext) -> Result<()> {
        let _read = self.read_budget.enter();
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_write_scope()?;
        let (definition, tick) = self.view_schedule_definition(id, host)?;
        // This is the same current authority required to enroll the cache itself.
        self.read_view(id, tick, ViewFreshness::AllowStale, host)?;
        let enrolled: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM view_selection WHERE id=?1 AND principal=?2)",
            params![id, host.principal],
            |r| r.get(0),
        )?;
        if !enrolled {
            return Err(err(
                "E_VIEW_ENROLLMENT",
                "incremental view enrollment required",
            ));
        }
        let sequence: i64 =
            self.conn
                .query_row("SELECT coalesce(max(sequence),0) FROM events", [], |r| {
                    r.get(0)
                })?;
        self.conn.execute("INSERT INTO view_schedule_cursors VALUES (?1,?2,NULL,0) ON CONFLICT(principal) DO NOTHING",params![host.principal,sequence])?;
        let inserted = self.conn.execute("INSERT INTO view_schedules(id,principal,requested_tick,pending,requested_sequence,age,processed_manifest) VALUES (?1,?2,?3,0,?4,0,NULL) ON CONFLICT(id,principal) DO NOTHING",params![id,host.principal,tick,sequence])?;
        // Duplicate enabling is idempotent and does not rewind a newer requested tick.
        let requested: Option<i64> = self.conn.query_row(
            "SELECT requested_tick FROM view_schedules WHERE id=?1 AND principal=?2",
            params![id, host.principal],
            |r| r.get(0),
        )?;
        if definition.clock == ViewClock::Tick && requested.is_none() {
            return Err(err("E_INTEGRITY", "tick view scheduling state unavailable"));
        }
        if inserted != 0 {
            self.enqueue_view(id, host, sequence)?;
        }
        tx.commit()?;
        Ok(())
    }
    fn enqueue_view(&self, id: &str, host: &HostContext, sequence: i64) -> Result<()> {
        let pending: Option<bool> = self
            .conn
            .query_row(
                "SELECT pending FROM view_schedules WHERE id=?1 AND principal=?2",
                params![id, host.principal],
                |r| r.get(0),
            )
            .optional()?;
        let Some(pending) = pending else {
            return Err(err("E_UNAVAILABLE", "scheduled view unavailable"));
        };
        if !pending {
            let ticket: i64 = self.conn.query_row(
                "SELECT ticket FROM view_schedule_cursors WHERE principal=?1",
                [&host.principal],
                |r| r.get(0),
            )?;
            let ticket = ticket
                .checked_add(1)
                .ok_or_else(|| err("E_BUDGET", "view scheduling order exhausted"))?;
            self.conn.execute(
                "UPDATE view_schedule_cursors SET ticket=?2 WHERE principal=?1",
                params![host.principal, ticket],
            )?;
            self.conn.execute(
                "UPDATE view_schedules SET pending=1,age=?3 WHERE id=?1 AND principal=?2",
                params![id, host.principal, ticket],
            )?;
        }
        self.conn.execute("UPDATE view_schedules SET requested_sequence=max(requested_sequence,?3) WHERE id=?1 AND principal=?2",params![id,host.principal,sequence])?;
        Ok(())
    }
    /// Explicit data-time request; authority always uses the engine's installed clock.
    pub fn request_view_tick(&mut self, id: &str, tick: i64, host: &HostContext) -> Result<()> {
        let _read = self.read_budget.enter();
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_write_scope()?;
        let (definition, processed) = self.view_schedule_definition(id, host)?;
        if definition.clock != ViewClock::Tick {
            return Err(err("E_CLOCK", "fixed view does not accept a tick"));
        }
        let requested: Option<Option<i64>> = self
            .conn
            .query_row(
                "SELECT requested_tick FROM view_schedules WHERE id=?1 AND principal=?2",
                params![id, host.principal],
                |r| r.get(0),
            )
            .optional()?;
        let requested = requested
            .flatten()
            .ok_or_else(|| err("E_UNAVAILABLE", "scheduled view unavailable"))?;
        if tick < requested || processed.is_some_and(|p| tick < p) {
            return Err(err("E_CLOCK", "view tick cannot move backwards"));
        }
        let sequence: i64 =
            self.conn
                .query_row("SELECT coalesce(max(sequence),0) FROM events", [], |r| {
                    r.get(0)
                })?;
        self.conn.execute(
            "UPDATE view_schedules SET requested_tick=?3 WHERE id=?1 AND principal=?2",
            params![id, host.principal, tick],
        )?;
        self.enqueue_view(id, host, sequence)?;
        tx.commit()?;
        Ok(())
    }
    /// Atomically page graph events into owner-local coalesced work. No graph mutation occurs.
    pub fn scan_view_work(
        &mut self,
        host: &HostContext,
        budget: ViewScanBudget,
    ) -> Result<ViewScanProgress> {
        self.scan_view_work_inner(host, budget, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn scan_view_work_test_before_commit(
        &mut self,
        host: &HostContext,
        budget: ViewScanBudget,
        before_commit: impl FnOnce(),
    ) -> Result<ViewScanProgress> {
        self.scan_view_work_inner(host, budget, before_commit)
    }
    fn scan_view_work_inner(
        &self,
        host: &HostContext,
        budget: ViewScanBudget,
        before_commit: impl FnOnce(),
    ) -> Result<ViewScanProgress> {
        if !valid_id(&host.principal) || budget.max_events > 1024 || budget.max_views > 256 {
            return Err(err("E_BUDGET", "view scan limits exceeded"));
        }
        let _read = self.read_budget.enter();
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_write_scope()?;
        let cursor:Option<(i64,Option<String>)>=self.conn.query_row("SELECT sequence,substr(page_id,1,513) FROM view_schedule_cursors WHERE principal=?1",[&host.principal],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((mut sequence, mut page_id)) = cursor else {
            tx.commit()?;
            return Ok(ViewScanProgress::default());
        };
        if sequence < 0 || page_id.as_ref().is_some_and(|id| !valid_id(id)) {
            return Err(err("E_INTEGRITY", "invalid scheduling cursor"));
        }
        let mut progress = ViewScanProgress::default();
        while progress.events_completed < budget.max_events
            && progress.view_notifications < budget.max_views
        {
            let event: Option<(i64, String, String)> = if page_id.is_some() {
                self.conn.query_row("SELECT sequence,substr(graph_id,1,513),substr(branch_id,1,513) FROM events WHERE sequence=?1",[sequence],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?
            } else {
                self.conn.query_row("SELECT sequence,substr(graph_id,1,513),substr(branch_id,1,513) FROM events WHERE sequence>?1 ORDER BY sequence LIMIT 1",[sequence],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?
            };
            let Some((event_sequence, graph, branch)) = event else {
                if page_id.is_some() {
                    return Err(err(
                        "E_INTEGRITY",
                        "unfinished view scheduling event unavailable",
                    ));
                }
                break;
            };
            if !valid_id(&graph) || !valid_id(&branch) {
                return Err(err("E_INTEGRITY", "invalid scheduling event"));
            }
            let remaining = budget.max_views - progress.view_notifications;
            // One extra bounded ID determines whether the current event fan-out is complete.
            self.read_budget.request()?;
            let page_bytes = (remaining + 1) * 4096;
            if page_bytes > self.read_budget.remaining() {
                return Err(err("E_BUDGET", "view scan page exceeds read budget"));
            }
            let mut statement=self.conn.prepare("SELECT substr(d.id,1,513) FROM view_dependencies d JOIN view_schedules s ON s.id=d.id AND s.principal=d.principal WHERE d.principal=?1 AND d.graph_id=?2 AND d.branch_id=?3 AND d.id>?4 ORDER BY d.id LIMIT ?5")?;
            let ids = statement
                .query_map(
                    params![
                        host.principal,
                        graph,
                        branch,
                        page_id.as_deref().unwrap_or(""),
                        (remaining + 1) as i64
                    ],
                    |r| r.get::<_, String>(0),
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            self.read_budget.charge(ids.iter().map(String::len).sum())?;
            for id in ids.iter().take(remaining) {
                if !valid_id(id) {
                    return Err(err("E_INTEGRITY", "invalid scheduled view identifier"));
                }
                self.enqueue_view(id, host, event_sequence)?;
                progress.view_notifications += 1;
            }
            sequence = event_sequence;
            if ids.len() > remaining {
                page_id = ids.get(remaining - 1).cloned();
            } else {
                page_id = None;
                progress.events_completed += 1;
            }
            self.conn.execute(
                "UPDATE view_schedule_cursors SET sequence=?2,page_id=?3 WHERE principal=?1",
                params![host.principal, sequence, page_id],
            )?;
        }
        before_commit();
        tx.commit()?;
        Ok(progress)
    }
    /// Drain at most one oldest pending view, bounded by normal operation budgets.
    /// Result, transition guards, cache, processed manifest and acknowledgement commit together.
    pub fn drain_view_work(&mut self, host: &HostContext) -> Result<Option<ViewWorkOutcome>> {
        self.drain_view_work_inner(host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn drain_view_work_test_before_commit(
        &mut self,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<Option<ViewWorkOutcome>> {
        self.drain_view_work_inner(host, before_commit)
    }
    fn drain_view_work_inner(
        &self,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<Option<ViewWorkOutcome>> {
        let mut failed = None;
        let result = self.drain_view_work_atomic(host, before_commit, &mut failed);
        if let (Err(error), Some(id)) = (&result, failed) {
            // Failed computation has already rolled back. Retry ordering is a distinct
            // bounded SQL/clock operation and never acknowledges or alters its input.
            self.rotate_failed_view(&id, host, &error.code)?;
        }
        result
    }
    fn rotate_failed_view(&self, id: &str, host: &HostContext, code: &str) -> Result<()> {
        let _read = self.read_budget.enter();
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_write_scope()?;
        let row: Option<i64> = self
            .conn
            .query_row(
                "SELECT attempts FROM view_schedules WHERE id=?1 AND principal=?2 AND pending=1",
                params![id, host.principal],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(attempts) = row {
            let ticket: i64 = self.conn.query_row(
                "SELECT ticket FROM view_schedule_cursors WHERE principal=?1",
                [&host.principal],
                |r| r.get(0),
            )?;
            let ticket = ticket
                .checked_add(1)
                .ok_or_else(|| err("E_BUDGET", "view scheduling order exhausted"))?;
            self.conn.execute(
                "UPDATE view_schedule_cursors SET ticket=?2 WHERE principal=?1",
                params![host.principal, ticket],
            )?;
            self.conn.execute("UPDATE view_schedules SET age=?3,attempts=?4,last_error=?5 WHERE id=?1 AND principal=?2 AND pending=1",params![id,host.principal,ticket,attempts.saturating_add(1),code.chars().take(64).collect::<String>()])?;
        }
        tx.commit()?;
        Ok(())
    }
    fn drain_view_work_atomic(
        &self,
        host: &HostContext,
        before_commit: impl FnOnce(),
        failed: &mut Option<String>,
    ) -> Result<Option<ViewWorkOutcome>> {
        if !valid_id(&host.principal) {
            return Err(err("E_ID", "principal required"));
        }
        let _read = self.read_budget.enter();
        let tx = self.conn.unchecked_transaction()?;
        let _clock = self.operation_write_scope()?;
        let pending:Option<(String,Option<i64>)>=self.conn.query_row("SELECT substr(id,1,513),requested_tick FROM view_schedules WHERE principal=?1 AND pending=1 ORDER BY age,id LIMIT 1",[&host.principal],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((id, requested_tick)) = pending else {
            tx.commit()?;
            return Ok(None);
        };
        if !valid_id(&id) {
            return Err(err("E_INTEGRITY", "invalid scheduled view identifier"));
        }
        *failed = Some(id.clone());
        let (definition, processed_tick) = self.view_schedule_definition(&id, host)?;
        // A synchronous refresh may have advanced fact time while this request was queued.
        let tick = match definition.clock {
            ViewClock::Fixed => None,
            ViewClock::Tick => Some(
                requested_tick
                    .into_iter()
                    .chain(processed_tick)
                    .max()
                    .ok_or_else(|| err("E_INTEGRITY", "scheduled tick unavailable"))?,
            ),
        };
        let (snapshot, work) = self.refresh_view_work_inner(&id, tick, host)?;
        self.read_budget.request()?;
        if self.read_budget.remaining() < 1001 * 8192 {
            return Err(err(
                "E_BUDGET",
                "view manifest dependencies exceed read budget",
            ));
        }
        let mut statement=self.conn.prepare("SELECT substr(graph_id,1,513),substr(branch_id,1,513) FROM view_dependencies WHERE id=?1 AND principal=?2 ORDER BY graph_id,branch_id LIMIT 1001")?;
        let dependencies = statement
            .query_map(params![id, host.principal], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if dependencies.len() > 1000 {
            return Err(err("E_BUDGET", "view manifest dependency count exceeded"));
        }
        self.read_budget
            .charge(dependencies.iter().map(|(g, b)| g.len() + b.len()).sum())?;
        let live_heads = dependencies
            .into_iter()
            .map(|(g, b)| {
                if !valid_id(&g) || !valid_id(&b) {
                    return Err(err("E_INTEGRITY", "invalid view dependency"));
                }
                Ok((g.clone(), b.clone(), self.head(&g, &b)?))
            })
            .collect::<Result<Vec<_>>>()?;
        let manifest = ProcessedManifest {
            evaluator: "selection-1/full-0.15",
            definition: self.view_definition_fingerprint(&definition, host)?,
            tick,
            generation: snapshot.generation,
            input_snapshots: &snapshot.result.input_snapshots,
            live_heads,
            schema: selection::fingerprint(&snapshot.result.graph.schema)?,
            complete: snapshot.current,
        };
        json_size(&manifest, 4 * 1024 * 1024)?;
        let encoded = serde_json::to_string(&manifest)?;
        let total:i64=self.conn.query_row("SELECT (SELECT coalesce(sum(length(CAST(state AS BLOB))),0) FROM view_selection WHERE principal=?1)+(SELECT coalesce(sum(length(CAST(processed_manifest AS BLOB))),0) FROM view_schedules WHERE principal=?1 AND id<>?2)",params![host.principal,id],|r|r.get(0))?;
        if usize::try_from(total)
            .unwrap_or(usize::MAX)
            .saturating_add(encoded.len())
            > 256 * 1024 * 1024
        {
            return Err(err("E_BUDGET", "view auxiliary quota exceeded"));
        }
        self.conn.execute("UPDATE view_schedules SET pending=0,requested_tick=?3,processed_manifest=?4,attempts=0,last_error=NULL WHERE id=?1 AND principal=?2",params![id,host.principal,tick,encoded])?;
        before_commit();
        tx.commit()?;
        Ok(Some(ViewWorkOutcome {
            view_id: id,
            generation: snapshot.generation,
            current: snapshot.current,
            work,
        }))
    }
}
