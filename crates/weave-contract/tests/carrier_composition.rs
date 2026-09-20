use serde_json::json;
use weave_contract::*;
fn ctx() -> AlgebraContext {
    AlgebraContext {
        principal: "reader".into(),
        max_objects: 1000,
        max_output_bytes: 8 * 1024 * 1024,
    }
}
fn choice() -> GraphInfluence {
    serde_json::from_value(json!({"derivations":[{"operator":"first","premises":[],"snapshot_premises":[{"graph_id":"A","revision":"r"}]},{"operator":"second","premises":[],"snapshot_premises":[{"graph_id":"B","revision":"r"}]}]})).unwrap()
}
fn input() -> QueryResult {
    serde_json::from_value(json!({"version":VERSION,"graph":{"nodes":[{"id":"n","entity_id":"n","space_id":"s"}],"edges":[{"id":"e","predicate":"p","from":"n","to":"n","valid_time":{"start":0,"end":10},"derivations":choice().derivations}]},"snapshots":{},"input_snapshots":[],"coverage":"complete","diagnostics":[],"provenance":[],"metadata_graphs":[],"node_origins":{"n":[]},"edge_origins":{"e":[]}})).unwrap()
}
fn allowed(groups: &[Derivation], a: bool, b: bool) -> bool {
    groups.is_empty()
        || groups.iter().any(|g| {
            g.snapshot_premises
                .iter()
                .all(|p| match p.graph_id.as_str() {
                    "A" => a,
                    "B" => b,
                    _ => false,
                })
        })
}
fn assert_choice(groups: &[Derivation]) {
    assert!(!groups.is_empty());
    for (a, b) in [(false, false), (false, true), (true, false), (true, true)] {
        assert_eq!(
            allowed(groups, a, b),
            a || b,
            "groups={groups:?}, a={a},b={b}"
        );
    }
}
#[test]
fn empty_value_or_survives_projection_union_scalar_and_detached_records() {
    let mut source = input();
    source.graph.influence = Some(choice());
    let empty = algebra::project(source, &[], &[], &ctx()).unwrap();
    let combined = algebra::union(empty.clone(), empty, &ctx()).unwrap();
    assert_choice(&combined.graph.influence.as_ref().unwrap().derivations);
    let entity = EntitySpace {
        entity_id: "n".into(),
        space_id: "s".into(),
    };
    let mut scalar = algebra::support(combined, "p", &entity, &entity, 5, &ctx()).unwrap();
    scalar.graph.influence = None;
    scalar.graph.context_typing = None;
    for n in &mut scalar.graph.nodes {
        n.readers.clear();
        assert_choice(&n.derivations);
    }
    influence::validate_graph(&scalar.graph).unwrap();
}
#[test]
fn snapshot_only_evidence_survives_rules_support_and_explanation() {
    let source = input();
    let entity = EntitySpace {
        entity_id: "n".into(),
        space_id: "s".into(),
    };
    let supported = algebra::support(source.clone(), "p", &entity, &entity, 5, &ctx()).unwrap();
    assert_choice(&supported.graph.nodes[0].derivations);
    assert_choice(&supported.graph.edges[0].derivations);
    let set:RuleSet=serde_json::from_value(json!({"id":"rules","revision":"1","rules":[{"id":"r","head":{"predicate":"q","from":{"kind":"variable","name":"x"},"to":{"kind":"variable","name":"y"},"polarity":"positive"},"body":[{"predicate":"p","from":{"kind":"variable","name":"x"},"to":{"kind":"variable","name":"y"},"polarity":"positive"}]}]})).unwrap();
    let budget = RuleBudget {
        max_steps: 10000,
        max_rounds: 10,
        max_facts: 100,
        max_derivations: 128,
    };
    let reasoned = rules::reason(source.clone(), &set, &budget, &ctx()).unwrap();
    assert_choice(
        &reasoned
            .graph
            .edges
            .iter()
            .find(|e| e.predicate == "q")
            .unwrap()
            .derivations,
    );
    let repeated = rules::reason(reasoned.clone(), &set, &budget, &ctx()).unwrap();
    assert_eq!(reasoned.graph, repeated.graph);
    let explained = identity::explain(&source, &ctx()).unwrap();
    let conclusion = explained
        .graph
        .nodes
        .iter()
        .find(|n| n.properties["kind"] == "conclusion")
        .unwrap();
    assert_choice(&conclusion.derivations);
    let snapshots: Vec<_> = explained
        .graph
        .nodes
        .iter()
        .filter(|n| n.type_id.as_deref() == Some("Snapshot"))
        .collect();
    assert_eq!(snapshots.len(), 2);
    assert!(snapshots
        .iter()
        .all(|n| n.derivations.len() == 1 && n.properties.get("revision") == Some(&json!("r"))));
    assert!(explained
        .graph
        .edges
        .iter()
        .all(|e| !e.derivations[0].snapshot_premises.is_empty()));
}
#[test]
fn attachment_local_or_is_not_promoted_but_existing_flat_gates_are() {
    let mut source = input();
    let attachment:MetadataAttachment=serde_json::from_value(json!({"id":"m","host":{"kind":"graph"},"key":"m","value":{"kind":"literal","value":1},"valid_time":{"start":0,"end":10},"derivations":choice().derivations,"derived_nodes":[{"graph_id":"D","revision":"r","node_id":"n"}]})).unwrap();
    source.graph.attachments.push(attachment);
    let carrier = influence::input_influence(&source.graph).unwrap().unwrap();
    assert!(carrier.derivations.is_empty() && carrier.snapshots.is_empty());
    assert_eq!(carrier.nodes[0].graph_id, "D");
    // An actual selected metadata path supplies the OR carrier explicitly, including
    // for empty targets; portable protection must preserve it after envelope removal.
    source.graph.influence = Some(choice());
    influence::protect_generated_result(&mut source, ctx().max_output_bytes).unwrap();
    assert_choice(&source.graph.attachments[0].derivations);
    assert_eq!(source.graph.attachments[0].derived_nodes[0].graph_id, "D");
}
#[test]
fn new_snapshot_profile_caps_raw_aggregate_and_protection_fails_atomically() {
    let mut source = input();
    source.graph.edges[0].derivations[0].snapshot_premises = vec![
        GraphRef {
            graph_id: "A".into(),
            revision: "r".into()
        };
        501
    ];
    source.graph.edges[0].derivations[1].snapshot_premises = vec![
        GraphRef {
            graph_id: "B".into(),
            revision: "r".into()
        };
        500
    ];
    assert!(influence::validate_graph(&source.graph).is_err());
    let mut source = input();
    source.graph.influence = Some(choice());
    let before = source.clone();
    assert!(influence::protect_generated_result(&mut source, 20).is_err());
    assert_eq!(source, before);
}

#[test]
fn snapshot_only_edge_routes_full_budget_profile_but_legacy_groups_keep_limits() {
    let mut source = input();
    source.graph.edges[0].derivations.truncate(1);
    source.graph.edges[0].derivations[0].snapshot_premises = (0..1000)
        .map(|n| GraphRef {
            graph_id: format!("S{n}"),
            revision: "r".into(),
        })
        .collect();
    source.graph.influence = Some(GraphInfluence {
        nodes: vec![NodeRef {
            graph_id: "D".into(),
            revision: "r".into(),
            node_id: "n".into(),
        }],
        ..GraphInfluence::default()
    });
    let before = source.clone();
    assert!(influence::protect_generated_result(&mut source, ctx().max_output_bytes).is_err());
    assert_eq!(source, before);
    let mut old = input();
    for (i, g) in old.graph.edges[0].derivations.iter_mut().enumerate() {
        g.snapshot_premises.clear();
        g.node_premises = (0..600)
            .map(|n| NodeRef {
                graph_id: format!("old{i}"),
                revision: "r".into(),
                node_id: n.to_string(),
            })
            .collect();
    }
    old.graph.influence = before.graph.influence;
    assert!(!carrier_profile::requires_v019(&old.graph));
    influence::protect_generated_result(&mut old, ctx().max_output_bytes).unwrap();
}

#[test]
fn unchanged_window_retains_empty_union_influence_through_generated_scalar() {
    let mut public = input();
    public.node_origins.insert(
        "n".into(),
        vec![NodeRef {
            graph_id: "public".into(),
            revision: "r".into(),
            node_id: "n".into(),
        }],
    );
    let mut empty = input();
    empty.graph = GraphData::default();
    empty.graph.influence = Some(choice());
    let combined = algebra::union(public, empty, &ctx()).unwrap();
    let before = combined.graph.clone();
    let bounds = Interval {
        start: -100,
        end: None,
    };
    let selected = temporal::window(combined, &bounds, &ctx()).unwrap();
    assert_eq!(selected.graph.nodes, before.nodes);
    assert_eq!(selected.graph.edges, before.edges);
    assert_choice(&selected.graph.influence.as_ref().unwrap().derivations);
    let again = temporal::window(selected, &bounds, &ctx()).unwrap();
    assert_choice(&again.graph.influence.as_ref().unwrap().derivations);
    let entity = EntitySpace {
        entity_id: "n".into(),
        space_id: "s".into(),
    };
    let mut scalar = algebra::support(again, "p", &entity, &entity, 5, &ctx()).unwrap();
    scalar.graph.influence = None;
    for node in &mut scalar.graph.nodes {
        node.readers.clear();
        assert_choice(&node.derivations);
    }
    influence::validate_graph(&scalar.graph).unwrap();
}
