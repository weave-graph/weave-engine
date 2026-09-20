//! Boundary oracle derived directly from interval definitions, not temporal helpers.
use serde_json::{json, Value};
use weave_contract::{CommandResult, QueryResult, VERSION};
use weave_engine::{Engine, HostContext};

fn execute(e: &mut Engine, commands: Value) -> Vec<CommandResult> {
    e.execute(
        &serde_json::from_value(json!({"version":VERSION,"commands":commands})).unwrap(),
        &HostContext::new("root-temporal", ["left".into(), "right".into()]),
    )
    .unwrap()
}
fn seed(e: &mut Engine, graph: &str, start: i64, end: i64, negative: bool) {
    let (from, to) = if graph == "left" {
        ("a", "shared")
    } else {
        ("shared", "c")
    };
    execute(
        e,
        json!([{"op":"commit","graph_id":graph,"data":{
            "nodes":[{"id":from,"entity_id":from,"space_id":"s"},{"id":to,"entity_id":to,"space_id":"s"}],
            "edges":[{"id":format!("{graph}-event"),"from":from,"to":to,"predicate":graph,"valid_time":{"start":start,"end":end},"polarity":if negative {"negative"} else {"positive"}}]
        }}]),
    );
}
fn evaluate(e: &mut Engine, value: Value) -> QueryResult {
    let mut result = execute(e, json!([{"op":"evaluate","value":value}]));
    let CommandResult::Queried { result } = result.remove(0) else {
        panic!()
    };
    *result
}

#[test]
fn root_sequence_relations_use_original_intervals_and_emit_two_clipped_occurrences() {
    for (left, right, window) in [
        ((0, 3), (4, 9), (0, 10)),
        ((0, 4), (4, 9), (0, 10)),
        ((0, 8), (6, 20), (7, 9)),
        ((0, 20), (6, 8), (7, 9)),
        ((6, 8), (0, 20), (7, 9)),
        ((0, 8), (0, 8), (3, 5)),
        ((0, 3), (4, 9), (3, 7)),
    ] {
        let mut e = Engine::memory().unwrap();
        seed(&mut e, "left", left.0, left.1, false);
        seed(&mut e, "right", right.0, right.1, false);
        for relation in ["before", "meets", "overlaps", "within"] {
            let lc = (left.0.max(window.0), left.1.min(window.1));
            let rc = (right.0.max(window.0), right.1.min(window.1));
            let applies = lc.0 < lc.1
                && rc.0 < rc.1
                && match relation {
                    "before" => left.1 < right.0,
                    "meets" => left.1 == right.0,
                    "overlaps" => left.0.max(right.0) < left.1.min(right.1),
                    "within" => right.0 <= left.0 && left.1 <= right.1,
                    _ => unreachable!(),
                };
            let result = evaluate(
                &mut e,
                json!({"kind":"sequence",
                    "left":{"kind":"query","query":{"graph_id":"left"}},
                    "right":{"kind":"query","query":{"graph_id":"right"}},
                    "relation":relation,"window":{"start":window.0,"end":window.1},"match_on":"entity_space_to_from"
                }),
            );
            let mut actual: Vec<_> = result
                .graph
                .edges
                .iter()
                .map(|edge| {
                    assert_ne!(edge.id, "left-event");
                    assert_ne!(edge.id, "right-event");
                    (
                        edge.predicate.as_str(),
                        edge.valid_time.start,
                        edge.valid_time.end.unwrap(),
                    )
                })
                .collect();
            actual.sort();
            let expected = if applies {
                vec![("left", lc.0, lc.1), ("right", rc.0, rc.1)]
            } else {
                vec![]
            };
            assert_eq!(
                actual, expected,
                "{relation}: {left:?}, {right:?}, window {window:?}"
            );
        }
    }
}

#[test]
fn root_window_preserves_refutation_and_half_open_clipping_without_reusing_origin_id() {
    let mut e = Engine::memory().unwrap();
    seed(&mut e, "left", 0, 8, true);
    let window = |start, end| json!({"kind":"window","input":{"kind":"query","query":{"graph_id":"left"}},"window":{"start":start,"end":end}});
    let result = evaluate(&mut e, window(3, 5));
    assert_eq!(result.graph.edges.len(), 1);
    let edge = &result.graph.edges[0];
    assert_eq!(edge.polarity, weave_contract::Polarity::Negative);
    assert_eq!((edge.valid_time.start, edge.valid_time.end), (3, Some(5)));
    assert_ne!(edge.id, "left-event");
    assert!(evaluate(&mut e, window(8, 10)).graph.edges.is_empty());
    let original = evaluate(&mut e, json!({"kind":"query","query":{"graph_id":"left"}}));
    assert_eq!(original.graph.edges[0].id, "left-event");
    assert_eq!(original.graph.edges[0].valid_time.end, Some(8));
}
