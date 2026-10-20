//! Phase A exit questions 2, 3 and 4 on the A5 fleet, answered by a scripted
//! MCP client that drives the real `weave-mcp` binary over stdio and uses
//! only the preview tools, for a restricted and an unrestricted reader:
//!
//! 2. "which units at site S ran firmware F on date D, as we knew it on date
//!    K" (`as_of`) and "what changed for unit U between K1 and K2"
//!    (`what_changed`), checked against the `weave-hardware` helpers;
//! 3. "maintenance notes semantically similar to an incident, restricted to
//!    the 2-hop neighborhood of the affected unit and valid at the incident
//!    time" (`hybrid_search` `nearest` with an `expand` as candidates);
//! 4. "two similar-unit candidate relations absent from the graph, each with
//!    provenance" (`hybrid_search` `similar_candidates`, then `query` to
//!    show each pair is unlinked);
//!
//! 3 and 4 are checked against an independent ranking over the vectors this
//! test wrote, as in `weave-hardware/tests/fleet_hybrid.rs`, whose
//! deterministic hashing embedder this test reuses. Restricted units never
//! appear for the analyst (question 5).
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use weave_contract::{Program, VERSION};
use weave_engine::{Engine, HostContext, ManualClock};
use weave_hardware::generate::{base_time, generate, updates_end, Config, Dataset, ADMIN, ANALYST};
use weave_hardware::import::{apply_updates, import_batch, Target};
use weave_hardware::mapping::node_id;
use weave_hardware::query::{units_at_site_running, At};
use weave_hardware::schema;
use weave_hardware::table::{parse_time, Row};

const GRAPH: &str = "fleet";
const EMBEDDER: &str = "note-embedder";
const ENCODER: &str = "weave-test-hash-embedder";
const DIMS: usize = 64;

/// The deterministic test embedder of `fleet_hybrid.rs`: signed feature
/// hashing of lowercase words, L2-normalized.
fn embed(text: &str) -> Vec<f32> {
    let mut v = vec![0f32; DIMS];
    for word in text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
    {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in word.bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
        }
        let sign = if (h >> 40) & 1 == 0 { 1.0 } else { -1.0 };
        v[(h % DIMS as u64) as usize] += sign;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        v[0] = 1.0;
        return v;
    }
    v.iter().map(|x| x / norm).collect()
}

/// The normative cosine distance, computed independently of the engine.
fn distance(a: &[f32], b: &[f32]) -> f64 {
    let (mut dot, mut na, mut nb) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (f64::from(*x), f64::from(*y));
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    (1.0 - dot / (na.sqrt() * nb.sqrt())).clamp(0.0, 2.0)
}

fn readers(row: &Row) -> Vec<String> {
    row.get("readers")
        .map(|r| {
            r.split(';')
                .filter(|p| !p.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}
fn admits(readers: &[String], who: &str) -> bool {
    readers.is_empty() || readers.iter().any(|r| r == who)
}

#[derive(Clone)]
struct Vector {
    id: String,
    subject: String,
    serial: String,
    vector: Vec<f32>,
    valid: (i64, Option<i64>),
    readers: Vec<String>,
}
impl Vector {
    fn valid_at(&self, t: i64) -> bool {
        self.valid.0 <= t && self.valid.1.is_none_or(|e| t < e)
    }
    fn fact(&self, space: &str) -> Value {
        json!({"kind": "embedding", "id": self.id, "space": {"id": space, "revision": "1"},
            "subject": {"kind": "node", "node_id": self.subject}, "vector": self.vector,
            "valid_time": {"start": self.valid.0, "end": self.valid.1}, "readers": self.readers,
            "provenance": {"method": {"kind": "model", "model": ENCODER, "version": "1"}, "source": "fleet-notes"}})
    }
}

fn space(id: &str, preprocessing: &str) -> Value {
    json!({"kind": "embedding_space", "id": id, "revision": "1", "encoder": ENCODER,
        "encoder_version": "1", "preprocessing": preprocessing, "dimensions": DIMS, "metric": "cosine_distance"})
}

/// The fleet's final rows, with every update applied.
fn final_rows(dataset: &Dataset) -> BTreeMap<String, BTreeMap<String, Row>> {
    let mut tables: BTreeMap<String, BTreeMap<String, Row>> = BTreeMap::new();
    let key = |table: &str| schema::table(table).unwrap().key;
    for (table, rows) in dataset
        .base
        .tables
        .iter()
        .chain(dataset.updates.iter().flat_map(|u| u.batch.tables.iter()))
    {
        for row in rows {
            tables
                .entry(table.clone())
                .or_default()
                .insert(row[key(table)].clone(), row.clone());
        }
    }
    tables
}

/// Embed notes, incident descriptions and unit profiles as `fleet_hybrid.rs`
/// does. Returns (notes and incidents, incidents with their open time, units).
type Embedded = (Vec<Vector>, Vec<(Vector, i64)>, Vec<Vector>);
fn embeddings(tables: &BTreeMap<String, BTreeMap<String, Row>>) -> Embedded {
    let rows = |table: &str| tables.get(table).into_iter().flat_map(|t| t.values());
    let (mut notes, mut incidents, mut units) = (vec![], vec![], vec![]);
    let mut profiles: BTreeMap<String, String> = BTreeMap::new();
    for row in rows("maintenance") {
        let id = format!("maint:{}", row["work_order"]);
        notes.push(Vector {
            id: format!("emb:note:{id}"),
            subject: id,
            serial: row["serial"].clone(),
            vector: embed(&row["note"]),
            valid: (parse_time(&row["started_at"]).unwrap(), None),
            readers: readers(row),
        });
        let profile = profiles.entry(row["serial"].clone()).or_default();
        profile.push(' ');
        profile.push_str(&row["note"]);
    }
    for row in rows("incidents") {
        let id = format!("incident:{}", row["incident_id"]);
        let opened = parse_time(&row["opened_at"]).unwrap();
        let v = Vector {
            id: format!("emb:note:{id}"),
            subject: id,
            serial: row["serial"].clone(),
            vector: embed(&row["description"]),
            valid: (opened, None),
            readers: readers(row),
        };
        notes.push(v.clone());
        incidents.push((v, opened));
        let profile = profiles.entry(row["serial"].clone()).or_default();
        profile.push(' ');
        profile.push_str(&row["title"]);
    }
    for row in rows("firmware_installs") {
        let profile = profiles.entry(row["serial"].clone()).or_default();
        profile.push(' ');
        profile.push_str(&row["firmware_id"]);
    }
    for row in rows("units") {
        let serial = &row["serial"];
        let text = format!(
            "{} {}",
            row["model_id"],
            profiles.get(serial).cloned().unwrap_or_default()
        );
        let end = row
            .get("decommissioned_at")
            .filter(|v| !v.is_empty())
            .map(|v| parse_time(v).unwrap());
        units.push(Vector {
            id: format!("emb:profile:unit:{serial}"),
            subject: format!("unit:{serial}"),
            serial: serial.clone(),
            vector: embed(&text),
            valid: (parse_time(&row["commissioned_at"]).unwrap(), end),
            readers: readers(row),
        });
    }
    (notes, incidents, units)
}

/// A scripted MCP client over the real binary's stdio.
struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    id: u64,
}

impl Client {
    fn spawn(db: &Path, actor: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_weave-mcp"))
            .args(["--db", db.to_str().unwrap(), "--actor", actor])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self {
            child,
            stdin,
            stdout,
            id: 0,
        };
        let init = client.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "exit-scenario", "version": "0"}}),
        );
        assert_eq!(init["serverInfo"]["name"], "weave-mcp");
        client.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        let tools = client.request("tools/list", json!({}));
        let names: BTreeSet<&str> = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for needed in ["as_of", "what_changed", "hybrid_search", "query"] {
            assert!(names.contains(needed));
        }
        client
    }
    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}));
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let mut response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], self.id);
        response["result"].take()
    }
    /// Call a tool and return its single query result.
    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let mut result = self.request(
            "tools/call",
            json!({"name": name, "arguments": arguments.clone()}),
        );
        assert_eq!(result["isError"], false, "{name} {arguments}: {result}");
        let content = &mut result["structuredContent"];
        assert_eq!(content["trust"], "untrusted_data");
        content["result"]["results"][0]["result"].take()
    }
    fn finish(mut self) {
        drop(self.stdin);
        assert!(self.child.wait().unwrap().success());
    }
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

fn cited(result: &Value) -> BTreeSet<String> {
    result["fact_provenance"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| e["fact"].to_string())
        .collect()
}

fn provenance_of<'a>(result: &'a Value, fact: &Value) -> &'a Value {
    result["fact_provenance"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| &e["fact"] == fact)
        .map(|e| &e["provenance"])
        .unwrap_or_else(|| panic!("provenance for {fact}"))
}

#[test]
fn an_mcp_client_answers_exit_questions_2_to_4() {
    let dataset = generate(&Config {
        units: 60,
        updates: 60,
        ..Config::default()
    });
    let tables = final_rows(&dataset);
    let (notes, incidents, units) = embeddings(&tables);
    let restricted: BTreeSet<String> = tables["units"]
        .values()
        .filter(|u| !readers(u).is_empty())
        .map(|u| u["serial"].clone())
        .collect();
    assert!(!restricted.is_empty());

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
        clock.set(updates_end() + 3_600_000);
        let mut changes = vec![
            json!({"change": "assert", "fact": space("hw-notes", "lowercase-words/v1")}),
            json!({"change": "assert", "fact": space("hw-units", "unit-profile/v1")}),
        ];
        changes.extend(
            notes
                .iter()
                .map(|v| json!({"change": "assert", "fact": v.fact("hw-notes")})),
        );
        changes.extend(
            units
                .iter()
                .map(|v| json!({"change": "assert", "fact": v.fact("hw-units")})),
        );
        let embedder = HostContext::new(EMBEDDER, [GRAPH.to_string()]);
        for batch in changes.chunks(2_000) {
            let program: Program = serde_json::from_value(json!({"version": VERSION,
                "commands": [{"op": "write", "graph_id": GRAPH, "changes": batch}]}))
            .unwrap();
            engine.execute(&program, &embedder).unwrap();
        }
    }
    // The server runs on the system clock, after every recorded time above.
    let oracle_clock = Arc::new(ManualClock::new(updates_end() + 86_400_000 * 2));
    let mut oracle = Engine::open_with_clock(&db, oracle_clock).unwrap();

    let rollouts: Vec<_> = dataset
        .updates
        .iter()
        .filter(|u| u.kind == "firmware_rollout")
        .take(6)
        .collect();
    assert!(!rollouts.is_empty());
    let mut incidents_by_time = incidents.clone();
    incidents_by_time.sort_by_key(|(v, t)| (*t, v.id.clone()));
    let mut chosen: Vec<&(Vector, i64)> = incidents_by_time.iter().rev().take(4).collect();
    if let Some(r) = incidents_by_time
        .iter()
        .find(|(v, _)| restricted.contains(&v.serial))
    {
        chosen.push(r);
    }
    let mut counts = BTreeMap::<&str, usize>::new();
    for who in [ADMIN, ANALYST] {
        let host = HostContext::new(who, Vec::<String>::new());
        let mut client = Client::spawn(&db, who);

        // ---- Question 2.
        for update in &rollouts {
            let old = &update.batch.tables["firmware_installs"][0];
            let new = &update.batch.tables["firmware_installs"][1];
            let (serial, fw) = (&new["serial"], &new["firmware_id"]);
            let d = parse_time(&new["valid_from"]).unwrap() + 3_600_000;
            let k2 = parse_time(&update.reported_at).unwrap();
            let k1 = k2 - 1;
            let unit = node_id(&schema::UNIT, serial);
            let placed = client.call(
                "as_of",
                json!({"unix_millis": k2, "query": {"graph_id": GRAPH, "predicate": "hw:located_at", "from": unit, "valid_at": d}}),
            );
            let Some(site) = placed["graph"]["edges"][0]["to"].as_str().map(String::from) else {
                assert!(who == ANALYST && restricted.contains(serial));
                continue;
            };
            let site_key = site
                .strip_prefix(&format!("{}:", schema::SITE.prefix))
                .unwrap()
                .to_string();
            for k in [k1, k2] {
                let by = |client: &mut Client, predicate: &str, target: &str| {
                    client.call(
                        "as_of",
                        json!({"unix_millis": k, "query": {"graph_id": GRAPH, "predicate": predicate, "to": target, "valid_at": d}}),
                    )
                };
                let runs = by(&mut client, "hw:runs", &node_id(&schema::FIRMWARE, fw));
                let located = by(&mut client, "hw:located_at", &site);
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
                assert_eq!(got, want.units, "q2 site {site} fw {fw} k {k} as {who}");
                for result in [&runs, &located] {
                    let cited = cited(result);
                    for edge in result["graph"]["edges"].as_array().unwrap() {
                        assert!(
                            cited
                                .iter()
                                .any(|c| c.contains(edge["id"].as_str().unwrap())),
                            "provenance for {}",
                            edge["id"]
                        );
                    }
                }
                if who == ANALYST {
                    assert!(got.iter().all(|u| !restricted.contains(u)));
                }
                *counts.entry("q2").or_default() += usize::from(!got.is_empty());
            }
            let changed = client.call(
                "what_changed",
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
                assert_eq!(entries, want, "q2 changes of {serial} as {who}");
            }
        }

        // ---- Question 3.
        for (incident, opened) in &chosen {
            let unit = format!("unit:{}", incident.serial);
            let neighborhood = json!({"kind": "expand", "graph_id": GRAPH, "seeds": [unit], "hops": 2, "direction": "both"});
            let got = client.call(
                "hybrid_search",
                json!({"kind": "nearest", "graph_id": GRAPH, "space": {"id": "hw-notes", "revision": "1"},
                    "query": {"kind": "vector", "vector": incident.vector}, "k": 10, "valid_at": opened,
                    "candidates": neighborhood}),
            );
            // Oracle: the reader's 2-hop region (A3's BFS-checked `expand`),
            // ranked independently over the vectors this test wrote.
            let region: BTreeSet<String> = client.call(
                "expand",
                json!({"graph_id": GRAPH, "seeds": [unit], "hops": 2, "direction": "both"}),
            )["graph"]["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n["id"].as_str().unwrap().to_string())
                .collect();
            let mut want: Vec<(f64, String, String)> = notes
                .iter()
                .filter(|v| {
                    admits(&v.readers, who) && region.contains(&v.subject) && v.valid_at(*opened)
                })
                .map(|v| {
                    (
                        distance(&incident.vector, &v.vector),
                        v.subject.clone(),
                        v.id.clone(),
                    )
                })
                .collect();
            want.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
            want.truncate(10);
            let neighbors = got["neighbors"].as_array().cloned().unwrap_or_default();
            let got_hits: Vec<(f64, String)> = neighbors
                .iter()
                .map(|n| {
                    (
                        n["score"].as_f64().unwrap(),
                        n["embedding_id"].as_str().unwrap().to_string(),
                    )
                })
                .collect();
            let want_hits: Vec<(f64, String)> =
                want.iter().map(|(s, _, id)| (*s, id.clone())).collect();
            assert_same_ranking(
                &got_hits,
                &want_hits,
                &format!("q3 {} as {who}", incident.id),
            );
            let hidden_unit = restricted.contains(&incident.serial);
            if who == ANALYST && hidden_unit {
                assert!(neighbors.is_empty(), "a restricted unit's notes are hidden");
            }
            if who == ADMIN {
                assert!(!neighbors.is_empty(), "the incident's unit has notes");
            }
            for n in &neighbors {
                let v = notes.iter().find(|v| v.id == n["embedding_id"]).unwrap();
                if who == ANALYST {
                    assert!(!restricted.contains(&v.serial));
                }
                // Each hit is explained: the embedding's writer and method.
                let fact = json!({"kind": "embedding", "graph_id": GRAPH,
                    "revision": provenance_revision(&got, &v.id), "embedding_id": v.id});
                let provenance = provenance_of(&got, &fact);
                assert_eq!(provenance["writer"], EMBEDDER);
                assert_eq!(provenance["method"]["model"], ENCODER);
            }
            *counts.entry("q3").or_default() += usize::from(!neighbors.is_empty());
        }

        // ---- Question 4.
        let at = updates_end();
        let got = client.call(
            "hybrid_search",
            json!({"kind": "similar_candidates", "graph_id": GRAPH, "space": {"id": "hw-units", "revision": "1"},
                "k": 1, "valid_at": at, "exclude_linked": true, "limit": 100_000}),
        );
        let visible: Vec<&Vector> = units
            .iter()
            .filter(|v| admits(&v.readers, who) && v.valid_at(at))
            .collect();
        let mut pairs: BTreeMap<(String, String), f64> = BTreeMap::new();
        for s in &visible {
            let best = visible
                .iter()
                .filter(|t| t.subject != s.subject)
                .map(|t| (distance(&s.vector, &t.vector), t))
                .min_by(|a, b| {
                    a.0.total_cmp(&b.0)
                        .then(a.1.subject.cmp(&b.1.subject))
                        .then(a.1.id.cmp(&b.1.id))
                })
                .unwrap();
            let (lo, hi) = if s.subject <= best.1.subject {
                (s.subject.clone(), best.1.subject.clone())
            } else {
                (best.1.subject.clone(), s.subject.clone())
            };
            pairs.insert((lo, hi), best.0);
        }
        let mut want: Vec<(f64, String, String)> =
            pairs.into_iter().map(|((a, b), s)| (s, a, b)).collect();
        want.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let candidates = got["candidates"].as_array().cloned().unwrap_or_default();
        let got_pairs: Vec<(f64, String, String)> = candidates
            .iter()
            .map(|c| {
                (
                    c["score"].as_f64().unwrap(),
                    c["from"].as_str().unwrap().to_string(),
                    c["to"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let key = |v: &[(f64, String, String)]| -> Vec<(f64, String)> {
            v.iter().map(|(s, a, b)| (*s, format!("{a} {b}"))).collect()
        };
        assert_same_ranking(&key(&got_pairs), &key(&want), &format!("q4 as {who}"));
        for candidate in &candidates[..2] {
            assert_eq!(candidate["predicate"], "weave:similar_to");
            // Absent from the graph: the reader's query finds no edge either way.
            for (from, to) in [
                (&candidate["from"], &candidate["to"]),
                (&candidate["to"], &candidate["from"]),
            ] {
                let read = client.call("query", json!({"graph_id": GRAPH, "from": from, "to": to}));
                assert_eq!(read["graph"]["edges"], json!([]), "{from} -> {to}");
            }
            let origins = candidate["origins"].as_array().unwrap();
            assert_eq!(origins.len(), 2);
            for origin in origins {
                let provenance = provenance_of(&got, origin);
                assert_eq!(provenance["writer"], EMBEDDER);
                assert_eq!(provenance["method"]["kind"], "model");
            }
            if who == ANALYST {
                for end in [&candidate["from"], &candidate["to"]] {
                    assert!(!restricted
                        .iter()
                        .any(|s| json!(format!("unit:{s}")) == *end));
                }
            }
        }
        *counts.entry("q4").or_default() += 1;
        client.finish();
    }
    assert!(
        counts["q2"] > 0 && counts["q3"] > 0 && counts["q4"] == 2,
        "{counts:?}"
    );
}

/// Equal IDs in equal order, and scores equal up to JSON text parsing:
/// `serde_json` without `float_roundtrip` may parse a printed `f64` one unit
/// in the last place off, so a score read back from the wire is compared with
/// a tolerance far below any score gap that could reorder hits.
fn assert_same_ranking(got: &[(f64, String)], want: &[(f64, String)], what: &str) {
    let ids = |v: &[(f64, String)]| v.iter().map(|(_, id)| id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(got), ids(want), "{what}");
    for ((a, id), (b, _)) in got.iter().zip(want) {
        assert!((a - b).abs() <= 1e-12, "{what}: score of {id} {a} != {b}");
    }
}

/// The revision a result cites for one embedding fact.
fn provenance_revision(result: &Value, embedding_id: &str) -> Value {
    result["fact_provenance"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["fact"]["embedding_id"] == embedding_id)
        .map(|e| e["fact"]["revision"].clone())
        .unwrap_or_else(|| panic!("no provenance for {embedding_id}"))
}
