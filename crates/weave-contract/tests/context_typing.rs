use serde_json::json;
use weave_contract::{context_typing::*, *};
fn pin(id: &str) -> GraphRef {
    GraphRef {
        graph_id: id.into(),
        revision: "r".into(),
    }
}
fn carrier(id: &str) -> ContextTyping {
    serde_json::from_value(json!({"selected":pin(id),"witnesses":[{"context":pin(id),"schema":{"reference":{"id":"World","revision":"1"},"axes":{"region":{"kind":"enum","members":["EE","FI"]}}},"definition":{"graph_id":id,"revision":"r","assertion_id":"definition"},"anchor_nodes":[{"graph_id":id,"revision":"r","node_id":"anchor"}]}]})).unwrap()
}
fn empty(typing: Option<ContextTyping>) -> QueryResult {
    let mut result:QueryResult=serde_json::from_value(json!({"version":VERSION,"graph":{"nodes":[],"edges":[]},"snapshots":{},"coverage":"complete","diagnostics":[],"provenance":[],"metadata_graphs":[]})).unwrap();
    result.selected_context = typing
        .as_ref()
        .and_then(|t| t.selected.clone())
        .map(|reference| ContextSelection::Pinned { reference });
    result.graph.context_typing = typing;
    result
}
fn ctx() -> AlgebraContext {
    AlgebraContext {
        principal: "alice".into(),
        max_objects: 1000,
        max_output_bytes: 4 * 1024 * 1024,
    }
}
#[test]
fn mixed_and_empty_composition_preserves_influence_without_promoting_untyped_values() {
    let a = empty(Some(carrier("A")));
    let b = empty(Some(carrier("B")));
    let out = algebra::union(a.clone(), b, &ctx()).unwrap();
    assert!(out.selected_context.is_none());
    let t = out.graph.context_typing.as_ref().unwrap();
    assert!(t.selected.is_none());
    assert_eq!(t.witnesses.len(), 2);
    let projection = algebra::project(out, &[], &[], &ctx()).unwrap();
    assert_eq!(
        projection
            .graph
            .context_typing
            .as_ref()
            .unwrap()
            .witnesses
            .len(),
        2
    );
    let untyped = empty(None);
    let out = algebra::union(a, untyped, &ctx()).unwrap();
    assert!(out
        .graph
        .context_typing
        .as_ref()
        .unwrap()
        .selected
        .is_none());
    assert_eq!(
        out.graph.context_typing.as_ref().unwrap().witnesses.len(),
        1
    );
}
#[test]
fn unknown_support_and_empty_explanation_retain_persistable_descriptor_gates() {
    let input = empty(Some(carrier("A")));
    let entity = EntitySpace {
        entity_id: "missing".into(),
        space_id: "s".into(),
    };
    let support = algebra::support(input.clone(), "p", &entity, &entity, 0, &ctx()).unwrap();
    let n = &support.graph.nodes[0];
    assert_eq!(n.derived_from[0].assertion_id, "definition");
    assert_eq!(n.derived_nodes[0].node_id, "anchor");
    assert_eq!(support.graph.context_typing, input.graph.context_typing);
    assert!(support
        .provenance
        .iter()
        .any(|a| a.assertion_id == "definition"));
    assert!(support.input_snapshots.contains(&pin("A")));
    let explain = identity::explain(&input, &ctx()).unwrap();
    assert_eq!(explain.graph.context_typing, input.graph.context_typing);
    let saved = serde_json::to_vec(&support.graph).unwrap();
    let decoded: GraphData = serde_json::from_slice(&saved).unwrap();
    assert_eq!(decoded.context_typing, support.graph.context_typing);
}
#[test]
fn invalid_markers_anchors_labels_and_selected_claims_reject() {
    let mut t = carrier("A");
    t.selected = Some(pin("B"));
    assert!(validate(&t).is_err());
    let mut t = carrier("A");
    t.witnesses[0].anchor_nodes[0].graph_id = "other".into();
    assert!(validate(&t).is_err());
    let mut t = carrier("A");
    t.witnesses[0].definition.assertion_id = "forged".into();
    assert!(validate(&t).is_err());
    let mut t = carrier("B");
    t.witnesses[0]
        .schema
        .axes
        .insert("region".into(), context_axes::ContextAxisType::String);
    assert_eq!(
        merge(Some(&carrier("A")), Some(&t)).unwrap_err().code,
        "E_CONTEXT_SCHEMA"
    );
    let mut result = empty(Some(carrier("A")));
    result.selected_context = None;
    assert!(validate_result(&result).is_err());
    let mut graph:GraphData=serde_json::from_value(json!({"nodes":[{"id":"n","entity_id":"n","space_id":"s"}],"edges":[{"id":"e","from":"n","to":"n","predicate":"p","valid_time":{"start":0}}]})).unwrap();
    graph.context_typing = Some(carrier("A"));
    assert_eq!(validate_graph(&graph).unwrap_err().code, "E_CONTEXT_SCOPE");
}
#[test]
fn generated_alternative_groups_and_descriptive_indexes_include_descriptor_pins() {
    let mut result = empty(Some(carrier("A")));
    result.graph.nodes =
        serde_json::from_value(json!([{"id":"n","entity_id":"n","space_id":"s"}])).unwrap();
    result.graph.edges=serde_json::from_value(json!([{"id":"e","from":"n","to":"n","predicate":"p","valid_time":{"start":0},"assertion_context":pin("A"),"derived_from":[{"graph_id":"source","revision":"r","assertion_id":"e"}],"derivations":[{"operator":"example","premises":[{"graph_id":"source","revision":"r","assertion_id":"e"}],"input_snapshots":[{"graph_id":"source","revision":"r"}]}]}])).unwrap();
    protect_result_generated(&mut result).unwrap();
    assert!(result.graph.edges[0].derivations[0]
        .input_snapshots
        .contains(&pin("A")));
    assert!(result.edge_origins["e"]
        .iter()
        .any(|r| r.assertion_id == "definition"));
    assert!(result.graph.nodes[0]
        .derived_nodes
        .iter()
        .any(|n| n.graph_id == "A"));
    let prior = result.clone();
    protect_result_generated(&mut result).unwrap();
    assert_eq!(prior, result);
}
#[test]
fn embedded_duplicate_schema_and_bounded_witness_arrays_reject_before_use() {
    let value = serde_json::to_string(&carrier("A")).unwrap();
    let duplicate = value.replace("\"id\":\"World\"", "\"id\":\"World\",\"id\":\"Other\"");
    assert!(serde_json::from_str::<ContextTyping>(&duplicate).is_err());
    let mut raw = serde_json::to_value(carrier("A")).unwrap();
    raw["witnesses"] = json!(vec![raw["witnesses"][0].clone(); 33]);
    assert!(serde_json::from_value::<ContextTyping>(raw).is_err());
    let mut t = carrier("A");
    t.witnesses[0].anchor_nodes = vec![t.witnesses[0].anchor_nodes[0].clone(); 17];
    assert!(validate(&t).is_err());
    let mut graph = empty(Some(carrier("A"))).graph;
    assert!(protect_generated_bounded(&mut graph, 1).is_err());
}

#[test]
fn graph_host_attachment_retains_typed_descriptor_gates_without_envelope() {
    let mut result = empty(Some(carrier("A")));
    result.graph.attachments.push(serde_json::from_value(json!({"id":"status","host":{"kind":"graph"},"key":"value","value":{"kind":"literal","value":0},"context":pin("A"),"valid_time":{"start":0,"end":null}})).unwrap());
    protect_result_generated(&mut result).unwrap();
    let attachment = result.graph.attachments.pop().unwrap();
    assert_eq!(attachment.derived_from[0].assertion_id, "definition");
    assert_eq!(attachment.derived_nodes[0].node_id, "anchor");
    assert_eq!(result.attachment_origins["status"], attachment.derived_from);
    let detached = GraphData {
        attachments: vec![attachment],
        ..GraphData::default()
    };
    let gates = influence::input_influence(&detached).unwrap().unwrap();
    assert_eq!(gates.assertions[0].assertion_id, "definition");
    assert_eq!(gates.nodes[0].node_id, "anchor");
    assert!(gates.snapshots.is_empty());
}
