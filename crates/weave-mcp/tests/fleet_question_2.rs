//! Phase A exit question 2 on the A5 fleet, answered by a scripted client
//! using only the preview tools: "which units at site S ran firmware F on
//! date D, as we knew it on date K" (`as_of`) and "what changed for unit U
//! between K1 and K2" (`what_changed`). The answers must equal the
//! `weave-hardware` helpers on the same store, for a restricted and an
//! unrestricted reader, and restricted units never appear for the analyst.
//! Questions 3 and 4 need `hybrid_search` (A4) and are added with it.
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;
use weave_engine::{Engine, HostContext, ManualClock};
use weave_hardware::generate::{base_time, generate, updates_end, Config, ADMIN, ANALYST};
use weave_hardware::import::{apply_updates, import_batch, Target};
use weave_hardware::mapping::node_id;
use weave_hardware::query::{units_at_site_running, At};
use weave_hardware::schema;
use weave_hardware::table::parse_time;
use weave_mcp::{Server, Tool};
use weave_science::ScienceSession;

const GRAPH: &str = "fleet";

fn call(server: &mut Server, tool: Tool, arguments: Value) -> Value {
    let response = server.call_tool(tool, arguments.clone());
    assert_eq!(
        response["isError"], false,
        "{tool:?} {arguments}: {response}"
    );
    let mut results = response["structuredContent"]["result"]["results"].clone();
    results[0]["result"].take()
}

/// Units with an edge of `predicate` to `target` at valid time `d`, as known at `k`.
fn units_with(server: &mut Server, predicate: &str, target: &str, d: i64, k: i64) -> Value {
    call(
        server,
        Tool::AsOf,
        json!({"unix_millis": k, "query": {"graph_id": GRAPH, "predicate": predicate, "to": target, "valid_at": d}}),
    )
}

fn serials(result: &Value) -> BTreeSet<String> {
    let prefix = format!("{}:", schema::UNIT.prefix);
    result["graph"]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["from"].as_str()?.strip_prefix(&prefix).map(String::from))
        .collect()
}

#[test]
fn an_mcp_client_answers_exit_question_2() {
    let dataset = generate(&Config {
        units: 60,
        updates: 60,
        ..Config::default()
    });
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("fleet.db");
    let clock = Arc::new(ManualClock::new(base_time()));
    {
        let mut engine = Engine::open_with_clock(&db, clock.clone()).unwrap();
        let writer = HostContext::new("etl-importer", [GRAPH.to_string()]);
        let target = Target::new(GRAPH);
        import_batch(&mut engine, &writer, &target, &dataset.base).unwrap();
        apply_updates(&mut engine, &writer, &target, &dataset.updates, |u| {
            clock.set(parse_time(&u.reported_at).unwrap())
        })
        .unwrap();
    }
    let now = Arc::new(ManualClock::new(updates_end() + 86_400_000));
    let open = |who: &str| {
        let engine = Engine::open_with_clock(&db, now.clone()).unwrap();
        ScienceSession::new(engine, HostContext::new(who, Vec::<String>::new()))
    };
    let restricted: BTreeSet<String> = dataset.base.tables["units"]
        .iter()
        .filter(|u| u.contains_key("readers"))
        .map(|u| u["serial"].clone())
        .collect();
    assert!(!restricted.is_empty());

    let rollouts: Vec<_> = dataset
        .updates
        .iter()
        .filter(|u| u.kind == "firmware_rollout")
        .take(8)
        .collect();
    assert!(!rollouts.is_empty());
    let (mut answered, mut nonempty, mut hidden_changes) = (0, 0, 0);
    for who in [ADMIN, ANALYST] {
        let mut server = Server::new(open(who));
        let mut oracle = Engine::open_with_clock(&db, now.clone()).unwrap();
        let host = HostContext::new(who, Vec::<String>::new());
        for update in &rollouts {
            let old = &update.batch.tables["firmware_installs"][0];
            let new = &update.batch.tables["firmware_installs"][1];
            let (serial, fw) = (&new["serial"], &new["firmware_id"]);
            let d = parse_time(&new["valid_from"]).unwrap() + 3_600_000;
            let k2 = parse_time(&update.reported_at).unwrap();
            let k1 = k2 - 1;
            let unit = node_id(&schema::UNIT, serial);
            // The agent finds the unit's site at D, as known at K2.
            let placed = call(
                &mut server,
                Tool::AsOf,
                json!({"unix_millis": k2, "query": {"graph_id": GRAPH, "predicate": "hw:located_at", "from": unit, "valid_at": d}}),
            );
            let Some(site) = placed["graph"]["edges"][0]["to"].as_str().map(String::from) else {
                // A restricted unit is invisible to the analyst, site included.
                assert!(who == ANALYST && restricted.contains(serial));
                continue;
            };
            let site_key = site
                .strip_prefix(&format!("{}:", schema::SITE.prefix))
                .unwrap()
                .to_string();
            for k in [k1, k2] {
                let runs = units_with(
                    &mut server,
                    "hw:runs",
                    &node_id(&schema::FIRMWARE, fw),
                    d,
                    k,
                );
                let located = units_with(&mut server, "hw:located_at", &site, d, k);
                let got: Vec<String> = serials(&runs)
                    .intersection(&serials(&located))
                    .cloned()
                    .collect();
                let want = units_at_site_running(
                    &mut oracle,
                    &host,
                    GRAPH,
                    &site_key,
                    fw,
                    d,
                    &At::AsOf(k),
                )
                .unwrap();
                assert_eq!(got, want.units, "site {site} fw {fw} d {d} k {k} as {who}");
                // Every answer cites its source facts.
                for result in [&runs, &located] {
                    let cited: BTreeSet<&str> = result["fact_provenance"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|e| e["fact"]["assertion_id"].as_str())
                        .collect();
                    for edge in result["graph"]["edges"].as_array().unwrap() {
                        let id = edge["id"].as_str().unwrap();
                        assert!(cited.contains(id), "provenance for {id}");
                    }
                }
                if who == ANALYST {
                    assert!(got.iter().all(|u| !restricted.contains(u)));
                }
                nonempty += usize::from(!got.is_empty());
                answered += 1;
            }
            // What changed for the unit between K1 and K2.
            let changed = call(
                &mut server,
                Tool::WhatChanged,
                json!({"graph_id": GRAPH, "from": {"kind": "as_of", "unix_millis": k1},
                    "to": {"kind": "as_of", "unix_millis": k2}, "entities": [unit]}),
            );
            let entries: BTreeSet<(String, String)> = changed["changes"]["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| {
                    (
                        e["id"].as_str().unwrap().to_string(),
                        e["change"].as_str().unwrap().to_string(),
                    )
                })
                .collect();
            let concurrent = dataset
                .updates
                .iter()
                .any(|u| u.reported_at == update.reported_at && u.seq != update.seq);
            if !concurrent {
                let want: BTreeSet<(String, String)> = [
                    (format!("runs:{}", old["install_id"]), "corrected".into()),
                    (format!("runs:{}", new["install_id"]), "asserted".into()),
                ]
                .into();
                assert_eq!(entries, want, "changes of {serial} as {who}");
            }
        }
        // A restricted unit's changes are hidden from the analyst.
        if who == ANALYST {
            for serial in restricted.iter().take(3) {
                let changed = call(
                    &mut server,
                    Tool::WhatChanged,
                    json!({"graph_id": GRAPH, "from": {"kind": "as_of", "unix_millis": 0},
                        "entities": [node_id(&schema::UNIT, serial)]}),
                );
                assert_eq!(changed["changes"]["entries"], json!([]));
                assert!(!changed.to_string().contains(serial.as_str()));
                hidden_changes += 1;
            }
        }
    }
    assert!(answered > 0 && nonempty > 0 && hidden_changes > 0);
}
