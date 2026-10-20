//! B4 (#40): the agent interface, MCP over the authenticated service.
//!
//! Agents of one team write and find each other's facts with provenance;
//! `describe_schema` is enough to build valid calls; revocation applies to
//! the next tool call; every call is audited; read tools return exactly
//! what `POST /v1/execute` returns for the same program; and the transport
//! follows MCP Streamable HTTP without sessions.
mod common;
mod mcp_common;
use axum::http::{Method, StatusCode};
use common::*;
use mcp_common::*;
use serde_json::{json, Value};

/// The provenance entry of `fact` (a `FactRef` JSON) in a result.
fn provenance_of<'a>(result: &'a Value, kind: &str, id: &str) -> Option<&'a Value> {
    let key = match kind {
        "node" => "node_id",
        "embedding" => "embedding_id",
        _ => "assertion_id",
    };
    result["fact_provenance"]
        .as_array()?
        .iter()
        .find(|entry| entry["fact"]["kind"] == kind && entry["fact"][key] == id)
        .map(|entry| &entry["provenance"])
}

fn assert_from_x(provenance: &Value) {
    assert_eq!(provenance["writer"], "x-bot", "{provenance}");
    assert_eq!(provenance["on_behalf_of"], json!(["alice"]));
    assert_eq!(
        provenance["method"],
        json!({"kind": "model", "model": MODEL, "version": "2026-10"})
    );
    assert_eq!(provenance["source"], "maintenance log 42");
    assert_eq!(
        provenance["evidence"],
        json!([{"kind": "artifact", "uri": DOC_URI, "digest": DOC_DIGEST}])
    );
}

/// Acceptance criterion 1: agent X asserts a fact from a document, agent Y
/// of the same team finds it with `hybrid_search`, and `explain` shows X as
/// the source.
#[tokio::test]
async fn agent_x_asserts_from_a_document_and_agent_y_finds_and_explains_it() {
    let h = Harness::new();
    let t = teams(&h).await;
    let asserted = ok(&h, &t.x, "assert_fact", finding()).await;
    // The service minted every ID and reports the one write.
    let kinds: Vec<&str> = asserted["facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["node", "edge", "embedding"]);
    assert_eq!(asserted["facts"][0]["ref"], "finding");
    let node = minted(&asserted, 0);
    let edge = minted(&asserted, 1);
    let embedding = minted(&asserted, 2);
    assert!(node.starts_with("r-") && edge.starts_with("r-") && embedding.starts_with("r-"));
    assert_eq!(asserted["results"][0]["kind"], "committed");
    assert_eq!(asserted["revision"], asserted["results"][0]["revision"]);

    // Y searches by similarity: the finding is the nearest note.
    let found = ok(&h, &t.y, "hybrid_search", nearest(json!([0.9, 0.0, 0.2]))).await;
    assert_eq!(neighbor_subjects(&found), [node.as_str(), "note-1"]);
    let hit = &found["results"][0]["result"];
    let text = hit["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == node.as_str())
        .unwrap();
    assert_eq!(text["properties"]["text"], FINDING_TEXT);
    // The hit itself already carries its provenance.
    assert_from_x(provenance_of(hit, "embedding", &embedding).unwrap());
    assert_from_x(provenance_of(hit, "node", &node).unwrap());

    // Y explains the hit's neighborhood in one call: explain an expand
    // seeded by the same search. The explanation's source fact is X's edge.
    let explained = ok(
        &h,
        &t.y,
        "explain",
        json!({"input": {"kind": "expand", "graph_id": GRAPH, "hops": 1, "direction": "out",
            "seeds_from": {"kind": "nearest", "graph_id": GRAPH,
                "space": {"id": SPACE, "revision": "1"},
                "query": {"kind": "vector", "vector": [0.9, 0.0, 0.2]}, "k": 1}}}),
    )
    .await;
    let explanation = &explained["results"][0]["result"];
    assert_eq!(explanation["coverage"], "complete");
    let sources: Vec<&Value> = explanation["fact_provenance"]
        .as_array()
        .unwrap()
        .iter()
        .collect();
    assert_eq!(sources.len(), 1, "{explanation}");
    assert_eq!(sources[0]["fact"]["assertion_id"], edge.as_str());
    assert_from_x(&sources[0]["provenance"]);
    assert!(explanation["graph"]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["predicate"] == "weave:explanation:alternative"));

    // The audit log names X as the writer and Y as a reader of the fact.
    let accesses = h.audit_facts(&t.root, GRAPH, &embedding).await;
    let write = accesses.iter().find(|a| a["access"] == "write").unwrap();
    assert_eq!(write["principal"], "x-bot");
    assert_eq!(write["on_behalf_of"], json!(["alice"]));
    assert!(accesses
        .iter()
        .any(|a| a["access"] == "read" && a["principal"] == "y-bot"));
    let entries = h.audit_entries(&t.root).await;
    let operations: Vec<&str> = entries
        .iter()
        .filter(|e| e["principal"] == "x-bot" || e["principal"] == "y-bot")
        .map(|e| e["operation"].as_str().unwrap())
        .collect();
    assert_eq!(
        operations,
        ["mcp.assert_fact", "mcp.hybrid_search", "mcp.explain"]
    );
}

/// Acceptance criterion 3: an agent that knows only the tool list builds
/// valid calls from `describe_schema`, without hard-coded field names.
#[tokio::test]
async fn describe_schema_is_enough_to_build_valid_calls() {
    let h = Harness::new();
    let t = teams(&h).await;
    // Names the agent cannot guess.
    let salt = format!("{:x}", h.now() % 0xfff_ffff);
    let predicate = format!("p{salt}:observed");
    let key = format!("k{salt}");
    h.seed("acme", "root", GRAPH, vec![
                json!({"kind": "node", "id": "obs-1", "entity_id": "obs-1", "space_id": format!("s{salt}"),
                    "properties": {key.clone(): 41}}),
                json!({"kind": "edge", "id": "obs-1-of", "predicate": predicate, "from": "obs-1",
                    "to": "unit-7", "valid_time": {"start": 0}, "properties": {"w": 1}}),
            ]);

    let tools = rpc(&h, &t.y, 1, "tools/list", json!({})).await;
    let schemas: Vec<(String, Value)> = tools.json["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["name"].as_str().unwrap().into(), t["inputSchema"].clone()))
        .collect();
    let schema_of = |tool: &str| schemas.iter().find(|(n, _)| n == tool).unwrap().1.clone();
    // Every example the description offers uses only fields its tool's
    // input schema declares.
    let declared = |tool: &str, arguments: &Value| {
        let schema = schema_of(tool);
        let fields: Vec<String> = match schema["properties"].as_object() {
            Some(properties) => properties.keys().cloned().collect(),
            None => schema["oneOf"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|s| s["properties"].as_object().unwrap().keys().cloned())
                .collect(),
        };
        for field in arguments.as_object().unwrap().keys() {
            assert!(fields.contains(field), "{tool} has no field {field}");
        }
    };

    // Step 1: which graphs can I see?
    let graphs = ok(&h, &t.y, "describe_schema", json!({})).await;
    let graph = graphs["graphs"][0]["graph_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(graphs["graphs"][0]["permission"], "write");
    assert_eq!(graphs["reader_groups"], json!(["team-a"]));
    // Step 2: what is in it?
    let description = ok(&h, &t.y, "describe_schema", json!({"graph_id": graph})).await;
    assert_eq!(description["permission"], "write");
    let entry = description["predicates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["predicate"].as_str().unwrap().ends_with(":observed"))
        .unwrap()
        .clone();
    assert_eq!(entry["property_keys"], json!(["w"]));
    // Step 3: run every example as given.
    let mut examples = vec![entry["example"].clone(), description["example"].clone()];
    examples.extend(
        description["embedding_spaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["example"].clone()),
    );
    for example in &examples {
        let tool = example["tool"].as_str().unwrap();
        declared(tool, &example["arguments"]);
        ok(&h, &t.y, tool, example["arguments"].clone()).await;
    }
    let edges = ok(&h, &t.y, "query", entry["example"]["arguments"].clone()).await;
    let edges = edges["results"][0]["result"]["graph"]["edges"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["predicate"], entry["predicate"]);
    // Step 4: a filtered query composed from the description alone.
    let property = description["node_properties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["types"] == json!(["number"]) && p["key"] != "severity")
        .unwrap();
    assert_eq!(property["key"], key.as_str());
    let space = &description["embedding_spaces"][0]["space"];
    let dimensions = space["dimensions"].as_u64().unwrap() as usize;
    let mut search = json!({"kind": "nearest", "graph_id": graph, "k": 1,
        "space": {"id": space["id"], "revision": space["revision"]},
        "query": {"kind": "vector", "vector": vec![0.5; dimensions]}});
    declared("hybrid_search", &search);
    ok(&h, &t.y, "hybrid_search", search.clone()).await;
    // A wrong dimension is the engine's own diagnostic, which names it.
    search["query"]["vector"] = json!(vec![0.5; dimensions + 1]);
    assert_eq!(fails(&h, &t.y, "hybrid_search", search).await, "E_SPACE");
}

/// Revoking a token, suspending an agent or removing its owner from a
/// group takes effect on the agent's next tool call.
#[tokio::test]
async fn revocation_takes_effect_on_the_next_tool_call() {
    let h = Harness::new();
    let t = teams(&h).await;
    let asserted = ok(&h, &t.x, "assert_fact", finding()).await;
    let node = minted(&asserted, 0);
    let search = nearest(json!([1.0, 0.0, 0.0]));
    assert_eq!(
        neighbor_subjects(&ok(&h, &t.y, "hybrid_search", search.clone()).await)[0],
        node
    );

    // alice leaves team A: Y no longer sees the finding, on its next call.
    let reply = h
        .registry(
            &t.root,
            json!([{"change": "remove_member", "group": "team-a", "member": "alice"}]),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    assert_eq!(
        neighbor_subjects(&ok(&h, &t.y, "hybrid_search", search.clone()).await),
        ["note-1"]
    );
    h.registry(
        &t.root,
        json!([{"change": "add_member", "group": "team-a", "member": "alice"}]),
    )
    .await;

    // A revoked token fails the next call with the uniform 401.
    let tokens = h.get("/v1/tokens", &t.root).await;
    let y_token = tokens.json["tokens"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tok| tok["principal"] == "y-bot")
        .unwrap()["token_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        h.send(
            Method::DELETE,
            &format!("/v1/tokens/{y_token}"),
            Some(&t.alice),
            None
        )
        .await
        .status,
        StatusCode::OK
    );
    let revoked = call(&h, &t.y, "hybrid_search", search.clone()).await;
    assert_eq!(revoked.status, StatusCode::UNAUTHORIZED);
    assert_eq!(revoked.code(), "E_UNAUTHENTICATED");

    // Suspending X fails its next call too, writes included.
    assert_eq!(
        ok(&h, &t.x, "describe_schema", json!({})).await["graphs"][0]["graph_id"],
        GRAPH
    );
    let reply = h
        .registry(
            &t.alice,
            json!([{"change": "set_status", "id": "x-bot", "status": "suspended"}]),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let suspended = call(&h, &t.x, "assert_fact", finding()).await;
    assert_eq!(suspended.status, StatusCode::UNAUTHORIZED);
    let tools = rpc(&h, &t.x, 1, "tools/list", json!({})).await;
    assert_eq!(tools.status, StatusCode::UNAUTHORIZED);
    // Nothing more was written.
    let all = ok(
        &h,
        &t.alice,
        "query",
        json!({"graph_id": GRAPH, "predicate": "maint:about"}),
    )
    .await;
    assert_eq!(
        all["results"][0]["result"]["graph"]["edges"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

/// `retract_fact` ends a fact; `as_of` and `what_changed` still show it.
#[tokio::test]
async fn retracted_facts_stay_in_history() {
    let h = Harness::new();
    let t = teams(&h).await;
    let asserted = ok(&h, &t.x, "assert_fact", finding()).await;
    let edge = minted(&asserted, 1);
    let before = h.now();
    h.advance(1_000);
    let retracted = ok(
        &h,
        &t.y,
        "retract_fact",
        json!({"graph_id": GRAPH, "facts": [{"kind": "edge", "id": edge}]}),
    )
    .await;
    assert_eq!(retracted["facts"], json!([{"kind": "edge", "id": edge}]));
    let about = json!({"graph_id": GRAPH, "predicate": "maint:about"});
    let edge_ids = |result: &Value| -> Vec<String> {
        let mut ids: Vec<String> = result["results"][0]["result"]["graph"]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(
        edge_ids(&ok(&h, &t.y, "query", about.clone()).await),
        ["note-1-about"]
    );
    let then = ok(
        &h,
        &t.y,
        "as_of",
        json!({"unix_millis": before, "query": about}),
    )
    .await;
    assert!(edge_ids(&then).contains(&edge));
    let changed = ok(
        &h,
        &t.y,
        "what_changed",
        json!({"graph_id": GRAPH, "from": {"kind": "as_of", "unix_millis": before}}),
    )
    .await;
    let entries = &changed["results"][0]["result"]["changes"]["entries"];
    assert_eq!(entries.as_array().unwrap().len(), 1, "{entries}");
    assert_eq!(entries[0]["change"], "retracted");
    assert_eq!(entries[0]["id"], edge.as_str());
    // The retraction is audited as Y's write of that fact.
    let accesses = h.audit_facts(&t.root, GRAPH, &edge).await;
    let writes: Vec<(&str, &str)> = accesses
        .iter()
        .filter(|a| a["access"] == "write")
        .map(|a| {
            (
                a["principal"].as_str().unwrap(),
                a["change"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(writes, [("x-bot", "assert"), ("y-bot", "retract")]);
    // Retracting again is the engine's error for a fact that is not live.
    let again = fails(
        &h,
        &t.y,
        "retract_fact",
        json!({"graph_id": GRAPH, "facts": [{"kind": "edge", "id": edge}]}),
    )
    .await;
    assert!(again.starts_with("E_"), "{again}");
}

/// Each read tool's `result` is exactly the `POST /v1/execute` body for the
/// same one-command program, as the same principal.
#[tokio::test]
async fn read_tools_return_exactly_the_execute_result() {
    let h = Harness::new();
    let t = teams(&h).await;
    ok(&h, &t.x, "assert_fact", finding()).await;
    let at = h.now();
    let query = json!({"graph_id": GRAPH, "predicate": "maint:about"});
    let calls = [
        ("query", query.clone()),
        (
            "expand",
            json!({"graph_id": GRAPH, "seeds": ["unit-7"], "hops": 2, "direction": "both"}),
        ),
        ("as_of", json!({"unix_millis": at, "query": query.clone()})),
        (
            "what_changed",
            json!({"graph_id": GRAPH, "from": {"kind": "as_of", "unix_millis": 0}}),
        ),
        ("hybrid_search", nearest(json!([1.0, 0.0, 0.0]))),
        (
            "hybrid_search",
            json!({"kind": "similar_candidates", "graph_id": GRAPH,
            "space": {"id": SPACE, "revision": "1"}, "k": 1}),
        ),
        (
            "explain",
            json!({"input": {"kind": "query", "query": query.clone()}}),
        ),
        ("query", json!({"graph_id": "no-such-graph"})),
    ];
    for token in [&t.y, &t.z, &t.w] {
        for (tool, arguments) in &calls {
            let tool_enum = weave_mcp::Tool::from_name(tool).unwrap();
            let program =
                weave_mcp::program(weave_mcp::expression(tool_enum, arguments.clone()).unwrap());
            let direct = h
                .post(
                    "/v1/execute",
                    token,
                    serde_json::to_value(&program).unwrap(),
                )
                .await;
            let reply = call(&h, token, tool, arguments.clone()).await;
            let structured = &reply.json["result"]["structuredContent"];
            if direct.status == StatusCode::OK {
                assert_eq!(structured["result"], direct.json, "{tool}");
            } else {
                assert_eq!(structured["error"], direct.json["error"], "{tool}");
            }
        }
    }
}

/// The transport: MCP Streamable HTTP, one JSON response per POST, no
/// sessions or server-sent events, behind the service's authentication.
#[tokio::test]
async fn the_transport_is_authenticated_streamable_http_without_sessions() {
    let h = Harness::new();
    let t = teams(&h).await;
    let init = rpc(
        &h,
        &t.y,
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}}),
    )
    .await;
    assert_eq!(init.status, StatusCode::OK);
    assert_eq!(init.headers["content-type"], "application/json");
    assert!(init.headers.get("mcp-session-id").is_none());
    assert_eq!(init.json["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init.json["result"]["serverInfo"]["name"], "weave-service");
    let names: Vec<String> = rpc(&h, &t.y, 2, "tools/list", json!({})).await.json["result"]
        ["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "assert_fact",
            "retract_fact",
            "query",
            "expand",
            "as_of",
            "what_changed",
            "hybrid_search",
            "explain",
            "describe_schema",
            "compile"
        ]
    );
    // A notification is accepted with no body.
    let note = h
        .post(
            "/v1/mcp",
            &t.y,
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        )
        .await;
    assert_eq!(note.status, StatusCode::ACCEPTED);
    assert!(note.bytes.is_empty());
    // No credential: the uniform 401, before anything else.
    let anonymous = h
        .send(
            Method::POST,
            "/v1/mcp",
            None,
            Some(json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})),
        )
        .await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert_eq!(anonymous.code(), "E_UNAUTHENTICATED");
    // No server-sent event stream and no session to delete.
    for method in [Method::GET, Method::DELETE] {
        let reply = h.send(method, "/v1/mcp", Some(&t.y), None).await;
        assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
        let reply = h.send(Method::GET, "/v1/mcp", None, None).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    // Malformed messages are JSON-RPC errors with status 400.
    for (bytes, code) in [
        (b"{".to_vec(), -32700),
        (b"[]".to_vec(), -32600),
        (
            br#"{"jsonrpc":"2.0","id":1,"id":2,"method":"ping"}"#.to_vec(),
            -32700,
        ),
        (
            br#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#.to_vec(),
            -32600,
        ),
    ] {
        let reply = h
            .send_raw(
                Method::POST,
                "/v1/mcp",
                Some(format!("Bearer {}", t.y)),
                Some(bytes),
            )
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
        assert_eq!(reply.json["error"]["code"], code);
    }
    let unknown = call(&h, &t.y, "commit", json!({})).await;
    assert_eq!(unknown.json["error"]["code"], -32602);
    assert_eq!(unknown.json["error"]["data"]["code"], "E_MCP_UNKNOWN_TOOL");
    let unknown = rpc(&h, &t.y, 3, "resources/list", json!({})).await;
    assert_eq!(unknown.json["error"]["code"], -32601);
    // Browser origins and unknown protocol versions are refused.
    let request = |header: &str, value: &str| {
        axum::http::Request::builder()
            .method(Method::POST)
            .uri("/v1/mcp")
            .header("authorization", format!("Bearer {}", t.y))
            .header(header, value)
            .body(axum::body::Body::from(
                r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            ))
            .unwrap()
    };
    let origin = h.request(request("origin", "https://evil.example")).await;
    assert_eq!(origin.status, StatusCode::FORBIDDEN);
    assert_eq!(origin.code(), "E_MCP_ORIGIN");
    let version = h
        .request(request("mcp-protocol-version", "1999-01-01"))
        .await;
    assert_eq!(version.status, StatusCode::BAD_REQUEST);
    assert_eq!(version.code(), "E_MCP_PROTOCOL_VERSION");
    let version = h
        .request(request("mcp-protocol-version", "2025-06-18"))
        .await;
    assert_eq!(version.status, StatusCode::OK);
    // Every message is audited, a failure with its code.
    let entries = h.audit_entries(&t.root).await;
    let mine: Vec<(String, String)> = entries
        .iter()
        .filter(|e| e["principal"] == "y-bot")
        .map(|e| {
            (
                e["operation"].as_str().unwrap().to_string(),
                e["outcome"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(mine.contains(&("mcp.initialize".into(), "ok".into())));
    assert!(mine.contains(&("mcp.tools_list".into(), "ok".into())));
    assert!(mine.contains(&("mcp.notification".into(), "ok".into())));
    assert!(mine.contains(&("mcp.invalid".into(), "error".into())));
    assert!(mine.contains(&("mcp.unknown_tool".into(), "error".into())));
}

/// B2 part 2 (contract 1.2.0) applies to agents: the tools emit 1.2.0
/// programs, so a fact broader than the restricted evidence it cites fails
/// `E_DERIVED_READERS`, and the engine's write codes reach the agent
/// unchanged.
#[tokio::test]
async fn agent_writes_follow_the_write_time_rules_of_contract_1_2() {
    let h = Harness::new();
    let t = teams(&h).await;
    let asserted = ok(&h, &t.x, "assert_fact", finding()).await;
    let node = minted(&asserted, 0);
    let revision = asserted["revision"].as_str().unwrap().to_string();
    let citing = |visibility: Value| {
        json!({"graph_id": GRAPH,
            "provenance": {"method": {"kind": "model", "model": MODEL, "version": "2026-10"},
                "evidence": [{"kind": "node", "graph_id": GRAPH, "revision": revision,
                    "node_id": node}]},
            "visibility": visibility,
            "facts": [{"kind": "node", "space_id": "summary",
                "properties": {"text": "Unit 7 had a bearing replaced"}}]})
    };
    // Publishing a summary of team A's finding to everyone is refused.
    assert_eq!(
        fails(&h, &t.y, "assert_fact", citing(json!({"public": true}))).await,
        "E_DERIVED_READERS"
    );
    // Within team A it is fine.
    ok(
        &h,
        &t.y,
        "assert_fact",
        citing(json!({"reader_groups": ["team-a"]})),
    )
    .await;
    // Retracting a fact the caller cannot read is the uniform
    // unavailable-fact error, as for one that does not exist.
    let hidden = call(
        &h,
        &t.z,
        "retract_fact",
        json!({"graph_id": GRAPH, "facts": [{"kind": "node", "id": node}]}),
    )
    .await;
    let missing = call(
        &h,
        &t.z,
        "retract_fact",
        json!({"graph_id": GRAPH, "facts": [{"kind": "node", "id": "r-none"}]}),
    )
    .await;
    let error = |reply: &Reply| reply.json["result"]["structuredContent"]["error"].clone();
    assert_eq!(
        error(&hidden).to_string().replace(&node, "<id>"),
        error(&missing).to_string().replace("r-none", "<id>")
    );
    // Every tool program is a 1.2.0 program.
    let program = weave_mcp::agent::plan_retract(
        json!({"graph_id": GRAPH, "facts": [{"kind": "node", "id": "n"}]}),
    )
    .unwrap()
    .program();
    assert!(weave_contract::version_at_least(&program.version, "1.2.0"));
}
