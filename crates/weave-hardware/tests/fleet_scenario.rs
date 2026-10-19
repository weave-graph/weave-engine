//! The Phase A exit scenario on the generated fleet, as far as the current
//! operators reach (A5): import the base export, re-import it (zero facts),
//! apply every update as its own fact write, then answer "which units at
//! site S ran firmware F on date D, as known on date K" and "what changed
//! for unit U between K1 and K2" against an independent oracle computed from
//! the rows, for a restricted and an unrestricted reader, before and after
//! reopening the store.
//!
//! Size: `WEAVE_FLEET_UNITS` (default 150) and `WEAVE_FLEET_UPDATES`
//! (default 150). The `hardware-fleet-5k` CI job runs 5,000 and 1,000.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use weave_contract::provenance::{FactRef, Method};
use weave_engine::{Engine, HostContext, ManualClock};
use weave_hardware::generate::{base_time, generate, updates_end, Config, Dataset, ADMIN, ANALYST};
use weave_hardware::import::{apply_updates, import_batch, read_csv_dir, Target};
use weave_hardware::query::{
    series_overlapping, unit_changes, units_at_site_running, At, ChangeKind,
};
use weave_hardware::table::{parse_time, Row};

const WRITER: &str = "etl-importer";
const GRAPH: &str = "fleet";

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn reader(principal: &str) -> HostContext {
    HostContext::new(principal, Vec::<String>::new())
}

/// The rows known at recorded time `k`: the base export (recorded at the
/// base time) and every update reported at or before `k`.
struct Oracle {
    tables: BTreeMap<String, BTreeMap<String, Row>>,
}

fn key_column(table: &str) -> &'static str {
    weave_hardware::schema::table(table).unwrap().key
}

impl Oracle {
    fn known_at(dataset: &Dataset, k: i64) -> Self {
        let mut tables: BTreeMap<String, BTreeMap<String, Row>> = BTreeMap::new();
        let mut put = |table: &str, row: &Row| {
            tables
                .entry(table.into())
                .or_default()
                .insert(row[key_column(table)].clone(), row.clone());
        };
        if k >= base_time() {
            for (table, rows) in &dataset.base.tables {
                rows.iter().for_each(|r| put(table, r));
            }
        }
        for update in &dataset.updates {
            if parse_time(&update.reported_at).unwrap() <= k {
                for (table, rows) in &update.batch.tables {
                    rows.iter().for_each(|r| put(table, r));
                }
            }
        }
        Self { tables }
    }
    fn rows(&self, table: &str) -> impl Iterator<Item = &Row> {
        self.tables.get(table).into_iter().flat_map(|t| t.values())
    }
    fn admits(row: &Row, principal: &str) -> bool {
        row.get("readers")
            .is_none_or(|r| r.split(';').any(|p| p == principal))
    }
    fn contains(row: &Row, start: &str, end: &str, t: i64) -> bool {
        let s = row.get(start).map_or(0, |v| parse_time(v).unwrap());
        let e = row.get(end).map(|v| parse_time(v).unwrap());
        s <= t && e.is_none_or(|e| t < e)
    }
    fn units_at_site_running(&self, site: &str, fw: &str, d: i64, who: &str) -> Vec<String> {
        let units = &self.tables["units"];
        let mut out = BTreeSet::new();
        for install in self.rows("firmware_installs") {
            if install["firmware_id"] != fw
                || !Self::contains(install, "valid_from", "valid_to", d)
                || !Self::admits(install, who)
            {
                continue;
            }
            let serial = &install["serial"];
            let unit = &units[serial];
            if !Self::admits(unit, who)
                || !Self::contains(unit, "commissioned_at", "decommissioned_at", d)
            {
                continue;
            }
            if self.rows("placements").any(|p| {
                &p["serial"] == serial
                    && p["site_id"] == site
                    && Self::contains(p, "valid_from", "valid_to", d)
                    && Self::admits(p, who)
            }) {
                out.insert(serial.clone());
            }
        }
        out.into_iter().collect()
    }
    fn site_at(&self, serial: &str, d: i64) -> Option<String> {
        self.rows("placements")
            .find(|p| p["serial"] == serial && Self::contains(p, "valid_from", "valid_to", d))
            .map(|p| p["site_id"].clone())
    }
}

#[test]
fn phase_a_fleet_scenario() {
    let config = Config {
        units: env("WEAVE_FLEET_UNITS", 150),
        updates: env("WEAVE_FLEET_UPDATES", 150),
        ..Config::default()
    };
    let dataset = generate(&config);
    assert_eq!(dataset.updates.len(), config.updates);
    assert_eq!(dataset.base.tables["units"].len(), config.units);
    assert_eq!(
        generate(&config).digest(),
        dataset.digest(),
        "deterministic"
    );
    println!("dataset {config:?} digest {}", dataset.digest());

    // The import reads the CSV files, as a user would.
    let dir = tempfile::tempdir().unwrap();
    dataset.write_to(dir.path()).unwrap();
    let base = read_csv_dir(&dir.path().join("base"), None).unwrap();
    assert_eq!(base, dataset.base, "CSV round trip");

    let db = dir.path().join("fleet.db");
    let clock = Arc::new(ManualClock::new(base_time()));
    let mut engine = Engine::open_with_clock(&db, clock.clone()).unwrap();
    let writer = HostContext::new(WRITER, [GRAPH.to_string()]);
    let target = Target::new(GRAPH);
    let started = std::time::Instant::now();
    let first = import_batch(&mut engine, &writer, &target, &base).unwrap();
    println!(
        "base import: {} facts in {:?}",
        first.facts,
        started.elapsed()
    );
    assert!(first.initialized);
    assert_eq!(first.asserted, first.facts);

    // Re-running the identical import writes nothing.
    let events = engine.event_count().unwrap();
    let again = import_batch(&mut engine, &writer, &target, &base).unwrap();
    assert_eq!((again.written(), again.unchanged), (0, first.facts));
    assert!(again.revisions.is_empty());
    assert_eq!(again.head, first.head);
    assert_eq!(engine.event_count().unwrap(), events);

    // Every update is one fact write, recorded when it was reported.
    let started = std::time::Instant::now();
    let applied = apply_updates(&mut engine, &writer, &target, &dataset.updates, |u| {
        clock.set(parse_time(&u.reported_at).unwrap())
    })
    .unwrap();
    println!(
        "{} updates: {} asserted, {} corrected in {:?}",
        dataset.updates.len(),
        applied.asserted,
        applied.corrected,
        started.elapsed()
    );
    assert_eq!(applied.revisions.len(), dataset.updates.len());
    // The trusted importer knows which revision each write produced and when.
    let mut written: Vec<(i64, String)> = vec![(base_time(), first.head.clone().unwrap())];
    for (update, revision) in dataset.updates.iter().zip(&applied.revisions) {
        written.push((parse_time(&update.reported_at).unwrap(), revision.clone()));
    }
    let revision_at = |k: i64| -> String {
        written
            .iter()
            .rev()
            .find(|(t, _)| *t <= k)
            .unwrap()
            .1
            .clone()
    };
    assert!(applied.corrected > 0 && applied.asserted > 0);
    // Re-applying the last update is a no-op.
    let last = dataset.updates.last().unwrap();
    let replay = import_batch(&mut engine, &writer, &target, &last.batch).unwrap();
    assert_eq!(replay.written(), 0);
    clock.set(updates_end() + 86_400_000);

    let restricted: BTreeSet<String> = dataset.base.tables["units"]
        .iter()
        .filter(|u| u.contains_key("readers"))
        .map(|u| u["serial"].clone())
        .collect();
    assert!(
        !restricted.is_empty(),
        "the fleet has customer-restricted units"
    );

    let mut checked = 0;
    let mut changed_between = 0;
    let mut restricted_seen_by_admin = 0;
    let mut answers = Vec::new();
    let rollouts: Vec<_> = dataset
        .updates
        .iter()
        .filter(|u| u.kind == "firmware_rollout")
        .take(12)
        .collect();
    assert!(!rollouts.is_empty());
    for update in rollouts {
        let new = &update.batch.tables["firmware_installs"][1];
        let old = &update.batch.tables["firmware_installs"][0];
        let serial = &new["serial"];
        let fw = &new["firmware_id"];
        let d = parse_time(&new["valid_from"]).unwrap() + 3_600_000;
        let k2 = parse_time(&update.reported_at).unwrap();
        let k1 = k2 - 1;
        let site = Oracle::known_at(&dataset, k2).site_at(serial, d).unwrap();
        for who in [ADMIN, ANALYST] {
            let host = reader(who);
            let mut per_k = Vec::new();
            for k in [k1, k2] {
                let expected =
                    Oracle::known_at(&dataset, k).units_at_site_running(&site, fw, d, who);
                let pinned = At::Revision(revision_at(k));
                let answer =
                    units_at_site_running(&mut engine, &host, GRAPH, &site, fw, d, &pinned)
                        .unwrap();
                // "As known at K" by recorded time for both readers (A3
                // `as_of`): the analyst gets the visible subset.
                let as_known =
                    units_at_site_running(&mut engine, &host, GRAPH, &site, fw, d, &At::AsOf(k))
                        .unwrap();
                assert_eq!(as_known, answer, "as_of {k} as {who}");
                // `recorded_query` still needs the whole revision visible
                // (ADR-0002): the admin can use it, the analyst cannot.
                let recorded = units_at_site_running(
                    &mut engine,
                    &host,
                    GRAPH,
                    &site,
                    fw,
                    d,
                    &At::Recorded(k),
                );
                if who == ADMIN {
                    assert_eq!(recorded.unwrap().units, answer.units);
                } else {
                    assert_eq!(recorded.unwrap_err().code, "E_HISTORY_UNAVAILABLE");
                }
                assert_eq!(
                    answer.units, expected,
                    "site {site} fw {fw} d {d} k {k} as {who}"
                );
                if who == ANALYST {
                    assert!(answer.units.iter().all(|u| !restricted.contains(u)));
                } else {
                    restricted_seen_by_admin += answer
                        .units
                        .iter()
                        .filter(|u| restricted.contains(*u))
                        .count();
                }
                // Every answer explains its source facts.
                let cited: BTreeSet<String> = answer
                    .provenance
                    .iter()
                    .filter_map(|e| match &e.fact {
                        FactRef::Assertion { assertion_id, .. } => Some(assertion_id.clone()),
                        _ => None,
                    })
                    .collect();
                for edges in answer.support.values() {
                    for edge in edges {
                        assert!(cited.contains(edge), "provenance for {edge}");
                    }
                }
                for entry in &answer.provenance {
                    assert_eq!(entry.provenance.writer, WRITER);
                    assert!(matches!(entry.provenance.method, Method::Import { .. }));
                }
                if let Some(entry) = answer.provenance.iter().find(|e| {
                    matches!(&e.fact, FactRef::Assertion { assertion_id, .. }
                        if assertion_id == &format!("runs:{}", new["install_id"]))
                }) {
                    assert_eq!(entry.provenance.source.as_deref(), Some("fleet-ops"));
                }
                answers.push((
                    site.clone(),
                    fw.clone(),
                    d,
                    pinned,
                    who,
                    answer.units.clone(),
                ));
                per_k.push(answer.units);
                checked += 1;
            }
            if per_k[0] != per_k[1] {
                changed_between += 1;
            }
            // What changed for the unit between K1 and K2: this rollout.
            let changes = unit_changes(
                &mut engine,
                &host,
                GRAPH,
                serial,
                &At::Revision(revision_at(k1)),
                &At::Revision(revision_at(k2)),
            )
            .unwrap();
            let visible = who == ADMIN || !restricted.contains(serial);
            let concurrent = dataset
                .updates
                .iter()
                .any(|u| u.reported_at == update.reported_at && u.seq != update.seq);
            if visible && !concurrent {
                let got: BTreeSet<(String, ChangeKind)> = changes
                    .iter()
                    .map(|c| (c.id.clone(), c.change.clone()))
                    .collect();
                let want: BTreeSet<(String, ChangeKind)> = [
                    (format!("runs:{}", old["install_id"]), ChangeKind::Corrected),
                    (format!("runs:{}", new["install_id"]), ChangeKind::Added),
                ]
                .into();
                assert_eq!(got, want, "changes of {serial} as {who}");
            }
            if !visible {
                assert!(
                    changes.is_empty(),
                    "restricted unit {serial} is hidden from {who}"
                );
            }
        }
    }
    assert!(checked > 0);
    assert!(
        changed_between > 0,
        "some answer depends on the recorded time"
    );
    assert!(restricted_seen_by_admin > 0 || restricted.len() * 10 < config.units);

    // Series references are hidden with their restricted units.
    let restricted_unit = restricted.iter().next().unwrap();
    let all = (i64::MIN / 2, None);
    let (admin_hits, _) = series_overlapping(
        &mut engine,
        &reader(ADMIN),
        GRAPH,
        Some(restricted_unit),
        all.0,
        all.1,
        &At::Head,
    )
    .unwrap();
    let (analyst_hits, _) = series_overlapping(
        &mut engine,
        &reader(ANALYST),
        GRAPH,
        Some(restricted_unit),
        all.0,
        all.1,
        &At::Head,
    )
    .unwrap();
    assert!(!admin_hits.is_empty());
    assert!(analyst_hits.is_empty());

    // Answers are reproducible after reopening the store.
    drop(engine);
    let mut reopened = Engine::open_with_clock(&db, clock.clone()).unwrap();
    for (site, fw, d, at, who, units) in answers {
        let answer =
            units_at_site_running(&mut reopened, &reader(who), GRAPH, &site, &fw, d, &at).unwrap();
        assert_eq!(answer.units, units);
    }
}
