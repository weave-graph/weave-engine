//! Deterministic finite-domain saturation with bounded, nonrecursive proof traces.
use crate::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{self, Write};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct FactKey {
    predicate: String,
    from: String,
    to: String,
    negative: bool,
    start: i64,
    end: Option<i64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Trace {
    module: String,
    revision: String,
    rule: String,
    bindings: BTreeMap<String, String>,
    premises: Vec<FactKey>,
    conclusion: FactKey,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Proof {
    leaves: Vec<AssertionRef>,
    nodes: Vec<NodeRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    snapshots: Vec<GraphRef>,
    trace: BTreeMap<String, Trace>,
    external: BTreeMap<String, Derivation>,
    contextual: bool,
}
#[derive(Clone, Serialize)]
struct State {
    bindings: BTreeMap<String, String>,
    start: i64,
    end: Option<i64>,
    proof: Proof,
    body: Vec<FactKey>,
}
type Facts = BTreeMap<FactKey, BTreeMap<String, Proof>>;
fn error(code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: message.into(),
    }
}
fn hash(value: &impl Serialize) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("rule records serialize"))
    )
}
fn unique<T: Serialize + Clone>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    items
        .into_iter()
        .map(|i| (hash(&i), i))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect()
}
struct Counter(usize);
impl Write for Counter {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(b.len())
            .ok_or_else(|| io::Error::other("rule byte budget"))?;
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn bytes(value: &impl Serialize, limit: usize) -> Result<usize, Diagnostic> {
    let mut c = Counter(limit);
    serde_json::to_writer(&mut c, value)
        .map_err(|_| error("E_RULE_BUDGET", "Rule memory byte budget exceeded"))?;
    Ok(limit - c.0)
}
fn charge(remaining: &mut usize, value: &impl Serialize) -> Result<(), Diagnostic> {
    *remaining = remaining
        .checked_sub(bytes(value, *remaining)?)
        .ok_or_else(|| error("E_RULE_BUDGET", "Rule byte budget exceeded"))?;
    Ok(())
}
fn tick(steps: &mut usize, limit: usize) -> Result<(), Diagnostic> {
    *steps = steps
        .checked_add(1)
        .filter(|n| *n <= limit)
        .ok_or_else(|| error("E_RULE_BUDGET", "Rule evaluation step budget exceeded"))?;
    Ok(())
}
fn proof_key(proof: &Proof) -> String {
    if proof.snapshots.is_empty() {
        hash(&(&proof.leaves, &proof.nodes, proof.contextual))
    } else {
        hash(&(
            &proof.leaves,
            &proof.nodes,
            &proof.snapshots,
            proof.contextual,
        ))
    }
}
fn insert(
    facts: &mut Facts,
    fact: FactKey,
    proof: Proof,
    limits: &RuleBudget,
    remaining: &mut usize,
) -> Result<bool, Diagnostic> {
    crate::influence::validate_record_refs(&proof.leaves, &proof.nodes, &proof.snapshots)?;
    let key = proof_key(&proof);
    if facts.get(&fact).is_some_and(|p| p.contains_key(&key)) {
        return Ok(false);
    }
    if !facts.contains_key(&fact) && facts.len() >= limits.max_facts {
        return Err(error("E_RULE_BUDGET", "Rule fact budget exceeded"));
    }
    if facts.get(&fact).map_or(0, BTreeMap::len) >= limits.max_derivations.min(128) {
        return Err(error(
            "E_RULE_BUDGET",
            "Rule alternative-support budget exceeded",
        ));
    }
    charge(remaining, &(&fact, &proof))?;
    facts.entry(fact).or_default().insert(key, proof);
    Ok(true)
}
fn atom_matches(atom: &RuleAtom, fact: &FactKey, bindings: &mut BTreeMap<String, String>) -> bool {
    if atom.predicate != fact.predicate || (atom.polarity == Polarity::Negative) != fact.negative {
        return false;
    }
    for (term, id) in [(&atom.from, &fact.from), (&atom.to, &fact.to)] {
        match term {
            RuleTerm::Node { id: constant } if constant != id => return false,
            RuleTerm::Variable { name } => {
                if let Some(existing) = bindings.get(name) {
                    if existing != id {
                        return false;
                    }
                } else {
                    bindings.insert(name.clone(), id.clone());
                }
            }
            _ => {}
        }
    }
    true
}
fn endpoint(term: &RuleTerm, bindings: &BTreeMap<String, String>) -> String {
    match term {
        RuleTerm::Node { id } => id.clone(),
        RuleTerm::Variable { name } => bindings[name].clone(),
    }
}
fn intersection(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}
fn merge(left: &Proof, right: &Proof) -> Result<Proof, Diagnostic> {
    let gates = crate::influence::merge(
        Some(&GraphInfluence {
            derivations: vec![],
            snapshots: left.snapshots.clone(),
            assertions: left.leaves.clone(),
            nodes: left.nodes.clone(),
        }),
        Some(&GraphInfluence {
            derivations: vec![],
            snapshots: right.snapshots.clone(),
            assertions: right.leaves.clone(),
            nodes: right.nodes.clone(),
        }),
    )?
    .expect("two proof groups");
    let mut trace = left.trace.clone();
    trace.extend(right.trace.clone());
    let mut external = left.external.clone();
    external.extend(right.external.clone());
    Ok(Proof {
        leaves: gates.assertions,
        nodes: gates.nodes,
        snapshots: gates.snapshots,
        trace,
        external,
        contextual: left.contextual || right.contextual,
    })
}
fn source_proofs(input: &QueryResult, edge: &Edge) -> Result<Vec<Proof>, Diagnostic> {
    let origins = input
        .edge_origins
        .get(&edge.id)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut groups = crate::algebra::edge_alternatives(edge, origins)?;
    if edge.derivations.is_empty() {
        for group in &mut groups {
            group.parameters.insert("attribution".into(), json!({"structural_ref":edge.structural_ref,"source":edge.assertion_source,"context":edge.assertion_context,"assertion_properties":edge.assertion_properties}));
        }
    }
    groups
        .into_iter()
        .map(|d| {
            if (d.premises.is_empty()
                && d.node_premises.is_empty()
                && d.snapshot_premises.is_empty())
                || d.premises.iter().any(|p| !origins.contains(p))
            {
                return Err(error(
                    "E_RULE_PROVENANCE",
                    "Rule support must contain nonempty authorized leaf premises",
                ));
            }
            let mut trace = BTreeMap::new();
            let mut external = BTreeMap::new();
            if d.operator == "weave:finite-rules-v1" {
                let traces: Vec<Trace> =
                    serde_json::from_value(d.parameters.get("trace").cloned().unwrap_or(json!([])))
                        .map_err(|_| error("E_RULE_PROVENANCE", "Invalid prior rule trace"))?;
                for t in traces {
                    trace.insert(hash(&t), t);
                }
                let prior: Vec<Derivation> = serde_json::from_value(
                    d.parameters.get("external").cloned().unwrap_or(json!([])),
                )
                .map_err(|_| error("E_RULE_PROVENANCE", "Invalid external rule premises"))?;
                for p in prior {
                    if (p.premises.is_empty()
                        && p.node_premises.is_empty()
                        && p.snapshot_premises.is_empty())
                        || p.premises.iter().any(|leaf| !d.premises.contains(leaf))
                        || p.snapshot_premises
                            .iter()
                            .any(|pin| !d.snapshot_premises.contains(pin))
                        || p.node_premises
                            .iter()
                            .any(|node| !d.node_premises.contains(node))
                    {
                        return Err(error(
                            "E_RULE_PROVENANCE",
                            "Prior trace refers outside its authorized leaf support",
                        ));
                    }
                    external.insert(hash(&p), p);
                }
            } else {
                external.insert(hash(&d), d.clone());
            }
            Ok(Proof {
                leaves: unique(d.premises),
                nodes: unique(d.node_premises),
                snapshots: unique(d.snapshot_premises),
                trace,
                external,
                contextual: crate::context::ensure_consumable(
                    input.selected_context.as_ref(),
                    edge.assertion_context.as_ref(),
                )
                .is_err(),
            })
        })
        .collect()
}
fn normalize_schema(graph: &mut GraphData) {
    let Some(source) = graph.schema.clone() else {
        return;
    };
    let type_id = |id: &str| {
        if id.starts_with("weave:algebra:type:") {
            id.to_owned()
        } else {
            format!("weave:algebra:type:{}", hash(&(&source, id)))
        }
    };
    for n in &mut graph.nodes {
        if let Some(id) = &n.type_id {
            n.type_id = Some(type_id(id));
        }
    }
    for e in &mut graph.edges {
        if let Some(id) = &e.type_id {
            e.type_id = Some(type_id(id));
        }
    }
    let nodes = source
        .nodes
        .iter()
        .map(|(id, def)| (type_id(id), def.clone()))
        .collect();
    let edges = source
        .edges
        .iter()
        .map(|(id, def)| {
            let mut shape = def.clone();
            shape.from_type = type_id(&def.from_type);
            shape.to_type = type_id(&def.to_type);
            (type_id(id), shape)
        })
        .collect();
    graph.schema = Some(GraphSchema {
        id: format!("weave:rule:schema:{}", hash(&(&nodes, &edges))),
        revision: "1".into(),
        nodes,
        edges,
    });
}
/// Complete finite fixed point or an explicit failure. Never silently truncate.
pub fn reason(
    mut input: QueryResult,
    set: &RuleSet,
    limits: &RuleBudget,
    ctx: &AlgebraContext,
) -> Result<QueryResult, Diagnostic> {
    if let Some(e) = validate_rule_set(set).into_iter().next() {
        return Err(e);
    }
    if ctx.principal.is_empty() {
        return Err(error("E_RULE_CONTEXT", "Rules require a trusted principal"));
    }
    if input.graph.profile != GraphProfile::Legacy
        || !input.graph.structural_edges.is_empty()
        || !input.graph.assertions.is_empty()
    {
        return Err(error(
            "E_RULE_PROFILE",
            "Rules consume authorized materialized graph values",
        ));
    }
    crate::context_typing::validate_result(&input)?;
    crate::influence::validate_graph(&input.graph)?;
    input.graph.influence = crate::influence::input_influence(&input.graph)?;
    if let Some(influence) = &input.graph.influence {
        if !influence.snapshots.is_empty() {
            input.input_snapshots = unique(
                input
                    .input_snapshots
                    .iter()
                    .chain(&influence.snapshots)
                    .cloned(),
            );
        }
    }
    bytes(&input, ctx.max_output_bytes)?;
    bytes(set, ctx.max_output_bytes)?;
    if let Some(d) = validate_schema_graph(&input.graph).into_iter().next() {
        return Err(d);
    }
    normalize_schema(&mut input.graph);
    let nodes: BTreeMap<_, _> = input
        .graph
        .nodes
        .iter()
        .map(|n| (n.id.clone(), n.clone()))
        .collect();
    if nodes.len() != input.graph.nodes.len()
        || input.graph.nodes.len() + input.graph.edges.len() + input.graph.attachments.len()
            > ctx.max_objects
    {
        return Err(error(
            "E_RULE_BUDGET",
            "Rule input object budget exceeded or node IDs are ambiguous",
        ));
    }
    for rule in &set.rules {
        for atom in rule.body.iter().chain(std::iter::once(&rule.head)) {
            for term in [&atom.from, &atom.to] {
                if let RuleTerm::Node { id } = term {
                    if !nodes.contains_key(id) {
                        return Err(error(
                            "E_RULE_DOMAIN",
                            "Rule constants must name nodes in the finite input domain",
                        ));
                    }
                }
            }
        }
    }
    let (context_assertions, context_nodes) = input
        .graph
        .context_typing
        .as_ref()
        .map(crate::context_typing::gates)
        .transpose()?
        .unwrap_or_default();
    let whole = crate::influence::merge(
        input.graph.influence.as_ref(),
        Some(&GraphInfluence {
            derivations: vec![],
            snapshots: vec![],
            assertions: context_assertions,
            nodes: context_nodes,
        }),
    )?
    .expect("context influence exists");
    let mut facts = Facts::new();
    let mut fact_bytes = ctx.max_output_bytes;
    for edge in &input.graph.edges {
        if !nodes.contains_key(&edge.from)
            || !nodes.contains_key(&edge.to)
            || !edge.valid_time.valid()
        {
            return Err(error(
                "E_RULE_INPUT",
                "Invalid rule input endpoints or interval",
            ));
        }
        let fact = FactKey {
            predicate: edge.predicate.clone(),
            from: edge.from.clone(),
            to: edge.to.clone(),
            negative: edge.polarity == Polarity::Negative,
            start: edge.valid_time.start,
            end: edge.valid_time.end,
        };
        for proof in source_proofs(&input, edge)? {
            let gates = crate::influence::merge(
                Some(&GraphInfluence {
                    assertions: proof.leaves.clone(),
                    nodes: proof.nodes.clone(),
                    snapshots: proof.snapshots.clone(),
                    derivations: vec![],
                }),
                Some(&whole),
            )?
            .expect("proof");
            if gates.derivations.is_empty() {
                let mut proof = proof;
                proof.leaves = gates.assertions;
                proof.nodes = gates.nodes;
                proof.snapshots = gates.snapshots;
                insert(&mut facts, fact.clone(), proof, limits, &mut fact_bytes)?;
            } else {
                use crate::carrier_algebra as c;
                let mut budget = c::Budget::new(c::Limits {
                    bytes: fact_bytes,
                    ..c::Limits::default()
                });
                let carrier = c::from_influence(&gates, &mut budget)?;
                for group in
                    c::distribute(&carrier, &c::Carrier::default(), &mut budget)?.alternatives
                {
                    let mut branch = proof.clone();
                    branch.external.insert(hash(&group), group.clone());
                    branch.leaves = group.premises;
                    branch.nodes = group.node_premises;
                    branch.snapshots = group.snapshot_premises;
                    insert(&mut facts, fact.clone(), branch, limits, &mut fact_bytes)?;
                }
            }
        }
    }
    let empty = Proof {
        snapshots: vec![],
        leaves: vec![],
        nodes: vec![],
        trace: BTreeMap::new(),
        external: BTreeMap::new(),
        contextual: false,
    };
    let mut steps = 0;
    let mut complete = false;
    for _round in 0..limits.max_rounds {
        let snapshot = facts.clone();
        let mut changed = false;
        for rule in &set.rules {
            let mut states = vec![State {
                bindings: BTreeMap::new(),
                start: i64::MIN,
                end: None,
                proof: empty.clone(),
                body: vec![],
            }];
            for atom in &rule.body {
                let mut next = Vec::new();
                let mut state_bytes = ctx.max_output_bytes;
                for state in &states {
                    for (fact, proofs) in &snapshot {
                        tick(&mut steps, limits.max_steps)?;
                        let mut bindings = state.bindings.clone();
                        if !atom_matches(atom, fact, &mut bindings) {
                            continue;
                        }
                        let start = state.start.max(fact.start);
                        let end = intersection(state.end, fact.end);
                        if end.is_some_and(|end| start >= end) {
                            continue;
                        }
                        for proof in proofs.values() {
                            tick(&mut steps, limits.max_steps)?;
                            if proof.contextual {
                                return Err(error(
                                    "E_CONTEXT_REQUIRED",
                                    "Contextual rules require explicit compatible context selection",
                                ));
                            }
                            let mut body = state.body.clone();
                            body.push(fact.clone());
                            let candidate = State {
                                bindings: bindings.clone(),
                                start,
                                end,
                                proof: merge(&state.proof, proof)?,
                                body,
                            };
                            if next.len() >= ctx.max_objects {
                                return Err(error("E_RULE_BUDGET", "Rule binding budget exceeded"));
                            }
                            charge(&mut state_bytes, &candidate)?;
                            next.push(candidate);
                        }
                    }
                }
                states = next;
                if states.is_empty() {
                    break;
                }
            }
            for state in states {
                tick(&mut steps, limits.max_steps)?;
                let fact = FactKey {
                    predicate: rule.head.predicate.clone(),
                    from: endpoint(&rule.head.from, &state.bindings),
                    to: endpoint(&rule.head.to, &state.bindings),
                    negative: rule.head.polarity == Polarity::Negative,
                    start: state.start,
                    end: state.end,
                };
                if !rule.allow_cross_space && nodes[&fact.from].space_id != nodes[&fact.to].space_id
                {
                    return Err(error(
                        "E_RULE_SPACE",
                        "A cross-space conclusion requires an explicit rule declaration",
                    ));
                }
                let mut proof = state.proof;
                // Same fact+leaf set already has a canonical witness. Do not grow
                // traces by repeatedly traversing a recursive proof cycle.
                if facts
                    .get(&fact)
                    .is_some_and(|p| p.contains_key(&proof_key(&proof)))
                {
                    continue;
                }
                let trace = Trace {
                    module: set.id.clone(),
                    revision: set.revision.clone(),
                    rule: rule.id.clone(),
                    bindings: state.bindings,
                    premises: state.body,
                    conclusion: fact.clone(),
                };
                proof.trace.insert(hash(&trace), trace);
                if proof.trace.len() > ctx.max_objects {
                    return Err(error("E_RULE_BUDGET", "Rule proof trace budget exceeded"));
                }
                changed |= insert(&mut facts, fact, proof, limits, &mut fact_bytes)?;
            }
        }
        if !changed {
            complete = true;
            break;
        }
    }
    if !complete {
        return Err(error(
            "E_RULE_BUDGET",
            "Rule round budget exhausted before a complete fixed point",
        ));
    }
    let set_digest = crate::identity::source_fingerprint(set)?;
    if input.source_revisions.iter().any(|source| {
        source.name == set.id && source.revision == set.revision && source.digest != set_digest
    }) {
        return Err(error(
            "E_SOURCE_REVISION",
            "Rule module label conflicts with an existing source digest",
        ));
    }
    let mut output_bytes = ctx.max_output_bytes;
    charge(&mut output_bytes, &input)?;
    let mut output: BTreeMap<_, _> = input
        .graph
        .edges
        .iter()
        .map(|e| (e.id.clone(), e.clone()))
        .collect();
    for (fact, proofs) in facts {
        let proofs = proofs
            .into_values()
            .filter(|p| !p.trace.is_empty())
            .collect::<Vec<_>>();
        if proofs.is_empty() {
            continue;
        }
        let identity = if whole.snapshots.is_empty() {
            hash(&(&set_digest, &fact, &input.selected_context))
        } else {
            hash(&(
                &set_digest,
                &fact,
                &input.selected_context,
                &whole.snapshots,
            ))
        };
        let id = format!("rule:{identity}");
        let mut type_id = None;
        if let Some(schema) = &mut input.graph.schema {
            let from_type = nodes[&fact.from].type_id.clone().ok_or_else(|| {
                error("E_SCHEMA_RULE", "Typed rule output has an untyped endpoint")
            })?;
            let to_type = nodes[&fact.to].type_id.clone().ok_or_else(|| {
                error("E_SCHEMA_RULE", "Typed rule output has an untyped endpoint")
            })?;
            let name = format!(
                "weave:algebra:type:rule:{}",
                hash(&(
                    &set_digest,
                    &fact.predicate,
                    &from_type,
                    &to_type,
                    nodes[&fact.from].space_id != nodes[&fact.to].space_id
                ))
            );
            let shape = EdgeSchema {
                from_type,
                to_type,
                properties: BTreeMap::new(),
                allow_cross_space: nodes[&fact.from].space_id != nodes[&fact.to].space_id,
                allow_extra_properties: false,
            };
            if schema.edges.get(&name).is_some_and(|old| old != &shape) {
                return Err(error(
                    "E_SCHEMA_RULE",
                    "Derived rule type collides with an existing type",
                ));
            }
            schema.edges.insert(name.clone(), shape);
            type_id = Some(name);
        }
        let mut derivations = Vec::new();
        for proof in proofs {
            let snapshots = unique(
                proof
                    .leaves
                    .iter()
                    .map(|p| GraphRef {
                        graph_id: p.graph_id.clone(),
                        revision: p.revision.clone(),
                    })
                    .chain(proof.nodes.iter().map(|p| GraphRef {
                        graph_id: p.graph_id.clone(),
                        revision: p.revision.clone(),
                    })),
            );
            derivations.push(Derivation {
                snapshot_premises: proof.snapshots.clone(),
                node_premises: proof.nodes,
                operator: "weave:finite-rules-v1".into(),
                premises: proof.leaves,
                parameters: [
                    (
                        "module".into(),
                        json!({"id":set.id,"revision":set.revision,"digest":set_digest}),
                    ),
                    (
                        "context".into(),
                        json!(input.selected_context.clone().unwrap_or_default()),
                    ),
                    (
                        "trace".into(),
                        json!(proof.trace.into_values().collect::<Vec<_>>()),
                    ),
                    (
                        "external".into(),
                        json!(proof.external.into_values().collect::<Vec<_>>()),
                    ),
                ]
                .into(),
                input_snapshots: unique(snapshots.into_iter().chain(proof.snapshots)),
            });
        }
        let origins = unique(derivations.iter().flat_map(|d| d.premises.clone()));
        let edge = Edge {
            derived_snapshots: whole.snapshots.clone(),
            derived_nodes: vec![],
            id: id.clone(),
            type_id,
            structural_ref: None,
            assertion_source: None,
            assertion_context: input
                .selected_context
                .as_ref()
                .and_then(ContextSelection::reference)
                .cloned(),
            assertion_properties: BTreeMap::new(),
            predicate: fact.predicate,
            from: fact.from,
            to: fact.to,
            valid_time: Interval {
                start: fact.start,
                end: fact.end,
            },
            polarity: if fact.negative {
                Polarity::Negative
            } else {
                Polarity::Positive
            },
            properties: BTreeMap::new(),
            metadata: vec![],
            readers: vec![ctx.principal.clone()],
            derived_from: origins.clone(),
            derivations,
        };
        if output.get(&id).is_some_and(|e| {
            e.predicate != edge.predicate
                || e.from != edge.from
                || e.to != edge.to
                || e.derivations
                    .iter()
                    .any(|d| d.operator != "weave:finite-rules-v1")
        }) {
            return Err(error(
                "E_RULE_ID",
                "Derived rule identity collides with a source object",
            ));
        }
        charge(&mut output_bytes, &edge)?;
        input.edge_origins.insert(id.clone(), origins);
        output.insert(id, edge);
        if input.graph.nodes.len() + output.len() + input.graph.attachments.len() > ctx.max_objects
        {
            return Err(error("E_RULE_BUDGET", "Rule output object budget exceeded"));
        }
    }
    input.graph.edges = output.into_values().collect();
    if let Some(schema) = &mut input.graph.schema {
        schema.id = format!(
            "weave:rule:schema:{}",
            hash(&(&schema.nodes, &schema.edges))
        );
        schema.revision = "1".into();
    }
    for node in &mut input.graph.nodes {
        node.readers = vec![ctx.principal.clone()];
    }
    for edge in &mut input.graph.edges {
        edge.readers = vec![ctx.principal.clone()];
    }
    for a in &mut input.graph.attachments {
        a.readers = vec![ctx.principal.clone()];
    }
    input.provenance = unique(input.edge_origins.values().flatten().cloned());
    input.source_revisions.push(SourceRevision {
        name: set.id.clone(),
        revision: set.revision.clone(),
        digest: set_digest,
    });
    input.source_revisions = unique(input.source_revisions);
    input.version = VERSION.into();
    crate::influence::validate_graph(&input.graph)?;
    if let Some(d) = validate_schema_graph(&input.graph).into_iter().next() {
        return Err(d);
    }
    bytes(&input, ctx.max_output_bytes)?;
    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> QueryResult {
        let mut value:QueryResult=serde_json::from_value(json!({"version":VERSION,"graph":{"nodes":[{"id":"a","entity_id":"a","space_id":"s"},{"id":"b","entity_id":"b","space_id":"s"},{"id":"c","entity_id":"c","space_id":"s"}],"edges":[{"id":"ab","predicate":"link","from":"a","to":"b","valid_time":{"start":0,"end":10}},{"id":"bc","predicate":"link","from":"b","to":"c","valid_time":{"start":5,"end":20}},{"id":"ca","predicate":"link","from":"c","to":"a","valid_time":{"start":0,"end":20}}]},"snapshots":{"G":"r1"},"input_snapshots":[{"graph_id":"G","revision":"r1"}],"coverage":"complete","diagnostics":[],"provenance":[],"metadata_graphs":[]})).unwrap();
        for e in &value.graph.edges {
            value.edge_origins.insert(
                e.id.clone(),
                vec![AssertionRef {
                    graph_id: "G".into(),
                    revision: "r1".into(),
                    assertion_id: e.id.clone(),
                }],
            );
        }
        for n in &value.graph.nodes {
            value.node_origins.insert(
                n.id.clone(),
                vec![NodeRef {
                    graph_id: "G".into(),
                    revision: "r1".into(),
                    node_id: n.id.clone(),
                }],
            );
        }
        value
    }
    fn variable(name: &str) -> RuleTerm {
        RuleTerm::Variable { name: name.into() }
    }
    fn atom(predicate: &str, from: &str, to: &str) -> RuleAtom {
        RuleAtom {
            predicate: predicate.into(),
            from: variable(from),
            to: variable(to),
            polarity: Polarity::Positive,
        }
    }
    fn rules() -> RuleSet {
        RuleSet {
            id: "reachability".into(),
            revision: "1".into(),
            rules: vec![
                Rule {
                    id: "seed".into(),
                    head: atom("reach", "x", "y"),
                    body: vec![atom("link", "x", "y")],
                    allow_cross_space: false,
                },
                Rule {
                    id: "step".into(),
                    head: atom("reach", "x", "z"),
                    body: vec![atom("reach", "x", "y"), atom("link", "y", "z")],
                    allow_cross_space: false,
                },
            ],
        }
    }
    fn limits() -> RuleBudget {
        RuleBudget {
            max_steps: 1_000_000,
            max_rounds: 32,
            max_facts: 1000,
            max_derivations: 128,
        }
    }
    fn ctx() -> AlgebraContext {
        AlgebraContext {
            principal: "reader".into(),
            max_objects: 2000,
            max_output_bytes: 4_000_000,
        }
    }
    #[test]
    fn cyclic_reachability_terminates_with_temporal_proof_and_is_idempotent() {
        let result = reason(fixture(), &rules(), &limits(), &ctx()).unwrap();
        let ac = result
            .graph
            .edges
            .iter()
            .find(|e| {
                e.predicate == "reach"
                    && e.from == "a"
                    && e.to == "c"
                    && e.valid_time.start == 5
                    && e.valid_time.end == Some(10)
            })
            .unwrap();
        assert!(ac.derivations.iter().any(|d| d.premises.len() == 2));
        assert!(ac
            .derivations
            .iter()
            .all(|d| d.parameters["trace"].is_array()));
        assert!(result
            .graph
            .edges
            .iter()
            .any(|e| e.predicate == "reach" && e.from == e.to));
        let repeated = reason(result.clone(), &rules(), &limits(), &ctx()).unwrap();
        assert_eq!(result.graph, repeated.graph);
        assert_eq!(result.coverage, Coverage::Complete);
    }
    #[test]
    fn unsafe_variables_constants_and_budget_exhaustion_are_not_empty_results() {
        let mut invalid = rules();
        invalid.rules[0].head.to = variable("unbound");
        assert_eq!(
            reason(fixture(), &invalid, &limits(), &ctx())
                .unwrap_err()
                .code,
            "E_RULE_RANGE"
        );
        invalid = rules();
        invalid.rules[0].head.to = RuleTerm::Node {
            id: "new-node".into(),
        };
        assert_eq!(
            reason(fixture(), &invalid, &limits(), &ctx())
                .unwrap_err()
                .code,
            "E_RULE_DOMAIN"
        );
        let mut budget = limits();
        budget.max_rounds = 1;
        assert_eq!(
            reason(fixture(), &rules(), &budget, &ctx())
                .unwrap_err()
                .code,
            "E_RULE_BUDGET"
        );
        budget = limits();
        budget.max_steps = 1;
        assert_eq!(
            reason(fixture(), &rules(), &budget, &ctx())
                .unwrap_err()
                .code,
            "E_RULE_BUDGET"
        );
        let mut context = ctx();
        context.max_output_bytes = 100;
        assert_eq!(
            reason(fixture(), &rules(), &limits(), &context)
                .unwrap_err()
                .code,
            "E_RULE_BUDGET"
        );
    }
    #[test]
    fn signed_negative_atoms_do_not_mean_absence_and_context_is_explicit() {
        let mut set = rules();
        set.rules = vec![Rule {
            id: "negative-evidence".into(),
            head: atom("refuted", "x", "y"),
            body: vec![RuleAtom {
                polarity: Polarity::Negative,
                ..atom("link", "x", "y")
            }],
            allow_cross_space: false,
        }];
        let result = reason(fixture(), &set, &limits(), &ctx()).unwrap();
        assert!(!result.graph.edges.iter().any(|e| e.predicate == "refuted"));
        let mut input = fixture();
        input.graph.edges[0].polarity = Polarity::Negative;
        assert!(reason(input, &set, &limits(), &ctx())
            .unwrap()
            .graph
            .edges
            .iter()
            .any(|e| e.predicate == "refuted"));
        let mut input = fixture();
        input.graph.edges[0].assertion_context = Some(GraphRef {
            graph_id: "world".into(),
            revision: "1".into(),
        });
        assert_eq!(
            reason(input, &rules(), &limits(), &ctx()).unwrap_err().code,
            "E_CONTEXT_REQUIRED"
        );
    }
    #[test]
    fn typed_rule_outputs_remain_composable_with_original_schema() {
        let mut input = fixture();
        input.graph.schema=Some(serde_json::from_value(json!({"id":"S","revision":"1","nodes":{"N":{}},"edges":{"Link":{"from_type":"N","to_type":"N"}}})).unwrap());
        for n in &mut input.graph.nodes {
            n.type_id = Some("N".into());
        }
        for e in &mut input.graph.edges {
            e.type_id = Some("Link".into());
        }
        let derived = reason(input.clone(), &rules(), &limits(), &ctx()).unwrap();
        assert!(validate_schema_graph(&derived.graph).is_empty());
        let merged = crate::algebra::union(derived, input, &ctx()).unwrap();
        assert!(validate_schema_graph(&merged.graph).is_empty());
        assert_eq!(merged.graph.nodes.len(), 3);
    }
    #[test]
    fn partial_input_stays_partial_and_disjoint_intervals_do_not_derive() {
        let mut input = fixture();
        input.graph.edges.remove(2);
        input.graph.edges[1].valid_time = Interval {
            start: 10,
            end: Some(20),
        };
        input.coverage = Coverage::Partial;
        let result = reason(input, &rules(), &limits(), &ctx()).unwrap();
        assert_eq!(result.coverage, Coverage::Partial);
        assert!(!result
            .graph
            .edges
            .iter()
            .any(|e| e.predicate == "reach" && e.from == "a" && e.to == "c"));
    }
    #[test]
    fn independent_support_sets_remain_alternative_and_limit_is_explicit() {
        let mut input = fixture();
        input.graph.edges.remove(2);
        let mut direct = input.graph.edges[0].clone();
        direct.id = "ac".into();
        direct.to = "c".into();
        direct.valid_time = Interval {
            start: 5,
            end: Some(10),
        };
        input.graph.edges.push(direct);
        input.edge_origins.insert(
            "ac".into(),
            vec![AssertionRef {
                graph_id: "G".into(),
                revision: "r1".into(),
                assertion_id: "ac".into(),
            }],
        );
        let result = reason(input.clone(), &rules(), &limits(), &ctx()).unwrap();
        let conclusion = result
            .graph
            .edges
            .iter()
            .find(|e| e.predicate == "reach" && e.from == "a" && e.to == "c")
            .unwrap();
        let alternatives: Vec<_> = conclusion
            .derivations
            .iter()
            .map(|d| {
                d.premises
                    .iter()
                    .map(|p| p.assertion_id.as_str())
                    .collect::<std::collections::BTreeSet<_>>()
            })
            .collect();
        assert!(alternatives.contains(&["ac"].into()));
        assert!(alternatives.contains(&["ab", "bc"].into()));
        let mut limited = limits();
        limited.max_derivations = 1;
        assert_eq!(
            reason(input, &rules(), &limited, &ctx()).unwrap_err().code,
            "E_RULE_BUDGET"
        );
    }
    #[test]
    fn malformed_prior_support_cannot_create_authority() {
        let mut input = fixture();
        input.graph.edges[0].derivations = vec![Derivation {
            snapshot_premises: vec![],
            node_premises: vec![],
            operator: "weave:finite-rules-v1".into(),
            premises: vec![],
            parameters: BTreeMap::new(),
            input_snapshots: vec![],
        }];
        assert_eq!(
            reason(input.clone(), &rules(), &limits(), &ctx())
                .unwrap_err()
                .code,
            "E_RULE_PROVENANCE"
        );
        input.graph.edges[0].derivations[0].premises = vec![AssertionRef {
            graph_id: "forged".into(),
            revision: "r".into(),
            assertion_id: "x".into(),
        }];
        assert_eq!(
            reason(input, &rules(), &limits(), &ctx()).unwrap_err().code,
            "E_RULE_PROVENANCE"
        );
        let mut set = rules();
        let mut input = fixture();
        input.source_revisions.push(SourceRevision {
            name: set.id.clone(),
            revision: set.revision.clone(),
            digest: "different".into(),
        });
        assert_eq!(
            reason(input, &set, &limits(), &ctx()).unwrap_err().code,
            "E_SOURCE_REVISION"
        );
        set.rules = vec![set.rules[0].clone(); 65];
        assert_eq!(validate_rule_set(&set).len(), 1);
        set.rules.truncate(1);
        set.rules[0].body = vec![set.rules[0].body[0].clone(); 9];
        assert_eq!(validate_rule_set(&set).len(), 1);
    }
}
