//! `docs/HARDWARE_MODEL.md` documents every type, relation, table and column,
//! and its example imports and shows every type and relation. The engine
//! enforces the schema for any writer.
use serde_json::json;
use std::collections::BTreeSet;
use weave_contract::facts::Fact;
use weave_contract::{CommandResult, MetadataHost, Program, VERSION};
use weave_engine::{Engine, HostContext};
use weave_hardware::mapping::row_facts;
use weave_hardware::schema::{self, NODE_TYPES, NOTE_KEY, RELATIONS, SERIES_KEY, TABLES};
use weave_hardware::series::{note_text, SeriesRef};
use weave_hardware::table::parse_json_batch;
use weave_hardware::{import_batch, Target};

const DOC: &str = include_str!("../../../docs/HARDWARE_MODEL.md");

fn json_blocks(section: &str) -> Vec<&'static str> {
    let start = DOC.find(section).expect("section exists");
    let mut rest = &DOC[start..];
    let mut blocks = Vec::new();
    while let Some(open) = rest.find("```json\n") {
        let body = &rest[open + 8..];
        let close = body.find("\n```").unwrap();
        blocks.push(&body[..close]);
        rest = &body[close..];
        if rest[4..].starts_with("\n\n## ") {
            break;
        }
    }
    blocks
}

fn writer() -> HostContext {
    HostContext::new("etl", ["fleet".to_string()])
}

#[test]
fn the_reference_documents_every_type_relation_table_and_column() {
    for node in NODE_TYPES {
        assert!(
            DOC.contains(&format!("`{}`", node.type_id)),
            "{}",
            node.type_id
        );
        assert!(
            DOC.contains(&format!("| `{}` |", node.prefix)),
            "{}",
            node.prefix
        );
    }
    for relation in RELATIONS {
        assert!(
            DOC.contains(&format!("| `{}` |", relation.predicate)),
            "{}",
            relation.predicate
        );
    }
    for table in TABLES {
        let line = DOC
            .lines()
            .find(|l| l.starts_with(&format!("| `{}` |", table.name)))
            .unwrap_or_else(|| panic!("table {} is documented", table.name));
        for column in table.columns() {
            if !schema::COMMON_COLUMNS.contains(&column) {
                assert!(line.contains(column), "column {} of {}", column, table.name);
            }
        }
    }
    for column in schema::COMMON_COLUMNS {
        assert!(DOC.contains(&format!("`{column}`")), "{column}");
    }
}

#[test]
fn the_example_imports_and_shows_every_type_and_relation() {
    let blocks = json_blocks("## Example");
    let batch = parse_json_batch(blocks[0]).unwrap();
    assert_eq!(
        batch
            .tables
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        TABLES.iter().map(|t| t.name).collect::<BTreeSet<_>>(),
        "the example fills every table"
    );
    let mut e = Engine::memory().unwrap();
    let report = import_batch(&mut e, &writer(), &Target::new("fleet"), &batch).unwrap();
    assert_eq!(report.asserted, report.facts);
    let program: Program = serde_json::from_value(json!({"version": VERSION, "commands": [
        {"op": "query", "query": {"graph_id": "fleet"}}
    ]}))
    .unwrap();
    let admin = HostContext::new("fleet-admin", Vec::<String>::new());
    let CommandResult::Queried { result } = e.execute(&program, &admin).unwrap().remove(0) else {
        panic!("query result expected")
    };
    let types: BTreeSet<&str> = result
        .graph
        .nodes
        .iter()
        .filter_map(|n| n.type_id.as_deref())
        .collect();
    assert_eq!(types, NODE_TYPES.iter().map(|n| n.type_id).collect());
    let predicates: BTreeSet<&str> = result
        .graph
        .edges
        .iter()
        .map(|e| e.predicate.as_str())
        .collect();
    assert_eq!(predicates, RELATIONS.iter().map(|r| r.predicate).collect());
    let series: Vec<_> = result
        .graph
        .attachments
        .iter()
        .filter(|a| a.key == SERIES_KEY)
        .collect();
    assert_eq!(series.len(), 1);
    assert_eq!(
        SeriesRef::from_attachment(series[0]).unwrap().series_id,
        "SN-000001-V"
    );
    let notes: Vec<_> = result
        .graph
        .attachments
        .iter()
        .filter(|a| a.key == NOTE_KEY)
        .collect();
    assert_eq!(notes.len(), 2);
    assert!(notes
        .iter()
        .all(|n| note_text(n).is_some_and(|t| t.len() > 20)));
    assert!(notes.iter().any(|n| n.id == "note:maint:WO-000002"
        && n.host
            == MetadataHost::Node {
                id: "maint:WO-000002".into()
            }));

    // The documented unit facts are exactly what the importer produces.
    let facts_block = blocks[1];
    let node_text = &facts_block[..facts_block.find("\n{\"kind\": \"edge\"").unwrap()];
    let documented: Fact = serde_json::from_str(node_text).unwrap();
    let units = schema::table("units").unwrap();
    let produced = row_facts(units, &batch.tables["units"][0], batch.source.as_deref()).unwrap();
    assert_eq!(produced[0], documented);
    assert_eq!(produced[1].id(), "has_model:SN-000001");
}

#[test]
fn the_engine_enforces_the_schema_for_every_writer() {
    let mut e = Engine::memory().unwrap();
    let batch = parse_json_batch(json_blocks("## Example")[0]).unwrap();
    import_batch(&mut e, &writer(), &Target::new("fleet"), &batch).unwrap();
    let write = |e: &mut Engine, fact: serde_json::Value| {
        let program: Program = serde_json::from_value(json!({"version": VERSION, "commands": [
            {"op": "write", "graph_id": "fleet", "changes": [{"change": "assert", "fact": fact}]}
        ]}))
        .unwrap();
        e.execute(&program, &writer()).map(|_| ()).unwrap_err().code
    };
    // A site cannot run firmware.
    let code = write(
        &mut e,
        json!({"kind": "edge", "id": "bad", "type_id": "hw:runs", "predicate": "hw:runs",
               "from": "site:S001", "to": "fw:compressor-2.2.0", "valid_time": {"start": 0}}),
    );
    assert!(code.starts_with("E_SCHEMA"), "{code}");
    // A unit has no colour.
    let code = write(
        &mut e,
        json!({"kind": "node", "id": "unit:X", "entity_id": "unit:X", "space_id": "hw",
               "type_id": "hw.Unit", "properties": {"serial": "X", "colour": "red"}}),
    );
    assert!(code.starts_with("E_SCHEMA"), "{code}");
    // An untyped node is not part of the model.
    let code = write(
        &mut e,
        json!({"kind": "edge", "id": "bad2", "type_id": "hw:runs", "predicate": "hw:runs",
               "from": "unit:SN-000001", "to": "unit:SN-000001", "valid_time": {"start": 0}}),
    );
    assert!(code.starts_with("E_SCHEMA"), "{code}");
}
