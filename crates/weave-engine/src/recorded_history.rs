//! Replica-local branch knowledge, distinct from revision creation/receipt time.
use super::*;
use serde::{Deserialize, Serialize};

pub const MAX_RECORDED_OBSERVATIONS: usize = 100_000;
pub const MAX_HISTORY_RANGE: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    Baseline,
    Committed,
    Accepted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecordedObservation {
    pub observer: String,
    pub checkpoint: String,
    pub graph: GraphRef,
    pub branch_id: String,
    pub recorded_at_ms: i64,
    pub kind: ObservationKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordedCut {
    Checkpoint {
        observer: String,
        checkpoint: String,
    },
    AtTime {
        observer: String,
        unix_millis: i64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordedQueryResult {
    pub observation: RecordedObservation,
    pub result: QueryResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecordedHistoryRange {
    pub interval: Interval,
    pub start_state: RecordedObservation,
    pub changes: Vec<RecordedObservation>,
}

struct StoredObservation {
    value: RecordedObservation,
    parent: Option<String>,
}
type ObservationRow = (String, String, String, String, Option<String>, i64, String);

fn unavailable() -> Error {
    err("E_HISTORY_UNAVAILABLE", "recorded observation unavailable")
}
fn kind_name(kind: &ObservationKind) -> &'static str {
    match kind {
        ObservationKind::Baseline => "baseline",
        ObservationKind::Committed => "committed",
        ObservationKind::Accepted => "accepted",
    }
}
fn identity(value: &RecordedObservation, parent: &Option<String>) -> Result<String> {
    Ok(format!(
        "observation:{:x}",
        Sha256::digest(serde_json::to_vec(&(
            "weave-head-observation-v1",
            &value.observer,
            &value.graph,
            &value.branch_id,
            parent,
            value.recorded_at_ms,
            kind_name(&value.kind),
        ))?)
    ))
}
fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ObservationRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}
const COLUMNS: &str = "substr(id,1,513),substr(graph_id,1,513),substr(branch_id,1,513),substr(revision,1,513),substr(parent,1,513),recorded_at_ms,substr(kind,1,33)";

impl Engine {
    pub(crate) fn initialize_recorded_history(&self, old_version: i64) -> Result<()> {
        if old_version == STORAGE_VERSION && !self.conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='head_observations')", [], |r| r.get::<_, bool>(0))? {
            return Err(err("E_INTEGRITY", "recorded history table unavailable"));
        }
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS head_observations(id TEXT PRIMARY KEY,graph_id TEXT NOT NULL,branch_id TEXT NOT NULL,revision TEXT NOT NULL REFERENCES revisions(revision),parent TEXT REFERENCES head_observations(id),recorded_at_ms INTEGER NOT NULL CHECK(recorded_at_ms>=0),kind TEXT NOT NULL CHECK(kind IN ('baseline','committed','accepted')));
CREATE INDEX IF NOT EXISTS head_observations_scope ON head_observations(graph_id,branch_id,recorded_at_ms);")?;
        if old_version < STORAGE_VERSION
            && self
                .conn
                .query_row("SELECT EXISTS(SELECT 1 FROM heads)", [], |r| {
                    r.get::<_, bool>(0)
                })?
        {
            let _clock = self.operation_write_scope()?;
            let mut statement = self.conn.prepare("SELECT substr(graph_id,1,513),substr(branch_id,1,513),substr(revision,1,513) FROM heads ORDER BY graph_id,branch_id")?;
            let rows = statement.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            for item in rows {
                let (graph_id, branch_id, revision) = item?;
                if !valid_id(&graph_id) || !valid_id(&branch_id) || !valid_id(&revision) {
                    return Err(err("E_INTEGRITY", "invalid legacy head identity"));
                }
                self.append_observation(
                    &GraphRef { graph_id, revision },
                    &branch_id,
                    ObservationKind::Baseline,
                )?;
            }
        }
        Ok(())
    }
    fn decode_observation(&self, row: ObservationRow) -> Result<StoredObservation> {
        let (checkpoint, graph_id, branch_id, revision, parent, recorded_at_ms, kind) = row;
        if [&checkpoint, &graph_id, &branch_id, &revision]
            .iter()
            .any(|s| !valid_id(s))
            || parent.as_ref().is_some_and(|s| !valid_id(s))
            || recorded_at_ms < 0
        {
            return Err(err("E_INTEGRITY", "invalid recorded observation"));
        }
        let kind = match kind.as_str() {
            "baseline" => ObservationKind::Baseline,
            "committed" => ObservationKind::Committed,
            "accepted" => ObservationKind::Accepted,
            _ => return Err(err("E_INTEGRITY", "invalid recorded observation")),
        };
        let value = RecordedObservation {
            observer: self.runtime_source_identity()?,
            checkpoint,
            graph: GraphRef { graph_id, revision },
            branch_id,
            recorded_at_ms,
            kind,
        };
        self.read_budget.charge(json_size(&value, 8192)?)?;
        if identity(&value, &parent)? != value.checkpoint {
            return Err(err("E_INTEGRITY", "recorded observation digest mismatch"));
        }
        if let Some(parent) = &parent {
            self.read_budget.request()?;
            let valid: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM head_observations p JOIN head_observations c ON c.id=?1 WHERE p.id=?2 AND p.graph_id=c.graph_id AND p.branch_id=c.branch_id AND p.recorded_at_ms<=c.recorded_at_ms AND p.id=(SELECT id FROM head_observations WHERE graph_id=c.graph_id AND branch_id=c.branch_id AND rowid<c.rowid ORDER BY rowid DESC LIMIT 1))", params![value.checkpoint, parent], |r| r.get(0))?;
            if !valid {
                return Err(err("E_INTEGRITY", "recorded predecessor unavailable"));
            }
        }
        Ok(StoredObservation { value, parent })
    }
    fn latest_observation(&self, graph: &str, branch: &str) -> Result<Option<StoredObservation>> {
        self.read_budget.request()?;
        self.conn.query_row(&format!("SELECT {COLUMNS} FROM head_observations WHERE graph_id=?1 AND branch_id=?2 ORDER BY rowid DESC LIMIT 1"),
            params![graph, branch], row).optional()?.map(|row| self.decode_observation(row)).transpose()
    }
    fn recorded_tip(&self, graph: &str, branch: &str) -> Result<StoredObservation> {
        let tip = self
            .latest_observation(graph, branch)?
            .ok_or_else(unavailable)?;
        if self.head(graph, branch)?.as_deref() != Some(&tip.value.graph.revision) {
            return Err(err("E_INTEGRITY", "head and recorded observation disagree"));
        }
        Ok(tip)
    }
    fn previous_observation(
        &self,
        current: &StoredObservation,
    ) -> Result<Option<StoredObservation>> {
        let Some(parent) = &current.parent else {
            return Ok(None);
        };
        self.read_budget.request()?;
        let row = self.conn.query_row(&format!("SELECT {COLUMNS} FROM head_observations WHERE id=?1 AND graph_id=?2 AND branch_id=?3"), params![parent, current.value.graph.graph_id, current.value.branch_id], row).optional()?.ok_or_else(|| err("E_INTEGRITY", "recorded predecessor unavailable"))?;
        Ok(Some(self.decode_observation(row)?))
    }
    fn append_observation(
        &self,
        reference: &GraphRef,
        branch: &str,
        kind: ObservationKind,
    ) -> Result<()> {
        let prior = self.latest_observation(&reference.graph_id, branch)?;
        let head = self.head(&reference.graph_id, branch)?;
        if let Some(prior) = &prior {
            if head.as_deref() != Some(&prior.value.graph.revision)
                || kind == ObservationKind::Baseline
            {
                return Err(err("E_INTEGRITY", "head and recorded observation disagree"));
            }
        } else if head.is_some() && kind != ObservationKind::Baseline {
            return Err(err("E_INTEGRITY", "head lacks recorded observation"));
        }
        let now = self.operation_time()?;
        if prior.as_ref().is_some_and(|p| p.value.recorded_at_ms > now) {
            return Err(err(
                "E_HISTORY_CLOCK",
                "recording clock moved backwards in this branch",
            ));
        }
        let count: i64 =
            self.conn
                .query_row("SELECT count(*) FROM head_observations", [], |r| r.get(0))?;
        if count >= MAX_RECORDED_OBSERVATIONS as i64 {
            return Err(err("E_BUDGET", "recorded observation capacity exhausted"));
        }
        let parent = prior.map(|p| p.value.checkpoint);
        let mut value = RecordedObservation {
            observer: self.runtime_source_identity()?,
            checkpoint: String::new(),
            graph: reference.clone(),
            branch_id: branch.into(),
            recorded_at_ms: now,
            kind,
        };
        value.checkpoint = identity(&value, &parent)?;
        self.conn.execute(
            "INSERT INTO head_observations VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                value.checkpoint,
                reference.graph_id,
                branch,
                reference.revision,
                parent,
                now,
                kind_name(&value.kind)
            ],
        )?;
        Ok(())
    }
    pub(crate) fn advance_head(
        &self,
        reference: &GraphRef,
        branch: &str,
        kind: ObservationKind,
    ) -> Result<()> {
        self.append_observation(reference, branch, kind)?;
        self.conn.execute("INSERT INTO heads VALUES (?1,?2,?3) ON CONFLICT(graph_id,branch_id) DO UPDATE SET revision=excluded.revision",
            params![reference.graph_id, branch, reference.revision])?;
        Ok(())
    }
    fn validate_history_scope(
        &self,
        graph: &str,
        branch: &str,
        observer: &str,
        host: &HostContext,
    ) -> Result<()> {
        if !valid_id(graph)
            || !valid_id(branch)
            || !valid_id(&host.principal)
            || !valid_id(observer)
        {
            return Err(err("E_ID", "invalid recorded selection identity"));
        }
        if observer != self.runtime_source_identity()? {
            return Err(unavailable());
        }
        Ok(())
    }
    fn authorize_observation(
        &self,
        observation: &RecordedObservation,
        host: &HostContext,
    ) -> Result<()> {
        let reference = &observation.graph;
        if !self.protected_reference_allowed(&reference.graph_id, &reference.revision, host)? {
            return Err(unavailable());
        }
        let data = self
            .load(&reference.graph_id, &reference.revision)?
            .ok_or_else(unavailable)?;
        let (visible, incomplete) = self.authorized(data.clone(), host)?;
        if !whole_graph_visible(&data, visible, incomplete) {
            return Err(unavailable());
        }
        Ok(())
    }
    fn select_observation(
        &self,
        graph: &str,
        branch: &str,
        cut: &RecordedCut,
        host: &HostContext,
    ) -> Result<RecordedObservation> {
        let observer = match cut {
            RecordedCut::Checkpoint { observer, .. } | RecordedCut::AtTime { observer, .. } => {
                observer
            }
        };
        self.validate_history_scope(graph, branch, observer, host)?;
        let tip = self.recorded_tip(graph, branch)?;
        let selected = match cut {
            RecordedCut::Checkpoint { checkpoint, .. } => {
                if !valid_id(checkpoint) {
                    return Err(err("E_ID", "invalid recorded checkpoint"));
                }
                self.read_budget.request()?;
                let row = self.conn.query_row(&format!("SELECT {COLUMNS} FROM head_observations WHERE graph_id=?1 AND branch_id=?2 AND id=?3"), params![graph, branch, checkpoint], row).optional()?.ok_or_else(unavailable)?;
                self.decode_observation(row)?.value
            }
            RecordedCut::AtTime { unix_millis, .. } => {
                if *unix_millis < 0 || *unix_millis > self.operation_time()? {
                    return Err(err(
                        "E_HISTORY_TIME",
                        "recording cut is outside the observed clock",
                    ));
                }
                // Follow the authenticated predecessor path. A date predicate
                // alone could skip a deleted/corrupt intermediate observation.
                let mut current = tip;
                loop {
                    if current.value.recorded_at_ms <= *unix_millis {
                        break current.value;
                    }
                    current = self
                        .previous_observation(&current)?
                        .ok_or_else(unavailable)?;
                }
            }
        };
        self.authorize_observation(&selected, host)?;
        Ok(selected)
    }
    /// Current whole-authorized branch observation; no global sequence or actor inventory.
    pub fn recorded_checkpoint_for(
        &self,
        graph: &str,
        branch: &str,
        host: &HostContext,
    ) -> Result<RecordedObservation> {
        let _transaction = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.validate_history_scope(graph, branch, &self.runtime_source_identity()?, host)?;
        let selected = self.recorded_tip(graph, branch)?.value;
        self.authorize_observation(&selected, host)?;
        Ok(selected)
    }
    /// Resolve one replica-local cut, then evaluate the exact selected revision.
    pub fn query_recorded_for(
        &self,
        query: &QueryPlan,
        cut: &RecordedCut,
        host: &HostContext,
    ) -> Result<RecordedQueryResult> {
        if query.revision.is_some() {
            return Err(err(
                "E_HISTORY_SELECTOR",
                "choose a revision or recorded cut",
            ));
        }
        let _transaction = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        let observation = self.select_observation(&query.graph_id, &query.branch_id, cut, host)?;
        let mut pinned = query.clone();
        pinned.revision = Some(observation.graph.revision.clone());
        let result = self.query(&pinned, host)?;
        Ok(RecordedQueryResult {
            observation,
            result,
        })
    }
    /// Authorized state at start plus every change in a half-open recorded interval.
    pub fn recorded_range_for(
        &self,
        graph: &str,
        branch: &str,
        observer: &str,
        interval: &Interval,
        limit: usize,
        host: &HostContext,
    ) -> Result<RecordedHistoryRange> {
        if !interval.valid() || interval.end.is_none() || !(1..=MAX_HISTORY_RANGE).contains(&limit)
        {
            return Err(err(
                "E_HISTORY_RANGE",
                "finite recorded interval and bounded limit required",
            ));
        }
        let _transaction = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        self.validate_history_scope(graph, branch, observer, host)?;
        let end = interval.end.expect("validated finite end");
        if end > self.operation_time()? {
            return Err(err(
                "E_HISTORY_TIME",
                "recording range exceeds the observed clock",
            ));
        }
        let start_state = self.select_observation(
            graph,
            branch,
            &RecordedCut::AtTime {
                observer: observer.into(),
                unix_millis: interval.start,
            },
            host,
        )?;
        let mut changes = Vec::new();
        let mut current = Some(self.recorded_tip(graph, branch)?);
        while let Some(selected) = current {
            if selected.value.recorded_at_ms < interval.start {
                break;
            }
            if selected.value.recorded_at_ms < end {
                self.authorize_observation(&selected.value, host)?;
                if changes.len() == limit {
                    // A bounded denial must not reveal an extra hidden event.
                    return Err(unavailable());
                }
                changes.push(selected.value.clone());
            }
            current = self.previous_observation(&selected)?;
        }
        changes.reverse();
        Ok(RecordedHistoryRange {
            interval: interval.clone(),
            start_state,
            changes,
        })
    }
}
