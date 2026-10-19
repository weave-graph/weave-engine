//! Incremental import: rows become facts, facts are reconciled against the
//! branch head, and only new or changed facts are written (A2 `write`).
use crate::mapping::batch_facts;
use crate::schema::{self, graph_schema};
use crate::table::{parse_csv, Batch, Update};
use crate::{error, Result};
use serde::Serialize;
use std::path::Path;
use weave_contract::facts::{Fact, FactChange, MAX_WRITE_CHANGES};
use weave_contract::{Command, CommandResult, GraphData, Program, QueryPlan, VERSION};
use weave_engine::{Engine, HostContext};

/// Where an import writes.
#[derive(Debug, Clone)]
pub struct Target {
    pub graph: String,
    pub branch: String,
}

impl Target {
    pub fn new(graph: impl Into<String>) -> Self {
        Self {
            graph: graph.into(),
            branch: "main".into(),
        }
    }
}

/// What one import did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    /// Facts the input maps to.
    pub facts: usize,
    /// Facts that were not live and were asserted.
    pub asserted: usize,
    /// Live facts that differed and were corrected (new versions).
    pub corrected: usize,
    /// Live facts that were already identical; nothing was written for them.
    pub unchanged: usize,
    /// Fact revisions written, in order. Empty when nothing changed.
    pub revisions: Vec<String>,
    /// True when this import created the graph with the model's schema.
    pub initialized: bool,
    /// Branch head after the import.
    pub head: Option<String>,
}

impl ImportReport {
    /// Facts written: asserted plus corrected.
    pub fn written(&self) -> usize {
        self.asserted + self.corrected
    }
    fn absorb(&mut self, other: ImportReport) {
        self.facts += other.facts;
        self.asserted += other.asserted;
        self.corrected += other.corrected;
        self.unchanged += other.unchanged;
        self.revisions.extend(other.revisions);
        self.initialized |= other.initialized;
        self.head = other.head;
    }
}

fn program(command: Command) -> Program {
    Program {
        source_revisions: vec![],
        version: VERSION.into(),
        commands: vec![command],
    }
}

/// Create the graph with the reference schema if the branch has no head. If
/// it has one, check that its schema is the reference schema.
pub fn ensure_graph(engine: &mut Engine, host: &HostContext, target: &Target) -> Result<bool> {
    if engine.head(&target.graph, &target.branch)?.is_none() {
        let data = GraphData {
            schema: Some(graph_schema()),
            ..GraphData::default()
        };
        engine.execute(
            &program(Command::Commit {
                graph_id: target.graph.clone(),
                branch_id: target.branch.clone(),
                expected_head: None,
                data,
            }),
            host,
        )?;
        return Ok(true);
    }
    // A predicate no edge has selects only the graph header.
    let results = engine.execute(
        &program(Command::Query {
            query: QueryPlan {
                graph_id: target.graph.clone(),
                revision: None,
                branch_id: target.branch.clone(),
                predicate: Some("hw:header-only".into()),
                from: None,
                to: None,
                valid_at: None,
                include_metadata: false,
                max_depth: 8,
            },
        }),
        host,
    )?;
    let schema = match results.first() {
        Some(CommandResult::Queried { result }) => result.graph.schema.clone(),
        _ => None,
    };
    if schema.as_ref() != Some(&graph_schema()) {
        return Err(error(
            "E_HW_SCHEMA",
            "the graph exists without the hardware reference schema of this importer",
        ));
    }
    Ok(false)
}

/// Write `facts` incrementally: reconcile them against the head and write
/// only the asserts and corrections, in chunks of at most 100,000 changes,
/// each one fact revision guarded by `expected_head`.
pub fn import_facts(
    engine: &mut Engine,
    host: &HostContext,
    target: &Target,
    facts: &[Fact],
) -> Result<ImportReport> {
    let mut report = ImportReport {
        facts: facts.len(),
        initialized: ensure_graph(engine, host, target)?,
        ..ImportReport::default()
    };
    for chunk in facts.chunks(MAX_WRITE_CHANGES) {
        let plan = engine.reconcile_facts(&target.graph, &target.branch, chunk, host)?;
        report.unchanged += chunk.len() - plan.changes.len();
        for change in &plan.changes {
            match change {
                FactChange::Assert { .. } => report.asserted += 1,
                _ => report.corrected += 1,
            }
        }
        if plan.changes.is_empty() {
            continue;
        }
        let results = engine.execute(
            &program(Command::Write {
                graph_id: target.graph.clone(),
                branch_id: target.branch.clone(),
                expected_head: Some(plan.head),
                changes: plan.changes,
            }),
            host,
        )?;
        if let Some(CommandResult::Committed { revision, .. }) = results.first() {
            report.revisions.push(revision.clone());
        }
    }
    report.head = engine.head(&target.graph, &target.branch)?;
    Ok(report)
}

/// Import one batch of rows.
pub fn import_batch(
    engine: &mut Engine,
    host: &HostContext,
    target: &Target,
    batch: &Batch,
) -> Result<ImportReport> {
    import_facts(engine, host, target, &batch_facts(batch)?)
}

/// Apply updates in order, each as its own import (one fact revision when it
/// changes anything). `before` runs before each update, for example to
/// advance a test clock.
pub fn apply_updates(
    engine: &mut Engine,
    host: &HostContext,
    target: &Target,
    updates: &[Update],
    mut before: impl FnMut(&Update),
) -> Result<ImportReport> {
    let mut total = ImportReport::default();
    for update in updates {
        before(update);
        total.absorb(import_batch(engine, host, target, &update.batch)?);
    }
    total.head = engine.head(&target.graph, &target.branch)?;
    Ok(total)
}

/// Read every `<table>.csv` in a directory. Files with other names are an
/// error, so a misspelt export is not silently skipped.
pub fn read_csv_dir(dir: &Path, source: Option<String>) -> Result<Batch> {
    let mut batch = Batch {
        source,
        ..Batch::default()
    };
    let entries = std::fs::read_dir(dir)
        .map_err(|e| error("E_HW_INPUT", &format!("cannot read {}: {e}", dir.display())))?;
    for entry in entries {
        let path = entry
            .map_err(|e| error("E_HW_INPUT", &e.to_string()))?
            .path();
        if path.extension().and_then(|e| e.to_str()) != Some("csv") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let table = schema::table(name)
            .ok_or_else(|| error("E_HW_INPUT", &format!("unknown table file {name}.csv")))?;
        let text = std::fs::read_to_string(&path).map_err(|e| {
            error(
                "E_HW_INPUT",
                &format!("cannot read {}: {e}", path.display()),
            )
        })?;
        batch
            .tables
            .insert(table.name.into(), parse_csv(table, &text)?);
    }
    Ok(batch)
}

/// Read an updates file: one JSON update per line, blank lines ignored.
pub fn read_updates(text: &str) -> Result<Vec<Update>> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(Update::parse)
        .collect()
}
