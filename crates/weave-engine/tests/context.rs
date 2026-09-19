use serde_json::json;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new(
        "alice",
        [
            "g".into(),
            "C".into(),
            "D".into(),
            "target".into(),
            "container".into(),
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
fn write(e: &mut Engine, g: &str, data: GraphData) -> GraphRef {
    let head = e.head(g, "main").unwrap();
    e.execute(
        &program(vec![Command::Commit {
            graph_id: g.into(),
            branch_id: "main".into(),
            expected_head: head,
            data,
        }]),
        &host(),
    )
    .unwrap();
    GraphRef {
        graph_id: g.into(),
        revision: e.head(g, "main").unwrap().unwrap(),
    }
}
fn setup() -> (Engine, GraphRef, GraphRef) {
    let mut e = Engine::memory().unwrap();
    let c = write(&mut e, "C", GraphData::default());
    let d = write(&mut e, "D", GraphData::default());
    (e, c, d)
}
fn query(g: &str) -> GraphExpression {
    GraphExpression::Query {
        query: serde_json::from_value(json!({"graph_id":g,"include_metadata":true})).unwrap(),
    }
}
fn selected(input: GraphExpression, c: &GraphRef) -> GraphExpression {
    GraphExpression::Context {
        input: Box::new(input),
        selection: ContextSelection::Pinned {
            reference: c.clone(),
        },
    }
}
fn evaluate(e: &mut Engine, value: GraphExpression) -> Result<QueryResult> {
    let out = e.execute(&program(vec![Command::Evaluate { value }]), &host())?;
    match out.into_iter().next().unwrap() {
        CommandResult::Queried { result } => Ok(*result),
        _ => panic!("expected result"),
    }
}
fn data(c: &GraphRef, d: &GraphRef) -> GraphData {
    serde_json::from_value(json!({"profile":"explicit","nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"},{"id":"z","entity_id":"Z","space_id":"s"}],"structural_edges":[{"id":"ab","predicate":"p","from":"a","to":"b"},{"id":"bz","predicate":"q","from":"b","to":"z"}],"assertions":[{"id":"positive","edge_id":"ab","source":"one","context":c,"valid_time":{"start":0,"end":10}},{"id":"negative","edge_id":"ab","source":"two","context":d,"polarity":"negative","valid_time":{"start":0,"end":10}},{"id":"step","edge_id":"bz","source":"three","context":c,"valid_time":{"start":0,"end":10}}]})).unwrap()
}
fn support(input: GraphExpression) -> GraphExpression {
    GraphExpression::Support {
        input: Box::new(input),
        predicate: "p".into(),
        from: EntitySpace {
            entity_id: "A".into(),
            space_id: "s".into(),
        },
        to: EntitySpace {
            entity_id: "B".into(),
            space_id: "s".into(),
        },
        valid_at: 5,
    }
}
#[test]
fn selected_support_never_conflates_worlds_and_default_is_not_a_wildcard() {
    let (mut e, c, d) = setup();
    write(&mut e, "g", data(&c, &d));
    assert_eq!(
        evaluate(&mut e, support(query("g"))).unwrap_err().code,
        "E_CONTEXT_REQUIRED"
    );
    let a = evaluate(&mut e, support(selected(query("g"), &c))).unwrap();
    assert_eq!(a.graph.nodes[0].properties["state"], "supported");
    assert_eq!(
        a.selected_context,
        Some(ContextSelection::Pinned {
            reference: c.clone()
        })
    );
    assert_eq!(a.graph.edges[0].assertion_context, Some(c));
    let b = evaluate(&mut e, support(selected(query("g"), &d))).unwrap();
    assert_eq!(b.graph.nodes[0].properties["state"], "refuted");
    let default = evaluate(
        &mut e,
        support(GraphExpression::Context {
            input: Box::new(query("g")),
            selection: ContextSelection::Default,
        }),
    )
    .unwrap();
    assert_eq!(default.graph.nodes[0].properties["state"], "unknown");
}
#[test]
fn selected_join_preserves_context_and_rejects_broadcast_even_without_matching_edges() {
    let (mut e, c, d) = setup();
    write(&mut e, "g", data(&c, &d));
    let filter = |input, p: &str| GraphExpression::Filter {
        input: Box::new(input),
        predicate: Some(p.into()),
        valid_at: None,
    };
    let join = |left, right| GraphExpression::Join {
        left: Box::new(left),
        right: Box::new(right),
        output_predicate: "path".into(),
        match_on: JoinMatch::EntitySpaceToFrom,
    };
    let same = evaluate(
        &mut e,
        join(
            filter(selected(query("g"), &c), "p"),
            filter(selected(query("g"), &c), "q"),
        ),
    )
    .unwrap();
    assert_eq!(same.graph.edges.len(), 1);
    assert_eq!(same.graph.edges[0].assertion_context, Some(c.clone()));
    assert_eq!(
        evaluate(
            &mut e,
            join(selected(query("g"), &c), selected(query("g"), &d))
        )
        .unwrap_err()
        .code,
        "E_CONTEXT_INCOMPATIBLE"
    );
    assert_eq!(
        evaluate(&mut e, join(selected(query("g"), &c), query("g")))
            .unwrap_err()
            .code,
        "E_CONTEXT_INCOMPATIBLE"
    );
}
fn container(target: &GraphRef, context: &GraphRef) -> GraphData {
    serde_json::from_value(json!({"attachments":[{"id":"evidence","host":{"kind":"graph"},"key":"evidence","context":context,"value":{"kind":"graph","reference":target},"valid_time":{"start":0,"end":10}}]})).unwrap()
}
fn extract(input: GraphExpression) -> GraphExpression {
    GraphExpression::Metadata {
        input: Box::new(input),
        host: MetadataHost::Graph,
        key: "evidence".into(),
    }
}
#[test]
fn contextual_metadata_requires_compatible_target_and_retains_path_scope() {
    let (mut e, c, d) = setup();
    let mut target = data(&c, &d);
    target.assertions.retain(|a| a.context.as_ref() == Some(&c));
    let target = write(&mut e, "target", target);
    write(&mut e, "container", container(&target, &c));
    assert_eq!(
        evaluate(&mut e, extract(query("container")))
            .unwrap_err()
            .code,
        "E_CONTEXT_REQUIRED"
    );
    let result = evaluate(&mut e, extract(selected(query("container"), &c))).unwrap();
    assert_eq!(
        result.selected_context,
        Some(ContextSelection::Pinned {
            reference: c.clone()
        })
    );
    assert!(result
        .graph
        .edges
        .iter()
        .all(|edge| edge.assertion_context.as_ref() == Some(&c)));
    assert!(result
        .provenance
        .iter()
        .any(|p| p.graph_id == "container" && p.assertion_id == "evidence"));
    assert_eq!(
        evaluate(
            &mut e,
            GraphExpression::Context {
                input: Box::new(extract(selected(query("container"), &c))),
                selection: ContextSelection::Default
            }
        )
        .unwrap_err()
        .code,
        "E_CONTEXT_SCOPE"
    );
    let target = write(&mut e, "target", data(&c, &d));
    write(&mut e, "container", container(&target, &c));
    assert_eq!(
        evaluate(&mut e, extract(selected(query("container"), &c)))
            .unwrap_err()
            .code,
        "E_CONTEXT_MISMATCH"
    );
    assert_eq!(
        evaluate(&mut e, query("target")).unwrap().graph.edges.len(),
        3
    );
}
#[test]
fn attachment_context_is_a_capsule_dependency_and_old_profiles_reject_it_atomically() {
    let (mut e, c, _) = setup();
    let target = write(&mut e, "target", GraphData::default());
    let root = write(&mut e, "container", container(&target, &c));
    let capsule = e.export_capsule(&root, &host()).unwrap();
    assert!(capsule
        .revisions
        .iter()
        .any(|r| r.graph_id == "C" && r.revision == c.revision));
    let mut peer = Engine::memory().unwrap();
    peer.receive_capsule(&capsule, &host()).unwrap();
    peer.fork_branch(&root, "main", &host()).unwrap();
    let value = evaluate(&mut peer, extract(selected(query("container"), &c))).unwrap();
    assert_eq!(
        value.selected_context,
        Some(ContextSelection::Pinned {
            reference: c.clone()
        })
    );
    assert!(value.graph.edges.is_empty());
    let mut old = program(vec![Command::Commit {
        graph_id: "g".into(),
        branch_id: "main".into(),
        expected_head: None,
        data: container(&target, &c),
    }]);
    old.version = "0.7.0".into();
    let count = e.event_count().unwrap();
    assert_eq!(e.execute(&old, &host()).unwrap_err().code, "E_VERSION");
    assert_eq!(e.event_count().unwrap(), count);
    assert_eq!(e.head("g", "main").unwrap(), None);
}
#[test]
fn context_pins_are_revision_exact_and_clocked_views_keep_scope_after_expiry() {
    let (mut e, c, d) = setup();
    write(&mut e, "g", data(&c, &d));
    let mut updated = GraphData::default();
    updated.nodes.push(
        serde_json::from_value(
            json!({"id":"revision","entity_id":"revision","space_id":"context"}),
        )
        .unwrap(),
    );
    let c2 = write(&mut e, "C", updated);
    assert_ne!(c, c2);
    let unknown = evaluate(&mut e, support(selected(query("g"), &c2))).unwrap();
    assert_eq!(unknown.graph.nodes[0].properties["state"], "unknown");
    let definition = ViewDefinition {
        id: "context-view".into(),
        expression: support(selected(query("g"), &c)),
        clock: ViewClock::Tick,
    };
    let live = e.register_view(&definition, Some(5), &host()).unwrap();
    assert_eq!(live.result.graph.nodes[0].properties["state"], "supported");
    let expired = e.refresh_view("context-view", Some(10), &host()).unwrap();
    assert_eq!(expired.result.graph.nodes[0].properties["state"], "unknown");
    assert_eq!(
        expired.result.selected_context,
        Some(ContextSelection::Pinned { reference: c })
    );
}
#[test]
fn nested_context_cannot_enter_old_program_profile() {
    let (mut e, c, d) = setup();
    let mut old = program(vec![
        Command::Commit {
            graph_id: "g".into(),
            branch_id: "main".into(),
            expected_head: None,
            data: data(&c, &d),
        },
        Command::Evaluate {
            value: GraphExpression::Filter {
                input: Box::new(selected(query("g"), &c)),
                predicate: None,
                valid_at: None,
            },
        },
    ]);
    old.version = "0.7.0".into();
    let events = e.event_count().unwrap();
    assert_eq!(e.execute(&old, &host()).unwrap_err().code, "E_VERSION");
    assert_eq!(e.event_count().unwrap(), events);
    assert_eq!(e.head("g", "main").unwrap(), None);
}
#[test]
fn empty_support_context_survives_mixed_union_persistence_and_capsule_closure() {
    let (mut e, c, d) = setup();
    write(&mut e, "g", GraphData::default());
    let union = GraphExpression::Union {
        left: Box::new(support(selected(query("g"), &c))),
        right: Box::new(support(selected(query("g"), &d))),
    };
    let value = evaluate(&mut e, union.clone()).unwrap();
    assert_eq!(value.selected_context, None);
    assert_eq!(value.graph.nodes.len(), 2);
    assert!(value.graph.nodes.iter().all(|n| n.context_scope.is_some()));
    let only = evaluate(&mut e, selected(union, &c)).unwrap();
    assert_eq!(only.graph.nodes.len(), 1);
    assert_eq!(
        only.graph.nodes[0].context_scope,
        Some(ContextSelection::Pinned {
            reference: c.clone()
        })
    );
    let root = write(&mut e, "container", only.graph);
    let capsule = e.export_capsule(&root, &host()).unwrap();
    assert!(capsule
        .revisions
        .iter()
        .any(|r| r.graph_id == c.graph_id && r.revision == c.revision));
}
