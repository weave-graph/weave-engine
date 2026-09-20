//! Independent semantic oracles for alternative authority, separate from implementation helpers.
use serde_json::{json, Value};
use weave_contract::{CommandResult, GraphData, GraphRef, Program, QueryResult, VERSION};
use weave_engine::{Engine, HostContext};

fn host(mask: usize) -> HostContext {
    HostContext::new(format!("principal-{mask}"), [])
}
fn run(e: &mut Engine, commands: Value) -> Vec<CommandResult> {
    let p: Program =
        serde_json::from_value(json!({"version":VERSION,"commands":commands})).unwrap();
    e.execute(
        &p,
        &HostContext::new(
            "principal-7",
            [
                "proofs".into(),
                "root".into(),
                "saved".into(),
                "before".into(),
            ],
        ),
    )
    .unwrap()
}
fn commit(e: &mut Engine, graph: &str, data: Value) -> GraphRef {
    run(e, json!([{"op":"commit","graph_id":graph,"data":data}]));
    GraphRef {
        graph_id: graph.into(),
        revision: e.head(graph, "main").unwrap().unwrap(),
    }
}
fn read(e: &Engine, pin: &GraphRef, mask: usize) -> weave_engine::Result<QueryResult> {
    e.query(
        &serde_json::from_value(
            json!({"graph_id":pin.graph_id,"revision":pin.revision,"include_metadata":true}),
        )
        .unwrap(),
        &host(mask),
    )
}
fn premises(e: &mut Engine) -> [Value; 3] {
    let nodes: Vec<_> = (0..3).map(|bit| json!({
        "id":format!("permit-{bit}"),"entity_id":format!("permit-{bit}"),"space_id":"s",
        "readers":(0..8).filter(|mask| mask & (1 << bit) != 0).map(|mask|format!("principal-{mask}")).collect::<Vec<_>>()
    })).collect();
    let pin = commit(e, "proofs", json!({"nodes":nodes}));
    std::array::from_fn(
        |i| json!({"graph_id":pin.graph_id,"revision":pin.revision,"node_id":format!("permit-{i}")}),
    )
}
fn branch(premise: &Value) -> Value {
    json!({"operator":"root-independent-proof","premises":[],"node_premises":[premise]})
}
fn n() -> Value {
    json!({"id":"result","entity_id":"result","space_id":"s"})
}

#[test]
fn root_eight_principals_obey_or_then_and_for_all_new_carriers() {
    for kind in ["node", "attachment", "value"] {
        let mut e = Engine::memory().unwrap();
        let [a, b, c] = premises(&mut e);
        let alternatives = json!([branch(&a), branch(&b)]);
        let data = match kind {
            "node" => {
                let mut node = n();
                node["derived_nodes"] = json!([c]);
                node["derivations"] = alternatives;
                json!({"nodes":[node]})
            }
            "attachment" => {
                json!({"attachments":[{"id":"result","host":{"kind":"graph"},"key":"result","value":{"kind":"literal","value":42},"valid_time":{"start":0},"derived_nodes":[c],"derivations":alternatives}]})
            }
            _ => json!({"nodes":[n()],"influence":{"nodes":[c],"derivations":alternatives}}),
        };
        let root = commit(&mut e, "root", data);
        for mask in 0..8 {
            let expected = mask & 4 != 0 && mask & 3 != 0;
            let result = read(&e, &root, mask);
            let visible = result
                .as_ref()
                .is_ok_and(|r| !r.graph.nodes.is_empty() || !r.graph.attachments.is_empty());
            assert_eq!(visible, expected, "{kind}, principal {mask}");
            if expected {
                let encoded = serde_json::to_string(&result.unwrap()).unwrap();
                for (bit, name) in [(1, "permit-0"), (2, "permit-1")] {
                    if mask & bit == 0 {
                        assert!(
                            !encoded.contains(name),
                            "denied branch leaked for {kind}/{mask}: {encoded}"
                        );
                    }
                }
            }
            let capsule = e.export_capsule(&root, &host(mask));
            assert_eq!(
                capsule.is_ok(),
                mask == 7,
                "whole export must reject pruning: {kind}/{mask}"
            );
            if mask == 7 {
                let capsule = capsule.unwrap();
                assert_eq!(capsule.format, "weave-capsule-0.4");
                for old in [
                    "weave-capsule-0.1",
                    "weave-capsule-0.2",
                    "weave-capsule-0.3",
                ] {
                    let mut old_capsule = capsule.clone();
                    old_capsule.format = old.into();
                    let mut receiver = Engine::memory().unwrap();
                    assert_eq!(
                        receiver
                            .receive_capsule(
                                &old_capsule,
                                &HostContext::new("principal-7", ["proofs".into(), "root".into()])
                            )
                            .unwrap_err()
                            .code,
                        "E_VERSION"
                    );
                    assert_eq!(receiver.event_count().unwrap(), 0);
                }
            }
        }
    }
}

#[test]
fn root_missing_metadata_support_copies_keep_or_condition_after_envelope_removed() {
    let mut e = Engine::memory().unwrap();
    let [a, b, c] = premises(&mut e);
    commit(
        &mut e,
        "root",
        json!({"attachments":[{
            "id":"path","host":{"kind":"graph"},"key":"evidence",
            "value":{"kind":"graph","reference":{"graph_id":"missing","revision":"missing"}},
            "valid_time":{"start":0},"derived_nodes":[c],"derivations":[branch(&a),branch(&b)]
        }]}),
    );
    let results = run(
        &mut e,
        json!([{"op":"evaluate","value":{
            "kind":"support","predicate":"p","from":{"entity_id":"a","space_id":"s"},"to":{"entity_id":"b","space_id":"s"},"valid_at":5,
            "input":{"kind":"metadata","input":{"kind":"query","query":{"graph_id":"root","include_metadata":true}},"host":{"kind":"graph"},"key":"evidence"}
        }}]),
    );
    let CommandResult::Queried { result } = &results[0] else {
        panic!()
    };
    assert_eq!(result.graph.nodes[0].properties["state"], "unknown");
    let mut copied: GraphData = result.graph.clone();
    copied.influence = None;
    for node in &mut copied.nodes {
        node.readers.clear();
    }
    for attachment in &mut copied.attachments {
        attachment.readers.clear();
    }
    let pin = commit(&mut e, "saved", serde_json::to_value(copied).unwrap());
    for mask in 0..8 {
        let visible = read(&e, &pin, mask).unwrap().graph.nodes.len();
        assert_eq!(
            visible,
            usize::from(mask & 4 != 0 && mask & 3 != 0),
            "copied scalar principal {mask}"
        );
    }
}

#[test]
fn root_cyclic_alternative_cannot_invent_support_or_poison_independent_branch() {
    let mut e = Engine::memory().unwrap();
    let [a, _, _] = premises(&mut e);
    let own = json!({"graph_id":"root","revision":"logical:root-cycle:root","node_id":"result"});
    let mut node = n();
    node["derivations"] = json!([branch(&own), branch(&a)]);
    run(
        &mut e,
        json!([{"op":"commit_batch","batch_id":"root-cycle","commits":[{"graph_id":"root","data":{"nodes":[node]}}]}]),
    );
    let pin = GraphRef {
        graph_id: "root".into(),
        revision: e.head("root", "main").unwrap().unwrap(),
    };
    for mask in 0..8 {
        assert_eq!(
            read(&e, &pin, mask).unwrap().graph.nodes.len(),
            usize::from(mask & 1 != 0),
            "cycle principal {mask}"
        );
    }
}

#[test]
fn root_old_wire_rejects_typed_new_carriers_before_first_write_but_not_literal_keys() {
    let premise = json!({"graph_id":"missing","revision":"missing","node_id":"missing"});
    let groups = json!([branch(&premise)]);
    let mut node = n();
    node["derivations"] = groups.clone();
    let samples = [
        json!({"nodes":[node]}),
        json!({"influence":{"derivations":groups}}),
        json!({"attachments":[{"id":"a","host":{"kind":"graph"},"key":"x","value":{"kind":"literal","value":0},"valid_time":{"start":0},"derivations":groups}]}),
        json!({"nodes":[n()],"edges":[{"id":"e","from":"result","to":"result","predicate":"p","valid_time":{"start":0},"derivations":[{"operator":"new","premises":[],"snapshot_premises":[{"graph_id":"missing","revision":"missing"}]}]}]}),
    ];
    for data in samples {
        let mut e = Engine::memory().unwrap();
        let p:Program=serde_json::from_value(json!({"version":"0.18.0","commands":[{"op":"commit","graph_id":"before","data":{}},{"op":"commit","graph_id":"root","data":data}]})).unwrap();
        assert_eq!(
            e.execute(
                &p,
                &HostContext::new("principal-7", ["before".into(), "root".into()])
            )
            .unwrap_err()
            .code,
            "E_VERSION"
        );
        assert_eq!(e.event_count().unwrap(), 0);
        assert!(e.head("before", "main").unwrap().is_none());
    }
    let query = json!({"kind":"query","query":{"graph_id":"root"}});
    for expression in [
        json!({"kind":"window","input":query,"window":{"start":0,"end":10}}),
        json!({"kind":"sequence","left":query,"right":query,"window":{"start":0,"end":10},"relation":"meets","match_on":"entity_space_to_from"}),
    ] {
        let mut e = Engine::memory().unwrap();
        let p: Program = serde_json::from_value(json!({"version":"0.18.0","commands":[
            {"op":"commit","graph_id":"before","data":{}},
            {"op":"evaluate","value":{"kind":"explain","input":expression}}
        ]}))
        .unwrap();
        assert_eq!(
            e.execute(&p, &HostContext::new("principal-7", ["before".into()]))
                .unwrap_err()
                .code,
            "E_VERSION"
        );
        assert_eq!(e.event_count().unwrap(), 0);
        assert!(e.head("before", "main").unwrap().is_none());
    }
    let mut e = Engine::memory().unwrap();
    let p:Program=serde_json::from_value(json!({"version":"0.18.0","commands":[{"op":"commit","graph_id":"root","data":{"nodes":[{"id":"literal","entity_id":"literal","space_id":"s","properties":{"derivations":groups,"snapshot_premises":["inert"],"kind":"window"}}]}}]})).unwrap();
    e.execute(&p, &HostContext::new("principal-7", ["root".into()]))
        .unwrap();
    assert!(e.head("root", "main").unwrap().is_some());
}
