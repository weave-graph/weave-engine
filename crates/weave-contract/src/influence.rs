//! Restriction-only influence references. Serialized references never grant authority.
use crate::carrier_algebra as carrier;
use crate::{AssertionRef, Diagnostic, GraphData, GraphRef, NodeRef};
use serde::{Deserialize, Serialize};

pub const MAX_REFERENCES: usize = 1000;
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphInfluence {
    /// Whole-value alternatives, conjunctive with all declared flat gates.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "crate::carrier_profile::bounded_derivations"
    )]
    pub derivations: Vec<crate::Derivation>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "bounded_refs"
    )]
    pub assertions: Vec<AssertionRef>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "bounded_refs"
    )]
    pub nodes: Vec<NodeRef>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "bounded_snapshot_refs"
    )]
    pub snapshots: Vec<GraphRef>,
}
/// Bounded decoding shared by every restriction-only snapshot carrier.
pub fn bounded_snapshot_refs<'de, D>(deserializer: D) -> Result<Vec<GraphRef>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    bounded_refs(deserializer)
}
pub fn bounded_refs<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Visitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "at most {MAX_REFERENCES} influence references")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut values: A,
        ) -> Result<Vec<T>, A::Error> {
            if values.size_hint().is_some_and(|n| n > MAX_REFERENCES) {
                return Err(serde::de::Error::custom("influence reference limit"));
            }
            let mut result = Vec::new();
            while let Some(value) = values.next_element()? {
                if result.len() == MAX_REFERENCES {
                    return Err(serde::de::Error::custom("influence reference limit"));
                }
                result.push(value);
            }
            Ok(result)
        }
    }
    deserializer.deserialize_seq(Visitor(std::marker::PhantomData))
}
fn error(code: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: "invalid or excessive graph influence".into(),
    }
}
fn id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512
}
pub fn assertion_key(reference: &AssertionRef) -> (&str, &str, &str) {
    (
        &reference.graph_id,
        &reference.revision,
        &reference.assertion_id,
    )
}
pub fn node_key(reference: &NodeRef) -> (&str, &str, &str) {
    (&reference.graph_id, &reference.revision, &reference.node_id)
}
pub fn validate_refs(assertions: &[AssertionRef], nodes: &[NodeRef]) -> Result<(), Diagnostic> {
    validate_record_refs(assertions, nodes, &[])
}
pub fn snapshot_key(reference: &GraphRef) -> (&str, &str) {
    (&reference.graph_id, &reference.revision)
}
pub fn validate_record_refs(
    assertions: &[AssertionRef],
    nodes: &[NodeRef],
    snapshots: &[GraphRef],
) -> Result<(), Diagnostic> {
    if assertions
        .len()
        .saturating_add(nodes.len())
        .saturating_add(snapshots.len())
        > MAX_REFERENCES
    {
        return Err(error("E_BUDGET"));
    }
    if assertions
        .iter()
        .any(|r| !id(&r.graph_id) || !id(&r.revision) || !id(&r.assertion_id))
        || nodes
            .iter()
            .any(|r| !id(&r.graph_id) || !id(&r.revision) || !id(&r.node_id))
        || snapshots
            .iter()
            .any(|r| !id(&r.graph_id) || !id(&r.revision))
    {
        return Err(error("E_INFLUENCE"));
    }
    Ok(())
}
pub fn validate(influence: &GraphInfluence) -> Result<(), Diagnostic> {
    validate_record_refs(
        &influence.assertions,
        &influence.nodes,
        &influence.snapshots,
    )?;
    if !influence.derivations.is_empty() {
        let mut budget = carrier::Budget::new(carrier::Limits::default());
        carrier::from_influence(influence, &mut budget)?;
    }
    Ok(())
}
pub fn canonicalize(influence: &mut GraphInfluence) {
    influence
        .assertions
        .sort_by(|a, b| assertion_key(a).cmp(&assertion_key(b)));
    influence.assertions.dedup();
    influence
        .nodes
        .sort_by(|a, b| node_key(a).cmp(&node_key(b)));
    influence.nodes.dedup();
    canonicalize_snapshots(&mut influence.snapshots);
}
pub fn canonicalize_snapshots(refs: &mut Vec<GraphRef>) {
    refs.sort_by(|a, b| snapshot_key(a).cmp(&snapshot_key(b)));
    refs.dedup();
}
pub fn merge(
    left: Option<&GraphInfluence>,
    right: Option<&GraphInfluence>,
) -> Result<Option<GraphInfluence>, Diagnostic> {
    for influence in left.into_iter().chain(right) {
        validate(influence)?;
    }
    if left.is_none() && right.is_none() {
        return Ok(None);
    }
    if left
        .into_iter()
        .chain(right)
        .any(|i| !i.derivations.is_empty())
    {
        let mut budget = carrier::Budget::new(carrier::Limits::default());
        let a = left
            .map(|i| carrier::from_influence(i, &mut budget))
            .transpose()?
            .unwrap_or_default();
        let b = right
            .map(|i| carrier::from_influence(i, &mut budget))
            .transpose()?
            .unwrap_or_default();
        return Ok(Some(carrier::into_influence(carrier::conjunction(
            &a,
            &b,
            &mut budget,
        )?)));
    }
    let mut assertion_keys = std::collections::BTreeSet::new();
    let mut node_keys = std::collections::BTreeSet::new();
    let mut snapshot_keys = std::collections::BTreeSet::new();
    let mut output = GraphInfluence::default();
    for influence in left.into_iter().chain(right) {
        for reference in &influence.assertions {
            if assertion_keys.insert(assertion_key(reference)) {
                if assertion_keys.len() + node_keys.len() + snapshot_keys.len() > MAX_REFERENCES {
                    return Err(error("E_BUDGET"));
                }
                output.assertions.push(reference.clone());
            }
        }
        for reference in &influence.nodes {
            if node_keys.insert(node_key(reference)) {
                if assertion_keys.len() + node_keys.len() + snapshot_keys.len() > MAX_REFERENCES {
                    return Err(error("E_BUDGET"));
                }
                output.nodes.push(reference.clone());
            }
        }
        for reference in &influence.snapshots {
            if snapshot_keys.insert(snapshot_key(reference)) {
                if assertion_keys.len() + node_keys.len() + snapshot_keys.len() > MAX_REFERENCES {
                    return Err(error("E_BUDGET"));
                }
                output.snapshots.push(reference.clone());
            }
        }
    }
    canonicalize(&mut output);
    Ok(Some(output))
}
pub fn validate_graph(data: &GraphData) -> Result<(), Diagnostic> {
    if let Some(influence) = &data.influence {
        validate(influence)?;
    }
    let mut carrier_budget = carrier::Budget::new(carrier::Limits::default());
    for node in &data.nodes {
        if !node.derivations.is_empty() {
            carrier::from_parts(
                &node.derived_from,
                &node.derived_nodes,
                &node.derived_snapshots,
                &node.derivations,
                &mut carrier_budget,
            )?;
        }
        validate_record_refs(
            &node.derived_from,
            &node.derived_nodes,
            &node.derived_snapshots,
        )?;
    }
    for attachment in &data.attachments {
        if !attachment.derivations.is_empty() {
            carrier::from_parts(
                &attachment.derived_from,
                &attachment.derived_nodes,
                &attachment.derived_snapshots,
                &attachment.derivations,
                &mut carrier_budget,
            )?;
        }
        validate_record_refs(
            &attachment.derived_from,
            &attachment.derived_nodes,
            &attachment.derived_snapshots,
        )?;
        if let Some(origin) = &attachment.origin {
            validate_refs(std::slice::from_ref(origin), &[])?;
        }
        if attachment
            .derived_from
            .len()
            .saturating_add(attachment.derived_nodes.len())
            .saturating_add(attachment.derived_snapshots.len())
            .saturating_add(usize::from(attachment.origin.is_some()))
            > MAX_REFERENCES
        {
            return Err(error("E_BUDGET"));
        }
    }
    for edge in &data.edges {
        if edge
            .derivations
            .iter()
            .any(|g| !g.snapshot_premises.is_empty())
        {
            carrier::from_parts(
                &[],
                &edge.derived_nodes,
                &edge.derived_snapshots,
                &edge.derivations,
                &mut carrier_budget,
            )?;
        }
        validate_record_refs(
            &edge.derived_from,
            &edge.derived_nodes,
            &edge.derived_snapshots,
        )?;
        if edge.derivations.len() > 128 {
            return Err(error("E_BUDGET"));
        }
        for group in &edge.derivations {
            validate_record_refs(
                &group.premises,
                &group.node_premises,
                &group.snapshot_premises,
            )?;
        }
    }
    for assertion in &data.assertions {
        if assertion
            .derivations
            .iter()
            .any(|g| !g.snapshot_premises.is_empty())
        {
            carrier::from_parts(
                &[],
                &assertion.derived_nodes,
                &assertion.derived_snapshots,
                &assertion.derivations,
                &mut carrier_budget,
            )?;
        }
        validate_record_refs(
            &assertion.derived_from,
            &assertion.derived_nodes,
            &assertion.derived_snapshots,
        )?;
        if assertion.derivations.len() > 128 {
            return Err(error("E_BUDGET"));
        }
        for group in &assertion.derivations {
            validate_record_refs(
                &group.premises,
                &group.node_premises,
                &group.snapshot_premises,
            )?;
        }
    }
    Ok(())
}

/// Collect only declared whole-snapshot gates, never descriptive execution pins.
/// Validate per-record raw counts before deduplication, and cap the union before cloning.
pub fn collect_declared_snapshots(data: &GraphData) -> Result<Vec<GraphRef>, Diagnostic> {
    validate_graph(data)?;
    let mut refs = std::collections::BTreeMap::new();
    let lists = data
        .influence
        .iter()
        .map(|i| i.snapshots.as_slice())
        .chain(data.nodes.iter().map(|n| n.derived_snapshots.as_slice()))
        .chain(data.edges.iter().map(|e| e.derived_snapshots.as_slice()))
        .chain(
            data.assertions
                .iter()
                .map(|a| a.derived_snapshots.as_slice()),
        )
        .chain(
            data.attachments
                .iter()
                .map(|a| a.derived_snapshots.as_slice()),
        );
    for reference in lists.flatten() {
        refs.insert(snapshot_key(reference), reference);
        if refs.len() > MAX_REFERENCES {
            return Err(error("E_BUDGET"));
        }
    }
    Ok(refs.into_values().cloned().collect())
}
/// Preserve declared snapshot gates and the three explicit attachment carrier kinds
/// when a pure operation can discard every member. Ordinary node/edge proof pins,
/// attachment origins, and descriptive query pins are not upgraded to global gates.
pub fn input_influence(data: &GraphData) -> Result<Option<GraphInfluence>, Diagnostic> {
    let snapshots = collect_declared_snapshots(data)?;
    let mut assertions = std::collections::BTreeMap::new();
    let mut nodes = std::collections::BTreeMap::new();
    for reference in data
        .influence
        .iter()
        .flat_map(|i| &i.assertions)
        .chain(data.attachments.iter().flat_map(|a| &a.derived_from))
    {
        assertions.insert(assertion_key(reference), reference);
        if assertions.len() + snapshots.len() > MAX_REFERENCES {
            return Err(error("E_BUDGET"));
        }
    }
    for reference in data
        .influence
        .iter()
        .flat_map(|i| &i.nodes)
        .chain(data.attachments.iter().flat_map(|a| &a.derived_nodes))
    {
        nodes.insert(node_key(reference), reference);
        if assertions.len() + nodes.len() + snapshots.len() > MAX_REFERENCES {
            return Err(error("E_BUDGET"));
        }
    }
    if data.influence.is_none() && assertions.is_empty() && nodes.is_empty() && snapshots.is_empty()
    {
        return Ok(None);
    }
    let grouped_refs = data
        .influence
        .iter()
        .flat_map(|i| &i.derivations)
        .map(|g| {
            g.premises
                .len()
                .saturating_add(g.node_premises.len())
                .saturating_add(g.snapshot_premises.len())
        })
        .sum::<usize>();
    if grouped_refs
        .saturating_add(assertions.len())
        .saturating_add(nodes.len())
        .saturating_add(snapshots.len())
        > MAX_REFERENCES
    {
        return Err(error("E_BUDGET"));
    }
    Ok(Some(GraphInfluence {
        derivations: data
            .influence
            .as_ref()
            .map(|i| i.derivations.clone())
            .unwrap_or_default(),
        assertions: assertions.into_values().cloned().collect(),
        nodes: nodes.into_values().cloned().collect(),
        snapshots,
    }))
}

/// New influence-only dependency snapshots; callers add legacy metadata/source paths too.
pub fn snapshots(data: &GraphData) -> Vec<crate::GraphRef> {
    let mut refs = std::collections::BTreeSet::new();
    for reference in data
        .influence
        .iter()
        .flat_map(|i| &i.snapshots)
        .chain(data.nodes.iter().flat_map(|n| &n.derived_snapshots))
        .chain(data.edges.iter().flat_map(|e| &e.derived_snapshots))
        .chain(data.assertions.iter().flat_map(|a| &a.derived_snapshots))
        .chain(data.attachments.iter().flat_map(|a| &a.derived_snapshots))
    {
        refs.insert((&reference.graph_id, &reference.revision));
    }
    if let Some(influence) = &data.influence {
        for r in &influence.assertions {
            refs.insert((&r.graph_id, &r.revision));
        }
        for r in &influence.nodes {
            refs.insert((&r.graph_id, &r.revision));
        }
    }
    for attachment in &data.attachments {
        for r in &attachment.derived_from {
            refs.insert((&r.graph_id, &r.revision));
        }
        for r in &attachment.derived_nodes {
            refs.insert((&r.graph_id, &r.revision));
        }
    }
    for nodes in data
        .edges
        .iter()
        .map(|e| &e.derived_nodes)
        .chain(data.assertions.iter().map(|a| &a.derived_nodes))
    {
        for r in nodes {
            refs.insert((&r.graph_id, &r.revision));
        }
    }
    for group in data
        .edges
        .iter()
        .flat_map(|e| &e.derivations)
        .chain(data.assertions.iter().flat_map(|a| &a.derivations))
        .chain(data.nodes.iter().flat_map(|a| &a.derivations))
        .chain(data.attachments.iter().flat_map(|a| &a.derivations))
        .chain(data.influence.iter().flat_map(|a| &a.derivations))
    {
        for r in &group.premises {
            refs.insert((&r.graph_id, &r.revision));
        }
        for r in &group.snapshot_premises {
            refs.insert((&r.graph_id, &r.revision));
        }
        for r in &group.node_premises {
            refs.insert((&r.graph_id, &r.revision));
        }
    }
    refs.into_iter()
        .map(|(graph_id, revision)| crate::GraphRef {
            graph_id: graph_id.clone(),
            revision: revision.clone(),
        })
        .collect()
}
struct Bytes(usize);
impl std::io::Write for Bytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(std::io::Error::other("influence output limit"));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn charge(bytes: &mut Bytes, value: &impl Serialize) -> Result<(), Diagnostic> {
    serde_json::to_writer(bytes, value).map_err(|_| error("E_BUDGET"))
}
/// Only for freshly generated records. Original pinned records must retain their payload;
/// operators retaining originals carry the whole-value envelope without using this helper.
pub fn protect_generated_result(
    result: &mut crate::QueryResult,
    limit: usize,
) -> Result<(), Diagnostic> {
    if result.graph.profile != crate::GraphProfile::Legacy
        || !result.graph.assertions.is_empty()
        || !result.graph.structural_edges.is_empty()
    {
        return Err(error("E_INFLUENCE_PROFILE"));
    }
    let Some(influence) = &result.graph.influence else {
        return Ok(());
    };
    validate(influence)?;
    if crate::carrier_profile::requires_v019(&result.graph) {
        return protect_alternatives(result, limit);
    }
    let mut budget = Bytes(limit.min(32 * 1024 * 1024));
    charge(&mut budget, result)?;
    // Charge every repeated proof insertion before mutating the output.
    for node in &result.graph.nodes {
        if node
            .derived_from
            .len()
            .saturating_add(node.derived_nodes.len())
            .saturating_add(node.derived_snapshots.len())
            .saturating_add(influence.snapshots.len())
            .saturating_add(influence.assertions.len())
            .saturating_add(influence.nodes.len())
            > MAX_REFERENCES
        {
            return Err(error("E_BUDGET"));
        }
        charge(&mut budget, influence)?;
    }
    for edge in &result.graph.edges {
        if edge
            .derived_from
            .len()
            .saturating_add(edge.derived_nodes.len())
            .saturating_add(edge.derived_snapshots.len())
            .saturating_add(influence.snapshots.len())
            .saturating_add(influence.assertions.len())
            .saturating_add(influence.nodes.len())
            > MAX_REFERENCES
        {
            return Err(error("E_BUDGET"));
        }
        charge(&mut budget, influence)?;
        for group in &edge.derivations {
            if group
                .premises
                .len()
                .saturating_add(group.node_premises.len())
                .saturating_add(influence.assertions.len())
                .saturating_add(influence.nodes.len())
                > MAX_REFERENCES
            {
                return Err(error("E_BUDGET"));
            }
            charge(&mut budget, influence)?;
            // Snapshot indexes repeat both existing and added proof pins. Charge their
            // actual serialized fields before constructing those repeated records.
            for (graph, revision) in group
                .premises
                .iter()
                .map(|p| (&p.graph_id, &p.revision))
                .chain(
                    group
                        .node_premises
                        .iter()
                        .map(|p| (&p.graph_id, &p.revision)),
                )
                .chain(
                    influence
                        .assertions
                        .iter()
                        .map(|p| (&p.graph_id, &p.revision)),
                )
                .chain(influence.nodes.iter().map(|p| (&p.graph_id, &p.revision)))
            {
                #[derive(Serialize)]
                struct Pin<'a> {
                    graph_id: &'a str,
                    revision: &'a str,
                }
                charge(
                    &mut budget,
                    &Pin {
                        graph_id: graph,
                        revision,
                    },
                )?;
            }
        }
    }
    for attachment in &result.graph.attachments {
        if attachment
            .derived_snapshots
            .len()
            .saturating_add(usize::from(attachment.origin.is_some()))
            .saturating_add(attachment.derived_from.len())
            .saturating_add(attachment.derived_nodes.len())
            .saturating_add(influence.assertions.len())
            .saturating_add(influence.nodes.len())
            .saturating_add(influence.snapshots.len())
            > MAX_REFERENCES
        {
            return Err(error("E_BUDGET"));
        }
        charge(&mut budget, influence)?;
    }
    for attachment in &mut result.graph.attachments {
        attachment
            .derived_from
            .extend(influence.assertions.iter().cloned());
        attachment
            .derived_from
            .sort_by(|a, b| assertion_key(a).cmp(&assertion_key(b)));
        attachment.derived_from.dedup();
        attachment
            .derived_nodes
            .extend(influence.nodes.iter().cloned());
        attachment
            .derived_nodes
            .sort_by(|a, b| node_key(a).cmp(&node_key(b)));
        attachment.derived_nodes.dedup();
        attachment
            .derived_snapshots
            .extend(influence.snapshots.iter().cloned());
        canonicalize_snapshots(&mut attachment.derived_snapshots);
    }
    for node in &mut result.graph.nodes {
        node.derived_snapshots
            .extend(influence.snapshots.iter().cloned());
        canonicalize_snapshots(&mut node.derived_snapshots);
        node.derived_from
            .extend(influence.assertions.iter().cloned());
        node.derived_from
            .sort_by(|a, b| assertion_key(a).cmp(&assertion_key(b)));
        node.derived_from.dedup();
        node.derived_nodes.extend(influence.nodes.iter().cloned());
        node.derived_nodes
            .sort_by(|a, b| node_key(a).cmp(&node_key(b)));
        node.derived_nodes.dedup();
    }
    for edge in &mut result.graph.edges {
        edge.derived_snapshots
            .extend(influence.snapshots.iter().cloned());
        canonicalize_snapshots(&mut edge.derived_snapshots);
        edge.derived_from
            .extend(influence.assertions.iter().cloned());
        edge.derived_from
            .sort_by(|a, b| assertion_key(a).cmp(&assertion_key(b)));
        edge.derived_from.dedup();
        edge.derived_nodes.extend(influence.nodes.iter().cloned());
        edge.derived_nodes
            .sort_by(|a, b| node_key(a).cmp(&node_key(b)));
        edge.derived_nodes.dedup();
        for group in &mut edge.derivations {
            group.premises.extend(influence.assertions.iter().cloned());
            group
                .premises
                .sort_by(|a, b| assertion_key(a).cmp(&assertion_key(b)));
            group.premises.dedup();
            group.node_premises.extend(influence.nodes.iter().cloned());
            group
                .node_premises
                .sort_by(|a, b| node_key(a).cmp(&node_key(b)));
            group.node_premises.dedup();
            let pins = group
                .premises
                .iter()
                .map(|p| crate::GraphRef {
                    graph_id: p.graph_id.clone(),
                    revision: p.revision.clone(),
                })
                .chain(group.node_premises.iter().map(|p| crate::GraphRef {
                    graph_id: p.graph_id.clone(),
                    revision: p.revision.clone(),
                }));
            group.input_snapshots.extend(pins);
            group
                .input_snapshots
                .sort_by(|a, b| (&a.graph_id, &a.revision).cmp(&(&b.graph_id, &b.revision)));
            group.input_snapshots.dedup();
        }
    }
    // Descriptive indexes and snapshots are part of the budget as well.
    for edge in &result.graph.edges {
        charge(&mut budget, &edge.derived_from)?;
    }
    for attachment in &result.graph.attachments {
        charge(&mut budget, &attachment.derived_from)?;
    }
    charge(&mut budget, &influence.assertions)?;
    let pins = snapshots(&result.graph);
    charge(&mut budget, &pins)?;
    for edge in &result.graph.edges {
        let refs = result.edge_origins.entry(edge.id.clone()).or_default();
        refs.extend(edge.derived_from.iter().cloned());
        refs.sort_by(|a, b| assertion_key(a).cmp(&assertion_key(b)));
        refs.dedup();
    }
    for attachment in &result.graph.attachments {
        let refs = result
            .attachment_origins
            .entry(attachment.id.clone())
            .or_default();
        refs.extend(attachment.derived_from.iter().cloned());
        refs.sort_by(|a, b| assertion_key(a).cmp(&assertion_key(b)));
        refs.dedup();
    }
    result
        .provenance
        .extend(influence.assertions.iter().cloned());
    result
        .provenance
        .sort_by(|a, b| assertion_key(a).cmp(&assertion_key(b)));
    result.provenance.dedup();
    result.input_snapshots.extend(pins);
    result
        .input_snapshots
        .sort_by(|a, b| (&a.graph_id, &a.revision).cmp(&(&b.graph_id, &b.revision)));
    result.input_snapshots.dedup();
    validate_graph(&result.graph)?;
    charge(&mut Bytes(limit.min(32 * 1024 * 1024)), result)
}

// The input is freshly generated. Stage every bounded replacement before mutation,
// including the descriptive indexes; a failed cross-product leaves the caller intact.
fn protect_alternatives(result: &mut crate::QueryResult, limit: usize) -> Result<(), Diagnostic> {
    charge(&mut Bytes(limit.min(32 * 1024 * 1024)), result)?;
    let mut staged = result.clone();
    protect_alternatives_staged(&mut staged, limit)?;
    *result = staged;
    Ok(())
}
fn protect_alternatives_staged(
    result: &mut crate::QueryResult,
    limit: usize,
) -> Result<(), Diagnostic> {
    let mut bytes = Bytes(limit.min(32 * 1024 * 1024));
    charge(&mut bytes, result)?;
    let mut budget = carrier::Budget::new(carrier::Limits {
        bytes: bytes.0,
        ..carrier::Limits::default()
    });
    let envelope = carrier::from_influence(
        result.graph.influence.as_ref().expect("influence"),
        &mut budget,
    )?;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut attachments = Vec::new();
    for n in &result.graph.nodes {
        let local = carrier::from_parts(
            &n.derived_from,
            &n.derived_nodes,
            &n.derived_snapshots,
            &n.derivations,
            &mut budget,
        )?;
        nodes.push(carrier::into_influence(carrier::conjunction(
            &local,
            &envelope,
            &mut budget,
        )?));
    }
    for e in &result.graph.edges {
        let flat = if e.derivations.is_empty() {
            e.derived_from.as_slice()
        } else {
            &[]
        };
        let local = carrier::from_parts(
            flat,
            &e.derived_nodes,
            &e.derived_snapshots,
            &e.derivations,
            &mut budget,
        )?;
        let mut value = carrier::conjunction(&local, &envelope, &mut budget)?;
        // Edges have no separate flat assertion field when alternatives exist.
        if !value.alternatives.is_empty()
            && (!value.flat.assertions.is_empty()
                || !value.flat.nodes.is_empty()
                || !value.flat.snapshots.is_empty())
        {
            let flat = carrier::Carrier {
                flat: value.flat.clone(),
                alternatives: vec![],
            };
            let branch = carrier::Carrier {
                flat: carrier::RefSet::default(),
                alternatives: value.alternatives,
            };
            // Explicitly distribute flat gates into every branch, retaining traces.
            value = carrier::distribute(&branch, &flat, &mut budget)?;
            // Retain legacy global node/snapshot gates as well as each branch's
            // closed proof, so either record representation preserves the restriction.
            value.flat.nodes = flat.flat.nodes;
            value.flat.snapshots = flat.flat.snapshots;
        }
        edges.push(carrier::into_influence(value));
    }
    for a in &result.graph.attachments {
        let local = carrier::from_parts(
            &a.derived_from,
            &a.derived_nodes,
            &a.derived_snapshots,
            &a.derivations,
            &mut budget,
        )?;
        attachments.push(carrier::into_influence(carrier::conjunction(
            &local,
            &envelope,
            &mut budget,
        )?));
    }
    for (n, i) in result.graph.nodes.iter_mut().zip(nodes) {
        n.derived_from = i.assertions;
        n.derived_nodes = i.nodes;
        n.derived_snapshots = i.snapshots;
        n.derivations = i.derivations;
    }
    for (e, i) in result.graph.edges.iter_mut().zip(edges) {
        e.derived_from = carrier::assertion_index(&i);
        e.derived_nodes = i.nodes;
        e.derived_snapshots = i.snapshots;
        e.derivations = i.derivations;
        result
            .edge_origins
            .entry(e.id.clone())
            .or_default()
            .extend(e.derived_from.iter().cloned());
    }
    for (a, i) in result.graph.attachments.iter_mut().zip(attachments) {
        let index = carrier::assertion_index(&i);
        a.derived_from = i.assertions;
        a.derived_nodes = i.nodes;
        a.derived_snapshots = i.snapshots;
        a.derivations = i.derivations;
        result
            .attachment_origins
            .entry(a.id.clone())
            .or_default()
            .extend(index);
    }
    for refs in result
        .edge_origins
        .values_mut()
        .chain(result.attachment_origins.values_mut())
    {
        refs.sort_by(|a, b| assertion_key(a).cmp(&assertion_key(b)));
        refs.dedup();
    }
    result.provenance.extend(
        result
            .edge_origins
            .values()
            .chain(result.attachment_origins.values())
            .flatten()
            .cloned(),
    );
    result
        .provenance
        .sort_by(|a, b| assertion_key(a).cmp(&assertion_key(b)));
    result.provenance.dedup();
    result.input_snapshots.extend(snapshots(&result.graph));
    canonicalize_snapshots(&mut result.input_snapshots);
    validate_graph(&result.graph)?;
    charge(&mut Bytes(limit.min(32 * 1024 * 1024)), result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(id: usize) -> NodeRef {
        NodeRef {
            graph_id: "source".into(),
            revision: "r".into(),
            node_id: id.to_string(),
        }
    }
    #[test]
    fn omitted_defaults_preserve_legacy_bytes_and_new_records_roundtrip() {
        assert_eq!(
            serde_json::to_string(&GraphData::default()).unwrap(),
            r#"{"nodes":[],"edges":[]}"#
        );
        let value = serde_json::json!({"nodes":[],"edges":[{"id":"e","predicate":"p","from":"a","to":"b","valid_time":{"start":0,"end":null}}]});
        let data: GraphData = serde_json::from_value(value).unwrap();
        assert_eq!(
            serde_json::to_string(&data).unwrap(),
            r#"{"nodes":[],"edges":[{"id":"e","predicate":"p","from":"a","to":"b","valid_time":{"start":0,"end":null},"polarity":"positive","properties":{},"metadata":[],"readers":[],"derived_from":[]}]}"#
        );
        let typed:GraphData=serde_json::from_value(serde_json::json!({"influence":{"nodes":[node(1)]},"edges":[{"id":"e","predicate":"p","from":"a","to":"b","valid_time":{"start":0,"end":null},"derived_nodes":[node(2)],"derivations":[{"operator":"test","premises":[],"node_premises":[node(3)]}]}]})).unwrap();
        validate_graph(&typed).unwrap();
        assert_eq!(
            serde_json::from_str::<GraphData>(&serde_json::to_string(&typed).unwrap()).unwrap(),
            typed
        );
    }
    #[test]
    fn canonical_merge_retains_empty_value_gates_without_duplicate_amplification() {
        let full = GraphInfluence {
            derivations: vec![],
            snapshots: vec![],
            assertions: vec![],
            nodes: (0..MAX_REFERENCES).map(node).collect(),
        };
        let merged = merge(Some(&full), Some(&full)).unwrap().unwrap();
        assert_eq!(merged.nodes.len(), MAX_REFERENCES);
        let mut fresh = full.clone();
        fresh.nodes[0] = node(MAX_REFERENCES);
        assert_eq!(
            merge(Some(&full), Some(&fresh)).unwrap_err().code,
            "E_BUDGET"
        );
        assert_eq!(merge(Some(&full), None).unwrap().unwrap(), merged);
    }
    #[test]
    fn cardinality_and_identifier_bounds_fail_before_use() {
        let value = serde_json::json!({"nodes":(0..=MAX_REFERENCES).map(node).collect::<Vec<_>>()});
        assert!(serde_json::from_value::<GraphInfluence>(value).is_err());
        let mut invalid = node(1);
        invalid.revision.clear();
        assert_eq!(
            validate(&GraphInfluence {
                derivations: vec![],
                snapshots: vec![],
                assertions: vec![],
                nodes: vec![invalid]
            })
            .unwrap_err()
            .code,
            "E_INFLUENCE"
        );
        assert!(
            serde_json::from_value::<GraphInfluence>(serde_json::json!({"allow":true})).is_err()
        );
    }
}
