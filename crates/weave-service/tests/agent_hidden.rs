//! B4 (#40), acceptance criterion 2 and the paired hidden-input tests of
//! every tool: agent Z of another team, which reads the shared graph but
//! not team A's records, and agent W, which has no access to the graph,
//! get the same answers whether or not agent X's team-A facts exist.
mod common;
mod mcp_common;
use axum::http::StatusCode;
use common::*;
use mcp_common::*;
use serde_json::{json, Value};

const OBSERVE_AT: i64 = START_MS + 10_000_000;

struct World {
    h: Harness,
    t: Teams,
    /// The IDs X minted (from the hidden world; reused verbatim in the
    /// plain world, where nothing holds them).
    hidden: Vec<String>,
}

/// The shared tenant. With `hidden`, agent X also asserted team-A facts:
/// a finding node about the public unit, its edge and embedding, an edge
/// from the unit to it, an attachment on the unit and a second embedding
/// on the public note.
async fn world(hidden: bool, ids: &[String]) -> World {
    let h = Harness::new();
    let t = teams(&h).await;
    let mut minted_ids = ids.to_vec();
    if hidden {
        let asserted = ok(&h, &t.x, "assert_fact", finding()).await;
        let node = minted(&asserted, 0);
        let more = ok(
            &h,
            &t.x,
            "assert_fact",
            json!({"graph_id": GRAPH,
            "provenance": {"method": {"kind": "model", "model": MODEL, "version": "2026-10"}},
            "visibility": {"reader_groups": ["team-a"]},
            "facts": [
                {"kind": "edge", "predicate": "maint:has_finding", "from": "unit-7", "to": node,
                 "valid_time": {"start": 0}, "properties": {"secret": "team A only"}},
                {"kind": "attachment", "host": {"kind": "node", "id": "unit-7"},
                 "key": "maint:status", "value": "under review", "valid_time": {"start": 0}},
                {"kind": "embedding", "space": {"id": SPACE, "revision": "1"},
                 "subject": {"kind": "node", "id": "note-1"}, "vector": [0.0, 0.9, 0.1],
                 "valid_time": {"start": 0}},
            ]}),
        )
        .await;
        minted_ids = (0..3)
            .map(|i| minted(&asserted, i))
            .chain((0..3).map(|i| minted(&more, i)))
            .collect();
    }
    h.clock.set(OBSERVE_AT);
    World {
        h,
        t,
        hidden: minted_ids,
    }
}

/// Every tool, called with arguments that touch the hidden facts' region
/// and name their IDs.
fn observations(ids: &[String]) -> Vec<(&'static str, Value)> {
    let (node, edge, embedding) = (&ids[0], &ids[1], &ids[2]);
    let space = json!({"id": SPACE, "revision": "1"});
    let search = nearest(json!([1.0, 0.0, 0.0]));
    vec![
        ("describe_schema", json!({})),
        ("describe_schema", json!({"graph_id": GRAPH})),
        ("query", json!({"graph_id": GRAPH})),
        (
            "query",
            json!({"graph_id": GRAPH, "include_metadata": true}),
        ),
        ("query", json!({"graph_id": GRAPH, "to": "unit-7"})),
        ("query", json!({"graph_id": GRAPH, "from": node})),
        (
            "expand",
            json!({"graph_id": GRAPH, "seeds": ["unit-7"], "hops": 2, "direction": "both"}),
        ),
        (
            "expand",
            json!({"graph_id": GRAPH, "seeds": ["unit-7", node], "hops": 1}),
        ),
        (
            "as_of",
            json!({"unix_millis": OBSERVE_AT, "query": {"graph_id": GRAPH}}),
        ),
        (
            "what_changed",
            json!({"graph_id": GRAPH, "from": {"kind": "as_of", "unix_millis": 0}}),
        ),
        (
            "what_changed",
            json!({"graph_id": GRAPH, "from": {"kind": "as_of", "unix_millis": 0},
            "entities": ["unit-7", node]}),
        ),
        ("hybrid_search", search.clone()),
        (
            "hybrid_search",
            json!({"kind": "nearest", "graph_id": GRAPH, "space": space, "k": 3,
            "query": {"kind": "embedding", "embedding_id": embedding}}),
        ),
        (
            "hybrid_search",
            json!({"kind": "nearest", "graph_id": GRAPH, "space": space, "k": 3,
            "query": {"kind": "vector", "vector": [0.0, 1.0, 0.0]},
            "candidates": {"kind": "expand", "graph_id": GRAPH, "seeds": ["unit-7"], "hops": 2,
                "direction": "both"}}),
        ),
        (
            "hybrid_search",
            json!({"kind": "similar_candidates", "graph_id": GRAPH, "space": space,
            "k": 2}),
        ),
        (
            "hybrid_search",
            json!({"kind": "similar_candidates", "graph_id": GRAPH, "space": space,
            "k": 2, "exclude_linked": true}),
        ),
        (
            "explain",
            json!({"input": {"kind": "query", "query": {"graph_id": GRAPH}}}),
        ),
        (
            "explain",
            json!({"input": {"kind": "expand", "graph_id": GRAPH, "hops": 1,
            "seeds_from": search}}),
        ),
        // Writes that name hidden facts behave as if they did not exist.
        (
            "retract_fact",
            json!({"graph_id": GRAPH, "facts": [{"kind": "edge", "id": edge}]}),
        ),
        (
            "retract_fact",
            json!({"graph_id": GRAPH, "facts": [{"kind": "node", "id": node}]}),
        ),
        (
            "retract_fact",
            json!({"graph_id": GRAPH, "facts": [{"kind": "embedding", "id": embedding}]}),
        ),
        (
            "assert_fact",
            json!({"graph_id": GRAPH,
            "provenance": {"method": {"kind": "model", "model": "z", "version": "1"}},
            "visibility": {"reader_groups": ["team-a"]},
            "facts": [{"kind": "node", "space_id": "z"}]}),
        ),
        (
            "assert_fact",
            json!({"graph_id": GRAPH,
            "provenance": {"method": {"kind": "model", "model": "z", "version": "1"}},
            "visibility": {"reader_groups": ["team-b"]},
            "facts": [{"kind": "node", "ref": "n", "space_id": "z", "properties": {"t": "note"}},
                {"kind": "edge", "predicate": "maint:about", "from": {"ref": "n"}, "to": "unit-7",
                 "valid_time": {"start": 0}},
                {"kind": "embedding", "space": space, "subject": {"kind": "node", "ref": "n"},
                 "vector": [1.0, 0.0, 0.0], "valid_time": {"start": 0}}]}),
        ),
        ("query", json!({"graph_id": GRAPH})),
        ("hybrid_search", search),
    ]
}

/// Revision IDs (a declared channel: they hash every fact) and the IDs
/// derived from them, and freshly minted IDs, are replaced by placeholders.
fn normalize(bytes: &[u8]) -> String {
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    'scan: while !rest.is_empty() {
        for prefix in [
            "fact:sha256:",
            "commit:fact:sha256:",
            "explanation:sha256:",
            "conclusion:sha256:",
            "derivation:sha256:",
            "premise:sha256:",
        ] {
            if let Some(tail) = rest.strip_prefix(prefix) {
                let hex = tail.chars().take_while(char::is_ascii_hexdigit).count();
                out.push_str("<derived>");
                rest = &tail[hex..];
                continue 'scan;
            }
        }
        if let Some(tail) = rest.strip_prefix("r-") {
            let shape = tail.len() >= 49
                && tail[..32].chars().all(|c| c.is_ascii_hexdigit())
                && tail.as_bytes()[32] == b'-'
                && tail[33..49].chars().all(|c| c.is_ascii_hexdigit());
            if shape {
                out.push_str("<minted>");
                rest = &tail[49..];
                continue;
            }
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Order-free form of a value: arrays sorted by their normalized text, and
/// objects keyed by derived IDs turned into sorted lists of their entries.
fn canonical(value: Value) -> Value {
    let order = |v: &Value| normalize(v.to_string().as_bytes());
    match value {
        Value::Array(items) => {
            let mut items: Vec<Value> = items.into_iter().map(canonical).collect();
            items.sort_by_key(order);
            Value::Array(items)
        }
        Value::Object(map) => {
            if map.keys().any(|k| normalize(k.as_bytes()) != *k) {
                let mut entries: Vec<Value> = map
                    .into_iter()
                    .map(|(k, v)| json!([k, canonical(v)]))
                    .collect();
                entries.sort_by_key(order);
                Value::Array(entries)
            } else {
                Value::Object(map.into_iter().map(|(k, v)| (k, canonical(v))).collect())
            }
        }
        other => other,
    }
}

/// Acceptance criterion 2 (and the paired test of every tool): agent Z of
/// team B cannot see team A's fact or any effect of it, and agent W, with
/// no access to the graph, cannot even tell the graph from a missing one.
#[tokio::test]
async fn another_teams_agent_sees_no_trace_of_hidden_facts_in_any_tool() {
    let hidden = world(true, &[]).await;
    let plain = world(false, &hidden.hidden).await;
    let secrets = [FINDING_TEXT, "team A only", "under review", MODEL, DOC_URI];
    let mut compared = 0;
    let mut outcomes = Vec::new();
    for who in ["z", "w"] {
        for (tool, arguments) in observations(&hidden.hidden) {
            let token = |w: &World| match who {
                "z" => w.t.z.clone(),
                _ => w.t.w.clone(),
            };
            let a = call(&plain.h, &token(&plain), tool, arguments.clone()).await;
            let b = call(&hidden.h, &token(&hidden), tool, arguments.clone()).await;
            assert_eq!(a.status, b.status, "{who} {tool} {arguments}");
            if tool == "explain" {
                // An explanation is a set of derived records keyed by hashes
                // of the input revision, so it is compared as a set.
                let set = |reply: &Reply| {
                    let structured = canonical(reply.json["result"]["structuredContent"].clone());
                    normalize(structured.to_string().as_bytes())
                };
                assert_eq!(set(&a), set(&b), "{who} {tool} {arguments}");
            } else {
                assert_eq!(
                    normalize(&a.bytes),
                    normalize(&b.bytes),
                    "{who} {tool} {arguments}"
                );
            }
            let structured = &b.json["result"]["structuredContent"];
            outcomes.push(format!(
                "{who} {tool} {}",
                structured["error"]["code"].as_str().unwrap_or("ok")
            ));
            let text = String::from_utf8_lossy(&b.bytes);
            for secret in secrets {
                assert!(!text.contains(secret), "{who} {tool}: {secret} in {text}");
            }
            for id in &hidden.hidden {
                // A hidden ID appears only where the caller itself named it,
                // in an error or an empty answer about it.
                if !arguments.to_string().contains(id.as_str()) {
                    assert!(!text.contains(id.as_str()), "{who} {tool}: {id} in {text}");
                }
            }
            compared += 1;
        }
    }
    assert_eq!(compared, 2 * observations(&hidden.hidden).len());
    // Z reads the graph and writes its own team's facts; it cannot name
    // team A's group or touch its facts. W sees no graph at all.
    let z_expected = [
        "describe_schema ok",
        "describe_schema ok",
        "query ok",
        "query ok",
        "query ok",
        "query ok",
        "expand ok",
        "expand ok",
        "as_of ok",
        "what_changed ok",
        "what_changed ok",
        "hybrid_search ok",
        "hybrid_search E_UNAVAILABLE",
        "hybrid_search ok",
        "hybrid_search ok",
        "hybrid_search ok",
        "explain ok",
        "explain ok",
        "retract_fact E_CONFLICT",
        "retract_fact E_CONFLICT",
        "retract_fact E_CONFLICT",
        "assert_fact E_UNAVAILABLE",
        "assert_fact ok",
        "query ok",
        "hybrid_search ok",
    ];
    let expected: Vec<String> = z_expected
        .iter()
        .map(|s| format!("z {s}"))
        .chain(
            observations(&hidden.hidden)
                .iter()
                .enumerate()
                .map(|(i, (tool, _))| {
                    format!("w {tool} {}", if i == 0 { "ok" } else { "E_UNAVAILABLE" })
                }),
        )
        .collect();
    assert_eq!(outcomes, expected);

    // W cannot tell the graph from one that does not exist, in any tool.
    for (tool, arguments) in observations(&hidden.hidden) {
        let missing: Value =
            serde_json::from_str(&arguments.to_string().replace(GRAPH, "never-made")).unwrap();
        let a = call(&hidden.h, &hidden.t.w, tool, arguments.clone()).await;
        let b = call(&hidden.h, &hidden.t.w, tool, missing).await;
        assert_eq!(
            normalize(&a.bytes).replace(GRAPH, "never-made"),
            normalize(&b.bytes),
            "{tool}"
        );
    }

    // The hidden world really holds the facts, and team A sees them.
    let y = call(&hidden.h, &hidden.t.y, "query", json!({"graph_id": GRAPH})).await;
    assert_eq!(y.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&y.bytes).contains(FINDING_TEXT));
    let found = ok(
        &hidden.h,
        &hidden.t.y,
        "hybrid_search",
        nearest(json!([1.0, 0.0, 0.0])),
    )
    .await;
    assert_eq!(neighbor_subjects(&found)[0], hidden.hidden[0]);
}
