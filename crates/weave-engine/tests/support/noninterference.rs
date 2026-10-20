//! Generated database pairs for the B2 noninterference harness (ADR-0008
//! section 7). Shared by `tests/noninterference.rs` and the science crate's
//! harness through `#[path]`.
//!
//! A [`Spec`] is drawn from one seed. [`World::build`] turns it into a store
//! twice: once with only the visible facts and once with the same visible
//! facts plus facts that the readers under test (`alice`, `carol`) cannot see.
//! The hidden facts touch visible records wherever they can: edges between
//! visible nodes, attachments and embeddings on visible nodes and entities,
//! assertions on visible structural edges, nodes of visible entities, exact
//! duplicates of visible vectors, corrections over time, a graph only
//! `mallory` can discover, and (in registry worlds) group-restricted records,
//! nested groups and registry state the readers cannot see.
//!
//! Revision IDs are content-derived over every fact, so a store with hidden
//! facts has other revision IDs (the documented exception of A3). Every
//! revision is therefore given a *label*: the visible state it represents.
//! Plans name revisions by label, each store substitutes its own revision,
//! and outputs are compared with revisions replaced by their labels.
#![allow(dead_code)]

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use weave_contract::{CommandResult, FactChange, Program, VERSION};
use weave_engine::{
    Engine, HostContext, ManualClock, RegistryChange, ReviewMode, ReviewProposalReceipt,
    ReviewVerdict, ReviewVote,
};

/// SplitMix64: deterministic on every platform.
#[derive(Clone)]
pub struct Rng(pub u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    pub fn u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, bound: u64) -> u64 {
        self.u64() % bound.max(1)
    }
    pub fn chance(&mut self, numerator: u64, denominator: u64) -> bool {
        self.below(denominator) < numerator
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
    pub fn pick_str(&mut self, items: &[&str]) -> String {
        (*self.pick(items)).to_string()
    }
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as u64) as i64
    }
}

/// The readers whose output must not depend on hidden facts.
pub const READERS: [&str; 2] = ["alice", "carol"];
/// An agent owned by `alice` in registry worlds, acting with all of its
/// owner's identities on the visible graphs (ADR-0007 section 10, item 5).
pub const AGENT: &str = "alice-bot";

/// The readers under test of a world: in registry worlds also `alice`'s agent.
pub fn readers(profile: Profile) -> Vec<&'static str> {
    let mut readers = READERS.to_vec();
    if profile == Profile::Registry {
        readers.push(AGENT);
    }
    readers
}
/// The principal who can see the hidden facts (sensitivity checks).
pub const HIDDEN_READER: &str = "mallory";
/// Visible-world entities and spaces.
pub const ENTITIES: [&str; 4] = ["E0", "E1", "E2", "E3"];
pub const SPACES: [&str; 2] = ["s0", "s1"];
pub const PREDICATES: [&str; 3] = ["p", "q", "twin"];
pub const KEYS: [&str; 2] = ["note", "tag"];
pub const DIMS: usize = 3;
/// A visible node of `g` with no visible dependents and one hidden one.
pub const LONELY: &str = "lonely";
/// Recorded times of the visible steps. Hidden facts are written as their own
/// revision right after each visible step, at the same recorded time, so every
/// time and every label selects one revision per store (see `World::alias`).
pub const STEP_TIMES: [i64; 4] = [10, 20, 30, 40];
pub const SNAP_TIMES: [i64; 2] = [15, 35];
pub const FINAL_TIME: i64 = 1_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Profile {
    /// The local trusted-host profile: literal readers, no registry.
    Local,
    /// A registry store (B1): group readers, graph ACLs and Discover.
    Registry,
}

/// One generated pair description. Everything a plan generator may name is
/// here, identical for both stores of the pair.
#[derive(Clone)]
pub struct Spec {
    pub seed: u64,
    pub profile: Profile,
    /// Fact writes to graph `g`: (time, visible changes, hidden changes).
    pub steps: Vec<(i64, Vec<Value>, Vec<Value>)>,
    /// Snapshot commits to graph `snap`: (time, visible data, hidden records).
    pub snaps: Vec<(i64, Value, Value)>,
    pub pub_data: Value,
    pub visible_nodes: Vec<String>,
    pub hidden_nodes: Vec<String>,
    pub visible_edges: Vec<String>,
    pub hidden_edges: Vec<String>,
    pub visible_embeddings: Vec<String>,
    pub hidden_embeddings: Vec<String>,
    pub vectors: Vec<Vec<f32>>,
    pub snap_nodes: Vec<String>,
    pub geometry_visible: Vec<String>,
    pub geometry_hidden: Vec<String>,
    pub snap_records: Vec<String>,
    pub snap_hidden_records: Vec<String>,
    /// The final version of each visible node of `g`.
    pub node_records: BTreeMap<String, Value>,
}

fn s(v: &str) -> String {
    v.to_string()
}
fn assert_change(fact: Value) -> Value {
    json!({"change": "assert", "fact": fact})
}
fn correct_change(fact: Value) -> Value {
    json!({"change": "correct", "fact": fact})
}

impl Spec {
    pub fn new(seed: u64, profile: Profile) -> Self {
        let mut rng = Rng::new(seed);
        let mut spec = Spec {
            seed,
            profile,
            steps: vec![],
            snaps: vec![],
            pub_data: json!({}),
            visible_nodes: vec![],
            hidden_nodes: vec![],
            visible_edges: vec![],
            hidden_edges: vec![],
            visible_embeddings: vec![],
            hidden_embeddings: vec![],
            vectors: vec![],
            snap_nodes: vec![],
            geometry_visible: vec![],
            geometry_hidden: vec![],
            snap_records: vec![],
            snap_hidden_records: vec![],
            node_records: BTreeMap::new(),
        };
        // Node records keyed by ID so corrections keep their structural identity.
        let mut nodes: BTreeMap<String, Value> = BTreeMap::new();
        let mut hidden_nodes: BTreeMap<String, Value> = BTreeMap::new();
        for (step, &time) in STEP_TIMES.iter().enumerate() {
            let mut visible = vec![];
            let mut hidden = vec![];
            if step == 0 {
                for (space, metric) in [("sp", "euclidean"), ("sc", "cosine_distance")] {
                    visible.push(assert_change(
                        json!({"kind": "embedding_space", "id": space,
                        "revision": "1", "encoder": "test", "encoder_version": "v1",
                        "preprocessing": "none", "dimensions": DIMS, "metric": metric}),
                    ));
                }
            }
            // Visible nodes.
            let count = if step == 0 {
                6
            } else {
                rng.range(0, 2) as usize
            };
            for _ in 0..count {
                let id = format!("v{}", spec.visible_nodes.len());
                let entity = rng.pick_str(&ENTITIES);
                let space = rng.pick_str(&SPACES);
                let mut node = json!({"kind": "node", "id": id, "entity_id": entity,
                    "space_id": space, "properties": {"x": rng.range(0, 9),
                    "vector": spec.random_vector(&mut rng)}});
                spec.restrict_visible(&mut rng, &mut node);
                if rng.chance(1, 5) {
                    let start = rng.range(0, 30);
                    node["valid_time"] = json!({"start": start, "end": start + rng.range(5, 30)});
                }
                nodes.insert(id.clone(), node.clone());
                visible.push(assert_change(node));
                spec.visible_nodes.push(id);
            }
            // A correction of a visible node (same readers, other properties).
            if step > 0 && rng.chance(1, 2) {
                let earlier = spec.visible_nodes.len() - count;
                let id = spec.visible_nodes[rng.below(earlier as u64) as usize].clone();
                let mut node = nodes[&id].clone();
                node["properties"]["x"] = json!(rng.range(10, 19));
                nodes.insert(id, node.clone());
                visible.push(correct_change(node));
            }
            // Visible edges, attachments and embeddings.
            for _ in 0..rng.range(1, 4) {
                let id = format!("e{}", spec.visible_edges.len());
                let (from, to, predicate) = spec.edge_ends(&mut rng, &nodes);
                let mut edge = json!({"kind": "edge", "id": id, "predicate": predicate,
                    "from": from, "to": to, "valid_time": spec.interval(&mut rng)});
                spec.restrict_visible(&mut rng, &mut edge);
                visible.push(assert_change(edge));
                spec.visible_edges.push(id);
            }
            for j in 0..rng.range(0, 2) {
                let id = format!("a{step}{j}");
                let mut attachment = json!({"kind": "attachment", "id": id,
                    "host": spec.attachment_host(&mut rng, &spec.visible_nodes, &nodes),
                    "key": rng.pick_str(&KEYS), "value": {"kind": "literal", "value": rng.range(0, 99)},
                    "valid_time": spec.interval(&mut rng)});
                spec.restrict_visible(&mut rng, &mut attachment);
                visible.push(assert_change(attachment));
            }
            for _ in 0..rng.range(1, 3) {
                let id = format!("m{}", spec.visible_embeddings.len());
                let vector = spec.random_vector(&mut rng);
                spec.vectors.push(vector.clone());
                let subject = rng.pick(&spec.visible_nodes).clone();
                let mut embedding = embedding(&id, &rng.pick_str(&["sp", "sc"]), &subject, &vector);
                embedding["valid_time"] = spec.interval(&mut rng);
                spec.restrict_visible(&mut rng, &mut embedding);
                visible.push(assert_change(embedding));
                spec.visible_embeddings.push(id);
            }
            // Hidden facts, written as a separate revision after the visible step.
            let hid = format!("h{step}");
            let mut node = json!({"kind": "node", "id": hid, "entity_id": rng.pick_str(&ENTITIES),
                "space_id": rng.pick_str(&SPACES), "properties": {"x": rng.range(0, 9),
                "vector": spec.random_vector(&mut rng)}});
            spec.restrict_hidden(&mut rng, &mut node);
            hidden_nodes.insert(hid.clone(), node.clone());
            hidden.push(assert_change(node));
            spec.hidden_nodes.push(hid.clone());
            if step == 0 {
                // A public node that only a hidden edge depends on, so the
                // write harness always reaches the dependent refusal.
                visible.push(assert_change(json!({"kind": "node", "id": LONELY,
                    "entity_id": "lonely", "space_id": "s0"})));
                let mut edge = json!({"kind": "edge", "id": "he-lonely", "predicate": "p",
                    "from": LONELY, "to": hid, "valid_time": {"start": 0}});
                spec.restrict_hidden(&mut rng, &mut edge);
                hidden.push(assert_change(edge));
                spec.hidden_edges.push("he-lonely".into());
            }
            if step > 0 && rng.chance(1, 2) {
                let previous = format!("h{}", step - 1);
                let mut node = hidden_nodes[&previous].clone();
                node["properties"]["x"] = json!(rng.range(20, 29));
                hidden_nodes.insert(previous, node.clone());
                hidden.push(correct_change(node));
            }
            let all: Vec<String> = spec
                .visible_nodes
                .iter()
                .chain(spec.hidden_nodes.iter())
                .cloned()
                .collect();
            let mut all_nodes = nodes.clone();
            all_nodes.extend(hidden_nodes.clone());
            for j in 0..rng.range(2, 4) {
                let id = format!("he{step}{j}");
                // Mostly between visible nodes, so a hidden edge sits right
                // next to visible topology.
                let (from, to, predicate) = if j == 0 {
                    let from = rng.pick(&spec.visible_nodes).clone();
                    (from, hid.clone(), rng.pick_str(&PREDICATES))
                } else if rng.chance(2, 3) {
                    spec.edge_ends(&mut rng, &nodes)
                } else {
                    spec.edge_ends(&mut rng, &all_nodes)
                };
                let _ = &all;
                let mut edge = json!({"kind": "edge", "id": id, "predicate": predicate,
                    "from": from, "to": to, "valid_time": spec.interval(&mut rng)});
                spec.restrict_hidden(&mut rng, &mut edge);
                hidden.push(assert_change(edge));
                spec.hidden_edges.push(id);
            }
            for j in 0..2 {
                let id = format!("ha{step}{j}");
                let mut attachment = json!({"kind": "attachment", "id": id,
                    "host": spec.attachment_host(&mut rng, &spec.visible_nodes, &nodes),
                    "key": rng.pick_str(&KEYS), "value": {"kind": "literal", "value": rng.range(100, 199)},
                    "valid_time": spec.interval(&mut rng)});
                spec.restrict_hidden(&mut rng, &mut attachment);
                hidden.push(assert_change(attachment));
            }
            for j in 0..rng.range(1, 3) {
                let id = format!("hm{step}{j}");
                // Exact duplicates of visible vectors tie with them.
                let vector = if !spec.vectors.is_empty() && rng.chance(2, 3) {
                    rng.pick(&spec.vectors).clone()
                } else {
                    spec.random_vector(&mut rng)
                };
                let space = rng.pick_str(&["sp", "sc"]);
                let mut embedding = if rng.chance(1, 2) {
                    // A hidden embedding on a visible node.
                    let mut e = embedding(&id, &space, rng.pick(&spec.visible_nodes), &vector);
                    spec.restrict_hidden(&mut rng, &mut e);
                    e
                } else {
                    // A public embedding on a hidden node.
                    embedding(&id, &space, &hid, &vector)
                };
                embedding["valid_time"] = spec.interval(&mut rng);
                hidden.push(assert_change(embedding));
                spec.hidden_embeddings.push(id);
            }
            spec.steps.push((time, visible, hidden));
        }
        spec.node_records = nodes;
        spec.snapshots(&mut rng);
        spec.pub_data = json!({
            "nodes": [
                {"id": "p0", "entity_id": "E0", "space_id": "s0"},
                {"id": "p1", "entity_id": "E1", "space_id": "s0"},
                {"id": "p2", "entity_id": "E1", "space_id": "s1"}
            ],
            "edges": [
                {"id": "pe0", "predicate": "p", "from": "p0", "to": "p1", "valid_time": {"start": 0}},
                {"id": "pe1", "predicate": "twin", "from": "p1", "to": "p2", "valid_time": {"start": 0, "end": 30}}
            ]
        });
        spec
    }

    pub fn random_vector(&self, rng: &mut Rng) -> Vec<f32> {
        (0..DIMS).map(|_| rng.range(1, 4) as f32).collect()
    }
    pub fn interval(&self, rng: &mut Rng) -> Value {
        let start = rng.range(0, 25);
        if rng.chance(1, 3) {
            json!({"start": start})
        } else {
            json!({"start": start, "end": start + rng.range(3, 25)})
        }
    }
    fn edge_ends(
        &self,
        rng: &mut Rng,
        nodes: &BTreeMap<String, Value>,
    ) -> (String, String, String) {
        let ids: Vec<&String> = nodes.keys().collect();
        let from = (*rng.pick(&ids)).clone();
        // A quarter of the edges link two manifestations of one entity
        // across spaces, for counterparts, joins and sequences.
        if rng.chance(1, 4) {
            let entity = &nodes[&from]["entity_id"];
            let space = &nodes[&from]["space_id"];
            let twins: Vec<&String> = ids
                .iter()
                .copied()
                .filter(|id| &nodes[*id]["entity_id"] == entity && &nodes[*id]["space_id"] != space)
                .collect();
            if !twins.is_empty() {
                return (from, (*rng.pick(&twins)).clone(), s("twin"));
            }
        }
        let to = (*rng.pick(&ids)).clone();
        (from, to, rng.pick_str(&["p", "q"]))
    }
    fn attachment_host(
        &self,
        rng: &mut Rng,
        ids: &[String],
        nodes: &BTreeMap<String, Value>,
    ) -> Value {
        if rng.chance(1, 3) {
            let id = rng.pick(ids);
            json!({"kind": "entity", "id": nodes[id]["entity_id"]})
        } else {
            json!({"kind": "node", "id": rng.pick(ids)})
        }
    }
    /// A restriction the readers under test may or may not satisfy.
    pub fn restrict_visible(&self, rng: &mut Rng, record: &mut Value) {
        let choice = rng.below(if self.profile == Profile::Registry {
            8
        } else {
            6
        });
        match choice {
            0..=2 => {}
            3 => record["readers"] = self.with_writer(&["alice"]),
            4 => record["readers"] = self.with_writer(&["bob"]),
            5 => record["readers"] = self.with_writer(&["alice", "bob"]),
            6 => record["reader_groups"] = json!(["team"]),
            _ => record["reader_groups"] = json!(["crew"]),
        }
    }
    /// Restricted visible records also list `mallory`, who therefore sees
    /// every record of both stores and so detects any hidden fact that reaches
    /// an output. The store's trusted writer is listed too: in a local store
    /// so the governance proposer can see whole revisions, and in a registry
    /// store because correcting a fact needs Read on it (#70).
    fn with_writer(&self, readers: &[&str]) -> Value {
        let mut readers: Vec<&str> = readers.to_vec();
        readers.push("mallory");
        readers.push(match self.profile {
            Profile::Local => "writer",
            Profile::Registry => "admin",
        });
        json!(readers)
    }
    /// A restriction neither reader under test satisfies, but `mallory` does.
    /// The trusted writers are listed too, so governance and identity setup
    /// may consume hidden records exactly as they consume visible ones.
    pub fn restrict_hidden(&self, rng: &mut Rng, record: &mut Value) {
        match (self.profile, rng.below(3)) {
            (Profile::Local, _) => record["readers"] = json!(["mallory", "writer"]),
            (Profile::Registry, 0) => record["readers"] = json!(["mallory", "admin"]),
            (Profile::Registry, 1) => record["reader_groups"] = json!(["vault"]),
            // A group nested in `vault`, which exists only with the hidden facts.
            (Profile::Registry, _) => record["reader_groups"] = json!(["ghosts"]),
        }
    }

    fn snapshots(&mut self, rng: &mut Rng) {
        let world = json!({"id": "world", "revision": "1",
            "geometry": {"kind": "physical3d", "frame": "world", "unit": "metre"}});
        let mut nodes = vec![];
        let mut structural = vec![];
        let mut assertions = vec![];
        let mut attachments = vec![];
        let mut hidden_nodes = vec![];
        let mut hidden_structural = vec![];
        let mut hidden_assertions = vec![];
        let mut hidden_attachments = vec![];
        for i in 0..5 {
            let id = format!("w{i}");
            let mut node = json!({"id": id, "entity_id": ENTITIES[i % 4], "space_id": "world"});
            if i >= 3 {
                self.restrict_visible(rng, &mut node);
            }
            nodes.push(node);
            self.snap_nodes.push(id.clone());
            structural.push(
                json!({"id": format!("sw{i}"), "predicate": "coordinates", "from": id, "to": id}),
            );
            let valid = if rng.chance(1, 3) {
                self.interval(rng)
            } else {
                json!({"start": 0})
            };
            let mut point = json!({"id": format!("g{i}"), "edge_id": format!("sw{i}"), "source": "survey",
                "valid_time": valid,
                "properties": {"weave.geometry": {"kind": "coordinates", "role": "position",
                    "values": [rng.range(0, 5), rng.range(0, 5), 0], "space": world}}});
            if rng.chance(1, 3) {
                self.restrict_visible(rng, &mut point);
            }
            assertions.push(point);
            self.geometry_visible.push(format!("g{i}"));
            // A hidden second position on the same visible structural edge.
            let mut hidden = json!({"id": format!("hg{i}"), "edge_id": format!("sw{i}"), "source": "spy",
                "valid_time": {"start": 0},
                "properties": {"weave.geometry": {"kind": "coordinates", "role": "position",
                    "values": [rng.range(0, 5), rng.range(0, 5), 1], "space": world}}});
            self.restrict_hidden(rng, &mut hidden);
            hidden_assertions.push(hidden);
            self.geometry_hidden.push(format!("hg{i}"));
        }
        for j in 0..4 {
            let from = format!("w{}", rng.below(5));
            let to = format!("w{}", rng.below(5));
            let sid = format!("sr{j}");
            let mut edge =
                json!({"id": sid, "predicate": rng.pick_str(&["p", "q"]), "from": from, "to": to});
            if j == 3 {
                self.restrict_visible(rng, &mut edge);
            }
            structural.push(edge);
            for k in 0..2 {
                let id = format!("c{j}{k}");
                let mut claim = json!({"id": id, "edge_id": sid, "source": format!("sensor{k}"),
                    "polarity": rng.pick_str(&["positive", "positive", "negative"]),
                    "valid_time": self.interval(rng)});
                if k == 1 {
                    self.restrict_visible(rng, &mut claim);
                }
                assertions.push(claim);
                self.snap_records.push(id);
            }
            // Hidden claims on visible structural edges.
            let id = format!("hc{j}");
            let mut claim = json!({"id": id, "edge_id": sid, "source": "spy",
                "polarity": rng.pick_str(&["positive", "negative"]), "valid_time": self.interval(rng)});
            self.restrict_hidden(rng, &mut claim);
            hidden_assertions.push(claim);
            self.snap_hidden_records.push(id);
        }
        // A hidden node of a visible entity, hidden structure to it.
        let mut node = json!({"id": "wh", "entity_id": "E1", "space_id": "world"});
        self.restrict_hidden(rng, &mut node);
        hidden_nodes.push(node);
        let mut edge = json!({"id": "srh", "predicate": "p", "from": "w0", "to": "wh"});
        self.restrict_hidden(rng, &mut edge);
        hidden_structural.push(edge);
        let mut claim =
            json!({"id": "hch", "edge_id": "srh", "source": "spy", "valid_time": {"start": 0}});
        self.restrict_hidden(rng, &mut claim);
        hidden_assertions.push(claim);
        self.snap_hidden_records.push(s("hch"));
        for i in 0..3 {
            let mut attachment = json!({"id": format!("sa{i}"), "host": {"kind": "node", "id": format!("w{i}")},
                "key": rng.pick_str(&KEYS), "value": {"kind": "literal", "value": i}, "valid_time": {"start": 0}});
            if i == 2 {
                self.restrict_visible(rng, &mut attachment);
            }
            attachments.push(attachment);
            let mut hidden = json!({"id": format!("hsa{i}"), "host": {"kind": "node", "id": format!("w{i}")},
                "key": rng.pick_str(&KEYS), "value": {"kind": "literal", "value": 100 + i}, "valid_time": {"start": 0}});
            self.restrict_hidden(rng, &mut hidden);
            hidden_attachments.push(hidden);
        }
        // A graph-valued attachment, so `metadata` resolves a graph (filled by
        // the builder with the head of `pub`).
        attachments.push(json!({"id": "proof", "host": {"kind": "node", "id": "w0"}, "key": "proof",
            "value": {"kind": "graph", "reference": {"graph_id": "pub", "revision": "~P1~"}}, "valid_time": {"start": 0}}));
        let visible = json!({"profile": "explicit", "nodes": nodes, "structural_edges": structural,
            "assertions": assertions, "attachments": attachments});
        let hidden = json!({"nodes": hidden_nodes, "structural_edges": hidden_structural,
            "assertions": hidden_assertions, "attachments": hidden_attachments});
        self.snaps
            .push((SNAP_TIMES[0], visible.clone(), hidden.clone()));
        // The second revision adds one node and one claim to each side.
        let mut visible2 = visible;
        visible2["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "w5", "entity_id": "E2", "space_id": "world"}));
        visible2["assertions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "c40", "edge_id": "sr0",
            "source": "late", "valid_time": {"start": 5}}));
        self.snap_records.push(s("c40"));
        let mut hidden2 = hidden;
        let mut late =
            json!({"id": "hc40", "edge_id": "sr1", "source": "spy", "valid_time": {"start": 5}});
        self.restrict_hidden(rng, &mut late);
        hidden2["assertions"].as_array_mut().unwrap().push(late);
        self.snap_hidden_records.push(s("hc40"));
        self.snaps.push((SNAP_TIMES[1], visible2, hidden2));
    }
}

pub fn embedding(id: &str, space: &str, node: &str, vector: &[f32]) -> Value {
    json!({"kind": "embedding", "id": id, "space": {"id": space, "revision": "1"},
        "subject": {"kind": "node", "node_id": node}, "vector": vector,
        "provenance": {"method": {"kind": "model", "model": "test", "version": "v1"}}})
}

/// One store of a pair, with its revision labels.
pub struct World {
    pub e: Engine,
    pub clock: Arc<ManualClock>,
    pub hidden: bool,
    pub profile: Profile,
    pub writer: HostContext,
    /// Label -> this store's string (revision, decision, digest, ...).
    pub tokens: BTreeMap<String, String>,
    /// The latest label of each graph's visible state.
    last: BTreeMap<String, String>,
    /// Label -> the latest of this store's revisions with that visible state.
    latest: BTreeMap<String, String>,
    fakes: u64,
}

pub fn program(commands: Value) -> Program {
    serde_json::from_value(json!({"version": VERSION, "commands": commands})).unwrap()
}

fn revision_of(results: weave_engine::Result<Vec<CommandResult>>, what: &str) -> String {
    match results {
        Ok(results) => match results.last() {
            Some(CommandResult::Committed { revision, .. })
            | Some(CommandResult::Unchanged { revision }) => revision.clone(),
            other => panic!("{what}: {other:?}"),
        },
        Err(error) => panic!("{what}: {error}"),
    }
}

impl World {
    pub fn build(spec: &Spec, hidden: bool) -> Self {
        let clock = Arc::new(ManualClock::new(1));
        let e = Engine::memory_with_clock(clock.clone()).unwrap();
        let writer = match spec.profile {
            Profile::Local => HostContext::new(
                "writer",
                [
                    "g",
                    "snap",
                    "pub",
                    "ctx",
                    "ctxh",
                    "physical",
                    "operations",
                    "private",
                ]
                .map(String::from),
            ),
            Profile::Registry => HostContext::new("admin", Vec::<String>::new()),
        };
        let mut world = World {
            e,
            clock,
            hidden,
            profile: spec.profile,
            writer,
            tokens: BTreeMap::new(),
            last: BTreeMap::new(),
            latest: BTreeMap::new(),
            fakes: 0,
        };
        if spec.profile == Profile::Registry {
            world.registry(hidden);
        }
        world.at(5);
        let r = world.run(json!([{"op": "commit", "graph_id": "pub", "data": spec.pub_data}]));
        world.label("pub", "P1", r);
        world.descriptor("ctx", "C1", json!([]), true);
        let hidden_descriptor = match spec.profile {
            Profile::Local => json!(["mallory", "writer"]),
            Profile::Registry => json!(["mallory", "admin"]),
        };
        world.descriptor("ctxh", "CH1", hidden_descriptor, hidden);
        let mut snaps = spec.snaps.iter().enumerate().peekable();
        for (step, (time, visible, hidden_changes)) in spec.steps.iter().enumerate() {
            while let Some((k, (snap_time, data, extra))) = snaps.peek() {
                if snap_time > time {
                    break;
                }
                let (k, snap_time) = (*k, *snap_time);
                world.at(snap_time);
                let mut data = (*data).clone();
                if hidden {
                    for field in ["nodes", "structural_edges", "assertions", "attachments"] {
                        let extra = extra[field].as_array().cloned().unwrap_or_default();
                        data[field].as_array_mut().unwrap().extend(extra);
                    }
                }
                let data = world.substitute(&data);
                let head = world.e.head("snap", "main").unwrap();
                let r = world.run(json!([{"op": "commit", "graph_id": "snap", "expected_head": head, "data": data}]));
                world.label("snap", &format!("S{}", k + 1), r);
                snaps.next();
            }
            world.at(*time);
            let r = world.run(json!([{"op": "write", "graph_id": "g", "changes": visible}]));
            world.label("g", &format!("R{}", step + 1), r);
            if hidden {
                world.at(*time);
                let r =
                    world.run(json!([{"op": "write", "graph_id": "g", "changes": hidden_changes}]));
                world.relabel("g", r);
            }
        }
        for (k, (snap_time, data, extra)) in snaps {
            world.at(*snap_time);
            let mut data = data.clone();
            if hidden {
                for field in ["nodes", "structural_edges", "assertions", "attachments"] {
                    let extra = extra[field].as_array().cloned().unwrap_or_default();
                    data[field].as_array_mut().unwrap().extend(extra);
                }
            }
            let data = world.substitute(&data);
            let head = world.e.head("snap", "main").unwrap();
            let r = world.run(
                json!([{"op": "commit", "graph_id": "snap", "expected_head": head, "data": data}]),
            );
            world.label("snap", &format!("S{}", k + 1), r);
        }
        if spec.profile == Profile::Registry {
            // A graph only `mallory` can discover, with data.
            world.at(48);
            if hidden {
                let r = world.run(json!([{"op": "write", "graph_id": "vault", "changes": [
                    assert_change(json!({"kind": "node", "id": "v0", "entity_id": "E0", "space_id": "s0"})),
                    assert_change(json!({"kind": "embedding_space", "id": "sp", "revision": "1", "encoder": "test",
                        "encoder_version": "v1", "preprocessing": "none", "dimensions": DIMS, "metric": "euclidean"}))
                ]}]));
                world.label("vault", "VAULT1", r);
            } else {
                world.fake("VAULT1", true);
            }
        }
        world.at(50);
        world
    }

    /// B5 review in registry worlds: graph `g` becomes reviewed by `admin`.
    /// Both stores publish one accepted proposal from `alice` (public facts
    /// with review provenance). The store with hidden facts also has a
    /// hidden accepted proposal and pending, rejected and withdrawn
    /// proposals whose facts are *public*: proposals that were not accepted
    /// are hidden input for every reader of the graph.
    pub fn review(&mut self, spec: &Spec) {
        if self.profile != Profile::Registry {
            return;
        }
        let admin = self.writer.clone();
        let alice = HostContext::new("alice", Vec::<String>::new());
        self.at(60);
        let mode = ReviewMode::Reviewed {
            reviewers: vec!["admin".into()],
            threshold: 1,
        };
        self.e.set_review_policy("g", &mode, 0, &admin).unwrap();
        let changes = |facts: Value| -> Vec<FactChange> {
            let changes: Vec<Value> = facts
                .as_array()
                .unwrap()
                .iter()
                .map(|f| json!({"change": "assert", "fact": f}))
                .collect();
            serde_json::from_value(Value::Array(changes)).unwrap()
        };
        let accept = |world: &mut World, receipt: &ReviewProposalReceipt, verdict| {
            world
                .e
                .vote_review(
                    &ReviewVote {
                        proposal_id: receipt.proposal_id.clone(),
                        digest: receipt.digest.clone(),
                        verdict,
                        note: None,
                    },
                    &admin,
                )
                .unwrap()
        };
        // `alice` proposes, so references go to the public node she reads.
        let target = LONELY.to_string();
        let vector = spec.vectors[0].clone();
        let facts = |prefix: &str| {
            let mut vector_fact =
                embedding(&format!("{prefix}m"), "sp", &format!("{prefix}n"), &vector);
            vector_fact["valid_time"] = json!({"start": 0});
            json!([
                {"kind": "node", "id": format!("{prefix}n"), "entity_id": "E0", "space_id": "s0",
                    "properties": {"x": 4}},
                {"kind": "edge", "id": format!("{prefix}e"), "predicate": "p",
                    "from": format!("{prefix}n"), "to": target, "valid_time": {"start": 0}},
                vector_fact,
            ])
        };
        let visible = self
            .e
            .propose_review("g", "main", &changes(facts("rv")), &alice)
            .unwrap();
        let accepted = accept(self, &visible, ReviewVerdict::Approve);
        let decision = accepted.decision.unwrap();
        self.label_revision("g", "R5", decision.revision.unwrap());
        self.token("RVP", visible.proposal_id);
        self.token("RVD", decision.decision_id);
        if !self.hidden {
            return;
        }
        // A hidden accepted proposal, at the same recorded time.
        let mut secret = facts("rh");
        for record in secret.as_array_mut().unwrap() {
            record["readers"] = json!(["mallory", "admin"]);
        }
        let hidden = self
            .e
            .propose_review("g", "main", &changes(secret), &alice)
            .unwrap();
        let accepted = accept(self, &hidden, ReviewVerdict::Approve);
        let revision = accepted.decision.unwrap().revision.unwrap();
        self.relabel_revision("g", &revision);
        // Public facts that never became accepted facts.
        let pending = self
            .e
            .propose_review("g", "main", &changes(facts("rp")), &alice)
            .unwrap();
        let _ = pending;
        let rejected = self
            .e
            .propose_review("g", "main", &changes(facts("rr")), &alice)
            .unwrap();
        accept(self, &rejected, ReviewVerdict::Reject);
        let withdrawn = self
            .e
            .propose_review("g", "main", &changes(facts("rw")), &alice)
            .unwrap();
        self.e
            .withdraw_review(&withdrawn.proposal_id, &alice)
            .unwrap();
        // Pending changes to visible records: a correction of a visible
        // node and a retraction of a visible edge.
        let public = spec
            .node_records
            .values()
            .find(|n| n.get("readers").is_none() && n.get("reader_groups").is_none());
        if let Some(node) = public {
            let mut corrected = node.clone();
            corrected["properties"]["x"] = json!(77);
            let correction: Vec<FactChange> =
                serde_json::from_value(json!([{"change": "correct", "fact": corrected}])).unwrap();
            self.e
                .propose_review("g", "main", &correction, &alice)
                .unwrap();
        }
        let retraction: Vec<FactChange> = serde_json::from_value(json!([{"change": "retract",
            "target": {"kind": "edge", "id": spec.visible_edges[0]}}]))
        .unwrap();
        // A retraction a dependent refuses is simply not proposed.
        let _ = self.e.propose_review("g", "main", &retraction, &alice);
    }

    fn registry(&mut self, hidden: bool) {
        self.at(2);
        let revision = self.e.enable_registry(&self.writer).unwrap();
        self.tokens.insert("~G0~".into(), revision);
        let mut changes = vec![];
        for user in ["alice", "bob", "carol", "mallory"] {
            changes.push(json!({"change": "add_principal", "id": user, "kind": "user"}));
        }
        for group in ["team", "crew", "vault"] {
            changes.push(json!({"change": "add_principal", "id": group, "kind": "group"}));
        }
        for (group, member) in [
            ("team", "alice"),
            ("team", "bob"),
            ("crew", "alice"),
            ("crew", "carol"),
            ("team", "mallory"),
            ("crew", "mallory"),
            ("vault", "mallory"),
            // The trusted writer reads everything it corrects (#70).
            ("team", "admin"),
            ("crew", "admin"),
            ("vault", "admin"),
        ] {
            changes.push(json!({"change": "add_member", "group": group, "member": member}));
        }
        for graph in ["g", "snap", "pub", "ctx"] {
            for user in ["alice", "bob", "carol", "mallory"] {
                changes.push(json!({"change": "grant", "graph_id": graph, "subject": user, "permission": "read"}));
            }
        }
        // `alice` also writes `g`, for the write-outcome harness. A grant
        // replaces the subject's earlier entry.
        changes.push(
            json!({"change": "grant", "graph_id": "g", "subject": "alice", "permission": "write"}),
        );
        // `alice`'s agent reads the visible graphs with all of `alice`'s
        // identities; its graph scope never names the hidden-only graphs.
        let scopes: Vec<Value> = ["g", "snap", "pub", "ctx"]
            .iter()
            .map(|graph| json!({"graph_id": graph, "permission": "read"}))
            .collect();
        changes.push(json!({"change": "add_agent", "id": AGENT, "owner": "alice",
            "delegation": {"scopes": scopes, "exercise": {"kind": "owner_all"}}}));
        self.apply(changes, "G1");
        if hidden {
            // Registry state the readers cannot discover: a hidden principal,
            // a group nested in `vault`, and grants on hidden-only graphs.
            self.at(2);
            let mut changes = vec![
                json!({"change": "add_principal", "id": "ghost", "kind": "user"}),
                json!({"change": "add_principal", "id": "ghosts", "kind": "group"}),
                json!({"change": "add_member", "group": "ghosts", "member": "mallory"}),
                json!({"change": "add_member", "group": "ghosts", "member": "ghost"}),
                json!({"change": "add_member", "group": "ghosts", "member": "admin"}),
                json!({"change": "add_member", "group": "vault", "member": "ghosts"}),
            ];
            for graph in ["vault", "ctxh"] {
                changes.push(json!({"change": "grant", "graph_id": graph, "subject": "mallory", "permission": "read"}));
            }
            let changes: Vec<RegistryChange> = serde_json::from_value(json!(changes)).unwrap();
            let revision = self.e.apply_registry(&changes, &self.writer).unwrap();
            self.alias(&revision, "G1");
        }
    }
    fn apply(&mut self, changes: Vec<Value>, label: &str) {
        let changes: Vec<RegistryChange> = serde_json::from_value(json!(changes)).unwrap();
        let revision = self.e.apply_registry(&changes, &self.writer).unwrap();
        self.tokens.insert(format!("~{label}~"), revision);
    }
    fn descriptor(&mut self, graph: &str, label: &str, readers: Value, present: bool) {
        if !present {
            self.fake(label, false);
            return;
        }
        let data = json!({"profile": "explicit",
            "nodes": [{"id": "anchor", "entity_id": "world", "space_id": "contexts", "readers": readers}],
            "structural_edges": [{"id": "describes", "predicate": "weave:context:definition", "from": "anchor", "to": "anchor"}],
            "assertions": [{"id": "definition", "edge_id": "describes", "source": "author",
                "valid_time": {"start": i64::MIN}, "properties": {"weave.context": {
                    "schema": context_schema(), "values": {"jurisdiction": "EE"}}}}]});
        let r = self.run(json!([{"op": "commit", "graph_id": graph, "data": data}]));
        self.label(graph, label, r);
    }

    pub fn at(&self, time: i64) {
        self.clock.set(time);
    }
    pub fn run(&mut self, commands: Value) -> weave_engine::Result<Vec<CommandResult>> {
        let host = self.writer.clone();
        self.e.execute(&program(commands), &host)
    }
    /// Record a visible revision under `label`.
    pub fn label(
        &mut self,
        graph: &str,
        label: &str,
        results: weave_engine::Result<Vec<CommandResult>>,
    ) {
        let revision = revision_of(results, label);
        self.tokens.insert(format!("~{label}~"), revision);
        self.last.insert(graph.into(), label.into());
    }
    /// Record a visible revision produced outside a program (an accepted
    /// review proposal, B5) under `label`.
    pub fn label_revision(&mut self, graph: &str, label: &str, revision: String) {
        self.tokens.insert(format!("~{label}~"), revision);
        self.last.insert(graph.into(), label.into());
    }
    /// A revision produced outside a program by hidden facts stands for the
    /// visible state it extends.
    pub fn relabel_revision(&mut self, graph: &str, revision: &str) {
        let label = self.last[graph].clone();
        self.alias(revision, &label);
    }
    /// A revision produced by hidden facts stands for the visible state it extends.
    fn relabel(&mut self, graph: &str, results: weave_engine::Result<Vec<CommandResult>>) {
        let revision = revision_of(results, "hidden write");
        let label = self.last[graph].clone();
        self.alias(&revision, &label);
    }
    /// A later revision of the same visible state. Plans that pin the label
    /// read this one, so a pinned read and a head read of one visible state
    /// name one revision in both stores.
    fn alias(&mut self, actual: &str, label: &str) {
        self.tokens
            .insert(format!("~{label}~#{}", self.tokens.len()), actual.into());
        self.latest.insert(format!("~{label}~"), actual.into());
    }
    /// Record a label that only the store with hidden facts has: the other
    /// store substitutes a well-formed revision that does not exist.
    pub fn fake(&mut self, label: &str, fact: bool) {
        self.fakes += 1;
        let digest = format!("{:064x}", self.fakes);
        let revision = if fact {
            format!("fact:sha256:{digest}")
        } else {
            format!("sha256:{digest}")
        };
        self.tokens.insert(format!("~{label}~"), revision);
    }
    pub fn token(&mut self, label: &str, actual: String) {
        self.tokens.insert(format!("~{label}~"), actual);
    }
    /// Replace every `~LABEL~` in `value` with this store's string.
    pub fn substitute(&self, value: &Value) -> Value {
        let mut text = value.to_string();
        if !text.contains('~') {
            return value.clone();
        }
        for (label, actual) in &self.tokens {
            if label.contains('#') {
                continue;
            }
            let actual = self.latest.get(label).unwrap_or(actual);
            text = text.replace(label.as_str(), actual);
        }
        serde_json::from_str(&text).unwrap()
    }
    /// The output text with every known store string replaced by its label.
    pub fn normalize(&self, text: &str) -> String {
        let mut pairs: Vec<(&String, String)> = self
            .tokens
            .iter()
            .map(|(label, actual)| {
                let label = label.split('#').next().unwrap();
                (actual, label.trim_end_matches('~').to_string() + "~")
            })
            .collect();
        // Longest first, so a revision inside an event ID is replaced whole.
        pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(b.0)));
        let mut text = text.to_string();
        for (actual, label) in pairs {
            text = text.replace(actual.as_str(), &label);
        }
        // Each store has its own random replica identity, and head-observation
        // IDs chain every head change of a line, like revision IDs (ADR-0002;
        // A3 decision "Paired tests normalize revision IDs").
        let text = scrub_hex(&text, "urn:weave:replica:");
        scrub_hex(&text, "observation:")
    }
}

/// True for an engine-generated ID that digests an output value, such as
/// `temporal:edge:sha256:...`: a 64-digit hex run that is not a revision.
fn derived(text: &str) -> bool {
    if text.starts_with("sha256:") || text.starts_with("fact:sha256:") {
        return false;
    }
    let mut run = 0;
    for c in text.chars() {
        if c.is_ascii_hexdigit() && !c.is_ascii_uppercase() {
            run += 1;
            if run >= 64 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// Generated IDs digest the value they identify, origins and so revision IDs
/// included, so they differ whenever revision IDs differ. They are renamed
/// `~D<n>~`, numbered in the order of the records that use them, with every
/// generated ID inside those records blanked first. Two outputs that are equal
/// up to this renaming have the same records, links and multiplicities.
pub fn rename_derived(value: &Value) -> Value {
    /// Where each generated ID occurs: the object holding it as a value, the
    /// value it keys, or the array holding it.
    fn index<'a>(value: &'a Value, out: &mut BTreeMap<String, Vec<(String, &'a Value)>>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    if let Some(s) = item.as_str().filter(|s| derived(s)) {
                        out.entry(s.into()).or_default().push(("in".into(), value));
                    }
                    index(item, out);
                }
            }
            Value::Object(map) => {
                for (k, v) in map {
                    if derived(k) {
                        out.entry(k.clone()).or_default().push(("key".into(), v));
                    }
                    if let Some(s) = v.as_str().filter(|s| derived(s)) {
                        out.entry(s.into())
                            .or_default()
                            .push((format!("{k} of"), value));
                    }
                    index(v, out);
                }
            }
            _ => {}
        }
    }
    /// Canonical text of `value` with each generated ID replaced by its class
    /// and arrays and objects compared as sorted sets of entries.
    fn blank(value: &Value, class: &BTreeMap<String, usize>) -> String {
        match value {
            Value::String(s) if derived(s) => format!("?{}", class[s]),
            Value::Array(items) => {
                let mut texts: Vec<String> = items.iter().map(|v| blank(v, class)).collect();
                texts.sort();
                format!("[{}]", texts.join(","))
            }
            Value::Object(map) => {
                let mut texts: Vec<String> = map
                    .iter()
                    .map(|(k, v)| {
                        let k = if derived(k) {
                            format!("?{}", class[k])
                        } else {
                            k.clone()
                        };
                        format!("{k}:{}", blank(v, class))
                    })
                    .collect();
                texts.sort();
                format!("{{{}}}", texts.join(","))
            }
            other => other.to_string(),
        }
    }
    let mut contexts = BTreeMap::new();
    index(value, &mut contexts);
    if contexts.is_empty() {
        return value.clone();
    }
    // Colour refinement: each round classifies IDs by their class and their
    // contexts under the previous round's classes. A class is the rank of its
    // first member, so a class of n members owns ranks c..c+n.
    let refine = |mut class: BTreeMap<String, usize>| {
        for _ in 0..64 {
            let mut keyed: Vec<(usize, Vec<String>, &String)> = contexts
                .iter()
                .map(|(id, places)| {
                    let mut key: Vec<String> = places
                        .iter()
                        .map(|(tag, place)| format!("{tag} {}", blank(place, &class)))
                        .collect();
                    key.sort();
                    (class[id], key, id)
                })
                .collect();
            keyed.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
            let mut next = BTreeMap::new();
            let mut rank = 0;
            for i in 0..keyed.len() {
                if i > 0 && (keyed[i - 1].0, &keyed[i - 1].1) != (keyed[i].0, &keyed[i].1) {
                    rank = i;
                }
                next.insert(keyed[i].2.clone(), rank);
            }
            if next == class {
                break;
            }
            class = next;
        }
        class
    };
    let mut class = refine(contexts.keys().map(|id| (id.clone(), 0)).collect());
    // Individualize: IDs refinement cannot tell apart are symmetric in the
    // output, so fixing any one of them and refining again gives the same
    // canonical form.
    loop {
        let mut members: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for (id, c) in &class {
            members.entry(*c).or_default().push(id.clone());
        }
        let Some((c, tied)) = members.into_iter().find(|(_, m)| m.len() > 1) else {
            break;
        };
        for other in &tied[1..] {
            class.insert(other.clone(), c + 1);
        }
        class = refine(class);
    }
    let names: BTreeMap<String, String> = class
        .into_iter()
        .map(|(id, c)| (id, format!("~D{c}~")))
        .collect();
    fn rename(value: &Value, names: &BTreeMap<String, String>) -> Value {
        match value {
            Value::String(s) => json!(names.get(s).cloned().unwrap_or_else(|| s.clone())),
            Value::Array(items) => Value::Array(items.iter().map(|v| rename(v, names)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| {
                        (
                            names.get(k).cloned().unwrap_or_else(|| k.clone()),
                            rename(v, names),
                        )
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    rename(value, &names)
}

/// Replace the hex digits after every `prefix` with `~`.
fn scrub_hex(text: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(prefix) {
        out.push_str(&rest[..at + prefix.len()]);
        rest = &rest[at + prefix.len()..];
        let hex = rest.chars().take_while(|c| c.is_ascii_hexdigit()).count();
        if hex > 0 {
            out.push('~');
        }
        rest = &rest[hex..];
    }
    out.push_str(rest);
    out
}

pub fn context_schema() -> Value {
    json!({"reference": {"id": "World", "revision": "1"},
        "axes": {"jurisdiction": {"kind": "enum", "members": ["EE", "FI"]}}})
}

/// Canonical comparison text of one outcome: errors by code and message, and
/// values with arrays sorted (records are ordered by ID, and generated IDs
/// follow revision IDs) while rank orders (`neighbors`, `candidates` and
/// change entries) are compared exactly too.
pub fn canonical(world: &World, outcome: Result<Value, (String, String)>) -> String {
    match outcome {
        Err((code, message)) => format!("error {code} {}", world.normalize(&message)),
        Ok(value) => {
            let text = world.normalize(&value.to_string());
            let mut value: Value = serde_json::from_str(&text).unwrap();
            flatten_conjunctions(&mut value);
            let value = rename_derived(&value);
            let ordered = json!({
                "neighbors": value.pointer("/neighbors").cloned(),
                "candidates": value.pointer("/candidates").cloned(),
                "changes": value.pointer("/changes/entries").cloned(),
            });
            let mut sorted = value;
            sort_arrays(&mut sorted);
            format!("{ordered} {sorted}")
        }
    }
}

const AND: &str = "weave:influence:and/v1";

/// A conjunction of proof alternatives is associative, commutative and
/// idempotent, and the engine folds it in record order, which follows
/// generated IDs. Nested conjunctions are compared as one set of conjuncts.
pub fn flatten_conjunctions(value: &mut Value) {
    fn conjuncts(value: &Value, out: &mut Vec<String>) {
        if value["operator"] == AND {
            if let Some(inputs) = value
                .pointer("/parameters/inputs")
                .and_then(Value::as_array)
            {
                inputs.iter().for_each(|input| conjuncts(input, out));
                return;
            }
        }
        let mut conjunct = value.clone();
        sort_arrays(&mut conjunct);
        out.push(conjunct.to_string());
    }
    match value {
        Value::Array(items) => items.iter_mut().for_each(flatten_conjunctions),
        Value::Object(map) => {
            map.values_mut().for_each(flatten_conjunctions);
            if map.get("operator").is_some_and(|o| o == AND) {
                let mut out = vec![];
                if let Some(inputs) = map
                    .get("parameters")
                    .and_then(|p| p.get("inputs"))
                    .and_then(Value::as_array)
                {
                    inputs.iter().for_each(|input| conjuncts(input, &mut out));
                }
                out.sort();
                out.dedup();
                let inputs: Vec<Value> = out
                    .iter()
                    .map(|t| serde_json::from_str(t).unwrap())
                    .collect();
                map.insert("parameters".into(), json!({ "inputs": inputs }));
            }
        }
        _ => {}
    }
}

/// A reference to a revision or to one record at a revision.
fn is_pin(value: &Value) -> bool {
    value.as_object().is_some_and(|m| {
        m.contains_key("graph_id")
            && m.contains_key("revision")
            && m.keys().all(|k| {
                [
                    "graph_id",
                    "revision",
                    "node_id",
                    "assertion_id",
                    "embedding_id",
                    "kind",
                ]
                .contains(&k.as_str())
            })
    })
}

pub fn sort_arrays(value: &mut Value) {
    match value {
        Value::Array(items) => {
            items.iter_mut().for_each(sort_arrays);
            let mut texts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
            texts.sort();
            // Two pins of one visible state are one pin: a head read in the
            // store with hidden facts pins a different revision ID than a
            // pinned read of the same visible state (the revision-ID
            // exception), so a list of pins is compared as a set.
            if items.iter().all(is_pin) {
                texts.dedup();
            }
            *items = texts
                .iter()
                .map(|t| serde_json::from_str(t).unwrap())
                .collect();
        }
        Value::Object(map) => map.values_mut().for_each(sort_arrays),
        _ => {}
    }
}

/// Cases per operator, from `WEAVE_NI_CASES`. The default, 40, keeps the
/// debug workspace suite fast; CI runs 500 per operator on every PR and
/// 10,000 in the scheduled full-count job, both optimized.
pub fn cases_per_operator() -> usize {
    std::env::var("WEAVE_NI_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40)
}

/// Worker threads: `WEAVE_NI_THREADS`, else the host's parallelism up to 8.
/// Every harness chooses its worlds independently of this number, so a run
/// on any host checks the same cases.
pub fn threads() -> usize {
    std::env::var("WEAVE_NI_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(2)
                .min(8)
        })
}

/// A first seed, so a failing case can be rerun alone (`WEAVE_NI_SEED`).
pub fn first_seed() -> u64 {
    std::env::var("WEAVE_NI_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}
