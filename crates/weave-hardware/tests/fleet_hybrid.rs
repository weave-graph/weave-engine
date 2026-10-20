//! Phase A exit questions 3 and 4 on the generated fleet (A4, ADR-0006).
//!
//! The fleet is imported as in `fleet_scenario.rs`. A deterministic test
//! embedder (feature hashing of words; no network, no model) then embeds
//! every maintenance note and incident description into the `hw-notes`
//! space and a profile of every unit into the `hw-units` space, written as
//! embedding facts through the A2 fact-write path with the rows' readers and
//! model provenance. Then:
//!
//! 3. "maintenance notes semantically similar to an incident, restricted to
//!    the 2-hop neighborhood of the affected unit and valid at the incident
//!    time" is one `nearest` plan whose candidates are an `expand`;
//! 4. "two similar-unit candidate relations absent from the graph, with
//!    provenance" is one `similar_candidates` plan;
//!
//! each checked against an independent oracle for a restricted and an
//! unrestricted reader, and reproduced after reopening the store. Restricted
//! units never appear for the analyst (question 5).
//!
//! Size: `WEAVE_FLEET_UNITS` (default 150) and `WEAVE_FLEET_UPDATES`
//! (default 150), as `fleet_scenario.rs`.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use weave_contract::provenance::{FactRef, Method};
use weave_contract::{CommandResult, Program, QueryResult, VERSION};
use weave_engine::{Engine, HostContext, ManualClock};
use weave_hardware::generate::{base_time, generate, updates_end, Config, ADMIN, ANALYST};
use weave_hardware::import::{apply_updates, import_batch, Target};
use weave_hardware::table::{parse_time, Row};

const GRAPH: &str = "fleet";
const IMPORTER: &str = "etl-importer";
const EMBEDDER: &str = "note-embedder";
const DIMS: usize = 64;
const ENCODER: &str = "weave-test-hash-embedder";

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Deterministic test embedder: signed feature hashing of lowercase words,
/// L2-normalized. Similar wording gives a small cosine distance.
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

/// One embedding fact as the oracle sees it.
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

fn program(commands: Value) -> Program {
    serde_json::from_value(json!({"version": VERSION, "commands": commands})).unwrap()
}
fn evaluate(engine: &mut Engine, who: &str, value: &Value) -> QueryResult {
    let host = HostContext::new(who, Vec::<String>::new());
    match engine
        .execute(&program(json!([{"op": "evaluate", "value": value}])), &host)
        .unwrap()
        .remove(0)
    {
        CommandResult::Queried { result } => *result,
        other => panic!("{other:?}"),
    }
}

#[test]
fn phase_a_hybrid_retrieval_questions_3_and_4() {
    let config = Config {
        units: env("WEAVE_FLEET_UNITS", 150),
        updates: env("WEAVE_FLEET_UPDATES", 150),
        ..Config::default()
    };
    let dataset = generate(&config);
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("fleet.db");
    let clock = Arc::new(ManualClock::new(base_time()));
    let mut engine = Engine::open_with_clock(&db, clock.clone()).unwrap();
    let importer = HostContext::new(IMPORTER, [GRAPH.to_string()]);
    let target = Target::new(GRAPH);
    import_batch(&mut engine, &importer, &target, &dataset.base).unwrap();
    apply_updates(&mut engine, &importer, &target, &dataset.updates, |u| {
        clock.set(parse_time(&u.reported_at).unwrap())
    })
    .unwrap();

    // The final rows: the base export with every update applied.
    let mut tables: BTreeMap<String, BTreeMap<String, Row>> = BTreeMap::new();
    let key = |table: &str| weave_hardware::schema::table(table).unwrap().key;
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
    let rows = |table: &str| tables.get(table).into_iter().flat_map(|t| t.values());
    let restricted: BTreeSet<String> = rows("units")
        .filter(|u| !readers(u).is_empty())
        .map(|u| u["serial"].clone())
        .collect();
    assert!(!restricted.is_empty());

    // Embed notes, incident descriptions and unit profiles (caller-supplied
    // embeddings through the A2 write path, one fact per vector).
    let mut notes: Vec<Vector> = vec![];
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
    let mut incidents: Vec<(Vector, i64)> = vec![];
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
    let mut units: Vec<Vector> = vec![];
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
    let embed_time = updates_end() + 3_600_000;
    clock.set(embed_time);
    let embedder = HostContext::new(EMBEDDER, [GRAPH.to_string()]);
    let mut changes: Vec<Value> = vec![
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
    let started = std::time::Instant::now();
    let mut revisions = 0;
    for batch in changes.chunks(2_000) {
        engine
            .execute(
                &program(json!([{"op": "write", "graph_id": GRAPH, "changes": batch}])),
                &embedder,
            )
            .unwrap();
        revisions += 1;
    }
    println!(
        "{} embeddings in {revisions} fact writes in {:?}",
        changes.len() - 2,
        started.elapsed()
    );
    clock.set(embed_time + 86_400_000);

    // ---- Question 3.
    let mut answers: Vec<(Value, &str, Vec<String>)> = vec![];
    let mut checked = 0;
    let mut restricted_seen_by_admin = 0;
    let mut incidents_by_time = incidents.clone();
    incidents_by_time.sort_by_key(|(v, t)| (*t, v.id.clone()));
    let mut chosen: Vec<&(Vector, i64)> = incidents_by_time.iter().rev().take(6).collect();
    if let Some(r) = incidents_by_time
        .iter()
        .find(|(v, _)| restricted.contains(&v.serial))
    {
        chosen.push(r);
    }
    for (incident, opened) in chosen {
        let unit = format!("unit:{}", incident.serial);
        let neighborhood = json!({"kind": "expand", "graph_id": GRAPH, "seeds": [unit], "hops": 2, "direction": "both"});
        let plan = json!({"kind": "nearest", "graph_id": GRAPH, "space": {"id": "hw-notes", "revision": "1"},
            "query": {"kind": "vector", "vector": incident.vector}, "k": 10, "valid_at": opened,
            "candidates": neighborhood});
        for who in [ADMIN, ANALYST] {
            let got = evaluate(&mut engine, who, &plan);
            // Oracle: the reader's 2-hop region (A3's BFS-checked `expand`),
            // ranked independently over the vectors this test wrote.
            let region: BTreeSet<String> = evaluate(&mut engine, who, &neighborhood)
                .graph
                .nodes
                .iter()
                .map(|n| n.id.clone())
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
            let got_hits: Vec<(f64, String)> = got
                .neighbors
                .iter()
                .map(|n| (n.score, n.embedding_id.clone()))
                .collect();
            let want_hits: Vec<(f64, String)> =
                want.iter().map(|(s, _, id)| (*s, id.clone())).collect();
            assert_eq!(got_hits, want_hits, "incident {} as {who}", incident.id);
            let hidden_unit = restricted.contains(&incident.serial);
            if who == ANALYST {
                for n in &got.neighbors {
                    let v = notes.iter().find(|v| v.id == n.embedding_id).unwrap();
                    assert!(
                        !restricted.contains(&v.serial),
                        "restricted unit stays hidden"
                    );
                }
                if hidden_unit {
                    assert!(
                        got.neighbors.is_empty(),
                        "a restricted unit's neighborhood is hidden"
                    );
                }
            } else {
                restricted_seen_by_admin += usize::from(hidden_unit && !got.neighbors.is_empty());
                assert!(
                    !got.neighbors.is_empty(),
                    "the incident's own unit has notes"
                );
            }
            // Every hit is a note valid at the incident time, explained by its sources.
            for n in &got.neighbors {
                let v = notes.iter().find(|v| v.id == n.embedding_id).unwrap();
                assert!(v.valid_at(*opened));
                let subject = got
                    .graph
                    .nodes
                    .iter()
                    .find(|x| x.id == v.subject)
                    .expect("hit subject");
                assert!(subject.id.starts_with("maint:") || subject.id.starts_with("incident:"));
                assert!(
                    got.graph
                        .attachments
                        .iter()
                        .any(|a| a.id == format!("note:{}", v.subject)),
                    "the note text is returned with its hit"
                );
                let entry = got
                    .fact_provenance
                    .iter()
                    .find(|p| matches!(&p.fact, FactRef::Embedding { embedding_id, .. } if embedding_id == &n.embedding_id))
                    .expect("embedding provenance");
                assert_eq!(entry.provenance.writer, EMBEDDER);
                assert!(
                    matches!(&entry.provenance.method, Method::Model { model, .. } if model == ENCODER)
                );
            }
            answers.push((
                plan.clone(),
                who,
                got.neighbors
                    .iter()
                    .map(|n| n.embedding_id.clone())
                    .collect(),
            ));
            checked += 1;
        }
    }
    assert!(checked >= 2);
    assert!(
        restricted_seen_by_admin > 0,
        "the admin sees a restricted unit's notes"
    );

    // ---- Question 4.
    let at = updates_end();
    let plan = json!({"kind": "similar_candidates", "graph_id": GRAPH, "space": {"id": "hw-units", "revision": "1"},
        "k": 1, "valid_at": at, "exclude_linked": true, "limit": 100_000});
    for who in [ADMIN, ANALYST] {
        let got = evaluate(&mut engine, who, &plan);
        // Oracle: each visible unit's nearest other visible unit, as an
        // unordered pair, ordered by score (units are never linked directly).
        let visible: Vec<&Vector> = units
            .iter()
            .filter(|v| admits(&v.readers, who) && v.valid_at(at))
            .collect();
        let mut pairs: BTreeMap<(String, String), f64> = BTreeMap::new();
        for s in &visible {
            let best = visible
                .iter()
                .filter(|t| t.subject != s.subject)
                .map(|t| {
                    (
                        distance(&s.vector, &t.vector),
                        t.subject.clone(),
                        t.id.clone(),
                        t,
                    )
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)))
                .unwrap();
            let (lo, hi) = if s.subject <= best.3.subject {
                (s.subject.clone(), best.3.subject.clone())
            } else {
                (best.3.subject.clone(), s.subject.clone())
            };
            pairs.insert((lo, hi), best.0);
        }
        let mut want: Vec<(f64, String, String)> =
            pairs.into_iter().map(|((a, b), s)| (s, a, b)).collect();
        want.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let got_pairs: Vec<(f64, String, String)> = got
            .candidates
            .iter()
            .map(|c| (c.score, c.from.clone(), c.to.clone()))
            .collect();
        assert_eq!(got_pairs, want, "candidates as {who}");
        let two = &got.candidates[..2];
        for candidate in two {
            assert_eq!(candidate.predicate, "weave:similar_to");
            // Absent from the graph: no edge joins the two units, either way.
            for (from, to) in [
                (&candidate.from, &candidate.to),
                (&candidate.to, &candidate.from),
            ] {
                let host = HostContext::new(ADMIN, Vec::<String>::new());
                let read = engine
                    .execute(
                        &program(json!([{"op": "query", "query": {"graph_id": GRAPH, "from": from, "to": to}}])),
                        &host,
                    )
                    .unwrap();
                let CommandResult::Queried { result } = &read[0] else {
                    panic!()
                };
                assert!(
                    result.graph.edges.is_empty(),
                    "{from} -> {to} is not in the graph"
                );
            }
            // With provenance: both origins are embedding facts written by the embedder.
            assert_eq!(candidate.origins.len(), 2);
            for origin in &candidate.origins {
                let entry = got
                    .fact_provenance
                    .iter()
                    .find(|p| &p.fact == origin)
                    .expect("origin provenance");
                assert_eq!(entry.provenance.writer, EMBEDDER);
                assert!(matches!(entry.provenance.method, Method::Model { .. }));
            }
            if who == ANALYST {
                for end in [&candidate.from, &candidate.to] {
                    assert!(!restricted.iter().any(|s| &format!("unit:{s}") == end));
                }
            }
        }
        println!(
            "question 4 as {who}: {} and {}",
            serde_json::to_string(&two[0]).unwrap(),
            serde_json::to_string(&two[1]).unwrap()
        );
        answers.push((
            plan.clone(),
            who,
            got.candidates.iter().map(|c| c.id.clone()).collect(),
        ));
        if who == ANALYST {
            assert!(got.candidates.iter().all(|c| ![&c.from, &c.to]
                .iter()
                .any(|end| restricted.iter().any(|s| &&format!("unit:{s}") == end))));
        }
    }
    // Candidates are never written.
    let head = engine.head(GRAPH, "main").unwrap();
    // Answers are reproducible after reopening the store.
    drop(engine);
    let mut reopened = Engine::open_with_clock(&db, clock.clone()).unwrap();
    assert_eq!(reopened.head(GRAPH, "main").unwrap(), head);
    for (plan, who, ids) in answers {
        let got = evaluate(&mut reopened, who, &plan);
        let again: Vec<String> = if plan["kind"] == "nearest" {
            got.neighbors
                .iter()
                .map(|n| n.embedding_id.clone())
                .collect()
        } else {
            got.candidates.iter().map(|c| c.id.clone()).collect()
        };
        assert_eq!(again, ids);
    }
}
