//! B3 (#39): the audit log answers "who read fact F" and "who wrote fact F",
//! covers every authenticated request, is append-only and hash-chained, and
//! is readable only by tenant auditors (administrators).
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use weave_service::tenant::{tenant_dir, AUDIT_FILE};

fn principals(accesses: &[Value], access: &str) -> Vec<String> {
    accesses
        .iter()
        .filter(|a| a["access"] == access)
        .map(|a| a["principal"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn the_audit_log_answers_who_read_and_who_wrote_a_fact() {
    let h = Harness::new();
    let (root, alice, bob) = standard_tenant(&h, "acme").await;
    let graph = h.post("/v1/graphs", &alice, json!({})).await.json["graph_id"]
        .as_str()
        .unwrap()
        .to_string();
    h.registry(
        &alice,
        json!([{"change": "grant", "graph_id": graph, "subject": "bob", "permission": "read"}]),
    )
    .await;
    let ids = h.ids(&alice, 3).await;
    let (shared, private, link) = (&ids[0], &ids[1], &ids[2]);
    let wrote = h
        .write(
            &alice,
            &graph,
            vec![
                node(shared, &[], &[]),
                node(private, &["alice"], &[]),
                edge(link, shared, private, &["alice"]),
            ],
        )
        .await;
    assert_eq!(wrote.status, StatusCode::OK, "{}", wrote.json);
    let revision = wrote.json["results"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();

    // Alice reads everything; Bob reads only the public node; Bob also
    // reads through an expand, which cites the facts it returns.
    assert_eq!(node_ids(&h.query(&alice, &graph).await).len(), 2);
    assert_eq!(
        node_ids(&h.query(&bob, &graph).await),
        std::slice::from_ref(shared)
    );
    let expanded = h
        .execute(
            &bob,
            json!([{"op": "evaluate", "value": {"kind": "expand", "graph_id": graph,
                "seeds": [shared], "hops": 1, "limit": 100}}]),
        )
        .await;
    assert_eq!(expanded.status, StatusCode::OK, "{}", expanded.json);

    // Alice corrects the shared node; Bob reads it again.
    let mut corrected = node(shared, &[], &[]);
    corrected["properties"]["label"] = json!("v2");
    let reply = h
        .execute(
            &alice,
            json!([{"op": "write", "graph_id": graph, "changes": [{"change": "correct", "fact": corrected}]}]),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    h.query(&bob, &graph).await;

    // Who wrote / who read the shared node.
    let accesses = h.audit_facts(&root, &graph, shared).await;
    assert_eq!(principals(&accesses, "write"), ["alice", "alice"]);
    let changes: Vec<&str> = accesses
        .iter()
        .filter(|a| a["access"] == "write")
        .map(|a| a["change"].as_str().unwrap())
        .collect();
    assert_eq!(changes, ["assert", "correct"]);
    assert_eq!(accesses[0]["revision"], revision.as_str());
    assert_eq!(
        principals(&accesses, "read"),
        ["alice", "bob", "bob", "bob"]
    );
    let operations: Vec<&str> = accesses
        .iter()
        .map(|a| a["operation"].as_str().unwrap())
        .collect();
    assert!(operations.iter().all(|o| *o == "execute"));

    // The private node and the edge were read by Alice only: Bob's reads
    // never cite a fact he could not see.
    let accesses = h.audit_facts(&root, &graph, private).await;
    assert_eq!(principals(&accesses, "write"), ["alice"]);
    assert_eq!(principals(&accesses, "read"), ["alice"]);
    let accesses = h.audit_facts(&root, &graph, link).await;
    assert_eq!(principals(&accesses, "read"), ["alice"]);

    // Filtered by access kind.
    let reply = h
        .get(
            &format!("/v1/audit/facts?graph_id={graph}&fact_id={shared}&access=write"),
            &root,
        )
        .await;
    assert_eq!(reply.json["accesses"].as_array().unwrap().len(), 2);

    // Retraction is a write.
    let reply = h
        .execute(
            &alice,
            json!([{"op": "write", "graph_id": graph, "changes": [
                {"change": "retract", "target": {"kind": "edge", "id": link}}]}]),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let accesses = h.audit_facts(&root, &graph, link).await;
    assert_eq!(
        accesses.last().unwrap()["change"],
        "retract",
        "{accesses:?}"
    );
}

#[tokio::test]
async fn every_authenticated_request_is_audited_and_the_log_is_append_only() {
    let h = Harness::new();
    let (root, alice, bob) = standard_tenant(&h, "acme").await;
    let before = h.audit_entries(&root).await.len();
    let graph = h.post("/v1/graphs", &alice, json!({})).await.json["graph_id"]
        .as_str()
        .unwrap()
        .to_string();
    let ids = h.ids(&alice, 1).await;
    h.write(&alice, &graph, vec![node(&ids[0], &[], &[])]).await;
    h.query(&alice, &graph).await;
    h.query(&bob, &graph).await; // denied: bob cannot discover it
    h.get("/v1/graphs", &bob).await;
    h.get("/v1/whoami", &bob).await;
    h.get("/v1/audit", &bob).await; // denied: not an auditor
    h.get("/v1/health", &bob).await; // the only unaudited route
    h.execute(&bob, json!([{"op": "commit", "graph_id": "x", "data": {}}]))
        .await; // unsupported command
    let entries = h.audit_entries(&root).await;
    let new: Vec<&Value> = entries[before..]
        .iter()
        .filter(|e| e["principal"] != "root")
        .collect();
    let summary: Vec<(String, String, String)> = new
        .iter()
        .map(|e| {
            (
                e["principal"].as_str().unwrap().into(),
                e["operation"].as_str().unwrap().into(),
                e["outcome"].as_str().unwrap().into(),
            )
        })
        .collect();
    let expect = |p: &str, o: &str, r: &str| (p.to_string(), o.to_string(), r.to_string());
    assert_eq!(
        summary,
        [
            expect("alice", "graphs.create", "ok"),
            expect("alice", "ids.mint", "ok"),
            expect("alice", "execute", "ok"),
            expect("alice", "execute", "ok"),
            expect("bob", "execute", "denied"),
            expect("bob", "graphs.list", "ok"),
            expect("bob", "whoami", "ok"),
            expect("bob", "audit.entries", "denied"),
            expect("bob", "execute", "error"),
        ]
    );
    // Bob's denied query records the graph he named and the code, nothing
    // else; every entry carries the digest of the exact response body.
    assert_eq!(new[4]["code"], "E_UNAVAILABLE");
    assert_eq!(new[4]["graphs"], json!([graph]));
    assert_eq!(new[4]["fact_count"], 0);
    for entry in &entries {
        assert!(entry["result_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        assert!(entry["credential"].as_str().unwrap().starts_with("tok-"));
    }
    // The response digest is of the exact body the client received.
    let reply = h.get("/v1/whoami", &bob).await;
    let last = h.audit_entries(&root).await;
    let whoami = last
        .iter()
        .rev()
        .find(|e| e["operation"] == "whoami")
        .unwrap();
    assert_eq!(
        whoami["result_digest"].as_str().unwrap(),
        format!("sha256:{}", weave_service::ids::sha256_hex(&reply.bytes))
    );

    // Append-only: SQL updates and deletes are rejected, and the hash chain
    // verifies.
    let tenant = h.service.tenant("acme").unwrap();
    let checked = tenant
        .with_state(|s| s.audit.verify_chain().unwrap())
        .unwrap();
    // The listing that produced `last` is itself the latest entry.
    assert_eq!(checked, Ok(last.len() as i64 + 1));
    let raw =
        rusqlite::Connection::open(tenant_dir(h.dir.path(), "acme").join(AUDIT_FILE)).unwrap();
    for sql in [
        "UPDATE audit_entries SET principal='mallory'",
        "DELETE FROM audit_entries",
        "UPDATE audit_facts SET fact_id='x'",
        "DELETE FROM audit_facts",
    ] {
        let error = raw.execute(sql, []).unwrap_err().to_string();
        assert!(error.contains("append-only"), "{sql}: {error}");
    }
}

#[tokio::test]
async fn a_tampered_audit_entry_breaks_the_chain() {
    let h = Harness::new();
    let (root, _alice, bob) = standard_tenant(&h, "acme").await;
    h.get("/v1/whoami", &bob).await;
    h.get("/v1/whoami", &root).await;
    let tenant = h.service.tenant("acme").unwrap();
    let path = tenant_dir(h.dir.path(), "acme").join(AUDIT_FILE);
    let raw = rusqlite::Connection::open(&path).unwrap();
    // Someone with file access drops the trigger and rewrites history.
    raw.execute_batch(
        "DROP TRIGGER audit_entries_no_update;
         UPDATE audit_entries SET principal='mallory' WHERE operation='whoami' AND principal='bob';",
    )
    .unwrap();
    let verdict = tenant
        .with_state(|s| s.audit.verify_chain().unwrap())
        .unwrap();
    assert!(verdict.is_err());
}

#[tokio::test]
async fn only_tenant_administrators_read_the_audit_log() {
    let h = Harness::new();
    let (root, alice, _bob) = standard_tenant(&h, "acme").await;
    for path in ["/v1/audit", "/v1/audit/facts?graph_id=g&fact_id=f"] {
        let reply = h.get(path, &alice).await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::FORBIDDEN, "E_FORBIDDEN")
        );
        assert_eq!(h.get(path, &root).await.status, StatusCode::OK);
    }
    for path in [
        "/v1/audit?limit=0",
        "/v1/audit?limit=1001",
        "/v1/audit/facts?graph_id=g&fact_id=f&limit=5000",
    ] {
        assert_eq!(h.get(path, &root).await.code(), "E_BUDGET", "{path}");
    }
    for path in [
        "/v1/audit?bogus=1",
        "/v1/audit/facts?graph_id=g",
        "/v1/audit/facts?graph_id=g&fact_id=f&access=delete",
    ] {
        assert_eq!(h.get(path, &root).await.code(), "E_INPUT", "{path}");
    }
}
