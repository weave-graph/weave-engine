//! Shared setup for the B4 agent-interface tests: two teams sharing one
//! graph, each with a human and agents, driven through `POST /v1/mcp`.
#![allow(dead_code)]
use crate::common::*;
use axum::http::StatusCode;
use serde_json::{json, Value};

pub const GRAPH: &str = "maintenance";
pub const SPACE: &str = "notes";
pub const MODEL: &str = "x-extractor";
pub const DOC_URI: &str = "doc://maintenance/log-42";
pub const DOC_DIGEST: &str =
    "sha256:6c0f4f0e5f8f2a1d9c7b3e4a5d6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f";
pub const FINDING_TEXT: &str = "Bearing on unit 7 replaced after the vibration alarm";

/// One JSON-RPC request to `POST /v1/mcp`.
pub async fn rpc(h: &Harness, token: &str, id: i64, method: &str, params: Value) -> Reply {
    h.post(
        "/v1/mcp",
        token,
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
    )
    .await
}

/// A tool call; returns the whole HTTP reply.
pub async fn call(h: &Harness, token: &str, tool: &str, arguments: Value) -> Reply {
    rpc(
        h,
        token,
        7,
        "tools/call",
        json!({"name": tool, "arguments": arguments}),
    )
    .await
}

/// A tool call that must succeed; returns its `result`.
pub async fn ok(h: &Harness, token: &str, tool: &str, arguments: Value) -> Value {
    let reply = call(h, token, tool, arguments.clone()).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let result = &reply.json["result"];
    assert_eq!(
        result["isError"], false,
        "{tool} {arguments}: {}",
        result["structuredContent"]
    );
    result["structuredContent"]["result"].clone()
}

/// A tool call that must fail; returns its error code.
pub async fn fails(h: &Harness, token: &str, tool: &str, arguments: Value) -> String {
    let reply = call(h, token, tool, arguments.clone()).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let result = &reply.json["result"];
    assert_eq!(result["isError"], true, "{tool} {arguments}: {result}");
    result["structuredContent"]["error"]["code"]
        .as_str()
        .unwrap()
        .to_string()
}

pub struct Teams {
    pub root: String,
    pub alice: String,
    pub carol: String,
    /// alice's agents, team A.
    pub x: String,
    pub y: String,
    /// carol's agent, team B: reads the graph, not team A's records.
    pub z: String,
    /// dave's agent: no grant on the graph at all.
    pub w: String,
}

/// Tenant `acme`: root administers; alice is in `team-a`, carol in
/// `team-b`, dave in neither. Graph `maintenance` (alice and carol write,
/// dave nothing) holds a public unit node and the public embedding space
/// `notes@1` (3 dimensions, cosine) with a public note on the unit. alice
/// owns agents `x-bot` and `y-bot`, carol `z-bot`, dave `w-bot`, each
/// delegated its owner's whole identity and Write on the graph.
pub async fn teams(h: &Harness) -> Teams {
    let root = h.tenant("acme", "root");
    let reply = h
        .registry(
            &root,
            json!([
                {"change": "add_principal", "id": "alice", "kind": "user"},
                {"change": "add_principal", "id": "carol", "kind": "user"},
                {"change": "add_principal", "id": "dave", "kind": "user"},
                {"change": "add_principal", "id": "team-a", "kind": "group"},
                {"change": "add_principal", "id": "team-b", "kind": "group"},
                {"change": "add_member", "group": "team-a", "member": "alice"},
                {"change": "add_member", "group": "team-b", "member": "carol"}
            ]),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let created = h
        .post("/v1/graphs", &root, json!({"graph_id": GRAPH}))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    let reply = h
        .registry(
            &root,
            json!([
                {"change": "grant", "graph_id": GRAPH, "subject": "alice", "permission": "write"},
                {"change": "grant", "graph_id": GRAPH, "subject": "carol", "permission": "write"}
            ]),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    h.seed(
        "acme",
        "root",
        GRAPH,
        vec![
            json!({"kind": "node", "id": "unit-7", "entity_id": "unit-7", "space_id": "unit",
                    "properties": {"serial": "U-0007"}}),
            json!({"kind": "node", "id": "site-1", "entity_id": "site-1", "space_id": "site",
                    "properties": {"name": "North"}}),
            json!({"kind": "edge", "id": "unit-7-at", "predicate": "hw:located_at",
                    "from": "unit-7", "to": "site-1", "valid_time": {"start": 0}}),
            json!({"kind": "embedding_space", "id": SPACE, "revision": "1",
                    "encoder": "test-encoder", "encoder_version": "1", "preprocessing": "none",
                    "dimensions": 3, "metric": "cosine_distance"}),
            json!({"kind": "node", "id": "note-1", "entity_id": "note-1", "space_id": "note",
                    "properties": {"text": "Routine inspection, no findings"}}),
            json!({"kind": "edge", "id": "note-1-about", "predicate": "maint:about",
                    "from": "note-1", "to": "unit-7", "valid_time": {"start": 0}}),
            json!({"kind": "embedding", "id": "note-1-vec", "space": {"id": SPACE, "revision": "1"},
                    "subject": {"kind": "node", "node_id": "note-1"}, "vector": [0.0, 1.0, 0.0],
                    "valid_time": {"start": 0}}),
        ],
    );
    let alice = h.token_for(&root, "alice").await;
    let carol = h.token_for(&root, "carol").await;
    let dave = h.token_for(&root, "dave").await;
    let agent = |id: &'static str| {
        json!({"change": "add_agent", "id": id, "delegation": {
            "scopes": [{"graph_id": GRAPH, "permission": "write"}],
            "exercise": {"kind": "owner_all"}}})
    };
    let mut x_change = agent("x-bot");
    x_change["owner"] = json!("alice");
    let mut y_change = agent("y-bot");
    y_change["owner"] = json!("alice");
    let reply = h.registry(&alice, json!([x_change, y_change])).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let mut z_change = agent("z-bot");
    z_change["owner"] = json!("carol");
    let reply = h.registry(&carol, json!([z_change])).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let w_change = json!({"change": "add_agent", "id": "w-bot", "owner": "dave",
        "delegation": {"scopes": [], "exercise": {"kind": "owner_all"}}});
    let reply = h.registry(&dave, json!([w_change])).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let x = h.token_for(&alice, "x-bot").await;
    let y = h.token_for(&alice, "y-bot").await;
    let z = h.token_for(&carol, "z-bot").await;
    let w = h.token_for(&dave, "w-bot").await;
    Teams {
        root,
        alice,
        carol,
        x,
        y,
        z,
        w,
    }
}

/// X's finding from maintenance log 42, readable by team A: a node about
/// unit 7, an edge to it and an embedding near `[1, 0, 0]`.
pub fn finding() -> Value {
    json!({
        "graph_id": GRAPH,
        "provenance": {
            "method": {"kind": "model", "model": MODEL, "version": "2026-10"},
            "source": "maintenance log 42",
            "evidence": [{"kind": "artifact", "uri": DOC_URI, "digest": DOC_DIGEST}],
        },
        "visibility": {"reader_groups": ["team-a"]},
        "facts": [
            {"kind": "node", "ref": "finding", "space_id": "finding",
             "properties": {"text": FINDING_TEXT, "severity": 3}},
            {"kind": "edge", "predicate": "maint:about", "from": {"ref": "finding"},
             "to": "unit-7", "valid_time": {"start": 1_000}},
            {"kind": "embedding", "space": {"id": SPACE, "revision": "1"},
             "subject": {"kind": "node", "ref": "finding"}, "vector": [1.0, 0.0, 0.1],
             "valid_time": {"start": 1_000}},
        ],
    })
}

/// The minted ID of the fact `ref` (or the n-th fact) in an assert result.
pub fn minted(result: &Value, index: usize) -> String {
    result["facts"][index]["id"].as_str().unwrap().to_string()
}

pub fn nearest(vector: Value) -> Value {
    json!({"kind": "nearest", "graph_id": GRAPH, "space": {"id": SPACE, "revision": "1"},
        "query": {"kind": "vector", "vector": vector}, "k": 5})
}

/// Node IDs of a nearest result's neighbors, in rank order.
pub fn neighbor_subjects(result: &Value) -> Vec<String> {
    result["results"][0]["result"]["neighbors"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|n| n["subject"]["node_id"].as_str().unwrap_or("?").to_string())
        .collect()
}
