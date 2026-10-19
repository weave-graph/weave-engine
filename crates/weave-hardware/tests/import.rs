//! CSV and JSON import: idempotent re-runs, incremental and attributable
//! corrections, strict input checking and reader restrictions.
use serde_json::json;
use std::path::Path;
use weave_contract::provenance::{FactRef, Method};
use weave_contract::{CommandResult, Program, QueryResult, VERSION};
use weave_engine::{Engine, HostContext};
use weave_hardware::import::{ensure_graph, read_csv_dir, read_updates, Target};
use weave_hardware::table::{batch_from_value, parse_csv, parse_json_batch};
use weave_hardware::{import_batch, schema, Batch};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/fleet-small");

fn writer() -> HostContext {
    HostContext::new("etl-importer", ["fleet".to_string()])
}
fn reader(principal: &str) -> HostContext {
    HostContext::new(principal, Vec::<String>::new())
}
fn base() -> Batch {
    read_csv_dir(&Path::new(FIXTURE).join("base"), None).unwrap()
}
fn query(e: &mut Engine, who: &HostContext, plan: serde_json::Value) -> QueryResult {
    let program: Program = serde_json::from_value(
        json!({"version": VERSION, "commands": [{"op": "query", "query": plan}]}),
    )
    .unwrap();
    match e.execute(&program, who).unwrap().remove(0) {
        CommandResult::Queried { result } => *result,
        other => panic!("{other:?}"),
    }
}

#[test]
fn reimporting_identical_csv_or_json_writes_zero_facts() {
    let mut e = Engine::memory().unwrap();
    let target = Target::new("fleet");
    let batch = base();
    let first = import_batch(&mut e, &writer(), &target, &batch).unwrap();
    assert!(first.initialized);
    assert_eq!(first.asserted, first.facts);
    assert_eq!(first.revisions.len(), 1);
    let events = e.event_count().unwrap();

    let second = import_batch(&mut e, &writer(), &target, &batch).unwrap();
    assert_eq!(second.written(), 0);
    assert_eq!(second.unchanged, first.facts);
    assert!(second.revisions.is_empty());
    assert_eq!(second.head, first.head);
    assert_eq!(e.event_count().unwrap(), events, "no new revision or event");

    // The same rows as a JSON batch map to the same facts.
    let text = serde_json::to_string(&batch).unwrap();
    let from_json = parse_json_batch(&text).unwrap();
    assert_eq!(from_json, batch);
    let third = import_batch(&mut e, &writer(), &target, &from_json).unwrap();
    assert_eq!((third.written(), third.unchanged), (0, first.facts));

    // Updates apply incrementally; replaying an update writes nothing more.
    let updates =
        read_updates(&std::fs::read_to_string(Path::new(FIXTURE).join("updates.jsonl")).unwrap())
            .unwrap();
    assert_eq!(updates.len(), 24);
    for update in &updates {
        let report = import_batch(&mut e, &writer(), &target, &update.batch).unwrap();
        assert_eq!(
            report.revisions.len(),
            1,
            "update {} is one write",
            update.seq
        );
        assert!(report.written() <= 10, "an update is a handful of facts");
        let replay = import_batch(&mut e, &writer(), &target, &update.batch).unwrap();
        assert_eq!(replay.written(), 0);
        assert_eq!(replay.head, report.head);
    }
}

#[test]
fn a_changed_row_is_one_attributed_correction() {
    let mut e = Engine::memory().unwrap();
    let target = Target::new("fleet");
    let batch = base();
    let first = import_batch(&mut e, &writer(), &target, &batch).unwrap();
    let r1 = first.head.clone().unwrap();

    // A changed attribute corrects exactly the fact that holds it.
    let mut changed = batch.clone();
    let unit = &mut changed.tables.get_mut("units").unwrap()[0];
    let serial = unit["serial"].clone();
    unit.insert("name".into(), "Renamed unit".into());
    let report = import_batch(&mut e, &writer(), &target, &changed).unwrap();
    assert_eq!((report.asserted, report.corrected), (0, 1));
    assert_eq!(report.revisions.len(), 1);
    // A row is one claim: naming another source re-attributes every fact
    // of the row (the unit node and its `hw:has_model` edge).
    let unit = &mut changed.tables.get_mut("units").unwrap()[0];
    unit.insert("source".into(), "data-steward".into());
    unit.insert("evidence_uri".into(), "ticket://DS-17".into());
    unit.insert("evidence_digest".into(), "sha256:00".into());
    let report = import_batch(&mut e, &writer(), &target, &changed).unwrap();
    assert_eq!((report.asserted, report.corrected), (0, 2));
    let r2 = report.head.clone().unwrap();
    assert_ne!(r1, r2);

    let node = format!("unit:{serial}");
    let now = query(
        &mut e,
        &reader("fleet-admin"),
        json!({"graph_id": "fleet", "from": node}),
    );
    let n = now.graph.nodes.iter().find(|n| n.id == node).unwrap();
    assert_eq!(n.properties["name"], "Renamed unit");
    let entry = now
        .fact_provenance
        .iter()
        .find(|p| matches!(&p.fact, FactRef::Node { node_id, .. } if node_id == &node))
        .unwrap();
    assert_eq!(entry.provenance.writer, "etl-importer");
    assert_eq!(entry.provenance.source.as_deref(), Some("data-steward"));
    assert!(
        matches!(&entry.provenance.method, Method::Import { importer, version }
        if importer == "weave-hardware" && version == "1")
    );
    assert_eq!(entry.provenance.evidence.len(), 1);
    // The earlier revision is unchanged: the correction is a new fact.
    let then = query(
        &mut e,
        &reader("fleet-admin"),
        json!({"graph_id": "fleet", "revision": r1, "from": node}),
    );
    let old = then.graph.nodes.iter().find(|n| n.id == node).unwrap();
    assert_ne!(old.properties.get("name"), Some(&json!("Renamed unit")));

    // A row that would move an edge to another unit cannot be a correction.
    let mut moved = batch.clone();
    let placement = &mut moved.tables.get_mut("placements").unwrap()[0];
    let other = batch.tables["units"][1]["serial"].clone();
    placement.insert("serial".into(), other);
    let error = import_batch(&mut e, &writer(), &target, &moved).unwrap_err();
    assert_eq!(error.code, "E_CONFLICT");
    assert_eq!(
        e.head("fleet", "main").unwrap().as_deref(),
        Some(r2.as_str())
    );
}

#[test]
fn input_is_checked_strictly() {
    let units = schema::table("units").unwrap();
    let code = |text: &str| parse_csv(units, text).unwrap_err().code;
    assert_eq!(code("serial,colour\nSN-1,red\n"), "E_HW_INPUT");
    assert_eq!(code("serial,serial\nSN-1,SN-1\n"), "E_HW_INPUT");

    let mut e = Engine::memory().unwrap();
    let target = Target::new("fleet");
    let import = |e: &mut Engine, value: serde_json::Value| {
        import_batch(e, &writer(), &target, &batch_from_value(value)?)
    };
    let cases = [
        (json!({"tables": {"rockets": []}}), "E_HW_INPUT"),
        (
            json!({"tables": {"units": [{"serial": "SN-1"}]}}),
            "E_HW_ROW",
        ),
        (
            json!({"tables": {"units": [{"serial": "SN-1", "model_id": "M", "commissioned_at": "soon"}]}}),
            "E_HW_ROW",
        ),
        (
            json!({"tables": {"units": [{"serial": "SN 1", "model_id": "M", "commissioned_at": "2024-01-01"}]}}),
            "E_HW_ROW",
        ),
        (
            json!({"tables": {"placements": [{"placement_id": "P", "serial": "SN-1", "site_id": "S",
                "valid_from": "2024-02-01", "valid_to": "2024-01-01"}]}}),
            "E_HW_ROW",
        ),
        (
            json!({"tables": {"sites": [{"site_id": "S", "name": "a"}, {"site_id": "S", "name": "b"}]}}),
            "E_HW_ROW",
        ),
        (
            json!({"tables": {"sensors": [{"sensor_id": "X", "kind": "vibration", "sample_rate_hz": "fast"}]}}),
            "E_HW_ROW",
        ),
        // The engine rejects an edge to a node that does not exist.
        (
            json!({"tables": {"units": [{"serial": "SN-1", "model_id": "NOPE", "commissioned_at": "2024-01-01"}]}}),
            "E_SCHEMA_ENDPOINT",
        ),
    ];
    for (value, want) in cases {
        let error = import(&mut e, value.clone()).unwrap_err();
        assert_eq!(error.code, want, "{value}: {}", error.message);
    }
    // Only the schema header was ever committed.
    assert_eq!(e.event_count().unwrap(), 1);

    // A graph created without the reference schema is refused.
    let mut other = Engine::memory().unwrap();
    let program: Program = serde_json::from_value(json!({"version": VERSION, "commands": [
        {"op": "commit", "graph_id": "fleet", "data": {"nodes": [{"id": "n", "entity_id": "n", "space_id": "s"}]}}
    ]}))
    .unwrap();
    other.execute(&program, &writer()).unwrap();
    assert_eq!(
        ensure_graph(&mut other, &writer(), &target)
            .unwrap_err()
            .code,
        "E_HW_SCHEMA"
    );
}

#[test]
fn restricted_rows_stay_restricted() {
    let mut e = Engine::memory().unwrap();
    let batch = base();
    import_batch(&mut e, &writer(), &Target::new("fleet"), &batch).unwrap();
    let restricted: Vec<String> = batch.tables["units"]
        .iter()
        .filter(|u| u.contains_key("readers"))
        .map(|u| format!("unit:{}", u["serial"]))
        .collect();
    assert!(!restricted.is_empty());
    let customer = batch.tables["units"]
        .iter()
        .find(|u| u.contains_key("readers"))
        .unwrap()["readers"]
        .split(';')
        .find(|r| r.starts_with("customer:"))
        .unwrap()
        .to_string();
    for (who, sees) in [
        ("fleet-admin", true),
        ("fleet-analyst", false),
        (customer.as_str(), true),
    ] {
        let result = query(
            &mut e,
            &reader(who),
            json!({"graph_id": "fleet", "predicate": "hw:runs"}),
        );
        let units: Vec<&str> = result.graph.nodes.iter().map(|n| n.id.as_str()).collect();
        let visible = restricted
            .iter()
            .filter(|r| units.contains(&r.as_str()))
            .count();
        if sees {
            assert!(visible > 0, "{who}");
        } else {
            assert_eq!(visible, 0, "{who}");
            assert!(result
                .graph
                .edges
                .iter()
                .all(|e| !restricted.contains(&e.from)));
            assert!(!units.is_empty(), "{who} still sees unrestricted units");
        }
    }
}
