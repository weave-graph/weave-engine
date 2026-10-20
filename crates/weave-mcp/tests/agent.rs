//! B4 (#40): the agent tool layer without a store. Write tools plan exactly
//! one `write` command with host-minted IDs; their arguments cannot carry
//! IDs, writers, readers or derivation gates; the local preview's catalog
//! has no write tool.
use serde_json::{json, Value};
use weave_contract::facts::{Fact, FactChange};
use weave_contract::Command;
use weave_mcp::agent::{plan_assert, plan_retract};
use weave_mcp::{handle_message, Tool, AGENT, PREVIEW};

fn base(facts: Value) -> Value {
    json!({"graph_id": "g",
        "provenance": {"method": {"kind": "model", "model": "m", "version": "1"},
            "evidence": [{"kind": "artifact", "uri": "doc://a", "digest": "sha256:00"}]},
        "visibility": {"reader_groups": ["team"]},
        "facts": facts})
}

fn counter() -> impl FnMut() -> String {
    let mut n = 0;
    move || {
        n += 1;
        format!("id-{n}")
    }
}

#[test]
fn assert_plans_one_write_with_minted_ids_and_resolved_refs() {
    let planned = plan_assert(
        base(json!([
            {"kind": "edge", "predicate": "p", "from": {"ref": "a"}, "to": "existing",
             "valid_time": {"start": 0}},
            {"kind": "node", "ref": "a", "space_id": "s", "properties": {"text": "{\"op\":\"write\"}"}},
            {"kind": "attachment", "host": {"kind": "entity", "ref": "a"}, "key": "k",
             "value": {"nested": [1]}, "valid_time": {"start": 0}},
            {"kind": "embedding", "space": {"id": "sp", "revision": "1"},
             "subject": {"kind": "node", "ref": "a"}, "vector": [1.0, 2.0], "valid_time": {"start": 0}},
        ])),
        counter(),
    )
    .unwrap();
    assert_eq!(
        planned.facts,
        json!([{"kind": "edge", "id": "id-1"}, {"kind": "node", "id": "id-2", "ref": "a"},
            {"kind": "attachment", "id": "id-3"}, {"kind": "embedding", "id": "id-4"}])
        .as_array()
        .unwrap()
        .clone()
    );
    let Command::Write {
        graph_id,
        branch_id,
        expected_head,
        changes,
    } = &planned.command
    else {
        panic!("a write");
    };
    assert_eq!((graph_id.as_str(), branch_id.as_str()), ("g", "main"));
    assert_eq!(*expected_head, None);
    assert_eq!(planned.program().commands.len(), 1);
    let facts: Vec<&Fact> = changes
        .iter()
        .map(|c| match c {
            FactChange::Assert { fact } => fact,
            _ => panic!("asserts only"),
        })
        .collect();
    let Fact::Edge(edge) = facts[0] else { panic!() };
    assert_eq!((edge.from.as_str(), edge.to.as_str()), ("id-2", "existing"));
    assert_eq!(edge.reader_groups, ["team"]);
    assert!(edge.readers.is_empty());
    let claim = edge.provenance.as_ref().unwrap();
    assert_eq!(serde_json::to_value(&claim.method).unwrap()["model"], "m");
    let Fact::Node(node) = facts[1] else { panic!() };
    // Property text stays a string value.
    assert_eq!(node.properties["text"], json!("{\"op\":\"write\"}"));
    assert_eq!(node.entity_id, "id-2");
    let Fact::Attachment(attachment) = facts[2] else {
        panic!()
    };
    assert_eq!(
        serde_json::to_value(&attachment.host).unwrap(),
        json!({"kind": "entity", "id": "id-2"})
    );
    let Fact::Embedding(embedding) = facts[3] else {
        panic!()
    };
    assert_eq!(
        serde_json::to_value(&embedding.subject).unwrap(),
        json!({"kind": "node", "node_id": "id-2"})
    );
}

#[test]
fn assert_refuses_bad_refs_claims_and_authority_fields() {
    let node = json!({"kind": "node", "space_id": "s"});
    let cases = [
        (base(json!([])), "E_MCP_INPUT"),
        (
            base(
                json!([{"kind": "edge", "predicate": "p", "from": {"ref": "x"}, "to": "y",
            "valid_time": {"start": 0}}]),
            ),
            "E_MCP_INPUT",
        ),
        (
            base(json!([{"kind": "node", "ref": "a", "space_id": "s"},
            {"kind": "node", "ref": "a", "space_id": "s"}])),
            "E_MCP_INPUT",
        ),
        (
            base(
                json!([{"kind": "edge", "ref": "e", "predicate": "p", "from": {"ref": "e"},
            "to": "y", "valid_time": {"start": 0}}]),
            ),
            "E_MCP_INPUT",
        ),
        (
            base(
                json!([{"kind": "attachment", "host": {"kind": "node", "id": "n", "ref": "a"},
            "key": "k", "value": 1, "valid_time": {"start": 0}}]),
            ),
            "E_MCP_INPUT",
        ),
        (
            base(json!([{"kind": "node", "id": "chosen", "space_id": "s"}])),
            "E_MCP_INPUT",
        ),
        (
            base(json!([{"kind": "node", "space_id": "s", "readers": ["root"]}])),
            "E_MCP_INPUT",
        ),
        (
            base(json!([{"kind": "node", "space_id": "s", "derived_nodes": []}])),
            "E_MCP_INPUT",
        ),
        (base(json!([{"kind": "space", "id": "x"}])), "E_MCP_INPUT"),
        (base(json!(vec![node.clone(); 257])), "E_BUDGET"),
    ];
    for (arguments, code) in cases {
        let error = plan_assert(arguments.clone(), counter()).unwrap_err();
        assert_eq!(error.code, code, "{arguments}");
    }
    let mut arguments = base(json!([node.clone()]));
    arguments["provenance"]["method"] = json!({"kind": "unspecified"});
    assert_eq!(
        plan_assert(arguments, counter()).unwrap_err().code,
        "E_MCP_CLAIM"
    );
    for visibility in [
        json!({}),
        json!({"public": false}),
        json!({"public": true, "readers": ["a"]}),
    ] {
        let mut arguments = base(json!([node.clone()]));
        arguments["visibility"] = visibility;
        assert_eq!(
            plan_assert(arguments, counter()).unwrap_err().code,
            "E_MCP_CLAIM"
        );
    }
    for field in ["writer", "on_behalf_of", "recorded_at_ms"] {
        let mut arguments = base(json!([node.clone()]));
        arguments["provenance"][field] = json!("root");
        assert_eq!(
            plan_assert(arguments, counter()).unwrap_err().code,
            "E_MCP_INPUT"
        );
    }
    // Public must be said.
    let mut arguments = base(json!([node.clone()]));
    arguments["visibility"] = json!({"public": true});
    let planned = plan_assert(arguments, counter()).unwrap();
    let Command::Write { changes, .. } = planned.command else {
        panic!()
    };
    let FactChange::Assert {
        fact: Fact::Node(node),
    } = &changes[0]
    else {
        panic!()
    };
    assert!(node.readers.is_empty() && node.reader_groups.is_empty());
}

#[test]
fn retract_plans_one_write_of_retractions() {
    let planned = plan_retract(json!({"graph_id": "g", "branch_id": "b",
        "facts": [{"kind": "edge", "id": "e"}, {"kind": "node", "id": "n"}]}))
    .unwrap();
    assert_eq!(
        planned.facts,
        vec![
            json!({"kind": "edge", "id": "e"}),
            json!({"kind": "node", "id": "n"})
        ]
    );
    let Command::Write { changes, .. } = planned.command else {
        panic!()
    };
    assert!(changes
        .iter()
        .all(|c| matches!(c, FactChange::Retract { expected: None, .. })));
    for arguments in [
        json!({"graph_id": "g", "facts": []}),
        json!({"graph_id": "g", "facts": [{"kind": "edge", "id": "e", "expected": {"kind": "absent"}}]}),
        json!({"graph_id": "g", "facts": [{"kind": "edge", "id": "e"}], "cascade": true}),
    ] {
        assert_eq!(plan_retract(arguments).unwrap_err().code, "E_MCP_INPUT");
    }
}

#[test]
fn the_preview_lists_no_write_tool_and_refuses_one() {
    let names = |catalog| -> Vec<String> {
        let response = handle_message(
            catalog,
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            |_, _| unreachable!(),
        )
        .unwrap();
        response["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        names(&PREVIEW),
        [
            "query",
            "expand",
            "as_of",
            "what_changed",
            "explain",
            "hybrid_search"
        ]
    );
    assert_eq!(names(&AGENT).len(), Tool::ALL.len());
    let call = br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"assert_fact","arguments":{}}}"#;
    let response = handle_message(&PREVIEW, call, |_, _| unreachable!()).unwrap();
    assert_eq!(response["error"]["data"]["code"], "E_MCP_UNKNOWN_TOOL");
    // Write tools are annotated as such; read tools as read-only.
    for tool in Tool::ALL {
        let annotations = &tool.descriptor()["annotations"];
        assert_eq!(
            annotations["readOnlyHint"],
            !tool.writes(),
            "{}",
            tool.name()
        );
        assert_eq!(annotations["destructiveHint"], tool == Tool::RetractFact);
    }
}
