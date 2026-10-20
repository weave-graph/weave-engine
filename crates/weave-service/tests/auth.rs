//! B3 (#39): authentication. Every endpoint rejects unauthenticated
//! requests with one uniform error; revocation, expiry and registry
//! suspension take effect on the next request; secrets never reach the log
//! or disk in clear.
mod common;
use axum::http::{Method, StatusCode};
use common::*;
use serde_json::json;
use weave_service::error::UNAUTHENTICATED_MESSAGE;
use weave_service::{Service, CONTROL_FILE};

fn concrete(path: &str) -> String {
    path.replace("{graph_id}", "g")
        .replace("{id}", "alice")
        .replace("{token_id}", "tok-0")
}

/// Every route the router serves, plus routes it does not serve.
fn every_path() -> Vec<(Method, String)> {
    let mut out: Vec<(Method, String)> = Service::endpoints()
        .into_iter()
        .map(|(m, p)| (m, concrete(p)))
        .collect();
    out.push((Method::GET, "/".into()));
    out.push((Method::GET, "/v1/nope".into()));
    out.push((Method::PUT, "/v1/execute".into()));
    out.push((Method::POST, "/v2/execute".into()));
    out
}

#[test]
fn the_endpoint_table_is_the_documented_api() {
    let mut endpoints: Vec<String> = Service::endpoints()
        .into_iter()
        .map(|(m, p)| format!("{m} {p}"))
        .collect();
    endpoints.sort();
    assert_eq!(
        endpoints,
        [
            "DELETE /v1/tokens/{token_id}",
            "GET /v1/audit",
            "GET /v1/audit/facts",
            "GET /v1/graphs",
            "GET /v1/graphs/{graph_id}/acl",
            "GET /v1/health",
            "GET /v1/manifest",
            "GET /v1/registry/groups/{id}/members",
            "GET /v1/registry/principals/{id}",
            "GET /v1/registry/principals/{id}/memberships",
            "GET /v1/tokens",
            "GET /v1/whoami",
            "POST /v1/execute",
            "POST /v1/graphs",
            "POST /v1/ids",
            "POST /v1/registry",
            "POST /v1/tokens",
        ]
    );
    // The service guide documents every endpoint.
    let guide = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/SERVICE.md"
    ))
    .unwrap();
    for endpoint in &endpoints {
        assert!(
            guide.contains(&format!("`{endpoint}`")),
            "{endpoint} undocumented"
        );
    }
}

#[tokio::test]
async fn every_endpoint_rejects_unauthenticated_requests_with_one_uniform_error() {
    let h = Harness::new();
    let root = h.tenant("acme", "root");
    let expired = h
        .service
        .issue_token("acme", "root", "test", 60_000, None)
        .unwrap()
        .token;
    let revoked = h
        .service
        .issue_token("acme", "root", "test", 60_000, None)
        .unwrap();
    assert!(h.service.revoke_token(&revoked.token_id).unwrap());
    h.advance(61_000);
    // A token that has the right shape but was never issued.
    let unknown = format!("wvs_{}", "ab".repeat(32));
    let wrong_case = root.to_uppercase();
    let credentials: Vec<Option<String>> = vec![
        None,
        Some(String::new()),
        Some("Bearer".into()),
        Some("Bearer ".into()),
        Some(format!("Basic {root}")),
        Some(format!("Bearer {unknown}")),
        Some(format!("Bearer {expired}")),
        Some(format!("Bearer {}", revoked.token)),
        Some(format!("Bearer {wrong_case}")),
        Some(format!("Bearer {root}x")),
        Some(format!("Bearer {}", &root[..root.len() - 1])),
        Some(format!("Bearer {}", "x".repeat(10_000))),
    ];
    let mut first: Option<Vec<u8>> = None;
    for (method, path) in every_path() {
        for credential in &credentials {
            let body = (method == Method::POST).then(|| b"{}".to_vec());
            let reply = h
                .send_raw(method.clone(), &path, credential.clone(), body)
                .await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method} {path}");
            assert_eq!(reply.headers["www-authenticate"], "Bearer");
            assert_eq!(reply.code(), "E_UNAUTHENTICATED");
            assert_eq!(reply.json["error"]["message"], UNAUTHENTICATED_MESSAGE);
            // Byte-identical for every endpoint and every failure.
            match &first {
                None => first = Some(reply.bytes.clone()),
                Some(bytes) => assert_eq!(bytes, &reply.bytes, "{method} {path}"),
            }
        }
    }
    // Two Authorization headers are no credential either.
    let reply = {
        use axum::body::Body;
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let request = axum::http::Request::builder()
            .uri("/v1/health")
            .header("authorization", format!("Bearer {root}"))
            .header("authorization", format!("Bearer {root}"))
            .body(Body::empty())
            .unwrap();
        let response = h.service.router().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, bytes.to_vec())
    };
    assert_eq!(reply.0, StatusCode::UNAUTHORIZED);
    assert_eq!(Some(reply.1), first);
    // The valid token reaches every endpoint (any status but 401).
    for (method, path) in every_path() {
        let body = (method == Method::POST).then(|| b"{}".to_vec());
        let reply = h
            .send_raw(method.clone(), &path, Some(format!("Bearer {root}")), body)
            .await;
        assert_ne!(reply.status, StatusCode::UNAUTHORIZED, "{method} {path}");
    }
    // Unknown routes and methods are reported only after authentication.
    let reply = h.send(Method::GET, "/v1/nope", Some(&root), None).await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::NOT_FOUND, "E_NOT_FOUND")
    );
    let reply = h.send(Method::PUT, "/v1/execute", Some(&root), None).await;
    assert_eq!(reply.code(), "E_METHOD");
}

#[tokio::test]
async fn revocation_and_expiry_take_effect_on_the_next_request() {
    let h = Harness::new();
    let (root, alice, _bob) = standard_tenant(&h, "acme").await;
    assert_eq!(h.get("/v1/whoami", &alice).await.status, StatusCode::OK);

    // Revoking through the API: the next request fails.
    let tokens = h.get("/v1/tokens", &alice).await;
    let alice_token_id = tokens.json["tokens"][0]["token_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(tokens.json["tokens"].as_array().unwrap().len(), 1);
    let reply = h
        .send(
            Method::DELETE,
            &format!("/v1/tokens/{alice_token_id}"),
            Some(&root),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    assert_eq!(
        h.get("/v1/whoami", &alice).await.status,
        StatusCode::UNAUTHORIZED
    );

    // Self-revocation.
    let alice = h.token_for(&root, "alice").await;
    let id = h.get("/v1/whoami", &alice).await.json["credential"]
        .as_str()
        .unwrap()
        .to_string();
    let reply = h
        .send(
            Method::DELETE,
            &format!("/v1/tokens/{id}"),
            Some(&alice),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        h.get("/v1/whoami", &alice).await.status,
        StatusCode::UNAUTHORIZED
    );

    // Revocation through the operator CLI path, on a running service.
    let alice = h.token_for(&root, "alice").await;
    let id = h.get("/v1/whoami", &alice).await.json["credential"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(h.service.revoke_token(&id).unwrap());
    assert_eq!(
        h.get("/v1/whoami", &alice).await.status,
        StatusCode::UNAUTHORIZED
    );

    // Expiry: valid until the trusted clock reaches expires_at_ms.
    let reply = h
        .post(
            "/v1/tokens",
            &root,
            json!({"principal": "alice", "expires_in_ms": 120_000}),
        )
        .await;
    let short = reply.json["token"].as_str().unwrap().to_string();
    let expires = reply.json["expires_at_ms"].as_i64().unwrap();
    assert_eq!(h.get("/v1/whoami", &short).await.status, StatusCode::OK);
    h.clock.set(expires - 1_000);
    assert_eq!(h.get("/v1/whoami", &short).await.status, StatusCode::OK);
    h.clock.set(expires);
    assert_eq!(
        h.get("/v1/whoami", &short).await.status,
        StatusCode::UNAUTHORIZED
    );

    // Registry suspension of the principal: its live token stops working on
    // the next request, and works again after reactivation.
    let alice = h.token_for(&root, "alice").await;
    assert_eq!(h.get("/v1/whoami", &alice).await.status, StatusCode::OK);
    let reply = h
        .registry(
            &root,
            json!([{"change": "set_status", "id": "alice", "status": "suspended"}]),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let denied = h.get("/v1/whoami", &alice).await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
    assert_eq!(denied.json["error"]["message"], UNAUTHENTICATED_MESSAGE);
    h.registry(
        &root,
        json!([{"change": "set_status", "id": "alice", "status": "active"}]),
    )
    .await;
    assert_eq!(h.get("/v1/whoami", &alice).await.status, StatusCode::OK);

    // A non-administrator cannot revoke someone else's token, and cannot
    // tell it from a token that does not exist.
    let bob_reply = h
        .send(
            Method::DELETE,
            &format!("/v1/tokens/{alice_token_id}"),
            Some(&h.token_for(&root, "bob").await),
            None,
        )
        .await;
    let missing = h
        .send(
            Method::DELETE,
            "/v1/tokens/tok-missing",
            Some(&h.token_for(&root, "bob").await),
            None,
        )
        .await;
    assert_eq!(bob_reply.status, StatusCode::NOT_FOUND);
    assert_eq!(bob_reply.bytes, missing.bytes);
}

#[tokio::test]
async fn token_issuance_is_for_administrators_and_active_users_only() {
    let h = Harness::new();
    let (root, alice, _) = standard_tenant(&h, "acme").await;
    let reply = h
        .post("/v1/tokens", &alice, json!({"principal": "alice"}))
        .await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::FORBIDDEN, "E_FORBIDDEN")
    );
    for principal in ["team", "nobody", "weave:administrators"] {
        let reply = h
            .post("/v1/tokens", &root, json!({"principal": principal}))
            .await;
        assert_eq!(reply.code(), "E_UNAVAILABLE", "{principal}");
    }
    for ttl in [0i64, 59_999, 365 * 24 * 3600 * 1000 + 1] {
        let reply = h
            .post(
                "/v1/tokens",
                &root,
                json!({"principal": "alice", "expires_in_ms": ttl}),
            )
            .await;
        assert_eq!(reply.code(), "E_INPUT", "{ttl}");
    }
    // Listings never carry the secret or its digest.
    let listing = h.get("/v1/tokens", &root).await;
    let text = String::from_utf8(listing.bytes.clone()).unwrap();
    assert!(!text.contains("wvs_") && !text.contains("sha256"));
    assert!(!text.contains(&alice));
}

#[tokio::test]
async fn tokens_never_reach_the_log_or_disk_in_clear() {
    let h = Harness::new();
    let (root, alice, bob) = standard_tenant(&h, "acme").await;
    let unknown = format!("wvs_{}", "cd".repeat(32));
    let graph = h.post("/v1/graphs", &alice, json!({})).await.json["graph_id"]
        .as_str()
        .unwrap()
        .to_string();
    let ids = h.ids(&alice, 1).await;
    h.write(&alice, &graph, vec![node(&ids[0], &[], &[])]).await;
    h.query(&alice, &graph).await;
    h.get("/v1/whoami", &unknown).await;
    h.send(Method::GET, "/v1/nope", Some(&bob), None).await;
    // A token inside a request body, a query string and a path is not
    // logged either.
    h.post(
        "/v1/registry",
        &root,
        json!({"changes": [], "token": alice}),
    )
    .await;
    h.get(&format!("/v1/audit?after={alice}"), &root).await;
    h.get(&format!("/v1/registry/principals/{alice}"), &root)
        .await;
    let log = h.log.contents();
    assert!(log.lines().count() >= 9, "{log}");
    for line in log.lines() {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(value["event"], "request");
    }
    let mut stored = Vec::new();
    for entry in walk(h.dir.path()) {
        stored.push((entry.clone(), std::fs::read(&entry).unwrap()));
    }
    for secret in [&root, &alice, &bob, &unknown] {
        assert!(!log.contains(secret.as_str()), "token in log");
        let hex = &secret[4..];
        assert!(!log.contains(hex), "token hex in log");
        for (path, bytes) in &stored {
            let found = bytes.windows(hex.len()).any(|w| w == hex.as_bytes());
            assert!(!found, "token in {}", path.display());
        }
    }
    // Only the digest is stored in the control store.
    let control = rusqlite::Connection::open(h.dir.path().join(CONTROL_FILE)).unwrap();
    let digest: String = control
        .query_row(
            "SELECT token_sha256 FROM api_tokens WHERE principal='alice'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(digest, weave_service::ids::sha256_hex(alice.as_bytes()));
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}
