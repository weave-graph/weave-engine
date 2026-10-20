//! B2 (#5, ADR-0008 section 7): the property-based noninterference harness.
//!
//! For every `GraphExpression` operator, generated pairs of stores differ only
//! in facts (and registry state) that the readers under test cannot see. Each
//! generated plan is evaluated by each reader on both stores, and the two
//! outcomes must be identical: values, counts, ranks, explanations, coverage,
//! diagnostics, `fact_provenance` and errors (code and message), after
//! revision IDs are replaced by the visible state they stand for.
//!
//! Three operators read a whole revision and are declared to answer one bit
//! about hidden records (ADR-0002, ADR-0008 section 7.3): `recorded_query`,
//! `accepted_graph` and `accepted_history`. For them the store with hidden
//! facts may instead return the operator's single "unavailable" error, and
//! the harness counts those outcomes separately.
//!
//! Every operator must also be *sensitive*: on some cases `mallory`, who can
//! see the hidden facts, gets different output from the two stores. That
//! shows the hidden facts reach each operator, so equality is not vacuous.
//!
//! Counts: `WEAVE_NI_CASES` cases per operator (default 500, the PR tier; the
//! full-count job uses 10,000). `WEAVE_NI_SEED` moves the first world seed.
//! The run prints one `NONINTERFERENCE {json}` line with per-operator counts.
#[path = "support/noninterference.rs"]
mod ni;

use ed25519_dalek::SigningKey;
use ni::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use weave_contract::view_registration::{
    seal_template, CompiledViewTemplate, VIEW_TEMPLATE_FORMAT,
};
use weave_contract::{CommandResult, GraphRef, Interval, NodeRef, ViewClock, VERSION};
use weave_engine::*;

/// How a pair of outcomes must relate.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rule {
    /// Identical outcomes.
    Equal,
    /// Identical, or the store with hidden facts answers exactly this error.
    WholeRevision(&'static str, &'static str),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Profiles {
    Both,
    /// Governance and identity acceptance are peripheral subsystems that a
    /// registry store does not extend (B1); they run in local stores only.
    LocalOnly,
}

struct Operator {
    name: &'static str,
    rule: Rule,
    profiles: Profiles,
    plan: fn(&mut Gen) -> Value,
}

const OPERATORS: &[Operator] = &[
    op("query", Rule::Equal, Profiles::Both, plan_query),
    op(
        "recorded_query",
        Rule::WholeRevision("E_HISTORY_UNAVAILABLE", "recorded observation unavailable"),
        Profiles::Both,
        plan_recorded_query,
    ),
    op("as_of", Rule::Equal, Profiles::Both, plan_as_of),
    op("expand", Rule::Equal, Profiles::Both, plan_expand),
    op("changes", Rule::Equal, Profiles::Both, plan_changes),
    op("nearest", Rule::Equal, Profiles::Both, plan_nearest),
    op(
        "similar_candidates",
        Rule::Equal,
        Profiles::Both,
        plan_similar,
    ),
    op("window", Rule::Equal, Profiles::Both, plan_window),
    op("sequence", Rule::Equal, Profiles::Both, plan_sequence),
    op(
        "accepted_history",
        Rule::WholeRevision("E_GOV_HISTORY_UNAVAILABLE", "accepted history unavailable"),
        Profiles::LocalOnly,
        plan_accepted_history,
    ),
    op(
        "accepted_graph",
        Rule::WholeRevision("E_GOV_UNAVAILABLE", "accepted graph unavailable"),
        Profiles::LocalOnly,
        plan_accepted_graph,
    ),
    op(
        "current_view",
        Rule::Equal,
        Profiles::Both,
        plan_current_view,
    ),
    op(
        "typed_context",
        Rule::Equal,
        Profiles::Both,
        plan_typed_context,
    ),
    op(
        "resolve_identity",
        Rule::Equal,
        Profiles::LocalOnly,
        plan_resolve_identity,
    ),
    op("cluster", Rule::Equal, Profiles::Both, plan_cluster),
    op(
        "counterparts",
        Rule::Equal,
        Profiles::Both,
        plan_counterparts,
    ),
    op("geometry", Rule::Equal, Profiles::Both, plan_geometry),
    op("explain", Rule::Equal, Profiles::Both, plan_explain),
    op("context", Rule::Equal, Profiles::Both, plan_context),
    op("reason", Rule::Equal, Profiles::Both, plan_reason),
    op("union", Rule::Equal, Profiles::Both, plan_union),
    op("diff", Rule::Equal, Profiles::Both, plan_diff),
    op("project", Rule::Equal, Profiles::Both, plan_project),
    op("support", Rule::Equal, Profiles::Both, plan_support),
    op("metadata", Rule::Equal, Profiles::Both, plan_metadata),
    op("reference", Rule::Equal, Profiles::Both, plan_reference),
    op("filter", Rule::Equal, Profiles::Both, plan_filter),
    op("join", Rule::Equal, Profiles::Both, plan_join),
];

const fn op(
    name: &'static str,
    rule: Rule,
    profiles: Profiles,
    plan: fn(&mut Gen) -> Value,
) -> Operator {
    Operator {
        name,
        rule,
        profiles,
        plan,
    }
}

/// Every `GraphExpression` kind the contract defines must be in the harness.
#[test]
fn the_harness_names_every_graph_expression_kind() {
    let source = include_str!("../../weave-contract/src/lib.rs");
    let start = source.find("pub enum GraphExpression {").unwrap();
    let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
    let mut kinds = vec![];
    for line in body.lines() {
        let line = line.trim();
        let Some(first) = line.chars().next() else {
            continue;
        };
        if !first.is_ascii_uppercase() || line.starts_with("pub enum") {
            continue;
        }
        let name: String = line
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let mut snake = String::new();
        for (i, c) in name.chars().enumerate() {
            if c.is_ascii_uppercase() {
                if i > 0 {
                    snake.push('_');
                }
                snake.push(c.to_ascii_lowercase());
            } else {
                snake.push(c);
            }
        }
        kinds.push(snake);
    }
    assert!(kinds.len() >= 28, "{kinds:?}");
    let covered: Vec<&str> = OPERATORS.iter().map(|o| o.name).collect();
    for kind in &kinds {
        assert!(covered.contains(&kind.as_str()), "{kind} has no generator");
    }
    assert_eq!(kinds.len(), covered.len(), "{kinds:?}");
}

// ---------------------------------------------------------------- generators

/// Plan generation context: the pair's spec and a plan-local RNG.
struct Gen<'a> {
    spec: &'a Spec,
    rng: Rng,
}

impl Gen<'_> {
    fn node(&mut self) -> String {
        // Visible, hidden and unknown IDs alike.
        match self.rng.below(8) {
            0 | 1 => self.rng.pick(&self.spec.hidden_nodes).clone(),
            2 => "nowhere".into(),
            _ => self.rng.pick(&self.spec.visible_nodes).clone(),
        }
    }
    fn edge(&mut self) -> String {
        if self.rng.chance(1, 3) {
            self.rng.pick(&self.spec.hidden_edges).clone()
        } else {
            self.rng.pick(&self.spec.visible_edges).clone()
        }
    }
    fn time(&mut self) -> i64 {
        self.rng.range(0, 60)
    }
    /// A recorded time: around the visible and hidden writes.
    fn recorded(&mut self) -> i64 {
        *self
            .rng
            .pick(&[0, 9, 10, 12, 15, 17, 20, 25, 30, 33, 35, 40, 45, 50, 999])
    }
    fn g_revision(&mut self) -> String {
        format!("~R{}~", self.rng.range(1, STEP_TIMES.len() as i64))
    }
    fn vector(&mut self) -> Vec<f32> {
        if self.rng.chance(1, 2) && !self.spec.vectors.is_empty() {
            self.rng.pick(&self.spec.vectors).clone()
        } else {
            self.spec.random_vector(&mut self.rng)
        }
    }
    /// A graph a plan names. Registry stores also name a graph only
    /// `mallory` can discover and one that does not exist (Discover).
    fn graph(&mut self, usual: &str) -> String {
        if self.spec.profile == Profile::Registry && self.rng.chance(1, 12) {
            self.rng.pick_str(&["vault", "nowhere"])
        } else {
            usual.into()
        }
    }
    fn query_plan(&mut self) -> Value {
        let graph = match self.rng.below(6) {
            0..=2 => self.graph("g"),
            3 | 4 => self.graph("snap"),
            _ => "pub".into(),
        };
        let mut plan = json!({"graph_id": graph});
        if self.rng.chance(1, 3) {
            plan["revision"] = json!(match graph.as_str() {
                "snap" => format!("~S{}~", self.rng.range(1, 2)),
                "pub" => "~P1~".to_string(),
                _ => self.g_revision(),
            });
        }
        if self.rng.chance(1, 3) {
            plan["predicate"] = json!(self.rng.pick_str(&PREDICATES));
        }
        if self.rng.chance(1, 5) {
            plan["from"] = json!(self.node());
        }
        if self.rng.chance(1, 6) {
            plan["to"] = json!(self.node());
        }
        if self.rng.chance(1, 3) {
            plan["valid_at"] = json!(self.time());
        }
        if self.rng.chance(1, 4) {
            plan["include_metadata"] = json!(true);
        }
        plan
    }
    fn query(&mut self) -> Value {
        json!({"kind": "query", "query": self.query_plan()})
    }
    fn expand(&mut self) -> Value {
        let graph = if self.rng.chance(1, 4) {
            self.graph("snap")
        } else {
            self.graph("g")
        };
        let mut seeds = vec![];
        for _ in 0..self.rng.range(1, 3) {
            seeds.push(if graph == "snap" {
                if self.rng.chance(1, 4) {
                    "wh".to_string()
                } else {
                    self.rng.pick(&self.spec.snap_nodes).clone()
                }
            } else {
                self.node()
            });
        }
        let mut plan = json!({"kind": "expand", "graph_id": graph, "seeds": seeds,
            "hops": self.rng.range(1, 3), "direction": self.rng.pick_str(&["out", "in", "both"])});
        if self.rng.chance(1, 3) {
            plan["predicates"] = json!([self.rng.pick_str(&PREDICATES)]);
        }
        if self.rng.chance(1, 3) {
            plan["valid_at"] = json!(self.time());
        }
        if self.rng.chance(1, 4) {
            plan["as_of"] = json!(self.recorded());
        } else if self.rng.chance(1, 5) && graph == "g" {
            plan["revision"] = json!(self.g_revision());
        }
        if self.rng.chance(1, 5) {
            plan["limit"] = json!(self.rng.range(1, 6));
        }
        if self.rng.chance(1, 6) && graph == "g" {
            plan["seeds_from"] = self.nearest_plan(false);
        }
        plan
    }
    fn nearest_plan(&mut self, nested: bool) -> Value {
        let space = self.rng.pick_str(&["sp", "sc"]);
        let query = match self.rng.below(5) {
            0 => {
                json!({"kind": "embedding", "embedding_id": self.rng.pick(&self.spec.visible_embeddings)})
            }
            1 => {
                json!({"kind": "embedding", "embedding_id": self.rng.pick(&self.spec.hidden_embeddings)})
            }
            _ => json!({"kind": "vector", "vector": self.vector()}),
        };
        let mut plan = json!({"kind": "nearest", "graph_id": self.graph("g"),
            "space": {"id": space, "revision": "1"}, "query": query, "k": self.rng.range(1, 6)});
        if self.rng.chance(1, 3) {
            plan["valid_at"] = json!(self.time());
        }
        if self.rng.chance(1, 4) {
            plan["as_of"] = json!(self.recorded());
        }
        if !nested && self.rng.chance(1, 3) {
            plan["candidates"] = if self.rng.chance(1, 2) {
                self.expand()
            } else {
                self.query()
            };
        }
        plan
    }
    /// Any value-producing expression, for composition.
    fn leaf(&mut self) -> Value {
        match self.rng.below(10) {
            0..=3 => self.query(),
            4 => {
                let mut query = self.query_plan();
                query.as_object_mut().unwrap().remove("revision");
                json!({"kind": "as_of", "query": query, "unix_millis": self.recorded()})
            }
            5 | 6 => self.expand(),
            7 => self.nearest_plan(false),
            8 => json!({"kind": "filter", "input": self.query(),
                "predicate": self.rng.pick_str(&PREDICATES)}),
            _ => json!({"kind": "window", "input": self.query(),
                "window": self.window()}),
        }
    }
    fn window(&mut self) -> Value {
        let start = self.rng.range(0, 30);
        json!({"start": start, "end": start + self.rng.range(1, 30)})
    }
}

fn evaluate(value: Value) -> Value {
    json!([{"op": "evaluate", "value": value}])
}

fn plan_query(g: &mut Gen) -> Value {
    if g.rng.chance(1, 5) {
        // The `query` command form.
        return json!([{"op": "query", "query": g.query_plan()}]);
    }
    evaluate(g.query())
}
fn plan_recorded_query(g: &mut Gen) -> Value {
    let mut query = g.query_plan();
    query.as_object_mut().unwrap().remove("revision");
    evaluate(json!({"kind": "recorded_query", "query": query,
        "selection": {"kind": "local_time", "unix_millis": g.recorded()}}))
}
fn plan_as_of(g: &mut Gen) -> Value {
    let mut query = g.query_plan();
    query.as_object_mut().unwrap().remove("revision");
    evaluate(json!({"kind": "as_of", "query": query, "unix_millis": g.recorded()}))
}
fn plan_expand(g: &mut Gen) -> Value {
    evaluate(g.expand())
}
fn plan_changes(g: &mut Gen) -> Value {
    let graph = if g.rng.chance(1, 3) {
        g.graph("snap")
    } else {
        g.graph("g")
    };
    let bound = |g: &mut Gen| match g.rng.below(4) {
        0 if graph == "g" => json!({"kind": "cursor", "revision": g.g_revision()}),
        0 => json!({"kind": "cursor", "revision": format!("~S{}~", g.rng.range(1, 2))}),
        1 => json!({"kind": "head"}),
        _ => json!({"kind": "as_of", "unix_millis": g.recorded()}),
    };
    let from = bound(g);
    let to = bound(g);
    let mut plan = json!({"kind": "changes", "graph_id": graph, "from": from, "to": to});
    if g.rng.chance(1, 3) {
        plan["entities"] = json!([g.rng.pick_str(&ENTITIES)]);
    }
    if g.rng.chance(1, 4) {
        plan["predicates"] = json!([g.rng.pick_str(&["p", "q", "twin", "note", "tag"])]);
    }
    if g.rng.chance(1, 4) {
        plan["limit"] = json!(g.rng.range(1, 8));
    }
    evaluate(plan)
}
fn plan_nearest(g: &mut Gen) -> Value {
    evaluate(g.nearest_plan(false))
}
fn plan_similar(g: &mut Gen) -> Value {
    let space = g.rng.pick_str(&["sp", "sc"]);
    let mut plan = json!({"kind": "similar_candidates", "graph_id": g.graph("g"),
        "space": {"id": space, "revision": "1"}, "k": g.rng.range(1, 3)});
    if g.rng.chance(1, 3) {
        plan["max_score"] = json!(*g.rng.pick(&[0.0, 0.5, 2.0]));
    }
    if g.rng.chance(1, 3) {
        plan["limit"] = json!(g.rng.range(1, 6));
    }
    if g.rng.chance(1, 3) {
        plan["valid_at"] = json!(g.time());
    }
    if g.rng.chance(1, 3) {
        plan["exclude_linked"] = json!(true);
    }
    if g.rng.chance(1, 4) {
        plan["sources"] = g.expand();
    }
    if g.rng.chance(1, 4) {
        plan["targets"] = g.query();
    }
    evaluate(plan)
}
fn plan_window(g: &mut Gen) -> Value {
    let input = g.leaf();
    evaluate(json!({"kind": "window", "input": input, "window": g.window()}))
}
fn plan_sequence(g: &mut Gen) -> Value {
    let left = g.leaf();
    let right = g.leaf();
    evaluate(json!({"kind": "sequence", "left": left, "right": right,
        "window": {"start": 0, "end": g.rng.range(10, 60)},
        "relation": g.rng.pick_str(&["before", "meets", "overlaps", "within"]),
        "match_on": "entity_space_to_from"}))
}
fn plan_accepted_history(g: &mut Gen) -> Value {
    let view = g.rng.pick_str(&["team", "mixed"]);
    let time = *g.rng.pick(&[53, 54, 55, 57, 58, 59, 62, 70, 900]);
    evaluate(json!({"kind": "accepted_history", "view_id": view,
        "selection": {"kind": "local_time", "unix_millis": time}}))
}
fn plan_accepted_graph(g: &mut Gen) -> Value {
    let (view, decision) = *g.rng.pick(&[
        ("team", "~D-team~"),
        ("mixed", "~D-mixed~"),
        ("team", "~D-mixed~"),
    ]);
    evaluate(
        json!({"kind": "accepted_graph", "selection": {"view_id": view, "decision_id": decision}}),
    )
}
fn plan_current_view(g: &mut Gen) -> Value {
    let view = g.rng.pick_str(&["~VIEW~", "~VIEW~", "v-nobody"]);
    evaluate(
        json!({"kind": "current_view", "selection": {"view_id": view,
        "definition_digest": "~VD~", "time": {"kind": "fixed"}}}),
    )
}
fn plan_typed_context(g: &mut Gen) -> Value {
    let reference = match g.rng.below(3) {
        0 => json!({"graph_id": "ctxh", "revision": "~CH1~"}),
        _ => json!({"graph_id": "ctx", "revision": "~C1~"}),
    };
    let mut schema = context_schema();
    if g.rng.chance(1, 5) {
        schema["axes"]["jurisdiction"]["members"] = json!(["EE"]);
    }
    let input = g.leaf();
    evaluate(
        json!({"kind": "typed_context", "input": input, "reference": reference,
        "expected_schema": schema}),
    )
}
fn plan_resolve_identity(g: &mut Gen) -> Value {
    let source = match g.rng.below(3) {
        0 => json!({"graph_id": "operations", "revision": "~OP1~", "node_id": "node"}),
        1 => json!({"graph_id": "private", "revision": "~PR1~", "node_id": "node"}),
        _ => json!({"graph_id": "physical", "revision": "~PH1~", "node_id": "node"}),
    };
    evaluate(
        json!({"kind": "resolve_identity", "selection": {"mapping_id": "equipment",
        "revision": "~M1~", "policy": {"id": "review", "revision": "1"}, "source": source,
        "target_space": g.rng.pick_str(&["operations", "physical", "private"]),
        "valid_at": g.rng.range(0, 12), "context": {"kind": "default"}}}),
    )
}
fn plan_cluster(g: &mut Gen) -> Value {
    let source = match g.rng.below(3) {
        0 => json!({"graph_id": "snap", "revision": format!("~S{}~", g.rng.range(1, 2))}),
        1 => json!({"graph_id": "pub", "revision": "~P1~"}),
        _ => json!({"graph_id": "g", "revision": g.g_revision()}),
    };
    evaluate(
        json!({"kind": "cluster", "selection": {"source": source, "context": {"kind": "default"},
        "valid_at": g.time(), "predicate": g.rng.pick_str(&PREDICATES), "levels": g.rng.range(1, 3)}}),
    )
}
fn plan_counterparts(g: &mut Gen) -> Value {
    let input = g.leaf();
    let from = g.rng.pick_str(&SPACES);
    let to = if from == "s0" { "s1" } else { "s0" };
    evaluate(
        json!({"kind": "counterparts", "input": input, "selection": {
        "predicate": g.rng.pick_str(&["twin", "twin", "p"]), "entity_id": g.rng.pick_str(&ENTITIES),
        "from_space_id": from, "to_space_id": to, "valid_at": g.time()}}),
    )
}
fn plan_geometry(g: &mut Gen) -> Value {
    let point = |g: &mut Gen| {
        let id = if g.rng.chance(1, 3) {
            g.rng.pick(&g.spec.geometry_hidden).clone()
        } else {
            g.rng.pick(&g.spec.geometry_visible).clone()
        };
        let mut query = json!({"graph_id": "snap"});
        if g.rng.chance(1, 3) {
            query["revision"] = json!(format!("~S{}~", g.rng.range(1, 2)));
        }
        json!({"input": {"kind": "query", "query": query}, "assertion_id": id})
    };
    let left = point(g);
    let right = point(g);
    evaluate(json!({"kind": "geometry", "valid_at": g.time(),
        "operation": {"kind": "distance", "left": left, "right": right}}))
}
fn plan_explain(g: &mut Gen) -> Value {
    evaluate(json!({"kind": "explain", "input": g.leaf()}))
}
fn plan_context(g: &mut Gen) -> Value {
    let selection = if g.rng.chance(1, 3) {
        json!({"kind": "pinned", "reference": {"graph_id": "ctx", "revision": "~C1~"}})
    } else {
        json!({"kind": "default"})
    };
    evaluate(json!({"kind": "context", "input": g.leaf(), "selection": selection}))
}
fn plan_reason(g: &mut Gen) -> Value {
    let base = g.rng.pick_str(&PREDICATES);
    let var = |n: &str| json!({"kind": "variable", "name": n});
    let rules = json!({"id": "reach", "revision": "1", "rules": [
        {"id": "seed", "head": {"predicate": "reach", "from": var("x"), "to": var("y")},
         "body": [{"predicate": base, "from": var("x"), "to": var("y")}],
         "allow_cross_space": true},
        {"id": "step", "head": {"predicate": "reach", "from": var("x"), "to": var("z")},
         "body": [{"predicate": "reach", "from": var("x"), "to": var("y")},
                  {"predicate": base, "from": var("y"), "to": var("z")}],
         "allow_cross_space": true}]});
    evaluate(json!({"kind": "reason", "input": g.leaf(), "rules": rules}))
}
fn plan_union(g: &mut Gen) -> Value {
    evaluate(json!({"kind": "union", "left": g.leaf(), "right": g.leaf()}))
}
fn plan_diff(g: &mut Gen) -> Value {
    let (before, after) = if g.rng.chance(1, 2) {
        let a = json!({"kind": "query", "query": {"graph_id": "g", "revision": g.g_revision()}});
        let b = json!({"kind": "query", "query": {"graph_id": "g", "revision": g.g_revision()}});
        (a, b)
    } else {
        (g.leaf(), g.leaf())
    };
    evaluate(json!({"kind": "diff", "before": before, "after": after}))
}
fn plan_project(g: &mut Gen) -> Value {
    // Members must be in the input (`E_PROJECT_MEMBER` otherwise, for a
    // hidden member exactly as for a missing one).
    let nodes: Vec<String> = (0..g.rng.range(0, 2)).map(|_| g.node()).collect();
    let edges: Vec<String> = (0..g.rng.range(0, 2)).map(|_| g.edge()).collect();
    let input = if g.rng.chance(1, 2) {
        json!({"kind": "query", "query": {"graph_id": "g"}})
    } else {
        g.leaf()
    };
    evaluate(json!({"kind": "project", "input": input, "node_ids": nodes, "edge_ids": edges}))
}
fn plan_support(g: &mut Gen) -> Value {
    let space = |g: &mut Gen| {
        if g.rng.chance(1, 3) {
            "world".to_string()
        } else {
            g.rng.pick_str(&SPACES)
        }
    };
    let from = json!({"entity_id": g.rng.pick_str(&ENTITIES), "space_id": space(g)});
    let to = json!({"entity_id": g.rng.pick_str(&ENTITIES), "space_id": space(g)});
    evaluate(
        json!({"kind": "support", "input": g.leaf(), "predicate": g.rng.pick_str(&PREDICATES),
        "from": from, "to": to, "valid_at": g.time()}),
    )
}
fn plan_metadata(g: &mut Gen) -> Value {
    let mut query = g.query_plan();
    query["include_metadata"] = json!(true);
    let host = match g.rng.below(5) {
        0 => json!({"kind": "graph"}),
        1 => json!({"kind": "entity", "id": g.rng.pick_str(&ENTITIES)}),
        2 => json!({"kind": "node", "id": g.rng.pick(&g.spec.snap_nodes).clone()}),
        3 => json!({"kind": "node", "id": "w0"}),
        _ => json!({"kind": "node", "id": g.node()}),
    };
    let key = g.rng.pick_str(&["note", "tag", "proof"]);
    evaluate(
        json!({"kind": "metadata", "input": {"kind": "query", "query": query},
        "host": host, "key": key}),
    )
}
fn plan_reference(g: &mut Gen) -> Value {
    let value = g.leaf();
    let mut reference = json!({"kind": "reference", "name": "bound"});
    if g.rng.chance(1, 2) {
        reference = json!({"kind": "union", "left": reference, "right": g.leaf()});
    }
    json!([{"op": "bind", "name": "bound", "value": value},
        {"op": "evaluate", "value": reference}])
}
fn plan_filter(g: &mut Gen) -> Value {
    let mut plan = json!({"kind": "filter", "input": g.leaf()});
    if g.rng.chance(2, 3) {
        plan["predicate"] = json!(g.rng.pick_str(&PREDICATES));
    }
    if g.rng.chance(1, 2) {
        plan["valid_at"] = json!(g.time());
    }
    evaluate(plan)
}
fn plan_join(g: &mut Gen) -> Value {
    if g.rng.chance(1, 4) {
        // The `join` command form takes query plans.
        return json!([{"op": "join", "left": g.query_plan(), "right": g.query_plan(),
            "output_predicate": "path", "match_on": "entity_space_to_from"}]);
    }
    evaluate(json!({"kind": "join", "left": g.leaf(), "right": g.leaf(),
        "output_predicate": "path", "match_on": "entity_space_to_from"}))
}

// ---------------------------------------------------------------- per-world setup

const GOVERNANCE_KEY: [u8; 32] = [73; 32];

/// Governance views, identity acceptance and compiled views on top of the
/// generated data, the same steps in both stores.
fn setup(world: &mut World, spec: &Spec) {
    if spec.profile == Profile::Local {
        governance(world);
        identity(world);
    }
    views(world, spec);
    world.at(FINAL_TIME);
}

fn governance(world: &mut World) {
    let key = SigningKey::from_bytes(&GOVERNANCE_KEY);
    let host = world.writer.clone();
    for (index, (view, graph, source_label)) in [("team", "pub", "~P1~"), ("mixed", "snap", "~S1~")]
        .into_iter()
        .enumerate()
    {
        world.at(52 + 4 * index as i64);
        world
            .e
            .install_governance_root(&GovernancePolicy {
                view_id: view.into(),
                reference: GovernancePolicyRef {
                    id: format!("policy-{view}"),
                    revision: "1".into(),
                },
                members: vec![weave_policy::public_key(&key)],
                threshold: 1,
                proposers: vec!["writer".into()],
                readers: vec![],
                allowed_sources: vec![GovernanceSourceScope {
                    graph_id: graph.into(),
                    branch_id: "main".into(),
                }],
                not_before_ms: 0,
                expires_at_ms: 100_000,
            })
            .unwrap();
        world.at(54 + 4 * index as i64);
        let head = world.e.inspect_governance_head(view, &host).unwrap();
        let source = world.tokens[source_label].clone();
        let proposal = GovernanceProposal {
            id: format!("publish-{view}"),
            view_id: view.into(),
            policy: head.policy,
            expected_head: head.decision_id,
            expires_at_ms: 90_000,
            action: GovernanceAction::Publish {
                source: GraphRef {
                    graph_id: graph.into(),
                    revision: source,
                },
                branch_id: "main".into(),
            },
        };
        let receipt = world.e.propose_governance(&proposal, &host).unwrap();
        let signed = sign_governance_approval(
            GovernanceApproval {
                proposal_id: proposal.id.clone(),
                proposal_digest: receipt.digest,
                view_id: view.into(),
                policy: proposal.policy.clone(),
                expected_head: proposal.expected_head.clone(),
                member: weave_policy::public_key(&key),
                issued_at_ms: 0,
                expires_at_ms: 80_000,
                nonce: proposal.id.clone(),
            },
            &key,
        )
        .unwrap();
        world.e.record_governance_approval(&signed, &host).unwrap();
        let accepted = world
            .e
            .accept_governance(
                &GovernanceDecisionRequest {
                    proposal_id: proposal.id.clone(),
                    nonce: proposal.id.clone(),
                },
                &host,
            )
            .unwrap();
        // The decision's own occurrence graph is content-addressed over the
        // store's replica identity; label its revision too.
        let occurrence = format!("weave:governance:decision:{}", accepted.decision_id);
        if let Ok(Some(revision)) = world.e.head(&occurrence, "main") {
            world.token(&format!("DO-{view}"), revision);
        }
        world.token(&format!("D-{view}"), accepted.decision_id);
    }
}

fn identity(world: &mut World) {
    world.at(64);
    let policy = IdentityPolicyRef {
        id: "review".into(),
        revision: "1".into(),
    };
    world
        .e
        .install_identity_policy(&IdentityPolicy {
            reference: policy.clone(),
            proposers: vec!["writer".into()],
            approvers: vec!["reviewer".into()],
            readers: vec![],
            allowed_spaces: vec!["physical".into(), "operations".into(), "private".into()],
            max_members: 16,
        })
        .unwrap();
    let mut members = vec![];
    let mut graphs = vec![
        ("physical", "PH1", "A", json!([])),
        ("operations", "OP1", "B", json!([])),
    ];
    if world.hidden {
        graphs.push((
            "private",
            "PR1",
            "C",
            json!(["mallory", "writer", "reviewer"]),
        ));
    } else {
        world.fake("PR1", false);
    }
    for (graph, label, entity, readers) in graphs {
        let r = world.run(json!([{"op": "commit", "graph_id": graph, "data": {"nodes": [
            {"id": "node", "entity_id": format!("independent-{entity}"), "space_id": graph, "readers": readers}]}}]));
        world.label(graph, label, r);
        members.push(NodeRef {
            graph_id: graph.into(),
            revision: world.tokens[&format!("~{label}~")].clone(),
            node_id: "node".into(),
        });
    }
    world
        .e
        .submit_identity_candidate(
            &IdentityCandidate {
                id: "candidate".into(),
                mapping_id: "equipment".into(),
                policy,
                groups: vec![members],
                evidence: vec![],
                valid_time: Interval {
                    start: 0,
                    end: Some(10),
                },
                context: None,
            },
            &world.writer.clone(),
        )
        .unwrap();
    let reviewer = HostContext::new("reviewer", world.writer.writable_graphs.clone());
    let accepted = world
        .e
        .accept_identity_candidate(
            &IdentityDecisionRequest {
                candidate_id: "candidate".into(),
                expected_head: None,
                nonce: "n".into(),
            },
            &reviewer,
        )
        .unwrap();
    world.token("M1", accepted.reference.revision);
}

/// Each principal registers its own compiled view over a generated expression.
fn views(world: &mut World, spec: &Spec) {
    world.at(66);
    let mut gen = Gen {
        spec,
        rng: Rng::new(spec.seed ^ 0x5157),
    };
    let expression = gen.leaf();
    let expression = world.substitute(&expression);
    let template = seal_template(CompiledViewTemplate {
        format: VIEW_TEMPLATE_FORMAT.into(),
        protocol: VERSION.into(),
        name: "generated".into(),
        revision: "1".into(),
        expression: serde_json::from_value(expression).unwrap(),
        clock: ViewClock::Fixed,
        source_revisions: vec![],
        definition_digest: String::new(),
    });
    let Ok(template) = template else {
        return;
    };
    for principal in READERS.iter().chain([&HIDDEN_READER]) {
        let host = HostContext::new(*principal, Vec::<String>::new());
        // A view whose registration fails is simply absent for that reader.
        if world
            .e
            .register_compiled_view(&format!("v-{principal}"), &template, None, &host)
            .is_ok()
        {
            world.token(
                &format!("VD-{principal}"),
                template.definition_digest.clone(),
            );
        } else {
            world.fake(&format!("VD-{principal}"), false);
        }
    }
}

// ---------------------------------------------------------------- evaluation

fn outcome(world: &mut World, principal: &str, commands: &Value) -> String {
    let text = commands
        .to_string()
        .replace("~VIEW~", &format!("v-{principal}"))
        .replace("~VD~", &format!("~VD-{principal}~"));
    let commands = world.substitute(&serde_json::from_str(&text).unwrap());
    let host = HostContext::new(principal, Vec::<String>::new());
    let result = world.e.execute(&program(commands), &host);
    let value = match result {
        Ok(mut results) => match results.pop() {
            Some(CommandResult::Queried { result }) => Ok(serde_json::to_value(result).unwrap()),
            other => Ok(serde_json::to_value(other).unwrap()),
        },
        Err(error) => Err((error.code, error.message)),
    };
    canonical(world, value)
}

#[derive(Default, Clone, serde::Serialize)]
struct Counts {
    cases: usize,
    ok: usize,
    errors: usize,
    declared: usize,
    sensitive: usize,
    /// Error outcomes by code, as the readers under test saw them.
    error_codes: BTreeMap<String, usize>,
}

fn run_world(
    seed: u64,
    plans: usize,
    todo: &[usize],
    counts: &Mutex<BTreeMap<&'static str, Counts>>,
) {
    let profile = if seed.is_multiple_of(2) {
        Profile::Local
    } else {
        Profile::Registry
    };
    let spec = Spec::new(seed, profile);
    let mut plain = World::build(&spec, false);
    let mut mixed = World::build(&spec, true);
    setup(&mut plain, &spec);
    setup(&mut mixed, &spec);
    for &index in todo {
        let operator = &OPERATORS[index];
        if operator.profiles == Profiles::LocalOnly && profile != Profile::Local {
            continue;
        }
        // Local-only operators run twice as many plans in local worlds.
        let plans = if operator.profiles == Profiles::LocalOnly {
            plans * 2
        } else {
            plans
        };
        let mut local = Counts::default();
        for case in 0..plans {
            let mut gen = Gen {
                spec: &spec,
                rng: Rng::new(seed.wrapping_mul(1_000_003) ^ ((index as u64) << 32) ^ case as u64),
            };
            let commands = (operator.plan)(&mut gen);
            local.cases += 1;
            for principal in READERS {
                let a = outcome(&mut plain, principal, &commands);
                let b = outcome(&mut mixed, principal, &commands);
                let ok = a == b;
                let declared = match operator.rule {
                    Rule::WholeRevision(code, message) => {
                        !ok && b == format!("error {code} {message}")
                    }
                    Rule::Equal => false,
                };
                if !ok && !declared {
                    if let Ok(dir) = std::env::var("WEAVE_NI_DUMP") {
                        let name = format!("{dir}/{}-{seed}-{case}-{principal}", operator.name);
                        std::fs::write(format!("{name}-without.json"), &a).unwrap();
                        std::fs::write(format!("{name}-with.json"), &b).unwrap();
                    }
                    panic!(
                        "hidden facts changed {principal}'s output of {} (seed {seed}, {profile:?}, case {case})\nplan {commands}\n{}",
                        operator.name,
                        difference(&a, &b)
                    );
                }
                local.declared += usize::from(declared);
                if let Some(rest) = a.strip_prefix("error ") {
                    let code = rest.split(' ').next().unwrap_or_default().to_string();
                    *local.error_codes.entry(code).or_default() += 1;
                    local.errors += 1;
                } else {
                    local.ok += 1;
                }
            }
            let a = outcome(&mut plain, HIDDEN_READER, &commands);
            let b = outcome(&mut mixed, HIDDEN_READER, &commands);
            local.sensitive += usize::from(a != b);
        }
        let mut counts = counts.lock().unwrap();
        let entry = counts.entry(operator.name).or_default();
        entry.cases += local.cases;
        entry.ok += local.ok;
        entry.errors += local.errors;
        entry.declared += local.declared;
        entry.sensitive += local.sensitive;
        for (code, n) in local.error_codes {
            *entry.error_codes.entry(code).or_default() += n;
        }
    }
}

/// The two texts around their first difference.
fn difference(a: &str, b: &str) -> String {
    let at = a
        .char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map(|((i, _), _)| i)
        .unwrap_or(a.len().min(b.len()));
    let window = |t: &str| {
        let mut start = at.saturating_sub(600);
        while !t.is_char_boundary(start) {
            start -= 1;
        }
        let mut end = (at + 600).min(t.len());
        while !t.is_char_boundary(end) {
            end += 1;
        }
        format!("...{}... ({} bytes)", &t[start..end], t.len())
    };
    format!(
        "without hidden: {}\nwith hidden:    {}",
        window(a),
        window(b)
    )
}

#[test]
fn hidden_facts_never_change_any_operators_output() {
    let target = cases_per_operator();
    let plans = 10usize;
    // Every world serves every operator; local-only operators get twice the
    // plans in the half of the worlds that are local.
    let worlds = target.div_ceil(plans).max(2);
    let todo: Vec<usize> = (0..OPERATORS.len()).collect();
    let counts = Mutex::new(BTreeMap::new());
    let next = AtomicUsize::new(0);
    let threads = threads();
    let base = first_seed();
    let started = std::time::Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let world = next.fetch_add(1, Ordering::SeqCst);
                if world >= worlds {
                    break;
                }
                run_world(base + world as u64, plans, &todo, &counts);
            });
        }
    });
    let counts = counts.into_inner().unwrap();
    println!(
        "NONINTERFERENCE {}",
        json!({"cases_per_operator_target": target, "worlds": worlds,
            "seconds": started.elapsed().as_secs_f64(), "operators": counts})
    );
    for operator in OPERATORS {
        let c = &counts[operator.name];
        assert!(c.cases >= target, "{}: {} cases", operator.name, c.cases);
        assert!(c.ok > 0, "{}: no successful outcome", operator.name);
        assert!(
            c.sensitive > 0,
            "{}: hidden facts never reached the operator",
            operator.name
        );
    }
}

// ---------------------------------------------------------------- regressions

/// Found by the harness: a rule explanation chose among equivalent witnesses
/// in an order that followed a digest over revision IDs, so a hidden record
/// in the same revision changed which visible witness `reason` showed.
#[test]
fn rule_witnesses_do_not_depend_on_hidden_records() {
    let var = |n: &str| json!({"kind": "variable", "name": n});
    let rules = json!({"id": "reach", "revision": "1", "rules": [
        {"id": "seed", "head": {"predicate": "reach", "from": var("x"), "to": var("y")},
         "body": [{"predicate": "p", "from": var("x"), "to": var("y")}]},
        {"id": "step", "head": {"predicate": "reach", "from": var("x"), "to": var("z")},
         "body": [{"predicate": "reach", "from": var("x"), "to": var("y")},
                  {"predicate": "p", "from": var("y"), "to": var("z")}]}]});
    let node = |id: &str, readers: Value| json!({"id": id, "entity_id": id, "space_id": "s", "readers": readers});
    let edge = |id: &str, from: &str, to: &str, start: i64| json!({"id": id, "predicate": "p", "from": from, "to": to, "valid_time": {"start": start}});
    // Parallel edges give each conclusion several equivalent witnesses.
    let visible_nodes = vec![
        node("a", json!([])),
        node("b", json!([])),
        node("c", json!([])),
    ];
    let edges = vec![
        edge("ab0", "a", "b", 0),
        edge("ab5", "a", "b", 5),
        edge("bc0", "b", "c", 0),
        edge("bc3", "b", "c", 3),
        edge("ca0", "c", "a", 2),
    ];
    let reason = |hidden: Option<usize>| {
        let mut e = Engine::memory_with_clock(std::sync::Arc::new(ManualClock::new(10))).unwrap();
        let mut nodes = visible_nodes.clone();
        if let Some(k) = hidden {
            nodes.push(node(&format!("secret{k}"), json!(["mallory"])));
        }
        let writer = HostContext::new("writer", ["g".to_string()]);
        let commit =
            json!([{"op": "commit", "graph_id": "g", "data": {"nodes": nodes, "edges": edges}}]);
        let results = e.execute(&program(commit), &writer).unwrap();
        let Some(CommandResult::Committed { revision, .. }) = results.last() else {
            panic!("{results:?}")
        };
        let plan = evaluate(json!({"kind": "reason",
            "input": {"kind": "query", "query": {"graph_id": "g"}}, "rules": rules}));
        let carol = HostContext::new("carol", Vec::<String>::new());
        let out = e.execute(&program(plan), &carol).unwrap();
        let text = serde_json::to_string(&out)
            .unwrap()
            .replace(revision.as_str(), "~REV~");
        let mut value: Value = serde_json::from_str(&text).unwrap();
        flatten_conjunctions(&mut value);
        let mut value = rename_derived(&value);
        sort_arrays(&mut value);
        value.to_string()
    };
    let plain = reason(None);
    for k in 0..24 {
        let hidden = reason(Some(k));
        if plain != hidden {
            if let Ok(dir) = std::env::var("WEAVE_NI_DUMP") {
                std::fs::write(
                    format!("{dir}/rule-{k}-without.json"),
                    format!("{{}} {plain}"),
                )
                .unwrap();
                std::fs::write(
                    format!("{dir}/rule-{k}-with.json"),
                    format!("{{}} {hidden}"),
                )
                .unwrap();
            }
        }
        assert!(plain == hidden, "hidden record {k} changed the explanation");
    }
}

/// Found by the 10,000-case run (seed 521): proof conjunctions were nested
/// pairs folded in record order, which follows generated IDs and so revision
/// IDs. A hidden record in the same revision changed the nesting, and with it
/// whether the carrier depth budget was exceeded: `E_INFLUENCE_BUDGET` with
/// the hidden facts, a result without them. Conjunctions are now one flat,
/// canonically ordered set of conjuncts.
#[test]
fn proof_conjunctions_do_not_depend_on_hidden_records() {
    let spec = Spec::new(521, Profile::Registry);
    let mut plain = World::build(&spec, false);
    let mut mixed = World::build(&spec, true);
    setup(&mut plain, &spec);
    setup(&mut mixed, &spec);
    let plan = evaluate(json!({"kind": "support", "predicate": "q",
        "input": {"kind": "window", "window": {"start": 19, "end": 35},
            "input": {"kind": "query", "query": {"graph_id": "snap", "valid_at": 15}}},
        "from": {"entity_id": "E0", "space_id": "s0"}, "to": {"entity_id": "E3", "space_id": "s1"},
        "valid_at": 60}));
    let a = outcome(&mut plain, "alice", &plan);
    let b = outcome(&mut mixed, "alice", &plan);
    assert!(!a.starts_with("error"), "{a}");
    assert!(a == b, "{}", difference(&a, &b));
}

// ---------------------------------------------------------------- write outcomes

/// The error of a retraction with a live dependent, which the store with
/// hidden facts may return where the other succeeds (the declared
/// dependent channel, ADR-0008 section 4).
/// Write cases per world; the last retracts the lonely node.
const WRITE_CASES: usize = 20;
const DEPENDENT: &str = "error E_DEPENDENT_FACT a retracted fact still has live dependent facts";

/// One generated write by `alice`, who has Write on `g` in registry stores.
/// It corrects, retracts and asserts visible, hidden and missing records.
/// Asserts never name hidden IDs and edges never reference hidden nodes:
/// those compare with hidden versions in every store until B2 part 2
/// (ADR-0008 section 4).
fn plan_write(g: &mut Gen, case: usize) -> Value {
    if case == WRITE_CASES - 1 {
        // Every world ends on the node only a hidden record depends on.
        return json!([{"op": "write", "graph_id": "g", "changes": [
            {"change": "retract", "target": {"kind": "node", "id": LONELY}}]}]);
    }
    let mut changes = vec![];
    let mut used = std::collections::BTreeSet::new();
    for j in 0..g.rng.range(1, 3) {
        let change = match g.rng.below(8) {
            0 => {
                let mut node = json!({"kind": "node", "id": format!("new{case}-{j}"),
                    "entity_id": g.rng.pick_str(&ENTITIES), "space_id": g.rng.pick_str(&SPACES)});
                g.spec.restrict_visible(&mut g.rng, &mut node);
                json!({"change": "assert", "fact": node})
            }
            1 => {
                let from = g.rng.pick(&g.spec.visible_nodes).clone();
                let to = g.rng.pick(&g.spec.visible_nodes).clone();
                json!({"change": "assert", "fact": {"kind": "edge", "id": format!("new-e{case}-{j}"),
                    "predicate": "p", "from": from, "to": to, "valid_time": {"start": 0}}})
            }
            2 | 3 => {
                let id = g.rng.pick(&g.spec.visible_nodes).clone();
                let mut node = g.spec.node_records[&id].clone();
                node["properties"]["x"] = json!(100 + case);
                json!({"change": "correct", "fact": node})
            }
            4 => {
                let id = g.rng.pick(&g.spec.hidden_nodes).clone();
                json!({"change": "correct", "fact": {"kind": "node", "id": id,
                    "entity_id": g.rng.pick_str(&ENTITIES), "space_id": g.rng.pick_str(&SPACES)}})
            }
            5 => {
                let (kind, id) = if g.rng.chance(1, 2) {
                    ("node", g.rng.pick(&g.spec.visible_nodes).clone())
                } else {
                    ("edge", g.rng.pick(&g.spec.visible_edges).clone())
                };
                json!({"change": "retract", "target": {"kind": kind, "id": id}})
            }
            6 => {
                let (kind, id) = if g.rng.chance(1, 2) {
                    ("node", g.rng.pick(&g.spec.hidden_nodes).clone())
                } else {
                    ("edge", g.rng.pick(&g.spec.hidden_edges).clone())
                };
                json!({"change": "retract", "target": {"kind": kind, "id": id}})
            }
            _ => json!({"change": "retract", "target": {"kind": "node", "id": "nowhere"}}),
        };
        let key = change
            .pointer("/fact/id")
            .or_else(|| change.pointer("/target/id"))
            .cloned()
            .unwrap();
        if used.insert(key.to_string()) {
            changes.push(change);
        }
    }
    json!([{"op": "write", "graph_id": "g", "changes": changes}])
}

/// Hidden facts never change a writer's outcome in a registry store, except
/// through the declared dependent channel. This includes the B3 rule (#70)
/// that `correct` and `retract` of a fact the writer cannot read fail
/// exactly as for a missing fact.
#[test]
fn hidden_facts_never_change_a_writers_outcome_in_registry_stores() {
    // The set of worlds depends only on the target, never on the number of
    // threads or their timing: worlds run in batches of seeds, and a batch is
    // added only after the previous one is complete. (The first version
    // stopped on a shared running count, so a 3-core CI host ran fewer worlds
    // than an 8-core one.)
    let target = cases_per_operator();
    let base = first_seed() | 1;
    let mut counts = Counts::default();
    let mut started = 0usize;
    while counts.cases < target {
        let batch = (target - counts.cases).div_ceil(WRITE_CASES).max(1);
        let next = AtomicUsize::new(started);
        let end = started + batch;
        let results = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..threads() {
                scope.spawn(|| loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    if index >= end {
                        break;
                    }
                    // Registry worlds are the odd seeds.
                    let local = write_world(base + 2 * index as u64);
                    results.lock().unwrap().push(local);
                });
            }
        });
        started = end;
        for local in results.into_inner().unwrap() {
            counts.cases += local.cases;
            counts.ok += local.ok;
            counts.errors += local.errors;
            counts.declared += local.declared;
            for (code, n) in local.error_codes {
                *counts.error_codes.entry(code).or_default() += n;
            }
        }
    }
    println!(
        "NONINTERFERENCE_WRITES {}",
        json!({"worlds": started, "counts": counts})
    );
    assert!(counts.cases >= target);
    assert!(counts.ok > 0 && counts.errors > 0);
    assert!(
        counts.error_codes.contains_key("E_CONFLICT"),
        "corrections and retractions of hidden facts were exercised"
    );
    // Every world ends on the lonely node, so every world reaches the
    // dependent refusal at least once.
    assert!(
        counts.declared >= started,
        "the hidden dependents were reached in every world"
    );
}

/// One registry world of generated writes by `alice`.
fn write_world(seed: u64) -> Counts {
    let spec = Spec::new(seed, Profile::Registry);
    let mut plain = World::build(&spec, false);
    let mut mixed = World::build(&spec, true);
    plain.at(FINAL_TIME);
    mixed.at(FINAL_TIME);
    let mut local = Counts::default();
    let host = HostContext::new("alice", Vec::<String>::new());
    for case in 0..WRITE_CASES {
        let mut gen = Gen {
            spec: &spec,
            rng: Rng::new(seed.wrapping_mul(9_176_219) ^ case as u64),
        };
        let commands = plan_write(&mut gen, case);
        let a = plain.e.execute(&program(commands.clone()), &host);
        let b = mixed.e.execute(&program(commands.clone()), &host);
        // A new revision of the same visible state gets one label.
        for (world, result) in [(&mut plain, &a), (&mut mixed, &b)] {
            if let Ok(results) = result {
                if let Some(
                    CommandResult::Committed { revision, .. }
                    | CommandResult::Unchanged { revision },
                ) = results.last()
                {
                    world.token(&format!("W{case}"), revision.clone());
                }
            }
        }
        let text = |world: &World, r: weave_engine::Result<Vec<CommandResult>>| {
            canonical(
                world,
                r.map(|v| serde_json::to_value(v).unwrap())
                    .map_err(|e| (e.code, e.message)),
            )
        };
        let (a, b) = (text(&plain, a), text(&mixed, b));
        local.cases += 1;
        if a != b {
            assert!(
                b == DEPENDENT && commands.to_string().contains("retract"),
                "hidden facts changed alice's write outcome (seed {seed}, case {case})\nplan {commands}\n{}",
                difference(&a, &b)
            );
            // The stores now differ in visible state: stop this world.
            local.declared += 1;
            break;
        }
        if a.starts_with("error") {
            let code = a.split(' ').nth(1).unwrap_or_default().to_string();
            *local.error_codes.entry(code).or_default() += 1;
            local.errors += 1;
        } else {
            local.ok += 1;
        }
    }
    local
}

/// The dependent channel is exactly one bit: a hidden dependent and a
/// visible one give the same error, which names neither, and nothing changes.
#[test]
fn a_hidden_dependent_refuses_a_retraction_like_a_visible_one() {
    for registry in [false, true] {
        let clock = std::sync::Arc::new(ManualClock::new(10));
        let e = Engine::memory_with_clock(clock.clone()).unwrap();
        let (writer, alice) = if registry {
            let admin = HostContext::new("admin", Vec::<String>::new());
            e.enable_registry(&admin).unwrap();
            clock.set(11);
            let changes: Vec<RegistryChange> = serde_json::from_value(json!([
                {"change": "add_principal", "id": "alice", "kind": "user"},
                {"change": "add_principal", "id": "mallory", "kind": "user"},
                {"change": "grant", "graph_id": "g", "subject": "alice", "permission": "write"}
            ]))
            .unwrap();
            e.apply_registry(&changes, &admin).unwrap();
            (admin, HostContext::new("alice", Vec::<String>::new()))
        } else {
            let all = ["g".to_string()];
            (
                HostContext::new("writer", all.clone()),
                HostContext::new("alice", all),
            )
        };
        let mut e = e;
        let node = |id: &str| json!({"kind": "node", "id": id, "entity_id": id, "space_id": "s"});
        let edge = |id: &str, from: &str, readers: Value| {
            json!({"kind": "edge", "id": id, "predicate": "p", "from": from, "to": from,
                "valid_time": {"start": 0}, "readers": readers})
        };
        let assert = |f: Value| json!({"change": "assert", "fact": f});
        clock.set(12);
        e.execute(
            &program(json!([{"op": "write", "graph_id": "g", "changes": [
                assert(node("seen")), assert(node("unseen")), assert(node("free")),
                assert(edge("visible-dependent", "seen", json!([]))),
                assert(edge("hidden-dependent", "unseen", json!(["mallory"])))
            ]}])),
            &writer,
        )
        .unwrap();
        clock.set(13);
        let retract = |e: &mut Engine, id: &str| {
            let head = e.head("g", "main").unwrap();
            let result = e.execute(
                &program(json!([{"op": "write", "graph_id": "g", "changes": [
                    {"change": "retract", "target": {"kind": "node", "id": id}}]}])),
                &alice,
            );
            if result.is_err() {
                assert_eq!(
                    e.head("g", "main").unwrap(),
                    head,
                    "a refused write changes nothing"
                );
            }
            result
                .map(|_| ())
                .map_err(|error| (error.code, error.message))
        };
        let visible = retract(&mut e, "seen");
        let hidden = retract(&mut e, "unseen");
        assert_eq!(visible, hidden, "registry {registry}");
        assert_eq!(visible.unwrap_err().0, "E_DEPENDENT_FACT");
        let head = e.head("g", "main").unwrap();
        assert!(retract(&mut e, "free").is_ok());
        assert_ne!(e.head("g", "main").unwrap(), head);
    }
}
