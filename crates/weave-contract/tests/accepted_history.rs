use serde_json::json;
use weave_contract::*;
fn input(decision: &str) -> QueryResult {
    let mut value:QueryResult=serde_json::from_value(json!({"version":VERSION,"graph":{},"snapshots":{},"input_snapshots":[{"graph_id":"g","revision":"r"},{"graph_id":"protected","revision":"occurrence"}],"coverage":"complete","diagnostics":[],"provenance":[],"metadata_graphs":[],"node_origins":{},"edge_origins":{}})).unwrap();
    value.accepted_observations.push(AcceptedViewObservation {
        observer: "local".into(),
        view_id: "team".into(),
        decision_id: decision.into(),
        accepted_at_ms: 10,
        occurrence: GraphRef {
            graph_id: "protected".into(),
            revision: "occurrence".into(),
        },
        source: GraphRef {
            graph_id: "g".into(),
            revision: "r".into(),
        },
        branch_id: "main".into(),
    });
    value
}
#[test]
fn accepted_witnesses_survive_empty_union_projection_and_window() {
    let ctx = AlgebraContext {
        principal: "reader".into(),
        max_objects: 100,
        max_output_bytes: 1024 * 1024,
    };
    let joined = algebra::union(input("a"), input("b"), &ctx).unwrap();
    assert_eq!(joined.accepted_observations.len(), 2);
    let projected = algebra::project(joined.clone(), &[], &[], &ctx).unwrap();
    assert_eq!(
        projected.accepted_observations,
        joined.accepted_observations
    );
    let window = temporal::window(
        joined.clone(),
        &Interval {
            start: 0,
            end: Some(20),
        },
        &ctx,
    )
    .unwrap();
    assert_eq!(window.accepted_observations, joined.accepted_observations);
    let mut conflict = input("a");
    conflict.accepted_observations[0].accepted_at_ms = 11;
    assert_eq!(
        algebra::union(input("a"), conflict, &ctx).unwrap_err().code,
        "E_ACCEPTED_SELECTOR"
    );
}
#[test]
fn descriptive_witnesses_require_both_exact_inputs_and_bounded_identity() {
    let mut value = input("a");
    accepted_history::validate_result(&value).unwrap();
    value.input_snapshots.pop();
    assert_eq!(
        accepted_history::validate_result(&value).unwrap_err().code,
        "E_ACCEPTED_SELECTOR"
    );
    value = input("a");
    value.accepted_observations[0].observer = "x".repeat(513);
    assert!(accepted_history::validate_result(&value).is_err());
    value = input("a");
    value.accepted_observations[0].accepted_at_ms = -1;
    assert!(accepted_history::validate_result(&value).is_err());
    value = input("a");
    value.accepted_observations = vec![value.accepted_observations[0].clone(); 1001];
    assert_eq!(
        accepted_history::validate_result(&value).unwrap_err().code,
        "E_BUDGET"
    );
    let mut old = input("a");
    old.accepted_observations.clear();
    assert!(serde_json::to_value(old)
        .unwrap()
        .get("accepted_observations")
        .is_none());
}
