use serde_json::json;
use weave_contract::{context_axes::ContextSchema, *};
use weave_engine::*;
fn host(name: &str) -> HostContext {
    HostContext::new(
        name,
        [
            "C".into(),
            "D".into(),
            "g".into(),
            "copy".into(),
            "empty".into(),
            "source".into(),
        ],
    )
}
fn program(commands: Vec<Command>) -> Program {
    Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands,
    }
}
fn schema() -> ContextSchema {
    ContextSchema::from_json(br#"{"reference":{"id":"World","revision":"1"},"axes":{"jurisdiction":{"kind":"enum","members":["EE","FI"]}}}"#).unwrap()
}
fn descriptor(readers: &[&str]) -> GraphData {
    serde_json::from_value(json!({"profile":"explicit","nodes":[{"id":"anchor","entity_id":"world","space_id":"contexts","readers":readers}],"structural_edges":[{"id":"describes","predicate":"weave:context:definition","from":"anchor","to":"anchor"}],"assertions":[{"id":"definition","edge_id":"describes","source":"test-author","valid_time":{"start":i64::MIN},"properties":{"weave.context":{"schema":schema(),"values":{"jurisdiction":"EE"}}}}]})).unwrap()
}
fn write(e: &mut Engine, id: &str, data: GraphData) -> GraphRef {
    e.execute(
        &program(vec![Command::Commit {
            graph_id: id.into(),
            branch_id: "main".into(),
            expected_head: e.head(id, "main").unwrap(),
            data,
        }]),
        &host("alice"),
    )
    .unwrap();
    GraphRef {
        graph_id: id.into(),
        revision: e.head(id, "main").unwrap().unwrap(),
    }
}
fn query(id: &str) -> GraphExpression {
    GraphExpression::Query {
        query: serde_json::from_value(json!({"graph_id":id})).unwrap(),
    }
}
fn typed(input: GraphExpression, pin: &GraphRef) -> GraphExpression {
    GraphExpression::TypedContext {
        input: Box::new(input),
        reference: pin.clone(),
        expected_schema: schema(),
    }
}
fn evaluate(e: &mut Engine, input: GraphExpression, who: &str) -> Result<QueryResult> {
    let results = e.execute(
        &program(vec![Command::Evaluate { value: input }]),
        &host(who),
    )?;
    match results.into_iter().next().unwrap() {
        CommandResult::Queried { result } => Ok(*result),
        _ => panic!(),
    }
}
fn support(input: GraphExpression) -> GraphExpression {
    GraphExpression::Support {
        input: Box::new(input),
        predicate: "p".into(),
        from: EntitySpace {
            entity_id: "a".into(),
            space_id: "s".into(),
        },
        to: EntitySpace {
            entity_id: "b".into(),
            space_id: "s".into(),
        },
        valid_at: 5,
    }
}
fn setup() -> (Engine, GraphRef) {
    let mut e = Engine::memory().unwrap();
    let c = write(&mut e, "C", descriptor(&["alice"]));
    write(&mut e, "g", GraphData::default());
    (e, c)
}
#[test]
fn private_empty_context_survives_union_support_persistence_and_capsule_boundary() {
    let (mut e, c) = setup();
    let empty = evaluate(&mut e, typed(query("g"), &c), "alice").unwrap();
    assert!(empty.graph.nodes.is_empty());
    assert_eq!(
        empty.graph.context_typing.as_ref().unwrap().selected,
        Some(c.clone())
    );
    write(&mut e, "empty", empty.graph);
    let reread = evaluate(&mut e, query("empty"), "alice").unwrap();
    assert_eq!(
        reread.selected_context,
        Some(ContextSelection::Pinned {
            reference: c.clone()
        })
    );
    let union = GraphExpression::Union {
        left: Box::new(query("empty")),
        right: Box::new(query("g")),
    };
    let mut out = evaluate(&mut e, support(union), "alice").unwrap();
    assert_eq!(out.graph.nodes[0].properties["state"], "unknown");
    assert!(out
        .graph
        .context_typing
        .as_ref()
        .unwrap()
        .selected
        .is_none());
    assert!(out.graph.nodes[0]
        .derived_from
        .iter()
        .any(|p| p.graph_id == "C"));
    for n in &mut out.graph.nodes {
        n.readers.clear();
    }
    out.graph.edges.clear();
    let copy = write(&mut e, "copy", out.graph);
    let denied = evaluate(&mut e, query("copy"), "bob").unwrap();
    assert_eq!(denied.coverage, Coverage::Partial);
    assert!(denied.graph.nodes.is_empty());
    assert!(denied.graph.context_typing.is_none());
    assert!(denied.selected_context.is_none());
    assert!(e.export_capsule(&copy, &host("bob")).is_err());
    assert_eq!(
        evaluate(&mut e, query("copy"), "alice")
            .unwrap()
            .graph
            .nodes
            .len(),
        1
    );
    let capsule = e.export_capsule(&copy, &host("alice")).unwrap();
    assert!(capsule.revisions.iter().any(|r| r.graph_id == "C"));
}
#[test]
fn explicit_interpretation_is_generic_and_never_returns_complete_empty_on_denial() {
    let (mut e, c) = setup();
    let denied = evaluate(&mut e, typed(query("g"), &c), "bob").unwrap_err();
    let missing = evaluate(
        &mut e,
        typed(
            query("g"),
            &GraphRef {
                graph_id: "missing".into(),
                revision: "r".into(),
            },
        ),
        "bob",
    )
    .unwrap_err();
    let mut bad = descriptor(&[]);
    bad.assertions[0].polarity = Polarity::Negative;
    let d = write(&mut e, "D", bad);
    let malformed = evaluate(&mut e, typed(query("g"), &d), "bob").unwrap_err();
    assert_eq!(denied.code, "E_CONTEXT_UNAVAILABLE");
    assert_eq!(
        (denied.code, denied.message),
        (missing.code, missing.message)
    );
    assert_eq!(malformed.code, "E_CONTEXT_UNAVAILABLE");
}
#[test]
fn old_protocols_reject_operator_and_carrier_before_any_writes() {
    let (mut e, c) = setup();
    let value = evaluate(&mut e, typed(query("g"), &c), "alice").unwrap();
    for payload in [
        Command::Evaluate {
            value: typed(query("g"), &c),
        },
        Command::Commit {
            graph_id: "copy".into(),
            branch_id: "main".into(),
            expected_head: None,
            data: value.graph,
        },
    ] {
        let mut p = program(vec![
            Command::Commit {
                graph_id: "source".into(),
                branch_id: "main".into(),
                expected_head: None,
                data: GraphData::default(),
            },
            payload,
        ]);
        p.version = "0.13.0".into();
        assert_eq!(e.execute(&p, &host("alice")).unwrap_err().code, "E_VERSION");
        assert!(e.head("source", "main").unwrap().is_none());
    }
}
#[test]
fn forged_witness_cannot_relabel_a_source_or_restore_authority() {
    let (mut e, c) = setup();
    let mut value = evaluate(&mut e, typed(query("g"), &c), "alice").unwrap();
    value.graph.context_typing.as_mut().unwrap().witnesses[0].anchor_nodes[0].node_id =
        "forged".into();
    write(&mut e, "copy", value.graph);
    let denied = evaluate(&mut e, query("copy"), "alice").unwrap();
    assert_eq!(denied.coverage, Coverage::Partial);
    assert!(denied.graph.context_typing.is_none());
}
#[test]
fn descriptor_current_policy_revocation_blocks_saved_values_and_stale_views_without_head_change() {
    let mut e = Engine::memory().unwrap();
    let src = write(
        &mut e,
        "source",
        serde_json::from_value(json!({"nodes":[{"id":"n","entity_id":"E","space_id":"s"}]}))
            .unwrap(),
    );
    let policy = IdentityPolicy {
        reference: IdentityPolicyRef {
            id: "review".into(),
            revision: "1".into(),
        },
        proposers: vec!["alice".into()],
        approvers: vec!["alice".into()],
        readers: vec![],
        allowed_spaces: vec!["s".into()],
        max_members: 4,
    };
    e.install_identity_policy(&policy).unwrap();
    let candidate = IdentityCandidate {
        id: "candidate".into(),
        mapping_id: "mapping".into(),
        policy: policy.reference.clone(),
        groups: vec![vec![NodeRef {
            graph_id: src.graph_id,
            revision: src.revision,
            node_id: "n".into(),
        }]],
        evidence: vec![],
        valid_time: Interval {
            start: 0,
            end: None,
        },
        context: None,
    };
    e.submit_identity_candidate(&candidate, &host("alice"))
        .unwrap();
    let accepted = e
        .accept_identity_candidate(
            &IdentityDecisionRequest {
                candidate_id: candidate.id,
                expected_head: None,
                nonce: "accept".into(),
            },
            &host("alice"),
        )
        .unwrap();
    let member=e.query(&serde_json::from_value(json!({"graph_id":accepted.reference.graph_id,"revision":accepted.reference.revision})).unwrap(),&host("alice")).unwrap();
    let mut definition = descriptor(&[]);
    definition.nodes[0].derived_nodes.push(NodeRef {
        graph_id: accepted.reference.graph_id,
        revision: accepted.reference.revision,
        node_id: member.graph.nodes[0].id.clone(),
    });
    let c = write(&mut e, "C", definition);
    write(&mut e, "g", GraphData::default());
    let expression = support(typed(query("g"), &c));
    let value = evaluate(&mut e, expression.clone(), "alice").unwrap();
    let copy = write(&mut e, "copy", value.graph);
    e.register_view(
        &ViewDefinition {
            id: "typed".into(),
            expression,
            clock: ViewClock::Fixed,
        },
        None,
        &host("alice"),
    )
    .unwrap();
    let typed_empty = evaluate(&mut e, typed(query("g"), &c), "alice").unwrap();
    let source = write(&mut e, "g", typed_empty.graph);
    let request = ClusterRequest {
        source,
        context: ContextSelection::Pinned {
            reference: c.clone(),
        },
        valid_at: 5,
        predicate: "p".into(),
        levels: 1,
    };
    let mut lineage = e
        .cluster_lineage(&request, &request, &host("alice"))
        .unwrap();
    lineage.graph.context_typing = None;
    for node in &mut lineage.graph.nodes {
        node.readers.clear();
    }
    write(&mut e, "empty", lineage.graph);
    let head = e.head("C", "main").unwrap();
    e.revoke_identity_policy(&policy.reference).unwrap();
    assert_eq!(e.head("C", "main").unwrap(), head);
    assert!(e
        .read_view("typed", None, ViewFreshness::AllowStale, &host("alice"))
        .is_err());
    let result = evaluate(&mut e, query("copy"), "alice").unwrap();
    assert!(result.graph.nodes.is_empty());
    assert!(result.graph.context_typing.is_none());
    assert_eq!(result.coverage, Coverage::Partial);
    assert!(e.export_capsule(&copy, &host("alice")).is_err());
    assert!(evaluate(&mut e, query("empty"), "alice")
        .unwrap()
        .graph
        .nodes
        .is_empty());
}
#[test]
fn carriers_reject_qualifier_forgery_and_schema_label_conflicts_without_partial_commits() {
    let (mut e, c) = setup();
    let original = evaluate(&mut e, typed(query("g"), &c), "alice").unwrap();
    let mut conflicting = original.graph.clone();
    conflicting.context_typing.as_mut().unwrap().witnesses[0]
        .schema
        .axes
        .insert("new-axis".into(), context_axes::ContextAxisType::Boolean);
    write(&mut e, "empty", original.graph.clone());
    // A changed expected schema cannot interpret an unchanged descriptor.
    let bad = GraphExpression::TypedContext {
        input: Box::new(query("g")),
        reference: c.clone(),
        expected_schema: conflicting.context_typing.as_ref().unwrap().witnesses[0]
            .schema
            .clone(),
    };
    assert_eq!(
        evaluate(&mut e, bad, "alice").unwrap_err().code,
        "E_CONTEXT_UNAVAILABLE"
    );
    let mut forged = original.graph;
    let mut unqualified:GraphData=serde_json::from_value(json!({"nodes":[{"id":"a","entity_id":"a","space_id":"s"}],"edges":[{"id":"e","predicate":"p","from":"a","to":"a","valid_time":{"start":0}}]})).unwrap();
    forged.nodes.append(&mut unqualified.nodes);
    forged.edges.append(&mut unqualified.edges);
    let before = e.event_count().unwrap();
    assert!(e
        .execute(
            &program(vec![
                Command::Commit {
                    graph_id: "source".into(),
                    branch_id: "main".into(),
                    expected_head: None,
                    data: GraphData::default()
                },
                Command::Commit {
                    graph_id: "copy".into(),
                    branch_id: "main".into(),
                    expected_head: None,
                    data: forged
                }
            ]),
            &host("alice")
        )
        .is_err());
    assert_eq!(e.event_count().unwrap(), before);
    assert!(e.head("source", "main").unwrap().is_none());
}
#[test]
fn generated_support_explanation_and_geometry_nodes_keep_descriptor_gates_without_carrier() {
    let (mut e, c) = setup();
    let point = |v: [f64; 3]| json!({"kind":"coordinates","space":{"id":"world","revision":"1","geometry":{"kind":"physical3d","frame":"world","unit":"metre"}},"role":"position","values":v});
    let data:GraphData=serde_json::from_value(json!({"profile":"explicit","nodes":[{"id":"a","entity_id":"A","space_id":"world"},{"id":"b","entity_id":"B","space_id":"world"}],"structural_edges":[{"id":"sa","predicate":"coordinates","from":"a","to":"a"},{"id":"sb","predicate":"coordinates","from":"b","to":"b"}],"assertions":[{"id":"pa","edge_id":"sa","source":"sensor-a","context":c,"properties":{"weave.geometry":point([0.,0.,0.])},"valid_time":{"start":0}},{"id":"pb","edge_id":"sb","source":"sensor-b","context":c,"properties":{"weave.geometry":point([3.,4.,0.])},"valid_time":{"start":0}}]})).unwrap();
    write(&mut e, "source", data);
    let geometry = GraphExpression::Geometry {
        operation: GeometryOperation::Distance {
            left: GeometryOperand {
                input: Box::new(typed(query("source"), &c)),
                assertion_id: "pa".into(),
            },
            right: GeometryOperand {
                input: Box::new(typed(query("source"), &c)),
                assertion_id: "pb".into(),
            },
        },
        valid_at: 5,
    };
    for expression in [
        support(typed(query("g"), &c)),
        GraphExpression::Explain {
            input: Box::new(geometry.clone()),
        },
        geometry,
    ] {
        let mut value = evaluate(&mut e, expression, "alice").unwrap();
        assert!(!value.graph.nodes.is_empty());
        assert!(value
            .graph
            .nodes
            .iter()
            .all(|n| n.derived_from.iter().any(|p| p.graph_id == "C")
                && n.derived_nodes.iter().any(|p| p.graph_id == "C")));
        let count = value.graph.nodes.len();
        value.graph.context_typing = None;
        value.graph.edges.clear();
        value.graph.attachments.clear();
        for n in &mut value.graph.nodes {
            n.readers.clear();
        }
        write(&mut e, "copy", value.graph);
        assert!(evaluate(&mut e, query("copy"), "bob")
            .unwrap()
            .graph
            .nodes
            .is_empty());
        assert_eq!(
            evaluate(&mut e, query("copy"), "alice")
                .unwrap()
                .graph
                .nodes
                .len(),
            count
        );
    }
}
#[test]
fn missing_metadata_navigation_preserves_empty_private_context_influence() {
    let (mut e, c) = setup();
    let expression = GraphExpression::Metadata {
        input: Box::new(typed(query("g"), &c)),
        host: MetadataHost::Graph,
        key: "missing".into(),
    };
    let value = evaluate(&mut e, support(expression), "alice").unwrap();
    assert!(value.graph.context_typing.is_some());
    assert_eq!(value.coverage, Coverage::Partial);
    let mut graph = value.graph;
    graph.context_typing = None;
    for n in &mut graph.nodes {
        n.readers.clear();
    }
    write(&mut e, "copy", graph);
    assert!(evaluate(&mut e, query("copy"), "bob")
        .unwrap()
        .graph
        .nodes
        .is_empty());
}
#[test]
fn native_cluster_and_accepted_identity_outputs_retain_typed_source_influence() {
    let (mut e, c) = setup();
    for (id, space) in [("source", "physical"), ("g", "operations")] {
        write(
            &mut e,
            id,
            serde_json::from_value(json!({"nodes":[{"id":"n","entity_id":id,"space_id":space}]}))
                .unwrap(),
        );
        let typed_value = evaluate(&mut e, typed(query(id), &c), "alice").unwrap();
        write(&mut e, id, typed_value.graph);
    }
    let source = NodeRef {
        graph_id: "source".into(),
        revision: e.head("source", "main").unwrap().unwrap(),
        node_id: "n".into(),
    };
    let target = NodeRef {
        graph_id: "g".into(),
        revision: e.head("g", "main").unwrap().unwrap(),
        node_id: "n".into(),
    };
    let selected = ContextSelection::Pinned {
        reference: c.clone(),
    };
    let navigation = e
        .cluster_navigation(
            &ClusterRequest {
                source: GraphRef {
                    graph_id: source.graph_id.clone(),
                    revision: source.revision.clone(),
                },
                context: selected.clone(),
                valid_at: 5,
                predicate: "connected".into(),
                levels: 0,
            },
            &host("alice"),
        )
        .unwrap();
    assert!(navigation.graph.context_typing.is_some());
    let policy = IdentityPolicy {
        reference: IdentityPolicyRef {
            id: "review".into(),
            revision: "typed".into(),
        },
        proposers: vec!["alice".into()],
        approvers: vec!["alice".into()],
        readers: vec![],
        allowed_spaces: vec!["physical".into(), "operations".into()],
        max_members: 4,
    };
    e.install_identity_policy(&policy).unwrap();
    e.submit_identity_candidate(
        &IdentityCandidate {
            id: "typed-candidate".into(),
            mapping_id: "equipment".into(),
            policy: policy.reference.clone(),
            groups: vec![vec![source.clone(), target]],
            evidence: vec![],
            valid_time: Interval {
                start: 0,
                end: None,
            },
            context: Some(c),
        },
        &host("alice"),
    )
    .unwrap();
    let accepted = e
        .accept_identity_candidate(
            &IdentityDecisionRequest {
                candidate_id: "typed-candidate".into(),
                expected_head: None,
                nonce: "typed-accept".into(),
            },
            &host("alice"),
        )
        .unwrap();
    let resolved = e
        .resolve_identity(
            &IdentityResolve {
                mapping_id: "equipment".into(),
                revision: accepted.reference.revision,
                policy: policy.reference,
                source,
                target_space: "operations".into(),
                valid_at: 5,
                context: selected,
            },
            &host("alice"),
        )
        .unwrap();
    assert!(resolved.graph.context_typing.is_some());
    for mut value in [navigation, resolved] {
        assert!(!value.graph.nodes.is_empty());
        value.graph.context_typing = None;
        value.graph.edges.clear();
        value.graph.attachments.clear();
        for node in &mut value.graph.nodes {
            node.readers.clear();
        }
        write(&mut e, "copy", value.graph);
        assert!(evaluate(&mut e, query("copy"), "bob")
            .unwrap()
            .graph
            .nodes
            .is_empty());
        assert!(!evaluate(&mut e, query("copy"), "alice")
            .unwrap()
            .graph
            .nodes
            .is_empty());
    }
}

#[test]
fn root_native_lineage_empty_typed_inputs_protect_reader_cleared_summary() {
    let (mut e, c) = setup();
    let value = evaluate(&mut e, typed(query("g"), &c), "alice").unwrap();
    let source = write(&mut e, "g", value.graph);
    let request = ClusterRequest {
        source,
        context: ContextSelection::Pinned { reference: c },
        valid_at: 5,
        predicate: "p".into(),
        levels: 1,
    };
    let mut result = e
        .cluster_lineage(&request, &request, &host("alice"))
        .unwrap();
    assert_eq!(result.graph.nodes.len(), 1);
    assert!(result.graph.context_typing.is_some());
    assert!(!result.graph.nodes[0].derived_from.is_empty());
    assert!(!result.graph.nodes[0].derived_nodes.is_empty());
    result.graph.context_typing = None;
    result.graph.nodes[0].readers.clear();
    write(&mut e, "copy", result.graph);
    assert!(evaluate(&mut e, query("copy"), "bob")
        .unwrap()
        .graph
        .nodes
        .is_empty());
    assert_eq!(
        evaluate(&mut e, query("copy"), "alice")
            .unwrap()
            .graph
            .nodes
            .len(),
        1
    );
}

#[test]
fn root_typed_lineage_records_match_navigation_and_context_gates_reserve_a_pin() {
    let (mut e, c) = setup();
    let graph: GraphData = serde_json::from_value(json!({
        "nodes":[{"id":"a","entity_id":"a","space_id":"s"},{"id":"b","entity_id":"b","space_id":"s"}],
        "edges":[{"id":"ab","from":"a","to":"b","predicate":"p","assertion_context":c,"valid_time":{"start":0}}]
    })).unwrap();
    write(&mut e, "g", graph);
    let typed_value = evaluate(&mut e, typed(query("g"), &c), "alice").unwrap();
    let source = write(&mut e, "g", typed_value.graph);
    let request = ClusterRequest {
        source,
        context: ContextSelection::Pinned {
            reference: c.clone(),
        },
        valid_at: 5,
        predicate: "p".into(),
        levels: 1,
    };
    let navigation = e.cluster_navigation(&request, &host("alice")).unwrap();
    let cluster = navigation
        .graph
        .nodes
        .iter()
        .find(|n| n.properties["kind"] == "cluster")
        .unwrap();
    let lineage = e
        .cluster_lineage(&request, &request, &host("alice"))
        .unwrap();
    let overlap = &lineage.graph.nodes[0].properties["lineage"]["overlaps"][0];
    assert_eq!(
        overlap["before"]["revision"],
        cluster.properties["record"]["revision"]
    );
    assert_eq!(
        overlap["after"]["revision"],
        cluster.properties["record"]["revision"]
    );
    let many: GraphData = serde_json::from_value(json!({"nodes":(0..998).map(|i|json!({"id":format!("n{i}"),"entity_id":format!("n{i}"),"space_id":"s"})).collect::<Vec<_>>()})).unwrap();
    write(&mut e, "source", many);
    let mut typed_value = evaluate(&mut e, typed(query("source"), &c), "alice").unwrap();
    // This snapshot's whole-value carrier protects public original nodes. Avoid
    // redundant per-record gates here to isolate the generated output capacity.
    for node in &mut typed_value.graph.nodes {
        node.derived_from.clear();
        node.derived_nodes.clear();
    }
    let source = write(&mut e, "source", typed_value.graph);
    let request = ClusterRequest {
        source,
        context: ContextSelection::Pinned { reference: c },
        valid_at: 5,
        predicate: "p".into(),
        levels: 0,
    };
    assert_eq!(
        e.cluster_lineage(&request, &request, &host("alice"))
            .unwrap_err()
            .code,
        "E_CLUSTER_BUDGET"
    );
    assert_eq!(
        e.cluster_navigation(&request, &host("alice"))
            .unwrap_err()
            .code,
        "E_CLUSTER_BUDGET"
    );
}
