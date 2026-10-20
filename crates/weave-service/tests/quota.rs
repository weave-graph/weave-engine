//! B4 (#40): per-agent quotas and rate limits. Agents get a request rate
//! and a fact-write quota by default; operators override them per principal
//! or per tenant; a write over the quota writes nothing; and concurrent
//! writes at the boundary commit exactly the permitted number, leaving no
//! fact, provenance or audit row behind for the rejected ones.
mod common;
mod mcp_common;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use common::*;
use mcp_common::*;
use serde_json::{json, Value};
use tower::ServiceExt;
use weave_service::quota::{Quota, AGENTS, HOUR_MS, MINUTE_MS};
use weave_service::Limits;

fn limits(requests: Option<u64>, writes: Option<u64>) -> Limits {
    Limits {
        agent_quota: Quota {
            requests_per_minute: requests,
            writes_per_hour: writes,
        },
        ..Limits::default()
    }
}

/// A one-node assert by an agent, team A only.
fn one_node(text: &str) -> Value {
    json!({"graph_id": GRAPH,
        "provenance": {"method": {"kind": "model", "model": "m", "version": "1"}},
        "visibility": {"reader_groups": ["team-a"]},
        "facts": [{"kind": "node", "space_id": "q", "properties": {"text": text}}]})
}

/// Start the clock at a whole hour, so the windows are predictable.
fn align(h: &Harness) {
    let now = h.now();
    h.clock.set((now / HOUR_MS + 1) * HOUR_MS);
}

/// Nodes of space `q` that `token` can see.
async fn q_nodes(h: &Harness, token: &str) -> Vec<Value> {
    let changes = ok(
        h,
        token,
        "what_changed",
        json!({"graph_id": GRAPH, "from": {"kind": "as_of", "unix_millis": 0}}),
    )
    .await;
    changes["results"][0]["result"]["changes"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["after"]["space_id"] == "q")
        .map(|e| e["after"].clone())
        .collect()
}

#[tokio::test]
async fn an_agent_has_a_request_rate_and_users_do_not_by_default() {
    let h = Harness::with_limits(limits(Some(5), None));
    let t = teams(&h).await;
    align(&h);
    for _ in 0..5 {
        assert_eq!(h.get("/v1/whoami", &t.x).await.status, StatusCode::OK);
    }
    // The sixth request in the minute, on any route, MCP included.
    let over = h.get("/v1/whoami", &t.x).await;
    assert_eq!(over.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(over.code(), "E_QUOTA");
    let retry: u64 = over.headers["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry), "{retry}");
    let rpc_over = call(&h, &t.x, "describe_schema", json!({})).await;
    assert_eq!(rpc_over.status, StatusCode::TOO_MANY_REQUESTS);
    // Rejections are not counted, and are audited as such.
    let entries = h.audit_entries(&t.root).await;
    let refused = entries
        .iter()
        .filter(|e| e["principal"] == "x-bot" && e["code"] == "E_QUOTA")
        .count();
    assert_eq!(refused, 2);
    // Other agents have their own windows; users are not limited.
    assert_eq!(h.get("/v1/whoami", &t.y).await.status, StatusCode::OK);
    for _ in 0..20 {
        assert_eq!(h.get("/v1/whoami", &t.alice).await.status, StatusCode::OK);
    }
    // The next minute admits X again.
    h.advance(MINUTE_MS);
    let quota = h.get("/v1/quota", &t.x).await;
    assert_eq!(quota.status, StatusCode::OK, "{}", quota.json);
    assert_eq!(quota.json["principal"], "x-bot");
    assert_eq!(quota.json["requests_per_minute"], 5);
    assert_eq!(quota.json["writes_per_hour"], Value::Null);
    assert_eq!(quota.json["requests_this_minute"], 1);
}

#[tokio::test]
async fn a_write_over_the_quota_writes_nothing() {
    let h = Harness::with_limits(limits(None, Some(4)));
    let t = teams(&h).await;
    align(&h);
    let mut three = one_node("a");
    three["facts"] = json!([
        {"kind": "node", "space_id": "q", "properties": {"text": "a"}},
        {"kind": "node", "space_id": "q", "properties": {"text": "b"}},
        {"kind": "node", "space_id": "q", "properties": {"text": "c"}},
    ]);
    ok(&h, &t.x, "assert_fact", three.clone()).await;
    // Three used: two more would pass the limit of four.
    let mut two = three.clone();
    two["facts"].as_array_mut().unwrap().truncate(2);
    assert_eq!(fails(&h, &t.x, "assert_fact", two).await, "E_QUOTA");
    assert_eq!(q_nodes(&h, &t.y).await.len(), 3);
    // One more fits; then nothing does, through any write path.
    ok(&h, &t.x, "assert_fact", one_node("d")).await;
    let node_id = {
        let all = q_nodes(&h, &t.y).await;
        all[0]["id"].as_str().unwrap().to_string()
    };
    assert_eq!(
        fails(
            &h,
            &t.x,
            "retract_fact",
            json!({"graph_id": GRAPH, "facts": [{"kind": "node", "id": node_id}]})
        )
        .await,
        "E_QUOTA"
    );
    let ids = h.ids(&t.x, 1).await;
    let direct = h
        .write(&t.x, GRAPH, vec![common::node(&ids[0], &[], &["team-a"])])
        .await;
    assert_eq!(direct.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(direct.code(), "E_QUOTA");
    assert!(direct.headers.get("retry-after").is_some());
    // Reads are not writes; other agents have their own quota.
    assert_eq!(q_nodes(&h, &t.x).await.len(), 4);
    ok(&h, &t.y, "assert_fact", one_node("y")).await;
    // The next hour admits X again.
    h.advance(HOUR_MS);
    ok(&h, &t.x, "assert_fact", one_node("e")).await;
}

#[tokio::test]
async fn operators_override_quotas_per_principal_and_per_tenant() {
    let h = Harness::with_limits(limits(Some(100), Some(100)));
    let t = teams(&h).await;
    align(&h);
    let whoami = |token: &str| {
        let token = token.to_string();
        let h = &h;
        async move { h.get("/v1/whoami", &token).await.status }
    };
    // A tenant default for agents replaces the service default.
    h.service
        .set_quota(
            "acme",
            AGENTS,
            Quota {
                requests_per_minute: Some(1),
                writes_per_hour: None,
            },
        )
        .unwrap();
    assert_eq!(whoami(&t.x).await, StatusCode::OK);
    assert_eq!(whoami(&t.x).await, StatusCode::TOO_MANY_REQUESTS);
    // A principal's own row wins, and `None` is unlimited.
    h.service
        .set_quota("acme", "x-bot", Quota::default())
        .unwrap();
    for _ in 0..5 {
        assert_eq!(whoami(&t.x).await, StatusCode::OK);
    }
    // Users are limited only by their own row.
    h.service
        .set_quota(
            "acme",
            "alice",
            Quota {
                requests_per_minute: Some(2),
                writes_per_hour: Some(0),
            },
        )
        .unwrap();
    assert_eq!(whoami(&t.alice).await, StatusCode::OK);
    let ids = h.ids(&t.alice, 1).await;
    let write = h
        .write(&t.alice, GRAPH, vec![common::node(&ids[0], &[], &[])])
        .await;
    assert_eq!(write.code(), "E_QUOTA");
    assert_eq!(whoami(&t.alice).await, StatusCode::TOO_MANY_REQUESTS);
    // Removing rows falls back, on the next request.
    assert!(h.service.remove_quota("acme", "alice").unwrap());
    assert_eq!(whoami(&t.alice).await, StatusCode::OK);
    assert!(h.service.remove_quota("acme", "x-bot").unwrap());
    assert_eq!(whoami(&t.x).await, StatusCode::TOO_MANY_REQUESTS);
    assert!(!h.service.remove_quota("acme", "x-bot").unwrap());
    let rows = h.service.quotas("acme").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, AGENTS);
    assert!(h
        .service
        .set_quota("nowhere", AGENTS, Quota::default())
        .is_err());
}

/// The concurrent boundary: N agents' worth of writes arrive at once for one
/// agent whose quota permits only some. Exactly the permitted number commit,
/// and the rejected ones leave no fact, provenance or audit write behind.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_writes_at_the_quota_boundary_commit_exactly_the_permitted_number() {
    for (facts_per_call, limit, calls, expected_calls) in [(1, 10, 30, 10), (3, 10, 20, 3)] {
        let h = Harness::with_limits(limits(None, Some(limit)));
        let t = teams(&h).await;
        align(&h);
        let router = h.router();
        let mut tasks = tokio::task::JoinSet::new();
        for n in 0..calls {
            let facts: Vec<Value> = (0..facts_per_call)
                .map(|i| json!({"kind": "node", "space_id": "q", "properties": {"text": format!("{n}-{i}")}}))
                .collect();
            let mut arguments = one_node("");
            arguments["facts"] = json!(facts);
            let body = json!({"jsonrpc": "2.0", "id": n, "method": "tools/call",
                "params": {"name": "assert_fact", "arguments": arguments}});
            let request = Request::builder()
                .method(Method::POST)
                .uri("/v1/mcp")
                .header("authorization", format!("Bearer {}", t.x))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap();
            let router = router.clone();
            tasks.spawn(async move {
                let response = router.oneshot(request).await.unwrap();
                let bytes = http_body_util::BodyExt::collect(response.into_body())
                    .await
                    .unwrap()
                    .to_bytes();
                serde_json::from_slice::<Value>(&bytes).unwrap()
            });
        }
        let mut committed = Vec::new();
        let mut refused = 0;
        while let Some(reply) = tasks.join_next().await {
            let result = &reply.unwrap()["result"];
            if result["isError"] == true {
                assert_eq!(result["structuredContent"]["error"]["code"], "E_QUOTA");
                refused += 1;
            } else {
                for fact in result["structuredContent"]["result"]["facts"]
                    .as_array()
                    .unwrap()
                {
                    committed.push(fact["id"].as_str().unwrap().to_string());
                }
            }
        }
        assert_eq!(committed.len(), expected_calls * facts_per_call);
        assert_eq!(refused, calls - expected_calls);
        // The store holds exactly the committed facts, each with X's
        // provenance.
        let stored = q_nodes(&h, &t.y).await;
        let mut stored_ids: Vec<String> = stored
            .iter()
            .map(|n| n["id"].as_str().unwrap().to_string())
            .collect();
        stored_ids.sort();
        committed.sort();
        assert_eq!(stored_ids, committed);
        let changes = ok(
            &h,
            &t.y,
            "what_changed",
            json!({"graph_id": GRAPH, "from": {"kind": "as_of", "unix_millis": 0}}),
        )
        .await;
        let writers: Vec<&Value> = changes["results"][0]["result"]["fact_provenance"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["provenance"]["writer"] == "x-bot")
            .collect();
        assert_eq!(writers.len(), committed.len());
        // The audit log has one write per committed fact and none for the
        // rejected calls.
        let entries = h.audit_entries(&t.root).await;
        // X's only requests are these asserts, whose fact rows are writes.
        let writes: u64 = entries
            .iter()
            .filter(|e| e["principal"] == "x-bot")
            .map(|e| e["fact_count"].as_u64().unwrap())
            .sum();
        assert_eq!(writes, committed.len() as u64);
        for id in &committed {
            let accesses = h.audit_facts(&t.root, GRAPH, id).await;
            let writers: Vec<&Value> = accesses.iter().filter(|a| a["access"] == "write").collect();
            assert_eq!(writers.len(), 1);
            assert_eq!(writers[0]["principal"], "x-bot");
        }
        let quota_failures = entries
            .iter()
            .filter(|e| e["principal"] == "x-bot" && e["code"] == "E_QUOTA")
            .count();
        assert_eq!(quota_failures, calls - expected_calls);
    }
}
