use crate::{failure, Analysis, ComponentMode, Limits, Metric};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use weave_contract::{Polarity, QueryResult};
use weave_engine::Result;

struct Work {
    remaining: u64,
}
impl Work {
    fn take(&mut self, amount: usize) -> Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(amount as u64)
            .ok_or_else(|| failure("E_SCIENCE_BUDGET", "algorithm exhausted its work budget"))?;
        Ok(())
    }
}

struct Topology {
    ids: Vec<String>,
    indices: BTreeMap<String, usize>,
    outgoing: Vec<Vec<usize>>,
    incoming: Vec<Vec<usize>>,
    edges: usize,
    negative: usize,
    outside_time: usize,
}
impl Topology {
    fn new(input: &QueryResult, valid_at: Option<i64>, work: &mut Work) -> Result<Self> {
        work.take(
            input
                .graph
                .nodes
                .len()
                .saturating_add(input.graph.edges.len()),
        )?;
        let mut ids: Vec<_> = input
            .graph
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect();
        ids.sort();
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(failure(
                "E_SCIENCE_TOPOLOGY",
                "selected graph contains duplicate node IDs",
            ));
        }
        let indices: BTreeMap<_, _> = ids
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, id)| (id, i))
            .collect();
        let mut topology = Self {
            outgoing: vec![vec![]; ids.len()],
            incoming: vec![vec![]; ids.len()],
            ids,
            indices,
            edges: 0,
            negative: 0,
            outside_time: 0,
        };
        for edge in &input.graph.edges {
            if edge.polarity == Polarity::Negative {
                topology.negative += 1;
                continue;
            }
            if valid_at.is_some_and(|time| !edge.valid_time.contains(time)) {
                topology.outside_time += 1;
                continue;
            }
            let from = topology.indices.get(&edge.from).copied().ok_or_else(|| {
                failure(
                    "E_SCIENCE_TOPOLOGY",
                    "selected positive edge has an unavailable endpoint",
                )
            })?;
            let to = topology.indices.get(&edge.to).copied().ok_or_else(|| {
                failure(
                    "E_SCIENCE_TOPOLOGY",
                    "selected positive edge has an unavailable endpoint",
                )
            })?;
            topology.outgoing[from].push(to);
            topology.incoming[to].push(from);
            topology.edges += 1;
        }
        for neighbors in topology.outgoing.iter_mut().chain(&mut topology.incoming) {
            neighbors.sort_unstable();
        }
        Ok(topology)
    }
    fn semantics(&self, valid_at: Option<i64>) -> Value {
        json!({
            "algorithm_version":"0.1.0", "topology":"directed_positive_multigraph",
            "node_scope":"all nodes of the selected authorized QueryResult",
            "weights":"unweighted; edge properties do not change transition or path weights",
            "self_loops":"one incoming and one outgoing incidence",
            "parallel_edges":"retained; each contributes to degree and PageRank transition probability",
            "negative_assertions":"excluded, without resolving or erasing the source evidence",
            "excluded_negative_edges":self.negative, "excluded_outside_time_edges":self.outside_time,
            "valid_at":valid_at, "time_scope":if valid_at.is_some() {"half_open_valid_time_sample"} else {"selected_interval_union"},
            "determinism":"lexical node/tie order; repeatable f64 operation order; compare platform math using tolerances",
            "authority":"recomputed from runtime-authorized inputs; scope is the caller's visible selected graph",
            "provenance":"complete original QueryResult retained in input; analytics are observations of that selection"
        })
    }
}

pub(super) fn analyze(
    input: &QueryResult,
    analysis: &Analysis,
    limits: &Limits,
) -> Result<(Value, Value)> {
    if input.graph.nodes.len() > limits.max_nodes || input.graph.edges.len() > limits.max_edges {
        return Err(failure(
            "E_SCIENCE_BUDGET",
            "selected graph exceeds its node or edge budget",
        ));
    }
    let mut work = Work {
        remaining: limits.max_work,
    };
    if let Analysis::NearestVectors {
        property,
        space_id,
        query,
        metric,
        k,
    } = analysis
    {
        let output = nearest(
            input, property, space_id, query, *metric, *k, limits, &mut work,
        )?;
        return Ok((
            output,
            json!({
                "algorithm_version":"0.1.0", "exact":true, "space_id":space_id,
                "property":property, "metric":metric, "distance_order":"ascending; ties by lexical node ID",
                "missing_property":"excluded", "invalid_vector":"rejected", "zero_cosine_vector":"rejected",
                "encoder_scope":"raw numeric vectors in an explicitly chosen space; caller owns encoder identity",
                "authority":"only nodes in the runtime-authorized selected QueryResult",
                "provenance":"complete original QueryResult retained in input"
            }),
        ));
    }
    let topology = Topology::new(input, analysis.valid_at(), &mut work)?;
    let output = match analysis {
        Analysis::Degree { .. } => degree(&topology, &mut work)?,
        Analysis::Components { mode, .. } => components(&topology, *mode, &mut work)?,
        Analysis::ShortestPaths {
            source,
            directed,
            target,
            ..
        } => shortest_paths(&topology, source, *directed, target.as_deref(), &mut work)?,
        Analysis::Pagerank {
            damping,
            tolerance,
            max_iterations,
            ..
        } => pagerank(&topology, *damping, *tolerance, *max_iterations, &mut work)?,
        Analysis::NearestVectors { .. } => unreachable!(),
    };
    Ok((output, topology.semantics(analysis.valid_at())))
}

fn degree(graph: &Topology, work: &mut Work) -> Result<Value> {
    work.take(graph.ids.len())?;
    let nodes: BTreeMap<_, _> = graph
        .ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            (
                id.clone(),
                json!({
                    "in_degree":graph.incoming[i].len(), "out_degree":graph.outgoing[i].len(),
                    "total_degree":graph.incoming[i].len() + graph.outgoing[i].len()
                }),
            )
        })
        .collect();
    Ok(
        json!({"algorithm":"degree","nodes":nodes,"node_count":graph.ids.len(),"edge_count":graph.edges}),
    )
}

fn components(graph: &Topology, mode: ComponentMode, work: &mut Work) -> Result<Value> {
    let n = graph.ids.len();
    let mut groups = vec![];
    let mut seen = vec![false; n];
    match mode {
        ComponentMode::Weak => {
            for start in 0..n {
                if seen[start] {
                    continue;
                }
                let mut stack = vec![start];
                seen[start] = true;
                let mut group = vec![];
                while let Some(node) = stack.pop() {
                    work.take(1 + graph.outgoing[node].len() + graph.incoming[node].len())?;
                    group.push(graph.ids[node].clone());
                    for &neighbor in graph.outgoing[node].iter().chain(&graph.incoming[node]) {
                        if !seen[neighbor] {
                            seen[neighbor] = true;
                            stack.push(neighbor);
                        }
                    }
                }
                group.sort();
                groups.push(group);
            }
        }
        ComponentMode::Strong => {
            // Iterative Kosaraju avoids recursion and process-stack limits on long chains.
            let mut order = Vec::with_capacity(n);
            for start in 0..n {
                if seen[start] {
                    continue;
                }
                seen[start] = true;
                let mut stack = vec![(start, 0)];
                while let Some((node, cursor)) = stack.last_mut() {
                    work.take(1)?;
                    if *cursor < graph.outgoing[*node].len() {
                        let neighbor = graph.outgoing[*node][*cursor];
                        *cursor += 1;
                        if !seen[neighbor] {
                            seen[neighbor] = true;
                            stack.push((neighbor, 0));
                        }
                    } else {
                        let (node, _) = stack.pop().expect("nonempty DFS stack");
                        order.push(node);
                    }
                }
            }
            seen.fill(false);
            for start in order.into_iter().rev() {
                if seen[start] {
                    continue;
                }
                let mut stack = vec![start];
                seen[start] = true;
                let mut group = vec![];
                while let Some(node) = stack.pop() {
                    work.take(1 + graph.incoming[node].len())?;
                    group.push(graph.ids[node].clone());
                    for &neighbor in &graph.incoming[node] {
                        if !seen[neighbor] {
                            seen[neighbor] = true;
                            stack.push(neighbor);
                        }
                    }
                }
                group.sort();
                groups.push(group);
            }
        }
    }
    groups.sort();
    Ok(
        json!({"algorithm":"components","mode":mode,"components":groups,"node_count":n,"edge_count":graph.edges}),
    )
}

fn shortest_paths(
    graph: &Topology,
    source: &str,
    directed: bool,
    target: Option<&str>,
    work: &mut Work,
) -> Result<Value> {
    let start = graph
        .indices
        .get(source)
        .copied()
        .ok_or_else(|| failure("E_SCIENCE_SOURCE", "source is not in the selected node set"))?;
    let goal = target
        .map(|id| {
            graph.indices.get(id).copied().ok_or_else(|| {
                failure("E_SCIENCE_TARGET", "target is not in the selected node set")
            })
        })
        .transpose()?;
    let mut distance = vec![None; graph.ids.len()];
    let mut previous = vec![None; graph.ids.len()];
    distance[start] = Some(0usize);
    let mut queue = VecDeque::from([start]);
    while let Some(node) = queue.pop_front() {
        let mut neighbors = graph.outgoing[node].clone();
        if !directed {
            neighbors.extend(&graph.incoming[node]);
            neighbors.sort_unstable();
        }
        work.take(1 + neighbors.len())?;
        for neighbor in neighbors {
            if distance[neighbor].is_none() {
                distance[neighbor] = Some(distance[node].expect("queued node has distance") + 1);
                previous[neighbor] = Some(node);
                queue.push_back(neighbor);
            }
        }
    }
    let distances: BTreeMap<_, _> = graph.ids.iter().cloned().zip(&distance).collect();
    let predecessors: BTreeMap<_, _> = graph
        .ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), previous[i].map(|j| graph.ids[j].clone())))
        .collect();
    let path = goal.and_then(|goal| {
        distance[goal]?;
        let mut path = vec![graph.ids[goal].clone()];
        let mut cursor = goal;
        while cursor != start {
            cursor = previous[cursor]?;
            path.push(graph.ids[cursor].clone());
        }
        path.reverse();
        Some(path)
    });
    Ok(
        json!({"algorithm":"shortest_paths","source":source,"directed":directed,"target":target,"distances":distances,"predecessors":predecessors,"path":path}),
    )
}

fn pagerank(
    graph: &Topology,
    damping: f64,
    tolerance: f64,
    max_iterations: usize,
    work: &mut Work,
) -> Result<Value> {
    if !damping.is_finite()
        || !(0.0..1.0).contains(&damping)
        || !tolerance.is_finite()
        || tolerance <= 0.0
        || max_iterations == 0
        || max_iterations > 10_000
    {
        return Err(failure("E_SCIENCE_PARAMETER", "PageRank requires 0 <= damping < 1, positive finite tolerance, and 1..10000 iterations"));
    }
    let n = graph.ids.len();
    if n == 0 {
        return Ok(
            json!({"algorithm":"pagerank","scores":{},"iterations":0,"converged":true,"residual":0.0}),
        );
    }
    let mut score = vec![1.0 / n as f64; n];
    let mut residual = 0.0;
    let mut completed = 0;
    let mut converged = false;
    for step in 1..=max_iterations {
        work.take(n.saturating_mul(3).saturating_add(graph.edges))?;
        let dangling: f64 = score
            .iter()
            .enumerate()
            .filter(|(i, _)| graph.outgoing[*i].is_empty())
            .map(|(_, value)| value)
            .sum();
        let base = (1.0 - damping + damping * dangling) / n as f64;
        let mut next = vec![base; n];
        for (node, neighbors) in graph.outgoing.iter().enumerate() {
            if !neighbors.is_empty() {
                let weight = damping * score[node] / neighbors.len() as f64;
                for &neighbor in neighbors {
                    next[neighbor] += weight;
                }
            }
        }
        residual = next.iter().zip(&score).map(|(a, b)| (a - b).abs()).sum();
        score = next;
        completed = step;
        if residual <= tolerance {
            converged = true;
            break;
        }
    }
    let scores: BTreeMap<_, _> = graph.ids.iter().cloned().zip(score).collect();
    Ok(
        json!({"algorithm":"pagerank","scores":scores,"iterations":completed,"converged":converged,"residual":residual,"residual_norm":"L1 absolute","damping":damping,"tolerance":tolerance}),
    )
}

fn normed(vector: &[f64]) -> Result<Vec<f64>> {
    let scale = vector.iter().map(|x| x.abs()).fold(0.0, f64::max);
    if scale == 0.0 {
        return Err(failure(
            "E_SCIENCE_VECTOR",
            "cosine distance requires nonzero vectors",
        ));
    }
    let scaled: Vec<_> = vector.iter().map(|x| x / scale).collect();
    let norm = scaled.iter().map(|x| x * x).sum::<f64>().sqrt();
    Ok(scaled.into_iter().map(|x| x / norm).collect())
}

#[allow(clippy::too_many_arguments)]
fn nearest(
    input: &QueryResult,
    property: &str,
    space_id: &str,
    query: &[f64],
    metric: Metric,
    k: usize,
    limits: &Limits,
    work: &mut Work,
) -> Result<Value> {
    if property.is_empty()
        || space_id.is_empty()
        || query.is_empty()
        || query.len() > limits.max_vector_dimensions
        || query.iter().any(|x| !x.is_finite())
        || k == 0
        || k > limits.max_nodes
    {
        return Err(failure(
            "E_SCIENCE_VECTOR",
            "bounded nonempty finite query, property, space, and positive k are required",
        ));
    }
    work.take(query.len())?;
    let normalized_query = match metric {
        Metric::Cosine => Some(normed(query)?),
        Metric::Euclidean => None,
    };
    let mut candidates = vec![];
    for node in &input.graph.nodes {
        work.take(1)?;
        if node.space_id != space_id {
            continue;
        }
        let Some(raw) = node.properties.get(property) else {
            continue;
        };
        let raw = raw.as_array().ok_or_else(|| {
            failure(
                "E_SCIENCE_VECTOR",
                "selected vector property must be an array",
            )
        })?;
        if raw.len() != query.len() {
            return Err(failure(
                "E_SCIENCE_VECTOR",
                "vectors in the selected space must share query dimensions",
            ));
        }
        work.take(query.len().saturating_mul(4))?;
        let vector: Vec<f64> = raw
            .iter()
            .map(|value| {
                value.as_f64().filter(|x| x.is_finite()).ok_or_else(|| {
                    failure(
                        "E_SCIENCE_VECTOR",
                        "selected vector coordinates must be finite numbers",
                    )
                })
            })
            .collect::<Result<_>>()?;
        let distance = match metric {
            Metric::Euclidean => vector
                .iter()
                .zip(query)
                .fold(0.0f64, |norm, (a, b)| norm.hypot(a - b)),
            Metric::Cosine => {
                let normalized = normed(&vector)?;
                let similarity: f64 = normalized
                    .iter()
                    .zip(normalized_query.as_ref().expect("cosine query"))
                    .map(|(a, b)| a * b)
                    .sum();
                1.0 - similarity.clamp(-1.0, 1.0)
            }
        };
        if !distance.is_finite() {
            return Err(failure(
                "E_SCIENCE_NUMERIC",
                "distance exceeds finite f64 range",
            ));
        }
        candidates.push((
            node.id.clone(),
            node.entity_id.clone(),
            node.space_id.clone(),
            distance,
        ));
    }
    let candidate_count = candidates.len();
    candidates
        .sort_by(|(id_a, _, _, a), (id_b, _, _, b)| a.total_cmp(b).then_with(|| id_a.cmp(id_b)));
    candidates.truncate(k);
    let neighbors: Vec<_> = candidates.into_iter().map(|(id, entity_id, space_id, distance)| json!({"id":id,"entity_id":entity_id,"space_id":space_id,"distance":distance})).collect();
    Ok(
        json!({"algorithm":"nearest_vectors","neighbors":neighbors,"candidate_count":candidate_count,"dimensions":query.len(),"metric":metric,"exact":true}),
    )
}
