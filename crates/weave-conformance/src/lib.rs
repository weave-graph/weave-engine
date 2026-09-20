//! Fixed conformance inputs only. No storage, host authentication, arbitrary plan API or I/O.
use serde_json::{json, Value};
use weave_contract::decimal::Decimal;
use weave_contract::quantity::{Quantity, RationalConversion, UnitDescriptor};
use weave_contract::{
    algebra, AlgebraContext, AssertionRef, ContextSelection, EntitySpace, GraphRef, NodeRef,
    QueryResult, VERSION,
};
fn decimal(s: &str) -> Decimal {
    s.parse().unwrap()
}
fn fixture(graph: &str, polarity: &str, start: i64, end: i64) -> QueryResult {
    let mut q:QueryResult=serde_json::from_value(json!({"version":VERSION,"graph":{"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"e","predicate":"p","from":"a","to":"b","valid_time":{"start":start,"end":end},"polarity":polarity}]},"snapshots":{},"input_snapshots":[{"graph_id":graph,"revision":"r"}],"coverage":"partial","diagnostics":[],"provenance":[],"metadata_graphs":[]})).unwrap();
    for node in ["a", "b"] {
        q.node_origins.insert(
            node.into(),
            vec![NodeRef {
                graph_id: graph.into(),
                revision: "r".into(),
                node_id: node.into(),
            }],
        );
    }
    let origin = AssertionRef {
        graph_id: graph.into(),
        revision: "r".into(),
        assertion_id: "e".into(),
    };
    q.edge_origins.insert("e".into(), vec![origin.clone()]);
    q.provenance.push(origin);
    q
}
pub fn golden_profile() -> Vec<u8> {
    let ctx = AlgebraContext {
        principal: "reader".into(),
        max_objects: 100,
        max_output_bytes: 1_000_000,
    };
    let a = fixture("positive", "positive", 0, 10);
    let b = fixture("negative", "negative", 5, 15);
    let union = algebra::union(a.clone(), b, &ctx).unwrap();
    assert_eq!(union.graph.nodes.len(), 4);
    assert_eq!(union.graph.edges.len(), 2);
    assert_eq!(algebra::union(union.clone(), a, &ctx).unwrap(), union);
    let target = |s: &str| EntitySpace {
        entity_id: s.into(),
        space_id: "s".into(),
    };
    let states: Vec<_> = [
        (0, "supported"),
        (5, "conflicted"),
        (10, "refuted"),
        (20, "unknown"),
    ]
    .into_iter()
    .map(|(at, state)| {
        let r = algebra::support(union.clone(), "p", &target("A"), &target("B"), at, &ctx).unwrap();
        assert_eq!(r.graph.nodes[0].properties["state"], state);
        json!({"at":at,"result":r})
    })
    .collect();
    let default =
        weave_contract::context::select(union.clone(), &ContextSelection::Default, &ctx).unwrap();
    let pinned = ContextSelection::Pinned {
        reference: GraphRef {
            graph_id: "world".into(),
            revision: "1".into(),
        },
    };
    let mismatch = weave_contract::context::select(default.clone(), &pinned, &ctx).unwrap_err();
    assert_eq!(mismatch.code, "E_CONTEXT_SCOPE");
    let m = UnitDescriptor::new("length".into(), "m".into(), "1".into()).unwrap();
    let cm = UnitDescriptor::new("length".into(), "cm".into(), "1".into()).unwrap();
    let q = Quantity::new(decimal("0.3"), m.clone());
    let converted = q
        .convert(&RationalConversion::new(m, cm.clone(), decimal("100"), decimal("1")).unwrap())
        .unwrap();
    assert_eq!(converted.amount(), decimal("30"));
    let mismatch_unit = q.checked_add(&Quantity::new(decimal("1"), cm)).unwrap_err();
    let sum = decimal("0.1").checked_add(decimal("0.2")).unwrap();
    assert_eq!(sum, decimal("0.3"));
    let nonterminating = decimal("1").checked_div(decimal("3")).unwrap_err();
    let huge = decimal("9007199254740993");
    assert_eq!(huge.to_string(), "9007199254740993");
    let cluster = cluster_profile();
    let geometry = geometry_profile();
    serde_json::to_vec(&json!({"profile":"weave-portable-golden-2","contract":VERSION,"cases":{
        "exact_decimal":{"sum":sum,"beyond_binary64":huge,"nonterminating_error":nonterminating.to_string()},
        "nominal_quantity":{"converted":converted,"mismatch_error":mismatch_unit.to_string()},
        "graph_union":union,"four_valued_temporal_support":states,"exact_context":{"selected":default,"reselection_error":mismatch},
        "authorized_geometry_fixture":geometry,"lazy_clustering":cluster,"typed_context_carrier":typed_context_profile(),"temporal_alternative_carriers":temporal_profile()
    },"limits":["fixed host-authorized test inputs; no authentication or persistent runtime","binary64 fixture agreement is not general bitwise geometry portability","no browser storage, network, mobile energy or adapter sandbox claim"]})).unwrap()
}
fn temporal_profile() -> Value {
    use weave_contract::{
        temporal, GraphData, GraphInfluence, Interval, JoinMatch, TemporalRelation,
    };
    let context = AlgebraContext {
        principal: "reader".into(),
        max_objects: 100,
        max_output_bytes: 1_000_000,
    };
    let left = fixture("left", "negative", 0, 10);
    let clipped = temporal::window(
        left.clone(),
        &Interval {
            start: 2,
            end: Some(8),
        },
        &context,
    )
    .unwrap();
    assert_eq!(
        clipped.graph.edges[0].valid_time,
        Interval {
            start: 2,
            end: Some(8)
        }
    );
    assert_eq!(
        clipped.graph.edges[0].polarity,
        weave_contract::Polarity::Negative
    );
    let repeated = temporal::window(
        clipped.clone(),
        &Interval {
            start: 2,
            end: Some(8),
        },
        &context,
    )
    .unwrap();
    assert_eq!(clipped.graph, repeated.graph);
    assert_eq!(clipped.edge_origins, repeated.edge_origins);
    let mut right = fixture("right", "positive", 10, 20);
    right.graph.nodes[0].entity_id = "B".into();
    right.graph.nodes[1].entity_id = "C".into();
    let paired = temporal::sequence(
        fixture("left", "positive", 0, 10),
        right,
        &Interval {
            start: 5,
            end: Some(15),
        },
        TemporalRelation::Meets,
        &JoinMatch::EntitySpaceToFrom,
        &context,
    )
    .unwrap();
    assert_eq!(paired.graph.edges.len(), 2);
    assert!(paired.graph.edges.iter().all(|e| !e.derivations.is_empty()));
    let mut empty = fixture("empty", "positive", 0, 10);
    empty.graph = GraphData::default();
    empty.node_origins.clear();
    empty.edge_origins.clear();
    empty.provenance.clear();
    empty.graph.influence = Some(GraphInfluence {
        derivations: ["A", "B"].into_iter().map(|graph| serde_json::from_value(json!({
            "operator":"weave:conformance-choice", "premises":[], "snapshot_premises":[{"graph_id":graph,"revision":"r"}]
        })).unwrap()).collect(),
        ..GraphInfluence::default()
    });
    let empty = temporal::window(
        empty,
        &Interval {
            start: 0,
            end: Some(20),
        },
        &context,
    )
    .unwrap();
    let target = |entity: &str| EntitySpace {
        entity_id: entity.into(),
        space_id: "s".into(),
    };
    let scalar =
        algebra::support(empty, "missing", &target("A"), &target("B"), 5, &context).unwrap();
    assert_eq!(scalar.graph.nodes[0].properties["state"], "unknown");
    assert_eq!(scalar.graph.nodes[0].derivations.len(), 2);
    json!({"window":clipped,"sequence":paired,"empty_choice_scalar":scalar})
}
fn typed_context_profile() -> Value {
    use weave_contract::context_axes::ContextDefinition;
    use weave_contract::{ContextTyping, TypedContextWitness};
    let source = br#"{"schema":{"reference":{"id":"scenario","revision":"1"},"axes":{"mode":{"kind":"enum","members":["live","test"]},"load":{"kind":"decimal"}}},"values":{"mode":"live","load":"0.3"}}"#;
    let definition = ContextDefinition::from_json(source).unwrap();
    let mut reordered = definition.clone();
    if let weave_contract::context_axes::ContextAxisType::Enum { members } =
        reordered.schema.axes.get_mut("mode").unwrap()
    {
        members.reverse();
    }
    assert_eq!(
        definition.fingerprint().unwrap(),
        reordered.fingerprint().unwrap()
    );
    assert!(ContextDefinition::from_json(br#"{"schema":{},"schema":{},"values":{}}"#).is_err());
    let reference = GraphRef {
        graph_id: "world".into(),
        revision: "1".into(),
    };
    let proof = AssertionRef {
        graph_id: "world".into(),
        revision: "1".into(),
        assertion_id: "definition".into(),
    };
    let typing = ContextTyping {
        selected: Some(reference.clone()),
        witnesses: vec![TypedContextWitness {
            context: reference.clone(),
            schema: definition.schema.clone(),
            definition: proof.clone(),
            anchor_nodes: vec![NodeRef {
                graph_id: "world".into(),
                revision: "1".into(),
                node_id: "anchor".into(),
            }],
        }],
    };
    let mut input:QueryResult=serde_json::from_value(json!({"version":VERSION,"graph":{},"snapshots":{},"input_snapshots":[],"coverage":"partial","diagnostics":[],"provenance":[],"metadata_graphs":[]})).unwrap();
    input.selected_context = Some(ContextSelection::Pinned { reference });
    input.graph.context_typing = Some(typing);
    let ctx = AlgebraContext {
        principal: "reader".into(),
        max_objects: 100,
        max_output_bytes: 1_000_000,
    };
    let t = EntitySpace {
        entity_id: "absent".into(),
        space_id: "s".into(),
    };
    let supported = algebra::support(input, "p", &t, &t, 5, &ctx).unwrap();
    assert_eq!(supported.graph.nodes[0].properties["state"], "unknown");
    assert!(supported.graph.nodes[0].derived_from.contains(&proof));
    assert!(supported.graph.context_typing.is_some());
    json!({"schema_fingerprint":definition.schema.fingerprint().unwrap(),"definition_fingerprint":definition.fingerprint().unwrap(),"unknown_with_retained_context":supported})
}
fn cluster_profile() -> Value {
    let source = weave_cluster::Snapshot {
        perspective: "p".into(),
        context: ContextSelection::Default,
        valid_at: 5,
        sources: vec![GraphRef {
            graph_id: "source".into(),
            revision: "r".into(),
        }],
        nodes: vec!["a".into(), "b".into(), "isolated".into()],
        links: vec![weave_cluster::Link {
            id: "ab".into(),
            from: "a".into(),
            to: "b".into(),
            evidence: vec![AssertionRef {
                graph_id: "source".into(),
                revision: "r".into(),
                assertion_id: "e".into(),
            }],
        }],
        partial: true,
    };
    let mut h = weave_cluster::Hierarchy::new(source).unwrap();
    let first = h.advance().unwrap();
    assert_eq!(first.frontier.len(), 2);
    let last = h.advance().unwrap();
    assert!(last.evidence_boundary);
    let cluster = first
        .frontier
        .iter()
        .find_map(|m| {
            if let weave_cluster::Member::Cluster(id) = m {
                h.cluster(id)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(cluster.leaves, vec!["a", "b"]);
    json!({"first":first,"last":last,"record":cluster,"evidence":h.evidence(&cluster.id,0,256).unwrap()})
}
fn geometry_profile() -> Value {
    use weave_spaces::*;
    let point = |values| Evidence {
        value: Coordinates {
            space: Space {
                id: "physical".into(),
                revision: "1".into(),
                geometry: Geometry::Physical3d {
                    frame: "origin".into(),
                    unit: Unit::Metre,
                },
            },
            role: VectorRole::Position,
            values,
        },
        valid_time: weave_contract::Interval {
            start: 0,
            end: Some(10),
        },
        visibility: Visibility::Principals(["reader".into()].into()),
        sources: [Source {
            graph_id: "survey".into(),
            revision: "1".into(),
            object_id: "sample".into(),
        }]
        .into(),
    };
    let a = point(vec![0.0, 0.0, 0.0]);
    let b = point(vec![3.0, 4.0, 0.0]);
    let d = distance("reader", 5, &a, &b).unwrap();
    assert_eq!(d.value.value, 5.0);
    let unavailable = distance("outsider", 5, &a, &b).unwrap_err();
    assert_eq!(unavailable.0, "E_UNAVAILABLE");
    json!({"measurement":d,"denied":unavailable.0,"expired":distance("reader",10,&a,&b).unwrap_err().0})
}
#[cfg(target_arch = "wasm32")]
fn wasm_profile() -> &'static Vec<u8> {
    static PROFILE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    PROFILE.get_or_init(golden_profile)
}
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn weave_golden_ptr() -> *const u8 {
    wasm_profile().as_ptr()
}
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn weave_golden_len() -> usize {
    wasm_profile().len()
}
#[cfg(test)]
mod tests {
    #[test]
    fn fixed_profile_is_deterministic() {
        assert_eq!(super::golden_profile(), super::golden_profile());
    }
}
