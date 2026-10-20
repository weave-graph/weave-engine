//! B3 (#39) with B1 agents: an agent token authenticates the agent, never
//! its owner; the agent acts with at most its owner's authority; its writes
//! are audited with `on_behalf_of`; and revoking, suspending, retiring or
//! expiring the agent, or suspending its owner, fails its next request.
mod common;
use axum::http::{Method, StatusCode};
use common::*;
use serde_json::{json, Value};

/// Tenant with alice (in `team`), bob, a graph `notes` alice writes and bob
/// cannot see, and alice's agent `alice-bot` with Write on `notes`,
/// exercising alice's identities. Returns (root, alice, bob, bot) tokens.
async fn setup(h: &Harness, expires_at_ms: Option<i64>) -> (String, String, String, String) {
    let (root, alice, bob) = standard_tenant(h, "acme").await;
    h.post("/v1/graphs", &root, json!({"graph_id": "notes"}))
        .await;
    h.registry(
        &root,
        json!([{"change": "grant", "graph_id": "notes", "subject": "alice", "permission": "write"}]),
    )
    .await;
    let mut delegation = json!({
        "scopes": [{"graph_id": "notes", "permission": "write"}],
        "exercise": {"kind": "owner_all"},
    });
    if let Some(expiry) = expires_at_ms {
        delegation["expires_at_ms"] = json!(expiry);
    }
    let reply = h
        .registry(
            &alice,
            json!([{"change": "add_agent", "id": "alice-bot", "owner": "alice",
                "delegation": delegation}]),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    // The owner issues its agent's token.
    let reply = h
        .post("/v1/tokens", &alice, json!({"principal": "alice-bot"}))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.json);
    let bot = reply.json["token"].as_str().unwrap().to_string();
    (root, alice, bob, bot)
}

#[tokio::test]
async fn an_agent_token_authenticates_the_agent_with_its_owners_ceiling() {
    let h = Harness::new();
    let (root, alice, _bob, bot) = setup(&h, None).await;
    let me = h.get("/v1/whoami", &bot).await;
    assert_eq!(me.json["principal"], "alice-bot");
    assert_eq!(me.json["agent"], true);
    assert_eq!(me.json["administrator"], false);
    assert_eq!(me.json["on_behalf_of"], json!(["alice"]));

    // The agent writes with minted IDs, and the write is audited as the
    // agent on behalf of alice; the fact log agrees.
    let ids = h.ids(&bot, 1).await;
    let reply = h.write(&bot, "notes", vec![node(&ids[0], &[], &[])]).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let read = h.query(&alice, "notes").await;
    let provenance = &read.json["results"][0]["result"]["fact_provenance"][0]["provenance"];
    assert_eq!(provenance["writer"], "alice-bot");
    assert_eq!(provenance["on_behalf_of"], json!(["alice"]));
    let accesses = h.audit_facts(&root, "notes", &ids[0]).await;
    let write: &Value = accesses.iter().find(|a| a["access"] == "write").unwrap();
    assert_eq!(write["principal"], "alice-bot");
    assert_eq!(write["on_behalf_of"], json!(["alice"]));
    let readers: Vec<&str> = accesses
        .iter()
        .filter(|a| a["access"] == "read")
        .map(|a| a["principal"].as_str().unwrap())
        .collect();
    assert_eq!(readers, ["alice"]);

    // Never more than its owner: no Administer, no graph creation, no
    // tokens, no audit log, no registry administration.
    let created = h.post("/v1/graphs", &bot, json!({})).await;
    assert_eq!(created.code(), "E_FORBIDDEN", "{}", created.json);
    let reply = h
        .post("/v1/tokens", &bot, json!({"principal": "alice-bot"}))
        .await;
    assert_eq!(reply.code(), "E_FORBIDDEN");
    assert_eq!(h.get("/v1/audit", &bot).await.code(), "E_FORBIDDEN");
    let reply = h
        .registry(
            &bot,
            json!([{"change": "add_member", "group": "team", "member": "alice-bot"}]),
        )
        .await;
    assert!(reply.status.is_client_error(), "{}", reply.json);
    // A graph the agent's scope does not list does not exist for it.
    h.post("/v1/graphs", &root, json!({"graph_id": "other"}))
        .await;
    h.registry(
        &root,
        json!([{"change": "grant", "graph_id": "other", "subject": "alice", "permission": "write"}]),
    )
    .await;
    let hidden = h.query(&bot, "other").await;
    let missing = h.query(&bot, "never-made").await;
    assert_eq!(
        (hidden.status, hidden.bytes),
        (missing.status, missing.bytes)
    );
    // When the owner loses Write, so does the agent, on its next request.
    h.registry(
        &root,
        json!([{"change": "grant", "graph_id": "notes", "subject": "alice", "permission": "read"}]),
    )
    .await;
    let ids = h.ids(&bot, 1).await;
    let reply = h.write(&bot, "notes", vec![node(&ids[0], &[], &[])]).await;
    assert_eq!(reply.code(), "E_FORBIDDEN", "{}", reply.json);
}

#[tokio::test]
async fn only_tenant_administrators_and_owners_issue_and_revoke_agent_tokens() {
    let h = Harness::new();
    let (root, alice, bob, bot) = setup(&h, None).await;
    // Bob does not own the agent: the same refusal as for a principal that
    // does not exist.
    let theirs = h
        .post("/v1/tokens", &bob, json!({"principal": "alice-bot"}))
        .await;
    let nobody = h
        .post("/v1/tokens", &bob, json!({"principal": "no-bot"}))
        .await;
    assert_eq!(theirs.status, StatusCode::FORBIDDEN);
    assert_eq!(theirs.bytes, nobody.bytes);
    // An owner cannot issue tokens for itself or other users.
    let reply = h
        .post("/v1/tokens", &alice, json!({"principal": "bob"}))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    // A tenant administrator can.
    let reply = h
        .post("/v1/tokens", &root, json!({"principal": "alice-bot"}))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    // Revocation: bob cannot revoke the agent's token, the owner can.
    let id = h.get("/v1/whoami", &bot).await.json["credential"]
        .as_str()
        .unwrap()
        .to_string();
    let reply = h
        .send(
            Method::DELETE,
            &format!("/v1/tokens/{id}"),
            Some(&bob),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(h.get("/v1/whoami", &bot).await.status, StatusCode::OK);
    let reply = h
        .send(
            Method::DELETE,
            &format!("/v1/tokens/{id}"),
            Some(&alice),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    assert_eq!(
        h.get("/v1/whoami", &bot).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_revoked_or_suspended_agent_fails_its_next_request() {
    let h = Harness::new();
    let (root, alice, _bob, bot) = setup(&h, None).await;
    let ok = |reply: &Reply| reply.status == StatusCode::OK;
    assert!(ok(&h.get("/v1/whoami", &bot).await));

    // The owner suspends its agent: the next request fails, uniformly.
    h.registry(
        &alice,
        json!([{"change": "set_status", "id": "alice-bot", "status": "suspended"}]),
    )
    .await;
    let denied = h.query(&bot, "notes").await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
    assert_eq!(denied.code(), "E_UNAUTHENTICATED");
    h.registry(
        &alice,
        json!([{"change": "set_status", "id": "alice-bot", "status": "active"}]),
    )
    .await;
    assert!(ok(&h.get("/v1/whoami", &bot).await));

    // Suspending the owner suspends the agent.
    h.registry(
        &root,
        json!([{"change": "set_status", "id": "alice", "status": "suspended"}]),
    )
    .await;
    assert_eq!(
        h.get("/v1/whoami", &bot).await.status,
        StatusCode::UNAUTHORIZED
    );
    h.registry(
        &root,
        json!([{"change": "set_status", "id": "alice", "status": "active"}]),
    )
    .await;
    assert!(ok(&h.get("/v1/whoami", &bot).await));

    // The agent's denial is audited in its tenant.
    let entries = h.audit_entries(&root).await;
    assert!(entries.iter().any(|e| e["principal"] == "alice-bot"
        && e["outcome"] == "denied"
        && e["code"] == "E_UNAUTHENTICATED"));

    // Retirement is final.
    h.registry(
        &alice,
        json!([{"change": "set_status", "id": "alice-bot", "status": "retired"}]),
    )
    .await;
    assert_eq!(
        h.get("/v1/whoami", &bot).await.status,
        StatusCode::UNAUTHORIZED
    );
    let reply = h
        .post("/v1/tokens", &root, json!({"principal": "alice-bot"}))
        .await;
    assert_eq!(reply.code(), "E_UNAVAILABLE");
}

#[tokio::test]
async fn an_expired_delegation_fails_the_agents_next_request() {
    let h = Harness::new();
    let expiry = START_MS + 1_000_000;
    let (_root, _alice, _bob, bot) = setup(&h, Some(expiry)).await;
    assert_eq!(h.get("/v1/whoami", &bot).await.status, StatusCode::OK);
    h.clock.set(expiry - 100);
    assert_eq!(h.get("/v1/whoami", &bot).await.status, StatusCode::OK);
    h.clock.set(expiry);
    assert_eq!(
        h.get("/v1/whoami", &bot).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_sub_agent_authenticates_with_its_whole_chain() {
    let h = Harness::new();
    let (root, alice, _bob, bot) = setup(&h, None).await;
    // Give alice-bot the right to delegate, then let it register a sub-agent.
    let reply = h
        .registry(
            &alice,
            json!([{"change": "set_delegation", "agent": "alice-bot", "delegation": {
                "scopes": [{"graph_id": "notes", "permission": "write"}],
                "exercise": {"kind": "owner_all"}, "may_delegate": true}}]),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let reply = h
        .registry(
            &bot,
            json!([{"change": "add_agent", "id": "sub-bot", "owner": "alice-bot", "delegation": {
                "scopes": [{"graph_id": "notes", "permission": "read"}]}}]),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let sub = h
        .post("/v1/tokens", &alice, json!({"principal": "sub-bot"}))
        .await;
    assert_eq!(sub.status, StatusCode::CREATED, "{}", sub.json);
    let sub = sub.json["token"].as_str().unwrap().to_string();
    let me = h.get("/v1/whoami", &sub).await;
    assert_eq!(me.json["on_behalf_of"], json!(["alice-bot", "alice"]));
    // Suspending the middle agent cuts the sub-agent off on its next request.
    h.registry(
        &root,
        json!([{"change": "set_status", "id": "alice-bot", "status": "suspended"}]),
    )
    .await;
    assert_eq!(
        h.get("/v1/whoami", &sub).await.status,
        StatusCode::UNAUTHORIZED
    );
}
