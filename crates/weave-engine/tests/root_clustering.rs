use serde_json::{json, Value};
use weave_contract::{
    CommandResult, ContextSelection, GraphData, GraphRef, Program, QueryPlan, QueryResult, VERSION,
};
use weave_engine::{ClusterRequest, Engine, HostContext};
fn host(actor: &str) -> HostContext {
    HostContext::new(
        actor,
        [
            "source".into(),
            "saved".into(),
            "world".into(),
            "private".into(),
        ],
    )
}
fn commit(e: &mut Engine, graph: &str, data: Value, head: Option<&str>) -> String {
    let p: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":head,"data":data}]})).unwrap();
    match &e.execute(&p, &host("alice")).unwrap()[0] {
        CommandResult::Committed { revision, .. } => revision.clone(),
        _ => panic!(),
    }
}
fn node(id: &str) -> Value {
    json!({"id":id,"entity_id":id,"space_id":"s"})
}
fn edge(id: &str, a: &str, b: &str) -> Value {
    json!({"id":id,"predicate":"p","from":a,"to":b,"valid_time":{"start":0,"end":10}})
}
fn request(r: &str, levels: usize) -> ClusterRequest {
    ClusterRequest {
        source: GraphRef {
            graph_id: "source".into(),
            revision: r.into(),
        },
        context: ContextSelection::Default,
        valid_at: 5,
        predicate: "p".into(),
        levels,
    }
}
fn read(e: &Engine, actor: &str) -> QueryResult {
    let q: QueryPlan = serde_json::from_value(json!({"graph_id":"saved"})).unwrap();
    e.query(&q, &host(actor)).unwrap()
}
fn manifest(v: &QueryResult) -> &Value {
    &v.graph
        .nodes
        .iter()
        .find(|n| n.properties["kind"] == "navigation_manifest")
        .unwrap()
        .properties["manifest"]
}
#[test]
fn lazy_navigation_is_expandable_and_never_changes_exact_query_recall() {
    let mut e = Engine::memory().unwrap();
    let r = commit(
        &mut e,
        "source",
        json!({"nodes":[node("a"),node("b"),node("c"),node("d"),node("isolated")],"edges":[edge("ab","a","b"),edge("bc","b","c"),edge("cd","c","d")]}),
        None,
    );
    let query: QueryPlan =
        serde_json::from_value(json!({"graph_id":"source","revision":r,"valid_at":5})).unwrap();
    let baseline = e.query(&query, &host("alice")).unwrap();
    let first = e
        .cluster_navigation(&request(&r, 1), &host("alice"))
        .unwrap();
    assert_eq!(manifest(&first)["level"], 1);
    assert_eq!(manifest(&first)["evidence_boundary"], false);
    assert!(first
        .graph
        .edges
        .iter()
        .any(|e| e.predicate == "weave:cluster:exists"));
    let all = e
        .cluster_navigation(&request(&r, 100), &host("alice"))
        .unwrap();
    assert_eq!(manifest(&all)["level"], 2);
    assert_eq!(manifest(&all)["evidence_boundary"], true);
    assert_eq!(manifest(&all)["frontier"].as_array().unwrap().len(), 2);
    assert!(all.graph.nodes.iter().any(|n| n.id == "isolated"));
    for cluster in all
        .graph
        .nodes
        .iter()
        .filter(|n| n.properties["kind"] == "cluster")
    {
        assert_eq!(
            all.graph
                .edges
                .iter()
                .filter(|e| e.from == cluster.id && e.predicate == "weave:cluster:member")
                .count(),
            2
        );
    }
    assert_eq!(e.query(&query, &host("alice")).unwrap(), baseline);
    assert_eq!(
        e.cluster_navigation(&request(&r, 100), &host("alice"))
            .unwrap(),
        all
    );
    assert!(all.node_origins.values().all(Vec::is_empty));
}
#[test]
fn node_only_private_influence_survives_reader_stripping_and_capsules() {
    let mut e = Engine::memory().unwrap();
    let mut secret = node("secret");
    secret["readers"] = json!(["alice"]);
    let r = commit(
        &mut e,
        "source",
        json!({"nodes":[node("public"),secret],"edges":[]}),
        None,
    );
    let mut v = e
        .cluster_navigation(&request(&r, 20), &host("alice"))
        .unwrap();
    assert_eq!(manifest(&v)["frontier"].as_array().unwrap().len(), 2);
    v.graph.edges.clear();
    for n in &mut v.graph.nodes {
        n.readers.clear();
    }
    let saved = commit(
        &mut e,
        "saved",
        serde_json::to_value(v.graph).unwrap(),
        None,
    );
    assert_eq!(read(&e, "alice").graph.nodes.len(), 3);
    assert!(read(&e, "bob").graph.nodes.is_empty());
    let capsule = e
        .export_capsule(
            &GraphRef {
                graph_id: "saved".into(),
                revision: saved.clone(),
            },
            &host("alice"),
        )
        .unwrap();
    let mut restored = Engine::memory().unwrap();
    restored.receive_capsule(&capsule, &host("alice")).unwrap();
    let q: QueryPlan =
        serde_json::from_value(json!({"graph_id":"saved","revision":saved})).unwrap();
    assert!(restored
        .query(&q, &host("bob"))
        .unwrap()
        .graph
        .nodes
        .is_empty());
    assert_eq!(
        restored
            .query(&q, &host("alice"))
            .unwrap()
            .graph
            .nodes
            .len(),
        3
    );
    let bob = e
        .cluster_navigation(&request(&r, 20), &host("bob"))
        .unwrap();
    assert_eq!(manifest(&bob)["frontier"].as_array().unwrap().len(), 1);
    assert!(!serde_json::to_string(&bob).unwrap().contains("secret"));
}
#[test]
fn private_claim_selection_cannot_be_released_by_removing_edges() {
    let mut e = Engine::memory().unwrap();
    let mut private = edge("private-link", "a", "b");
    private["readers"] = json!(["alice"]);
    let r = commit(
        &mut e,
        "source",
        json!({"nodes":[node("a"),node("b")],"edges":[private]}),
        None,
    );
    let mut v = e
        .cluster_navigation(&request(&r, 1), &host("alice"))
        .unwrap();
    assert_eq!(manifest(&v)["frontier"].as_array().unwrap().len(), 1);
    v.graph.edges.clear();
    for n in &mut v.graph.nodes {
        n.readers.clear();
    }
    commit(
        &mut e,
        "saved",
        serde_json::to_value(v.graph).unwrap(),
        None,
    );
    assert!(read(&e, "bob").graph.nodes.is_empty());
    let bob = e.cluster_navigation(&request(&r, 1), &host("bob")).unwrap();
    assert_eq!(manifest(&bob)["frontier"].as_array().unwrap().len(), 2);
}
#[test]
fn exact_time_context_polarity_and_source_pins_are_enforced() {
    let mut e = Engine::memory().unwrap();
    let world = commit(&mut e, "world", json!({"nodes":[],"edges":[]}), None);
    let mut negative = edge("negative", "a", "b");
    negative["polarity"] = json!("negative");
    let mut contextual = edge("contextual", "a", "b");
    contextual["assertion_context"] = json!({"graph_id":"world","revision":world});
    let r = commit(
        &mut e,
        "source",
        json!({"nodes":[node("a"),node("b")],"edges":[negative,contextual]}),
        None,
    );
    let default = e
        .cluster_navigation(&request(&r, 2), &host("alice"))
        .unwrap();
    assert_eq!(manifest(&default)["level"], 0);
    let mut q = request(&r, 2);
    q.context = ContextSelection::Pinned {
        reference: GraphRef {
            graph_id: "world".into(),
            revision: world,
        },
    };
    assert_eq!(
        manifest(&e.cluster_navigation(&q, &host("alice")).unwrap())["level"],
        1
    );
    q.valid_at = 10;
    assert_eq!(
        manifest(&e.cluster_navigation(&q, &host("alice")).unwrap())["level"],
        0
    );
    q.source.revision = "missing".into();
    assert!(e.cluster_navigation(&q, &host("alice")).is_err());
}
#[test]
fn bounded_inputs_fail_without_mutation_and_private_only_updates_preserve_layout() {
    let mut e = Engine::memory().unwrap();
    let data = json!({"nodes":[node("a"),node("b")],"edges":[edge("ab","a","b")]});
    let r = commit(&mut e, "source", data.clone(), None);
    let before = e.cluster_navigation(&request(&r, 1), &host("bob")).unwrap();
    let mut changed = data;
    let mut hidden = node("hidden");
    hidden["readers"] = json!(["alice"]);
    changed["nodes"].as_array_mut().unwrap().push(hidden);
    let r2 = commit(&mut e, "source", changed, Some(&r));
    let after = e
        .cluster_navigation(&request(&r2, 1), &host("bob"))
        .unwrap();
    assert_eq!(manifest(&before), manifest(&after));
    assert_eq!(before.coverage, after.coverage);
    assert_eq!(before.diagnostics, after.diagnostics);
    let mut q = request(&r2, 10_001);
    assert_eq!(
        e.cluster_navigation(&q, &host("alice")).unwrap_err().code,
        "E_CLUSTER_INPUT"
    );
    q.levels = 1;
    q.valid_at = i64::MAX;
    assert_eq!(
        e.cluster_navigation(&q, &host("alice")).unwrap_err().code,
        "E_CLUSTER_TIME"
    );
    let data: GraphData = serde_json::from_value(
        json!({"nodes":(0..1000).map(|i|node(&format!("n{i}"))).collect::<Vec<_>>(),"edges":[]}),
    )
    .unwrap();
    let r3 = commit(
        &mut e,
        "source",
        serde_json::to_value(data).unwrap(),
        Some(&r2),
    );
    assert_eq!(
        e.cluster_navigation(&request(&r3, 1), &host("alice"))
            .unwrap_err()
            .code,
        "E_CLUSTER_BUDGET"
    );
    assert_eq!(
        e.head("source", "main").unwrap().as_deref(),
        Some(r3.as_str())
    );
    assert!(e.head("saved", "main").unwrap().is_none());
}

#[test]
fn navigation_values_compose_idempotently_for_connected_and_isolated_evidence() {
    for connected in [false, true] {
        let mut e = Engine::memory().unwrap();
        let edges = if connected {
            vec![edge("ab", "a", "b")]
        } else {
            vec![]
        };
        let r = commit(
            &mut e,
            "source",
            json!({"nodes":[node("a"),node("b")],"edges":edges}),
            None,
        );
        let value = e
            .cluster_navigation(&request(&r, 2), &host("alice"))
            .unwrap();
        let ctx = weave_contract::AlgebraContext {
            principal: "alice".into(),
            max_objects: 1000,
            max_output_bytes: 16 * 1024 * 1024,
        };
        let first = weave_contract::algebra::union(value.clone(), value.clone(), &ctx).unwrap();
        let repeated = weave_contract::algebra::union(first.clone(), value, &ctx).unwrap();
        assert_eq!(first.graph, repeated.graph);
        let difference = weave_contract::algebra::diff(first.clone(), first, &ctx).unwrap();
        assert!(difference.graph.attachments.is_empty());
    }
}
