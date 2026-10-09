//! Portable witness composition does not install runtime history authority.
use serde_json::json;
use weave_contract::*;
fn input() -> QueryResult {
    serde_json::from_value(json!({"version":VERSION,"graph":{},"snapshots":{},"input_snapshots":[{"graph_id":"g","revision":"r"}],"coverage":"complete","diagnostics":[],"provenance":[],"metadata_graphs":[],"node_origins":{},"edge_origins":{}})).unwrap()
}
fn witness(checkpoint: &str) -> RecordedObservation {
    RecordedObservation {
        observer: "local".into(),
        checkpoint: checkpoint.into(),
        graph: GraphRef {
            graph_id: "g".into(),
            revision: "r".into(),
        },
        branch_id: "main".into(),
        recorded_at_ms: 10,
        kind: ObservationKind::Committed,
    }
}
#[test]
fn absent_manifest_preserves_legacy_wire_shape() {
    let value = input();
    assert!(value.recorded_observations.is_empty());
    assert!(serde_json::to_value(&value)
        .unwrap()
        .get("recorded_observations")
        .is_none());
    assert_eq!(
        serde_json::from_slice::<QueryResult>(&serde_json::to_vec(&value).unwrap()).unwrap(),
        value
    );
}
#[test]
fn empty_composition_preserves_all_selections_and_rejects_conflicting_descriptions() {
    let mut left = input();
    left.recorded_observations = vec![witness("a")];
    let mut right = input();
    right.recorded_observations = vec![witness("b")];
    let ctx = AlgebraContext {
        principal: "reader".into(),
        max_objects: 100,
        max_output_bytes: 1024 * 1024,
    };
    let joined = algebra::union(left.clone(), right, &ctx).unwrap();
    assert_eq!(
        joined.recorded_observations,
        vec![witness("a"), witness("b")]
    );
    assert_eq!(
        algebra::project(joined.clone(), &[], &[], &ctx)
            .unwrap()
            .recorded_observations,
        joined.recorded_observations
    );
    let mut conflict = left.clone();
    conflict.recorded_observations[0].recorded_at_ms = 11;
    assert_eq!(
        algebra::union(left, conflict, &ctx).unwrap_err().code,
        "E_HISTORY_SELECTOR"
    );
}
#[test]
fn witness_bounds_and_exact_input_reference_are_enforced_before_pure_use() {
    let mut value = input();
    value.recorded_observations = vec![witness("a")];
    recorded_history::validate_result(&value).unwrap();
    value.input_snapshots.clear();
    assert_eq!(
        recorded_history::validate_result(&value).unwrap_err().code,
        "E_HISTORY_SELECTOR"
    );
    value = input();
    value.recorded_observations = vec![witness("a"); 1001];
    assert_eq!(
        recorded_history::validate_result(&value).unwrap_err().code,
        "E_BUDGET"
    );
    value.recorded_observations = vec![witness("a")];
    value.recorded_observations[0].recorded_at_ms = -1;
    assert_eq!(
        recorded_history::validate_result(&value).unwrap_err().code,
        "E_HISTORY_SELECTOR"
    );
    value.recorded_observations[0] = witness(&"x".repeat(513));
    assert_eq!(
        recorded_history::validate_result(&value).unwrap_err().code,
        "E_HISTORY_SELECTOR"
    );
}
