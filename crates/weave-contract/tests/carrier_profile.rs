use serde_json::json;
use weave_contract::{Derivation, GraphInfluence};

fn snapshot_group(count: usize) -> serde_json::Value {
    json!({"operator":"test", "premises":[], "snapshot_premises":
        (0..count).map(|_| json!({"graph_id":"g","revision":"r"})).collect::<Vec<_>>()})
}
#[test]
fn absent_carrier_fields_preserve_historical_bytes() {
    let old = r#"{"operator":"test","premises":[],"parameters":{},"input_snapshots":[]}"#;
    let group: Derivation = serde_json::from_str(old).unwrap();
    assert!(group.snapshot_premises.is_empty());
    assert_eq!(serde_json::to_string(&group).unwrap(), old);
    let flat: GraphInfluence = serde_json::from_str("{}").unwrap();
    assert_eq!(serde_json::to_string(&flat).unwrap(), "{}");
}
#[test]
fn alternative_decoder_bounds_raw_groups_and_shared_gate_references() {
    let good = json!({"derivations": [snapshot_group(500), snapshot_group(500)]});
    assert!(serde_json::from_value::<GraphInfluence>(good).is_ok());
    let bad = json!({"derivations": [snapshot_group(500), snapshot_group(501)]});
    assert!(serde_json::from_value::<GraphInfluence>(bad).is_err());
    let good = json!({"derivations":vec![snapshot_group(1);128]});
    assert!(serde_json::from_value::<GraphInfluence>(good).is_ok());
    let bad = json!({"derivations":vec![snapshot_group(1);129]});
    assert!(serde_json::from_value::<GraphInfluence>(bad).is_err());
    let mixed = json!({"derivations":[{"operator":"test","premises":[],
        "node_premises":vec![json!({"graph_id":"g","revision":"r","node_id":"n"});501],
        "snapshot_premises":vec![json!({"graph_id":"g","revision":"r"});500]}]});
    assert!(serde_json::from_value::<GraphInfluence>(mixed).is_err());
}
#[test]
fn alternative_decoder_rejects_unknown_and_duplicate_fields() {
    for wire in [
        r#"{"derivations":[{"operator":"test","premises":[],"snapshot_premises":[],"snapshot_premises":[]}]}"#,
        r#"{"derivations":[{"operator":"test","premises":[],"unknown":[]}]}"#,
    ] {
        assert!(serde_json::from_str::<GraphInfluence>(wire).is_err());
    }
}
