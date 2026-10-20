//! B3 (#39): one store per tenant. A credential of tenant A can never read,
//! discover, time an error on, or write tenant B's data, and the attempt is
//! audited in A.
mod common;
use axum::http::{Method, StatusCode};
use common::*;
use serde_json::{json, Value};
use weave_engine::{Engine, HostContext};
use weave_service::tenant::{tenant_dir, AUDIT_FILE, STORE_FILE};

/// Tenant B with a graph `secret` holding public and restricted records.
async fn tenant_b(h: &Harness) -> (String, String) {
    let root_b = h.tenant("beta", "root");
    let created = h
        .post("/v1/graphs", &root_b, json!({"graph_id": "secret"}))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    let reply = h
        .write(
            &root_b,
            "secret",
            vec![node("b-public", &[], &[]), node("b-root", &["root"], &[])],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let created = h.post("/v1/graphs", &root_b, json!({})).await;
    (
        root_b,
        created.json["graph_id"].as_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn two_tenants_are_two_separate_databases() {
    let h = Harness::new();
    let (root_a, _alice, _bob) = standard_tenant(&h, "alpha").await;
    let (_root_b, _) = tenant_b(&h).await;
    h.post("/v1/graphs", &root_a, json!({"graph_id": "mine"}))
        .await;
    h.write(&root_a, "mine", vec![node("a-node", &[], &[])])
        .await;

    let a = tenant_dir(h.dir.path(), "alpha");
    let b = tenant_dir(h.dir.path(), "beta");
    assert_ne!(a, b);
    for dir in [&a, &b] {
        assert!(dir.join(STORE_FILE).is_file());
        assert!(dir.join(AUDIT_FILE).is_file());
    }
    // Each store, opened on its own, holds only its own tenant's graphs and
    // registry: the same principal ID `root` is a different principal in each.
    let graphs = |dir: &std::path::Path| -> Vec<String> {
        let engine = Engine::open(dir.join(STORE_FILE)).unwrap();
        engine
            .list_graphs(&HostContext::new("root", Vec::<String>::new()))
            .unwrap()
            .into_iter()
            .map(|g| g.graph_id)
            .collect()
    };
    assert_eq!(graphs(&a), ["mine"]);
    let in_b = graphs(&b);
    assert!(in_b.contains(&"secret".to_string()) && !in_b.contains(&"mine".to_string()));
    let alice_in_b = Engine::open(b.join(STORE_FILE))
        .unwrap()
        .registry_principal("alice", &HostContext::new("root", Vec::<String>::new()));
    assert_eq!(alice_in_b.unwrap_err().code, "E_UNAVAILABLE");
    // The control store holds no graph data.
    let control = rusqlite::Connection::open(h.dir.path().join("control.db")).unwrap();
    let tables: Vec<String> = control
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(tables, ["api_tokens", "control_meta", "tenants"]);
}

/// A response with the graph ID the request named replaced, so responses
/// that echo the requested ID compare equal.
fn same(bytes: &[u8], named: &str, other: &str) -> Vec<u8> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace(named, other)
        .into_bytes()
}

#[tokio::test]
async fn a_tenant_a_credential_cannot_read_discover_time_or_write_tenant_b() {
    let h = Harness::new();
    let (root_a, alice, _bob) = standard_tenant(&h, "alpha").await;
    let (_root_b, b_created) = tenant_b(&h).await;
    let beta = h.service.tenant("beta").unwrap();
    let alpha = h.service.tenant("alpha").unwrap();
    let b_before = beta.checkouts();
    let a_before = alpha.checkouts();

    // The same principal ID in A (root) and the A administrator see nothing of
    // B: B's graph IDs answer exactly like IDs that exist nowhere.
    for token in [&root_a, &alice] {
        for (b_graph, nowhere) in [("secret", "never-made"), (b_created.as_str(), "g-00")] {
            let pairs: Vec<(Value, Value)> = vec![
                (
                    json!([{"op": "query", "query": {"graph_id": b_graph}}]),
                    json!([{"op": "query", "query": {"graph_id": nowhere}}]),
                ),
                (
                    json!([{"op": "evaluate", "value": {"kind": "expand", "graph_id": b_graph,
                        "seeds": ["b-public"], "hops": 1, "limit": 100}}]),
                    json!([{"op": "evaluate", "value": {"kind": "expand", "graph_id": nowhere,
                        "seeds": ["b-public"], "hops": 1, "limit": 100}}]),
                ),
                (
                    json!([{"op": "write", "graph_id": b_graph, "changes": [
                        {"change": "retract", "target": {"kind": "node", "id": "b-public"}}]}]),
                    json!([{"op": "write", "graph_id": nowhere, "changes": [
                        {"change": "retract", "target": {"kind": "node", "id": "b-public"}}]}]),
                ),
            ];
            for (named, absent) in pairs {
                let x = h.execute(token, named.clone()).await;
                let y = h.execute(token, absent).await;
                assert_eq!(x.status, y.status, "{named}");
                assert_eq!(same(&x.bytes, b_graph, nowhere), y.bytes, "{named}");
                assert!(x.status.is_client_error(), "{}", x.json);
            }
            for path in [format!("/v1/graphs/{b_graph}/acl")] {
                let x = h.get(&path, token).await;
                let y = h.get(&format!("/v1/graphs/{nowhere}/acl"), token).await;
                assert_eq!(x.status, y.status);
                assert_eq!(same(&x.bytes, b_graph, nowhere), y.bytes);
            }
        }
        // Listings never mention B.
        let listing = h.get("/v1/graphs", token).await;
        let text = String::from_utf8(listing.bytes).unwrap();
        assert!(!text.contains("secret") && !text.contains(&b_created));
    }
    // A's administrator cannot write into B by choosing B's graph ID either:
    // it creates a new graph `secret` in A.
    let reply = h
        .post("/v1/graphs", &root_a, json!({"graph_id": "secret"}))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let ids = h.ids(&alice, 1).await;
    h.write(&root_a, "secret", vec![node(&ids[0], &[], &[])])
        .await;

    // No request of tenant A touched tenant B's store: the work and timing
    // of every A request are independent of B's data.
    assert_eq!(beta.checkouts(), b_before);
    assert!(alpha.checkouts() > a_before);
    let b_root = h.service.tenant("beta").unwrap();
    let b_nodes = b_root
        .with_state(|state| {
            state
                .engine
                .execute(
                    &serde_json::from_value(json!({"version": weave_contract::VERSION,
                        "commands": [{"op": "query", "query": {"graph_id": "secret"}}]}))
                    .unwrap(),
                    &HostContext::new("root", Vec::<String>::new()),
                )
                .unwrap()
        })
        .unwrap();
    let b_nodes = serde_json::to_value(&b_nodes).unwrap();
    assert_eq!(
        b_nodes[0]["result"]["graph"]["nodes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // The cross-tenant attempts are audited in A, as denials naming the
    // graph IDs the request named, and B's audit log has none of them.
    let entries = h.audit_entries(&root_a).await;
    let denied: Vec<&Value> = entries
        .iter()
        .filter(|e| {
            e["outcome"] == "denied" && e["graphs"].as_array().unwrap().contains(&json!("secret"))
        })
        .collect();
    assert!(denied.len() >= 6, "{entries:?}");
    for entry in &denied {
        assert_eq!(entry["code"], "E_UNAVAILABLE");
        assert_eq!(entry["fact_count"], 0);
    }
    let b_entries = b_root
        .with_state(|s| s.audit.entries(0, 1000).unwrap())
        .unwrap();
    assert!(b_entries
        .iter()
        .all(|e| e.principal == "root" && e.outcome == weave_service::audit::Outcome::Ok));
}

#[tokio::test]
async fn the_tenant_comes_from_the_credential_never_from_the_request() {
    let h = Harness::new();
    let (root_a, _, _) = standard_tenant(&h, "alpha").await;
    let (_root_b, _) = tenant_b(&h).await;
    // There is no tenant parameter anywhere: a header, a query parameter or
    // a body field naming B changes nothing, or is rejected.
    let plain = h.get("/v1/whoami", &root_a).await;
    assert_eq!(plain.json["tenant"], "alpha");
    let reply = h
        .send_raw(
            Method::GET,
            "/v1/graphs?tenant=beta",
            Some(format!("Bearer {root_a}")),
            None,
        )
        .await;
    assert!(!String::from_utf8(reply.bytes).unwrap().contains("secret"));
    let reply = h
        .post(
            "/v1/execute",
            &root_a,
            json!({"version": weave_contract::VERSION, "tenant": "beta",
                "commands": [{"op": "query", "query": {"graph_id": "secret"}}]}),
        )
        .await;
    assert!(reply.status.is_client_error());
    assert_ne!(reply.status, StatusCode::OK);
}
