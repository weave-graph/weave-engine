//! Fleet questions answered with the existing query operators.
//!
//! Each helper runs ordinary engine programs under the caller's host
//! authority, so per-record `readers` apply and hidden facts never appear.
//! [`At`] selects the state read: the branch head, a pinned revision, or the
//! graph as recorded at a Unix millisecond (`recorded_query`, or `as_of` for
//! readers who cannot see every record). The A3 operators
//! (`expand`, `changes`) will answer some of these questions in one plan;
//! until then these helpers compose filtered reads.
use crate::mapping::node_id;
use crate::schema::{self, SERIES_KEY};
use crate::series::SeriesRef;
use crate::{error, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use weave_contract::provenance::ProvenanceEntry;
use weave_contract::{CommandResult, Coverage, Program, QueryResult, VERSION};
use weave_engine::{Engine, HostContext};

/// Which recorded state a read selects ("as known when").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum At {
    /// The branch head when the read starts.
    #[default]
    Head,
    /// An exact revision, for example one an import report returned.
    Revision(String),
    /// The last head this replica recorded at or before this Unix
    /// millisecond (`recorded_query` with `local_time`). The engine admits
    /// it only for a reader who can see the whole selected revision
    /// (ADR-0002); others get `E_HISTORY_UNAVAILABLE`.
    Recorded(i64),
    /// The same recorded-time selection through the A3 `as_of` expression:
    /// the revision is authorized record by record, so a reader who cannot see
    /// restricted units gets the visible subset as known at this millisecond.
    /// No checkpoint witness is returned.
    AsOf(i64),
}

/// One filtered read of `graph` on `branch`.
#[derive(Debug, Clone, Default)]
pub struct Read<'a> {
    pub graph: &'a str,
    pub branch: &'a str,
    pub predicate: Option<&'a str>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub valid_at: Option<i64>,
    pub at: At,
}

/// Run one read and return its result.
pub fn read(engine: &mut Engine, host: &HostContext, read: &Read<'_>) -> Result<QueryResult> {
    let revision = match &read.at {
        At::Revision(r) => Some(r.clone()),
        _ => None,
    };
    let plan = json!({
        "graph_id": read.graph,
        "revision": revision,
        "branch_id": read.branch,
        "predicate": read.predicate,
        "from": read.from,
        "to": read.to,
        "valid_at": read.valid_at,
    });
    let command = match read.at {
        At::Head | At::Revision(_) => json!({"op": "query", "query": plan}),
        At::Recorded(k) => json!({"op": "evaluate", "value": {
            "kind": "recorded_query", "query": plan,
            "selection": {"kind": "local_time", "unix_millis": k}
        }}),
        At::AsOf(k) => json!({"op": "evaluate", "value": {
            "kind": "as_of", "query": plan, "unix_millis": k
        }}),
    };
    let program: Program =
        serde_json::from_value(json!({"version": VERSION, "commands": [command]}))
            .map_err(|e| error("E_HW_QUERY", &e.to_string()))?;
    match engine.execute(&program, host)?.into_iter().next() {
        Some(CommandResult::Queried { result }) => Ok(*result),
        _ => Err(error("E_HW_QUERY", "the read returned no query result")),
    }
}

/// An answer with the facts it rests on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Answer {
    /// Unit serial numbers, sorted.
    pub units: Vec<String>,
    /// For each unit, the edge IDs that support it.
    pub support: BTreeMap<String, Vec<String>>,
    /// Engine-recorded provenance of every supporting fact.
    pub provenance: Vec<ProvenanceEntry>,
    /// `partial` when any read was partial.
    pub coverage: Coverage,
}

fn serial_of(node: &str) -> Option<&str> {
    node.strip_prefix(schema::UNIT.prefix)?.strip_prefix(':')
}

fn cited(result: &QueryResult, ids: &BTreeSet<String>) -> Vec<ProvenanceEntry> {
    result
        .fact_provenance
        .iter()
        .filter(|entry| match &entry.fact {
            weave_contract::provenance::FactRef::Assertion { assertion_id, .. } => {
                ids.contains(assertion_id)
            }
            _ => false,
        })
        .cloned()
        .collect()
}

/// "Which units at `site` ran `firmware` at valid time `valid_at`, as known
/// at `at`?" Two indexed reads (`hw:runs` to the firmware and
/// `hw:located_at` to the site, both at `valid_at`) intersected by unit.
pub fn units_at_site_running(
    engine: &mut Engine,
    host: &HostContext,
    graph: &str,
    site: &str,
    firmware: &str,
    valid_at: i64,
    at: &At,
) -> Result<Answer> {
    let base = Read {
        graph,
        branch: "main",
        valid_at: Some(valid_at),
        at: at.clone(),
        ..Read::default()
    };
    let runs = read(
        engine,
        host,
        &Read {
            predicate: Some("hw:runs"),
            to: Some(node_id(&schema::FIRMWARE, firmware)),
            ..base.clone()
        },
    )?;
    let located = read(
        engine,
        host,
        &Read {
            predicate: Some("hw:located_at"),
            to: Some(node_id(&schema::SITE, site)),
            ..base
        },
    )?;
    let by_unit = |result: &QueryResult| -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for edge in &result.graph.edges {
            if let Some(serial) = serial_of(&edge.from) {
                out.entry(serial.into()).or_default().push(edge.id.clone());
            }
        }
        out
    };
    let (runs_by, located_by) = (by_unit(&runs), by_unit(&located));
    let mut support = BTreeMap::new();
    for (unit, mut edges) in runs_by {
        if let Some(more) = located_by.get(&unit) {
            edges.extend(more.iter().cloned());
            edges.sort();
            support.insert(unit, edges);
        }
    }
    let ids: BTreeSet<String> = support.values().flatten().cloned().collect();
    let mut provenance = cited(&runs, &ids);
    provenance.extend(cited(&located, &ids));
    let coverage = if runs.coverage == Coverage::Complete && located.coverage == Coverage::Complete
    {
        Coverage::Complete
    } else {
        Coverage::Partial
    };
    Ok(Answer {
        units: support.keys().cloned().collect(),
        support,
        provenance,
        coverage,
    })
}

/// A series reference found by [`series_overlapping`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeriesHit {
    /// The `hw:series` attachment ID.
    pub id: String,
    pub sensor: String,
    /// Units the sensor was mounted on during the overlap, sorted.
    pub units: Vec<String>,
    pub series: SeriesRef,
}

/// Series references whose range overlaps `[start, end)` (`end: None` is
/// unbounded), on the sensors of `unit` or of every unit. Reads the
/// `hw:measured_by` edges (indexed by predicate and unit) with the sensors
/// and their attachments, then keeps references whose half-open range
/// shares an instant with the window. A sensor's units are those whose
/// mount interval overlaps both the window and the series range.
pub fn series_overlapping(
    engine: &mut Engine,
    host: &HostContext,
    graph: &str,
    unit: Option<&str>,
    start: i64,
    end: Option<i64>,
    at: &At,
) -> Result<(Vec<SeriesHit>, Coverage)> {
    if end.is_some_and(|e| e <= start) {
        return Err(error("E_INTERVAL", "window end must exceed its start"));
    }
    let result = read(
        engine,
        host,
        &Read {
            graph,
            branch: "main",
            predicate: Some("hw:measured_by"),
            from: unit.map(|u| node_id(&schema::UNIT, u)),
            at: at.clone(),
            ..Read::default()
        },
    )?;
    let overlaps = |s: i64, e: Option<i64>, s2: i64, e2: Option<i64>| {
        s < e2.unwrap_or(i64::MAX) && s2 < e.unwrap_or(i64::MAX)
    };
    let mut hits = Vec::new();
    for attachment in &result.graph.attachments {
        if attachment.key != SERIES_KEY {
            continue;
        }
        let weave_contract::MetadataHost::Node { id: sensor } = &attachment.host else {
            continue;
        };
        let series = SeriesRef::from_attachment(attachment)?;
        if !series.overlaps(start, end) {
            continue;
        }
        let lo = start.max(series.start);
        let hi = end.map_or(series.end, |e| e.min(series.end));
        let mut units: Vec<String> = result
            .graph
            .edges
            .iter()
            .filter(|e| &e.to == sensor)
            .filter(|e| overlaps(e.valid_time.start, e.valid_time.end, lo, Some(hi)))
            .filter_map(|e| serial_of(&e.from).map(str::to_string))
            .collect();
        units.sort();
        units.dedup();
        if unit.is_some() && units.is_empty() {
            continue;
        }
        hits.push(SeriesHit {
            id: attachment.id.clone(),
            sensor: sensor.clone(),
            units,
            series,
        });
    }
    hits.sort_by(|a, b| a.id.cmp(&b.id));
    Ok((hits, result.coverage))
}

/// How one record changed between two recorded times.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Corrected,
}

/// One changed record of a unit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct UnitChange {
    /// `node`, `edge` or `attachment`.
    pub kind: String,
    pub id: String,
    pub change: ChangeKind,
}

fn records(result: &QueryResult, unit_node: &str) -> BTreeMap<(String, String), Value> {
    let mut out = BTreeMap::new();
    for node in &result.graph.nodes {
        if node.id == unit_node {
            // A read cites the node's own origin at the revision it read;
            // that names the read, not the fact, so it is not compared.
            let mut node = node.clone();
            node.derived_nodes.retain(|r| r.node_id != unit_node);
            out.insert(
                ("node".into(), node.id.clone()),
                serde_json::to_value(&node).unwrap_or(Value::Null),
            );
        }
    }
    for edge in &result.graph.edges {
        if edge.from == unit_node || edge.to == unit_node {
            out.insert(
                ("edge".into(), edge.id.clone()),
                serde_json::to_value(edge).unwrap_or(Value::Null),
            );
        }
    }
    for attachment in &result.graph.attachments {
        if attachment.host
            == (weave_contract::MetadataHost::Node {
                id: unit_node.into(),
            })
        {
            out.insert(
                ("attachment".into(), attachment.id.clone()),
                serde_json::to_value(attachment).unwrap_or(Value::Null),
            );
        }
    }
    out
}

/// "What changed for `unit` between `before` and `after`?" The unit node,
/// every edge from or to it and its attachments, as known at each state,
/// compared by ID: added, removed, or corrected (a new version). A change of
/// valid time only (a firmware rollout closing an install) is a correction.
pub fn unit_changes(
    engine: &mut Engine,
    host: &HostContext,
    graph: &str,
    unit: &str,
    before: &At,
    after: &At,
) -> Result<Vec<UnitChange>> {
    let unit_node = node_id(&schema::UNIT, unit);
    let mut state = |at: &At| -> Result<BTreeMap<(String, String), Value>> {
        let mut all = BTreeMap::new();
        for (from, to) in [
            (Some(unit_node.clone()), None),
            (None, Some(unit_node.clone())),
        ] {
            let result = read(
                engine,
                host,
                &Read {
                    graph,
                    branch: "main",
                    from,
                    to,
                    at: at.clone(),
                    ..Read::default()
                },
            )?;
            all.extend(records(&result, &unit_node));
        }
        Ok(all)
    };
    let (before, after) = (state(before)?, state(after)?);
    let mut changes = Vec::new();
    for (key, value) in &after {
        match before.get(key) {
            None => changes.push((key, ChangeKind::Added)),
            Some(old) if old != value => changes.push((key, ChangeKind::Corrected)),
            _ => {}
        }
    }
    for key in before.keys() {
        if !after.contains_key(key) {
            changes.push((key, ChangeKind::Removed));
        }
    }
    let mut out: Vec<UnitChange> = changes
        .into_iter()
        .map(|((kind, id), change)| UnitChange {
            kind: kind.clone(),
            id: id.clone(),
            change,
        })
        .collect();
    out.sort();
    Ok(out)
}
