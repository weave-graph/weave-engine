//! B3 (#39): paired hidden-input tests at the HTTP layer. Two tenants that
//! differ only in data and registry state hidden from the caller give the
//! caller byte-identical responses. Plus the service's own input rules:
//! engine-assigned graph IDs, minted record IDs and bounded requests.
mod common;
use axum::http::{Method, StatusCode};
use common::*;
use serde_json::{json, Value};
use weave_service::Limits;

const OBSERVE_AT: i64 = START_MS + 10_000_000;

struct World {
    h: Harness,
    root: String,
    bob: String,
    head: String,
}

/// A tenant where Bob can write graph `shared`. With `hidden`, the tenant
/// also has a graph Bob cannot discover, a group he cannot discover, and
/// records in `shared` he cannot read.
async fn world(hidden: bool) -> World {
    let h = Harness::new();
    let (root, _alice, bob) = standard_tenant(&h, "acme").await;
    let created = h
        .post("/v1/graphs", &root, json!({"graph_id": "shared"}))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    h.registry(
        &root,
        json!([{"change": "grant", "graph_id": "shared", "subject": "bob", "permission": "write"}]),
    )
    .await;
    let reply = h
        .write(
            &root,
            "shared",
            vec![
                node("n1", &[], &[]),
                node("n2", &[], &[]),
                edge("e1", "n1", "n2", &[]),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    if hidden {
        h.post("/v1/graphs", &root, json!({"graph_id": "hidden"}))
            .await;
        h.registry(
            &root,
            json!([
                {"change": "add_principal", "id": "secret-team", "kind": "group"},
                {"change": "add_member", "group": "secret-team", "member": "alice"},
                {"change": "grant", "graph_id": "hidden", "subject": "alice", "permission": "read"}
            ]),
        )
        .await;
        let reply = h
            .write(
                &root,
                "hidden",
                vec![node("n1", &[], &[]), node("x", &[], &[])],
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
        let reply = h
            .write(
                &root,
                "shared",
                vec![
                    node("h-root", &["root"], &[]),
                    node("h-team", &[], &["secret-team"]),
                    edge("h-edge", "n1", "h-root", &["root"]),
                    edge("h-edge2", "n2", "n1", &["root"]),
                ],
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    }
    let tenant = h.service.tenant("acme").unwrap();
    let head = tenant
        .with_state(|s| s.engine.head("shared", "main").unwrap().unwrap())
        .unwrap();
    h.clock.set(OBSERVE_AT);
    World { h, root, bob, head }
}

/// Bob's requests, each with a method, path and optional body.
fn observations() -> Vec<(Method, String, Option<Value>)> {
    let program =
        |commands: Value| Some(json!({"version": weave_contract::VERSION, "commands": commands}));
    let exec = |commands: Value| (Method::POST, "/v1/execute".to_string(), program(commands));
    vec![
        (Method::GET, "/v1/graphs".into(), None),
        (Method::GET, "/v1/manifest".into(), None),
        exec(json!([{"op": "query", "query": {"graph_id": "shared"}}])),
        exec(
            json!([{"op": "evaluate", "value": {"kind": "expand", "graph_id": "shared",
            "seeds": ["n1", "n2"], "hops": 2, "limit": 100}}]),
        ),
        exec(
            json!([{"op": "evaluate", "value": {"kind": "expand", "graph_id": "shared",
            "seeds": ["n1", "h-root"], "hops": 1, "limit": 100}}]),
        ),
        exec(
            json!([{"op": "evaluate", "value": {"kind": "changes", "graph_id": "shared",
            "from": {"kind": "as_of", "unix_millis": 0}}}]),
        ),
        exec(
            json!([{"op": "evaluate", "value": {"kind": "as_of", "unix_millis": OBSERVE_AT,
            "query": {"graph_id": "shared"}}}]),
        ),
        // The hidden graph, as if it did not exist.
        exec(json!([{"op": "query", "query": {"graph_id": "hidden"}}])),
        exec(json!([{"op": "write", "graph_id": "hidden", "changes": [
            {"change": "retract", "target": {"kind": "node", "id": "x"}}]}])),
        (Method::GET, "/v1/graphs/hidden/acl".into(), None),
        (
            Method::POST,
            "/v1/graphs".into(),
            Some(json!({"graph_id": "hidden"})),
        ),
        // The hidden group, as if it did not exist.
        exec(
            json!([{"op": "write", "graph_id": "shared", "changes": [{"change": "correct",
            "fact": {"kind": "node", "id": "n1", "entity_id": "E-n1", "space_id": "s",
                "reader_groups": ["secret-team"]}}]}]),
        ),
        (
            Method::GET,
            "/v1/registry/principals/secret-team".into(),
            None,
        ),
        (
            Method::GET,
            "/v1/registry/groups/secret-team/members".into(),
            None,
        ),
        // Hidden records, as if they did not exist: correct, retract and
        // assert of their IDs.
        exec(
            json!([{"op": "write", "graph_id": "shared", "changes": [{"change": "retract",
            "target": {"kind": "node", "id": "h-root"}}]}]),
        ),
        exec(
            json!([{"op": "write", "graph_id": "shared", "changes": [{"change": "correct",
            "fact": {"kind": "node", "id": "h-team", "entity_id": "E-h-team", "space_id": "s"}}]}]),
        ),
        exec(
            json!([{"op": "write", "graph_id": "shared", "changes": [{"change": "assert",
            "fact": {"kind": "node", "id": "h-root", "entity_id": "E-h-root", "space_id": "s"}}]}]),
        ),
        exec(
            json!([{"op": "write", "graph_id": "shared", "changes": [{"change": "retract",
            "target": {"kind": "edge", "id": "h-edge"}}]}]),
        ),
    ]
}

#[tokio::test]
async fn hidden_graphs_groups_and_records_never_change_a_callers_responses() {
    let plain = world(false).await;
    let hidden = world(true).await;
    assert_ne!(plain.head, hidden.head);
    for (method, path, body) in observations() {
        let a = plain
            .h
            .send(method.clone(), &path, Some(&plain.bob), body.clone())
            .await;
        let b = hidden
            .h
            .send(method.clone(), &path, Some(&hidden.bob), body.clone())
            .await;
        // Revision IDs identify whole immutable snapshots (a declared
        // channel, see SECURITY.md), so each world's head is normalized.
        let normalize = |bytes: &[u8], head: &str| {
            String::from_utf8(bytes.to_vec())
                .unwrap()
                .replace(head, "<head>")
        };
        assert_eq!(a.status, b.status, "{method} {path} {body:?}: {}", b.json);
        assert_eq!(
            normalize(&a.bytes, &plain.head),
            normalize(&b.bytes, &hidden.head),
            "{method} {path} {body:?}"
        );
        assert!(
            !String::from_utf8_lossy(&b.bytes).contains("h-edge2"),
            "{path}: {}",
            b.json
        );
    }
    // An explanation's derived record IDs hash the input revision, so only
    // its facts and their provenance are compared.
    let explain = json!([{"op": "evaluate", "value": {"kind": "explain", "input":
        {"kind": "query", "query": {"graph_id": "shared"}}}}]);
    let a = plain.h.execute(&plain.bob, explain.clone()).await;
    let b = hidden.h.execute(&hidden.bob, explain).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.json);
    for field in ["fact_provenance", "provenance", "coverage", "diagnostics"] {
        let normalize = |v: &Value, head: &str| v.to_string().replace(head, "<head>");
        assert_eq!(
            normalize(&a.json["results"][0]["result"][field], &plain.head),
            normalize(&b.json["results"][0]["result"][field], &hidden.head),
            "{field}"
        );
    }
    // The hidden world really holds what Bob could not see.
    let all = hidden.h.query(&hidden.root, "shared").await;
    assert!(node_ids(&all).contains(&"h-root".to_string()));
}

#[tokio::test]
async fn graphs_get_engine_assigned_ids_unless_an_administrator_chooses() {
    let h = Harness::new();
    let (root, alice, bob) = standard_tenant(&h, "acme").await;
    let created = h.post("/v1/graphs", &alice, json!({})).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    let graph = created.json["graph_id"].as_str().unwrap().to_string();
    assert!(graph.starts_with(weave_engine::CREATED_GRAPH_PREFIX) && graph.len() == 34);
    // The creator administers it; nobody else can discover it.
    let acl = h.get(&format!("/v1/graphs/{graph}/acl"), &alice).await;
    assert_eq!(
        acl.json["grants"],
        json!([{"subject": "alice", "permission": "administer"}])
    );
    let listing = h.get("/v1/graphs", &bob).await;
    assert_eq!(listing.json["graphs"], json!([]));
    assert_ne!(
        h.post("/v1/graphs", &alice, json!({})).await.json["graph_id"],
        json!(graph)
    );
    // A non-administrator cannot choose an ID, whether or not it is taken.
    h.post("/v1/graphs", &root, json!({"graph_id": "taken"}))
        .await;
    let taken = h
        .post("/v1/graphs", &bob, json!({"graph_id": "taken"}))
        .await;
    let free = h
        .post("/v1/graphs", &bob, json!({"graph_id": "free"}))
        .await;
    assert_eq!(taken.status, StatusCode::FORBIDDEN);
    assert_eq!(taken.bytes, free.bytes);
    // An administrator can, and sees every collision.
    let again = h
        .post("/v1/graphs", &root, json!({"graph_id": "taken"}))
        .await;
    assert_eq!(again.code(), "E_CONFLICT");
    let reserved = h
        .post("/v1/graphs", &root, json!({"graph_id": "weave:registry"}))
        .await;
    assert_eq!(reserved.code(), "E_RESERVED_NAMESPACE");
}

#[tokio::test]
async fn non_administrators_assert_only_ids_minted_for_them() {
    let h = Harness::new();
    let (root, alice, bob) = standard_tenant(&h, "acme").await;
    let graph = h.post("/v1/graphs", &alice, json!({})).await.json["graph_id"]
        .as_str()
        .unwrap()
        .to_string();
    h.registry(
        &alice,
        json!([{"change": "grant", "graph_id": graph, "subject": "bob", "permission": "write"}]),
    )
    .await;
    let alice_ids = h.ids(&alice, 2).await;
    let bob_ids = h.ids(&bob, 1).await;
    assert!(alice_ids
        .iter()
        .all(|id| id.starts_with("r-") && id.len() == 51));
    // Chosen IDs and IDs minted for someone else are refused before the
    // engine sees them, so occupancy by hidden records cannot be probed.
    for id in ["n1", alice_ids[0].as_str()] {
        let reply = h.write(&bob, &graph, vec![node(id, &[], &[])]).await;
        assert_eq!(reply.code(), "E_ID_NOT_MINTED", "{id}");
    }
    let reply = h
        .write(&bob, &graph, vec![node(&bob_ids[0], &[], &[])])
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    // Edges reference any visible node; only the asserted ID is checked.
    let reply = h
        .write(
            &alice,
            &graph,
            vec![
                node(&alice_ids[0], &[], &[]),
                edge(&alice_ids[1], &alice_ids[0], &bob_ids[0], &[]),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    // Corrections and retractions name existing facts and need no minting.
    let reply = h
        .execute(
            &bob,
            json!([{"op": "write", "graph_id": graph, "changes": [{"change": "retract",
                "target": {"kind": "edge", "id": alice_ids[1]}}]}]),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    // Tenant administrators see every record, so they may choose IDs.
    let reply = h.write(&root, &graph, vec![node("chosen", &[], &[])]).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    // Minting is bounded.
    for count in [0, 1001] {
        let reply = h.post("/v1/ids", &bob, json!({"count": count})).await;
        assert_eq!(reply.code(), "E_BUDGET");
    }
}

#[tokio::test]
async fn programs_and_requests_are_bounded() {
    let h = Harness::with_limits(Limits {
        max_body_bytes: 4096,
        ..Limits::default()
    });
    let root = h.tenant("acme", "root");
    let big = json!({"version": weave_contract::VERSION, "commands": [], "pad": "x".repeat(5000)});
    let reply = h.post("/v1/execute", &root, big).await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::PAYLOAD_TOO_LARGE, "E_BODY_LIMIT")
    );
    let old = h
        .post(
            "/v1/execute",
            &root,
            json!({"version": "0.24.0", "commands": [{"op": "query", "query": {"graph_id": "g"}}]}),
        )
        .await;
    assert_eq!(old.code(), "E_VERSION");
    let none = h.execute(&root, json!([])).await;
    assert_eq!(none.code(), "E_BUDGET");
    let many: Vec<Value> = (0..65)
        .map(|_| json!({"op": "query", "query": {"graph_id": "g"}}))
        .collect();
    let h2 = Harness::new();
    let root2 = h2.tenant("acme", "root");
    assert_eq!(h2.execute(&root2, json!(many)).await.code(), "E_BUDGET");
    for command in [
        json!({"op": "commit", "graph_id": "g", "data": {}}),
        json!({"op": "commit_batch", "batch_id": "b", "commits": []}),
    ] {
        let reply = h.execute(&root, json!([command])).await;
        assert!(
            ["E_UNSUPPORTED", "E_DECODE", "E_UNKNOWN_FIELD"].contains(&reply.code()),
            "{}",
            reply.json
        );
    }
    let reply = h
        .execute(
            &root,
            json!([{"op": "commit", "graph_id": "g", "data": {"nodes": [], "edges": []}}]),
        )
        .await;
    assert_eq!(reply.code(), "E_UNSUPPORTED", "{}", reply.json);
    let duplicate = h
        .send_raw(
            Method::POST,
            "/v1/execute",
            Some(format!("Bearer {root}")),
            Some(br#"{"version":"1.1.0","version":"1.1.0","commands":[]}"#.to_vec()),
        )
        .await;
    assert_eq!(duplicate.status, StatusCode::BAD_REQUEST);
    let malformed = h
        .send_raw(
            Method::POST,
            "/v1/registry",
            Some(format!("Bearer {root}")),
            Some(b"{".to_vec()),
        )
        .await;
    assert_eq!(malformed.code(), "E_INPUT");
    let long = "x".repeat(600);
    let reply = h
        .get(&format!("/v1/registry/principals/{long}"), &root)
        .await;
    assert_eq!(reply.code(), "E_INPUT");
}
