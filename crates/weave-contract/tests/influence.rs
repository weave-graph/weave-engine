use serde_json::json;
use weave_contract::*;
fn ctx() -> AlgebraContext {
    AlgebraContext {
        principal: "alice".into(),
        max_objects: 1000,
        max_output_bytes: 1024 * 1024,
    }
}
fn input() -> QueryResult {
    serde_json::from_value(json!({"version":VERSION,"graph":{"nodes":[{"id":"n","entity_id":"n","space_id":"s"}],"edges":[{"id":"p","predicate":"p","from":"n","to":"n","valid_time":{"start":0,"end":10},"derivations":[
        {"operator":"first","premises":[],"node_premises":[{"graph_id":"a","revision":"r","node_id":"n"}]},
        {"operator":"second","premises":[],"node_premises":[{"graph_id":"b","revision":"r","node_id":"n"}]}]}]},
        "snapshots":{},"input_snapshots":[],"coverage":"complete","diagnostics":[],"provenance":[],"metadata_graphs":[],"edge_origins":{"p":[]},"node_origins":{"n":[]}
    })).unwrap()
}
fn rules() -> RuleSet {
    let atom = |predicate: &str| RuleAtom {
        predicate: predicate.into(),
        from: RuleTerm::Variable { name: "x".into() },
        to: RuleTerm::Variable { name: "y".into() },
        polarity: Polarity::Positive,
    };
    RuleSet {
        id: "test".into(),
        revision: "1".into(),
        rules: vec![Rule {
            id: "step".into(),
            head: atom("q"),
            body: vec![atom("p")],
            allow_cross_space: false,
        }],
    }
}
#[test]
fn node_only_alternatives_remain_distinct_in_rules_support_and_explanation() {
    let source = input();
    let budget = RuleBudget {
        max_steps: 10000,
        max_rounds: 10,
        max_facts: 100,
        max_derivations: 128,
    };
    let out = rules::reason(source.clone(), &rules(), &budget, &ctx()).unwrap();
    let derived = out.graph.edges.iter().find(|e| e.predicate == "q").unwrap();
    assert_eq!(derived.derivations.len(), 2);
    assert!(derived
        .derivations
        .iter()
        .all(|g| g.premises.is_empty() && g.node_premises.len() == 1));
    assert_ne!(
        derived.derivations[0].node_premises,
        derived.derivations[1].node_premises
    );
    let repeated = rules::reason(out.clone(), &rules(), &budget, &ctx()).unwrap();
    assert_eq!(out.graph, repeated.graph);
    let entity = EntitySpace {
        entity_id: "n".into(),
        space_id: "s".into(),
    };
    let support = algebra::support(source.clone(), "p", &entity, &entity, 5, &ctx()).unwrap();
    assert_eq!(support.graph.edges[0].derivations.len(), 2);
    assert_eq!(support.graph.nodes[0].derived_nodes.len(), 2);
    let explained = identity::explain(&source, &ctx()).unwrap();
    let groups: Vec<_> = explained
        .graph
        .nodes
        .iter()
        .filter(|n| n.properties["kind"] == "derivation")
        .collect();
    assert_eq!(groups.len(), 2);
    assert!(groups.iter().all(|n| n.derived_nodes.len() == 1));
    assert_ne!(groups[0].derived_nodes, groups[1].derived_nodes);
    assert_eq!(
        explained
            .graph
            .nodes
            .iter()
            .filter(|n| n.properties["kind"] == "node_premise")
            .count(),
        2
    );
    assert!(explained
        .graph
        .edges
        .iter()
        .all(|e| e.derivations[0].node_premises.len() == 1));
}
#[test]
fn generated_protection_checks_profile_and_exact_final_byte_bound() {
    let mut source = input();
    source.graph.influence = Some(GraphInfluence {
        snapshots: vec![],
        assertions: vec![],
        nodes: vec![NodeRef {
            graph_id: "secret".into(),
            revision: "r".into(),
            node_id: "n".into(),
        }],
    });
    let mut protected = source.clone();
    influence::protect_generated_result(&mut protected, 1024 * 1024).unwrap();
    assert!(protected
        .graph
        .edges
        .iter()
        .flat_map(|e| &e.derivations)
        .all(|g| g.input_snapshots.iter().any(|p| p.graph_id == "secret")));
    let exact = serde_json::to_vec(&protected).unwrap().len();
    assert!(influence::protect_generated_result(&mut source.clone(), exact - 1).is_err());
    let mut explicit = source;
    explicit.graph.profile = GraphProfile::Explicit;
    assert_eq!(
        influence::protect_generated_result(&mut explicit, 1024 * 1024)
            .unwrap_err()
            .code,
        "E_INFLUENCE_PROFILE"
    );
}

fn snapshot(name: &str) -> GraphRef {
    GraphRef {
        graph_id: name.into(),
        revision: "exact".into(),
    }
}
fn snapshots_of(result: &QueryResult) -> Vec<GraphRef> {
    result
        .graph
        .influence
        .as_ref()
        .map(|i| i.snapshots.clone())
        .unwrap_or_default()
}
#[test]
fn declared_snapshot_gates_survive_empty_projection_selection_and_union_without_promoting_trace() {
    let mut source = input();
    source.graph.nodes[0].derived_snapshots = vec![snapshot("node")];
    source.graph.edges[0].derived_snapshots = vec![snapshot("edge")];
    source.input_snapshots.push(snapshot("descriptive-only"));
    source.graph.nodes[0].readers = vec!["alice".into()];
    source.graph.edges[0].readers = vec!["alice".into()];
    let original_node = source.graph.nodes[0].clone();
    let original_edge = source.graph.edges[0].clone();
    let projected = algebra::project(source.clone(), &[], &[], &ctx()).unwrap();
    assert!(projected.graph.nodes.is_empty() && projected.graph.edges.is_empty());
    assert_eq!(
        snapshots_of(&projected),
        vec![snapshot("edge"), snapshot("node")]
    );
    let selected = context::select(
        source.clone(),
        &ContextSelection::Pinned {
            reference: snapshot("world"),
        },
        &ctx(),
    )
    .unwrap();
    assert!(selected.graph.edges.is_empty());
    assert_eq!(snapshots_of(&selected), snapshots_of(&projected));
    let retained = algebra::project(source, &[], &["p".into()], &ctx()).unwrap();
    assert_eq!(retained.graph.nodes[0], original_node);
    assert_eq!(retained.graph.edges[0], original_edge);
    let united = algebra::union(projected.clone(), projected.clone(), &ctx()).unwrap();
    assert_eq!(snapshots_of(&united), snapshots_of(&projected));
    let changed = algebra::diff(projected.clone(), projected.clone(), &ctx()).unwrap();
    assert_eq!(snapshots_of(&changed), snapshots_of(&projected));
    let plain = input();
    assert!(influence::input_influence(&plain.graph).unwrap().is_none());
}
#[test]
fn generated_support_explanation_rules_and_graph_attachment_keep_snapshot_gates() {
    let mut source = input();
    source.graph.influence = Some(GraphInfluence {
        snapshots: vec![snapshot("empty-source")],
        ..GraphInfluence::default()
    });
    let entity = EntitySpace {
        entity_id: "n".into(),
        space_id: "s".into(),
    };
    let supported = algebra::support(source.clone(), "p", &entity, &entity, 5, &ctx()).unwrap();
    assert_eq!(
        supported.graph.nodes[0].derived_snapshots,
        vec![snapshot("empty-source")]
    );
    assert_eq!(
        supported.graph.edges[0].derived_snapshots,
        vec![snapshot("empty-source")]
    );
    // Snapshot gates are global AND; the two node-proof OR alternatives remain separate.
    assert_eq!(supported.graph.edges[0].derivations.len(), 2);
    assert!(supported.graph.edges[0]
        .derivations
        .iter()
        .all(|d| d.premises.is_empty() && d.node_premises.len() == 1));
    let explained = identity::explain(&source, &ctx()).unwrap();
    let pin = explained
        .graph
        .nodes
        .iter()
        .find(|n| n.type_id.as_deref() == Some("Snapshot"))
        .unwrap();
    assert_eq!(pin.properties["graph_id"], "empty-source");
    assert_eq!(pin.properties["revision"], "exact");
    assert_eq!(pin.derived_snapshots, vec![snapshot("empty-source")]);
    assert!(explained.provenance.is_empty());
    assert!(validate_schema_graph(&explained.graph).is_empty());
    let budget = RuleBudget {
        max_steps: 10000,
        max_rounds: 10,
        max_facts: 100,
        max_derivations: 128,
    };
    let reasoned = rules::reason(source.clone(), &rules(), &budget, &ctx()).unwrap();
    assert_eq!(
        reasoned
            .graph
            .edges
            .iter()
            .find(|e| e.predicate == "q")
            .unwrap()
            .derived_snapshots,
        vec![snapshot("empty-source")]
    );
    let repeated = rules::reason(reasoned.clone(), &rules(), &budget, &ctx()).unwrap();
    assert_eq!(repeated.graph, reasoned.graph);
    let mut generated = input();
    generated.graph.influence = source.graph.influence;
    generated.graph.attachments.push(serde_json::from_value(json!({"id":"summary","host":{"kind":"graph"},"key":"count","value":{"kind":"literal","value":0},"valid_time":{"start":0,"end":null}})).unwrap());
    influence::protect_generated_result(&mut generated, ctx().max_output_bytes).unwrap();
    let attachment = generated.graph.attachments.pop().unwrap();
    assert_eq!(attachment.derived_snapshots, vec![snapshot("empty-source")]);
    let detached = GraphData {
        attachments: vec![attachment],
        ..GraphData::default()
    };
    assert_eq!(
        influence::input_influence(&detached)
            .unwrap()
            .unwrap()
            .snapshots,
        vec![snapshot("empty-source")]
    );
}
#[test]
fn empty_support_and_explanation_do_not_invent_assertion_provenance() {
    let mut source = input();
    source.graph = GraphData {
        influence: Some(GraphInfluence {
            snapshots: vec![snapshot("empty")],
            ..GraphInfluence::default()
        }),
        ..GraphData::default()
    };
    source.node_origins.clear();
    source.edge_origins.clear();
    let entity = EntitySpace {
        entity_id: "n".into(),
        space_id: "s".into(),
    };
    let status = algebra::support(source.clone(), "p", &entity, &entity, 5, &ctx()).unwrap();
    assert_eq!(status.graph.nodes[0].properties["state"], "unknown");
    assert_eq!(
        status.graph.nodes[0].derived_snapshots,
        vec![snapshot("empty")]
    );
    assert!(status.graph.nodes[0].derived_from.is_empty() && status.provenance.is_empty());
    let explained = identity::explain(&source, &ctx()).unwrap();
    assert_eq!(explained.graph.nodes.len(), 1);
    assert!(explained.graph.edges.is_empty() && explained.provenance.is_empty());
    assert_eq!(explained.input_snapshots, vec![snapshot("empty")]);
    let union = algebra::union(explained.clone(), explained, &ctx()).unwrap();
    assert_eq!(union.graph.nodes.len(), 1);
}
#[test]
fn snapshot_fields_are_bounded_strict_and_part_of_combined_record_limits() {
    let oversized = vec![snapshot("x"); 1001];
    for data in [
        json!({"influence":{"snapshots":oversized}}),
        json!({"nodes":[{"id":"n","entity_id":"n","space_id":"s","derived_snapshots":oversized}]}),
        json!({"attachments":[{"id":"a","host":{"kind":"graph"},"key":"x","value":{"kind":"literal","value":0},"valid_time":{"start":0,"end":null},"derived_snapshots":oversized}]}),
    ] {
        assert!(serde_json::from_value::<GraphData>(data).is_err());
    }
    let mut source = input();
    source.graph.nodes[0].derived_snapshots = vec![snapshot("s"); 1000];
    source.graph.nodes[0].derived_nodes.push(NodeRef {
        graph_id: "g".into(),
        revision: "r".into(),
        node_id: "n".into(),
    });
    assert_eq!(
        influence::validate_graph(&source.graph).unwrap_err().code,
        "E_BUDGET"
    );
    let malformed = GraphInfluence {
        snapshots: vec![GraphRef {
            graph_id: "x".into(),
            revision: String::new(),
        }],
        ..GraphInfluence::default()
    };
    assert_eq!(
        influence::validate(&malformed).unwrap_err().code,
        "E_INFLUENCE"
    );
    let a = GraphInfluence {
        snapshots: (0..1000).map(|i| snapshot(&format!("g{i}"))).collect(),
        ..GraphInfluence::default()
    };
    assert_eq!(
        influence::merge(Some(&a), Some(&a))
            .unwrap()
            .unwrap()
            .snapshots
            .len(),
        1000
    );
    let b = GraphInfluence {
        snapshots: vec![snapshot("new")],
        ..GraphInfluence::default()
    };
    assert_eq!(
        influence::merge(Some(&a), Some(&b)).unwrap_err().code,
        "E_BUDGET"
    );
}

#[test]
fn legacy_graph_host_attachment_gates_are_not_silently_dropped() {
    let mut value = input();
    let node = NodeRef {
        graph_id: "secret".into(),
        revision: "r".into(),
        node_id: "n".into(),
    };
    let assertion = AssertionRef {
        graph_id: "secret".into(),
        revision: "r".into(),
        assertion_id: "a".into(),
    };
    value.graph.influence = Some(GraphInfluence {
        assertions: vec![assertion.clone()],
        nodes: vec![node.clone()],
        snapshots: vec![snapshot("gate")],
    });
    value.graph.attachments.push(serde_json::from_value(json!({"id":"summary","host":{"kind":"graph"},"key":"count","value":{"kind":"literal","value":0},"valid_time":{"start":0,"end":null}})).unwrap());
    influence::protect_generated_result(&mut value, ctx().max_output_bytes).unwrap();
    let attachment = value.graph.attachments.pop().unwrap();
    assert_eq!(attachment.derived_from, vec![assertion.clone()]);
    assert_eq!(attachment.derived_nodes, vec![node]);
    assert_eq!(attachment.derived_snapshots, vec![snapshot("gate")]);
    assert_eq!(value.attachment_origins["summary"], vec![assertion]);
    let detached = GraphData {
        attachments: vec![attachment],
        ..GraphData::default()
    };
    influence::validate_graph(&detached).unwrap();
    let pins = influence::snapshots(&detached);
    assert!(pins.contains(&snapshot("gate")));
    assert!(pins.contains(&GraphRef {
        graph_id: "secret".into(),
        revision: "r".into()
    }));
}

#[test]
fn diff_attachments_and_bridge_empty_selection_keep_declared_gates() {
    let mut source = input();
    source.graph.influence = Some(GraphInfluence {
        snapshots: vec![snapshot("gate")],
        ..GraphInfluence::default()
    });
    let empty = algebra::project(source.clone(), &[], &[], &ctx()).unwrap();
    let changed = algebra::diff(empty, source.clone(), &ctx()).unwrap();
    assert!(!changed.graph.attachments.is_empty());
    assert!(changed
        .graph
        .attachments
        .iter()
        .all(|a| a.derived_snapshots == vec![snapshot("gate")]));
    let bridge = counterpart::select(
        source,
        &CounterpartSelection {
            predicate: "absent".into(),
            entity_id: "n".into(),
            from_space_id: "s".into(),
            to_space_id: "other".into(),
            valid_at: 5,
        },
        &ctx(),
    )
    .unwrap();
    assert!(bridge.graph.edges.is_empty());
    assert_eq!(snapshots_of(&bridge), vec![snapshot("gate")]);
}
#[test]
fn collector_caps_union_before_clone_and_generated_precharge_prevents_mutation() {
    let mut source = input();
    source.graph.nodes[0].derived_snapshots =
        (0..1000).map(|i| snapshot(&format!("g{i}"))).collect();
    source.graph.edges[0].derived_snapshots = vec![snapshot("extra")];
    assert_eq!(
        influence::collect_declared_snapshots(&source.graph)
            .unwrap_err()
            .code,
        "E_BUDGET"
    );
    let mut generated = input();
    generated.graph.influence = Some(GraphInfluence {
        snapshots: vec![snapshot("gate")],
        ..GraphInfluence::default()
    });
    let original = generated.clone();
    let exact_existing_bytes = serde_json::to_vec(&generated).unwrap().len();
    assert_eq!(
        influence::protect_generated_result(&mut generated, exact_existing_bytes)
            .unwrap_err()
            .code,
        "E_BUDGET"
    );
    assert_eq!(generated, original);
    let groups = generated.graph.edges[0].derivations.clone();
    influence::protect_generated_result(&mut generated, ctx().max_output_bytes).unwrap();
    // Snapshot AND restrictions must not be folded into OR derivation groups.
    // Existing descriptive group pins are filled by the pre-existing helper.
    assert_eq!(
        groups
            .iter()
            .map(|g| (&g.premises, &g.node_premises, &g.parameters))
            .collect::<Vec<_>>(),
        generated.graph.edges[0]
            .derivations
            .iter()
            .map(|g| (&g.premises, &g.node_premises, &g.parameters))
            .collect::<Vec<_>>()
    );
}

#[test]
fn detached_attachment_legacy_gates_survive_empty_semantic_selection() {
    let mut source = input();
    let assertion = AssertionRef {
        graph_id: "private".into(),
        revision: "r".into(),
        assertion_id: "a".into(),
    };
    let node = NodeRef {
        graph_id: "private".into(),
        revision: "r".into(),
        node_id: "n".into(),
    };
    source.graph.attachments.push(serde_json::from_value(json!({"id":"summary","host":{"kind":"node","id":"n"},"key":"count","value":{"kind":"literal","value":0},"valid_time":{"start":0,"end":null},"derived_from":[assertion],"derived_nodes":[node]})).unwrap());
    let empty = algebra::project(source, &[], &[], &ctx()).unwrap();
    assert!(
        empty.graph.nodes.is_empty()
            && empty.graph.edges.is_empty()
            && empty.graph.attachments.is_empty()
    );
    let gates = empty.graph.influence.as_ref().unwrap();
    assert_eq!(gates.assertions, vec![assertion]);
    assert_eq!(gates.nodes, vec![node]);
    assert!(gates.snapshots.is_empty());
    let entity = EntitySpace {
        entity_id: "n".into(),
        space_id: "s".into(),
    };
    let status = algebra::support(empty, "p", &entity, &entity, 5, &ctx()).unwrap();
    assert_eq!(status.graph.nodes[0].derived_from.len(), 1);
    assert_eq!(status.graph.nodes[0].derived_nodes.len(), 1);
}
