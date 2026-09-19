use serde_json::json;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("alice", ["g".into(), "derived".into(), "evidence".into()])
}
fn explicit() -> GraphData {
    serde_json::from_value(json!({"profile":"explicit","nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"structural_edges":[{"id":"relation","predicate":"p","from":"a","to":"b"}],"assertions":[{"id":"claim-a","edge_id":"relation","source":"sensor:a","polarity":"positive","valid_time":{"start":0,"end":10}},{"id":"claim-b","edge_id":"relation","source":"sensor:b","polarity":"negative","valid_time":{"start":5,"end":20}}]})).unwrap()
}
fn write(e: &mut Engine, g: &str, data: GraphData) -> Result<Vec<CommandResult>> {
    e.execute(
        &Program {
            source_revisions: vec![],
            version: VERSION.into(),
            commands: vec![Command::Commit {
                graph_id: g.into(),
                branch_id: "main".into(),
                expected_head: e.head(g, "main").unwrap(),
                data,
            }],
        },
        &host(),
    )
}
fn query() -> QueryPlan {
    serde_json::from_value(json!({"graph_id":"g","include_metadata":true,"valid_at":7})).unwrap()
}
#[test]
fn structural_relationship_without_assertions_has_no_implicit_support() {
    let mut e = Engine::memory().unwrap();
    let mut graph = explicit();
    graph.assertions.clear();
    write(&mut e, "g", graph).unwrap();
    let result = e.query(&query(), &host()).unwrap();
    assert!(result.graph.edges.is_empty());
}
#[test]
fn source_claims_are_distinct_and_preserve_structural_identity_and_attribution() {
    let mut e = Engine::memory().unwrap();
    let mut graph = explicit();
    graph.assertions[0]
        .properties
        .insert("confidence".into(), json!(7));
    write(&mut e, "g", graph).unwrap();
    let result = e.query(&query(), &host()).unwrap();
    assert_eq!(result.graph.edges.len(), 2);
    for edge in &result.graph.edges {
        assert_eq!(edge.structural_ref.as_ref().unwrap().edge_id, "relation");
        assert_eq!(result.edge_origins[&edge.id][0].assertion_id, edge.id);
        assert!(edge
            .assertion_source
            .as_ref()
            .unwrap()
            .starts_with("sensor:"));
    }
    assert_eq!(result.graph.edges[0].assertion_properties["confidence"], 7);
    assert!(result.graph.edges[0].properties.is_empty());
    let support = algebra::support(
        result,
        "p",
        &EntitySpace {
            entity_id: "A".into(),
            space_id: "s".into(),
        },
        &EntitySpace {
            entity_id: "B".into(),
            space_id: "s".into(),
        },
        7,
        &AlgebraContext {
            principal: "alice".into(),
            max_objects: 100,
            max_output_bytes: 1_000_000,
        },
    )
    .unwrap();
    assert_eq!(support.graph.nodes[0].properties["state"], "conflicted");
}
#[test]
fn explicit_visibility_intersects_structure_and_source_and_derivations() {
    let mut e = Engine::memory().unwrap();
    let mut graph = explicit();
    graph.structural_edges[0].readers = vec!["alice".into()];
    write(&mut e, "g", graph).unwrap();
    let alice = e.query(&query(), &host()).unwrap();
    assert!(alice.graph.edges.iter().all(|e| e.readers == ["alice"]));
    assert!(e
        .query(&query(), &HostContext::new("bob", []))
        .unwrap()
        .graph
        .edges
        .is_empty());
    let mut derived:GraphData=serde_json::from_value(json!({"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"d","predicate":"p","from":"a","to":"b","valid_time":{"start":0},"derived_from":[{"graph_id":"g","revision":e.head("g","main").unwrap().unwrap(),"assertion_id":"claim-a"}]}]})).unwrap();
    derived.edges[0].readers.clear();
    write(&mut e, "derived", derived).unwrap();
    let mut q = query();
    q.graph_id = "derived".into();
    assert!(e
        .query(&q, &HostContext::new("bob", []))
        .unwrap()
        .graph
        .edges
        .is_empty());
}
#[test]
fn assertion_and_structural_ids_cannot_rebind_after_deletion_and_profile_is_explicit() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, "g", explicit()).unwrap();
    let mut empty = explicit();
    empty.assertions.clear();
    write(&mut e, "g", empty).unwrap();
    let mut changed = explicit();
    changed.assertions[0].source = "other-source".into();
    assert_eq!(
        write(&mut e, "g", changed).unwrap_err().code,
        "E_ASSERTION_IDENTITY"
    );
    let mut changed = explicit();
    changed.structural_edges[0].predicate = "different".into();
    assert_eq!(
        write(&mut e, "g", changed).unwrap_err().code,
        "E_EDGE_IDENTITY"
    );
    let mut changed = explicit();
    changed.profile = GraphProfile::Legacy;
    assert!(write(&mut e, "g", changed).is_err());
}
#[test]
fn explicit_capsule_roundtrip_deduplicates_and_keeps_source_claims() {
    let mut source = Engine::memory().unwrap();
    write(&mut source, "g", explicit()).unwrap();
    let reference = GraphRef {
        graph_id: "g".into(),
        revision: source.head("g", "main").unwrap().unwrap(),
    };
    let capsule = source.export_capsule(&reference, &host()).unwrap();
    let mut target = Engine::memory().unwrap();
    assert_eq!(target.receive_capsule(&capsule, &host()).unwrap(), 1);
    assert_eq!(target.receive_capsule(&capsule, &host()).unwrap(), 0);
    target
        .accept_revision(&reference, "main", None, &host())
        .unwrap();
    assert_eq!(
        target.query(&query(), &host()).unwrap(),
        source.query(&query(), &host()).unwrap()
    );
}

#[test]
fn assertion_host_metadata_keeps_original_attachment_and_claim_origin() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, "evidence", explicit()).unwrap();
    let mut g = explicit();
    g.attachments.push(serde_json::from_value(json!({"id":"proof","host":{"kind":"assertion","id":"claim-a"},"key":"proof","value":{"kind":"graph","reference":{"graph_id":"evidence","revision":e.head("evidence","main").unwrap().unwrap()}},"valid_time":{"start":0}})).unwrap());
    write(&mut e, "g", g).unwrap();
    let value = GraphExpression::Metadata {
        input: Box::new(GraphExpression::Query { query: query() }),
        host: MetadataHost::Assertion {
            id: "claim-a".into(),
        },
        key: "proof".into(),
    };
    let result = e
        .execute(
            &Program {
                source_revisions: vec![],
                version: VERSION.into(),
                commands: vec![Command::Evaluate { value }],
            },
            &host(),
        )
        .unwrap();
    let CommandResult::Queried { result } = &result[0] else {
        panic!()
    };
    assert_eq!(result.graph.edges.len(), 2);
    assert!(result
        .provenance
        .iter()
        .any(|p| p.graph_id == "g" && p.assertion_id == "proof"));
    assert!(result
        .graph
        .edges
        .iter()
        .all(|edge| edge.valid_time.end == Some(10)));
}
#[test]
fn explicit_derivation_dependencies_are_included_in_capsules() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, "evidence", explicit()).unwrap();
    let mut graph = explicit();
    graph.assertions[0].derived_from = vec![AssertionRef {
        graph_id: "evidence".into(),
        revision: e.head("evidence", "main").unwrap().unwrap(),
        assertion_id: "claim-a".into(),
    }];
    write(&mut e, "g", graph).unwrap();
    let capsule = e
        .export_capsule(
            &GraphRef {
                graph_id: "g".into(),
                revision: e.head("g", "main").unwrap().unwrap(),
            },
            &host(),
        )
        .unwrap();
    assert_eq!(capsule.revisions.len(), 2);
    assert!(capsule.external_dependencies.is_empty());
}
