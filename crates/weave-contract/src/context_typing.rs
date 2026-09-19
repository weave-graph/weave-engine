//! Persistable typed-context influence. These records are data, not authority:
//! the runtime resolves and authorizes every exact descriptor before consumption.
use crate::context_axes::{ContextAxisType, ContextSchema};
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TypedContextWitness {
    pub context: GraphRef,
    pub schema: ContextSchema,
    pub definition: AssertionRef,
    #[serde(deserialize_with = "anchors")]
    pub anchor_nodes: Vec<NodeRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextTyping {
    pub selected: Option<GraphRef>,
    #[serde(deserialize_with = "witnesses")]
    pub witnesses: Vec<TypedContextWitness>,
}
fn anchors<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<NodeRef>, D::Error> {
    bounded::<D, NodeRef, 16>(d)
}
fn witnesses<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<TypedContextWitness>, D::Error> {
    bounded::<D, TypedContextWitness, 32>(d)
}
fn bounded<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>, const N: usize>(
    d: D,
) -> Result<Vec<T>, D::Error> {
    struct List<T, const N: usize>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const N: usize> serde::de::Visitor<'de> for List<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "at most {N} elements")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut access: A,
        ) -> Result<Vec<T>, A::Error> {
            struct Element<T> {
                allow: bool,
                marker: std::marker::PhantomData<T>,
            }
            impl<'de, T: Deserialize<'de>> serde::de::DeserializeSeed<'de> for Element<T> {
                type Value = T;
                fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<T, D::Error> {
                    if !self.allow {
                        return Err(serde::de::Error::custom("context witness budget"));
                    }
                    T::deserialize(d)
                }
            }
            let mut out = Vec::new();
            while let Some(value) = access.next_element_seed(Element {
                allow: out.len() < N,
                marker: std::marker::PhantomData,
            })? {
                out.push(value);
            }
            Ok(out)
        }
    }
    d.deserialize_seq(List::<T, N>(std::marker::PhantomData))
}
fn error(code: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: "Invalid or incompatible typed context influence".into(),
    }
}
fn id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
fn pin(reference: &GraphRef) -> bool {
    id(&reference.graph_id) && id(&reference.revision)
}
fn label(reference: &GraphRef) -> (&str, &str) {
    (&reference.graph_id, &reference.revision)
}
fn node_label(reference: &NodeRef) -> (&str, &str, &str) {
    (&reference.graph_id, &reference.revision, &reference.node_id)
}
fn assertion_label(reference: &AssertionRef) -> (&str, &str, &str) {
    (
        &reference.graph_id,
        &reference.revision,
        &reference.assertion_id,
    )
}
struct Bytes(usize);
impl Write for Bytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| std::io::Error::other("typed context budget"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn charge(remaining: &mut Bytes, value: &impl Serialize) -> Result<(), Diagnostic> {
    serde_json::to_writer(remaining, value).map_err(|_| error("E_CONTEXT_BUDGET"))
}

pub fn validate(typing: &ContextTyping) -> Result<(), Diagnostic> {
    if typing.witnesses.is_empty() || typing.witnesses.len() > 32 {
        return Err(error("E_CONTEXT_BUDGET"));
    }
    let mut schemas = context_axes::ContextSchemas::default();
    let mut contexts = BTreeSet::new();
    for witness in &typing.witnesses {
        if !pin(&witness.context)
            || !contexts.insert(label(&witness.context))
            || witness.definition.graph_id != witness.context.graph_id
            || witness.definition.revision != witness.context.revision
            || witness.definition.assertion_id != "definition"
        {
            return Err(error("E_CONTEXT_TYPING"));
        }
        if witness.anchor_nodes.is_empty() || witness.anchor_nodes.len() > 16 {
            return Err(error("E_CONTEXT_BUDGET"));
        }
        let mut nodes = BTreeSet::new();
        for node in &witness.anchor_nodes {
            if node.graph_id != witness.context.graph_id
                || node.revision != witness.context.revision
                || !id(&node.node_id)
                || !nodes.insert(node_label(node))
            {
                return Err(error("E_CONTEXT_TYPING"));
            }
        }
        schemas
            .register(&witness.schema)
            .map_err(|_| error("E_CONTEXT_SCHEMA"))?;
    }
    if let Some(selected) = &typing.selected {
        if !pin(selected) || !contexts.contains(&label(selected)) {
            return Err(error("E_CONTEXT_TYPING"));
        }
    }
    charge(&mut Bytes(3 * 1024 * 1024), typing)
}
pub fn validate_graph(graph: &GraphData) -> Result<(), Diagnostic> {
    if let Some(typing) = &graph.context_typing {
        validate(typing)?;
        if let Some(selected) = &typing.selected {
            let edge_bad = graph
                .edges
                .iter()
                .any(|e| e.assertion_context.as_ref() != Some(selected));
            let claim_bad = graph
                .assertions
                .iter()
                .any(|a| a.context.as_ref() != Some(selected));
            let attachment_bad = graph
                .attachments
                .iter()
                .any(|a| a.context.as_ref() != Some(selected));
            let node_bad = graph.nodes.iter().any(|n| {
                n.context_scope
                    .as_ref()
                    .is_some_and(|s| s.reference() != Some(selected))
            });
            if edge_bad || claim_bad || attachment_bad || node_bad {
                return Err(error("E_CONTEXT_SCOPE"));
            }
        }
    }
    Ok(())
}
pub fn validate_result(result: &QueryResult) -> Result<(), Diagnostic> {
    validate_graph(&result.graph)?;
    if result
        .graph
        .context_typing
        .as_ref()
        .and_then(|t| t.selected.as_ref())
        .is_some_and(|s| {
            result
                .selected_context
                .as_ref()
                .and_then(ContextSelection::reference)
                != Some(s)
        })
    {
        return Err(error("E_CONTEXT_SCOPE"));
    }
    Ok(())
}
/// Same label conflicts are local to this composition; reading cannot install global schemas.
pub fn merge(
    left: Option<&ContextTyping>,
    right: Option<&ContextTyping>,
) -> Result<Option<ContextTyping>, Diagnostic> {
    if left.is_none() && right.is_none() {
        return Ok(None);
    }
    let mut records = BTreeMap::new();
    let mut schemas = context_axes::ContextSchemas::default();
    for typing in [left, right].into_iter().flatten() {
        validate(typing)?;
        for witness in &typing.witnesses {
            schemas
                .register(&witness.schema)
                .map_err(|_| error("E_CONTEXT_SCHEMA"))?;
            if let Some(old) = records.insert(label(&witness.context), witness) {
                let same_schema = old
                    .schema
                    .canonical_bytes()
                    .map_err(|_| error("E_CONTEXT_SCHEMA"))?
                    == witness
                        .schema
                        .canonical_bytes()
                        .map_err(|_| error("E_CONTEXT_SCHEMA"))?;
                let a: BTreeSet<_> = old.anchor_nodes.iter().map(node_label).collect();
                let b: BTreeSet<_> = witness.anchor_nodes.iter().map(node_label).collect();
                if !same_schema || old.definition != witness.definition || a != b {
                    return Err(error("E_CONTEXT_TYPING"));
                }
            }
            if records.len() > 32 {
                return Err(error("E_CONTEXT_BUDGET"));
            }
        }
    }
    let selected = match (left, right) {
        (Some(l), Some(r)) if l.selected == r.selected => l.selected.clone(),
        _ => None,
    };
    let mut witnesses = Vec::new();
    for witness in records.into_values() {
        let mut canonical = witness.clone();
        for kind in canonical.schema.axes.values_mut() {
            if let ContextAxisType::Enum { members } = kind {
                members.sort();
            }
        }
        canonical
            .anchor_nodes
            .sort_by(|a, b| node_label(a).cmp(&node_label(b)));
        witnesses.push(canonical);
    }
    let result = ContextTyping {
        selected,
        witnesses,
    };
    validate(&result)?;
    Ok(Some(result))
}
pub fn gates(typing: &ContextTyping) -> Result<(Vec<AssertionRef>, Vec<NodeRef>), Diagnostic> {
    validate(typing)?;
    let assertions: BTreeMap<_, _> = typing
        .witnesses
        .iter()
        .map(|w| (assertion_label(&w.definition), &w.definition))
        .collect();
    let nodes: BTreeMap<_, _> = typing
        .witnesses
        .iter()
        .flat_map(|w| w.anchor_nodes.iter())
        .map(|n| (node_label(n), n))
        .collect();
    Ok((
        assertions.into_values().cloned().collect(),
        nodes.into_values().cloned().collect(),
    ))
}
/// Call only for newly generated records. Original immutable origin records are not rewritten.
pub fn protect_generated(graph: &mut GraphData) -> Result<(), Diagnostic> {
    protect_generated_bounded(graph, 32 * 1024 * 1024)
}
pub fn protect_generated_bounded(graph: &mut GraphData, limit: usize) -> Result<(), Diagnostic> {
    let Some(typing) = &graph.context_typing else {
        return Ok(());
    };
    let (assertions, nodes) = gates(typing)?;
    // Precharge all additions before any record clone/append. The existing graph must fit too.
    let mut bytes = Bytes(limit.min(32 * 1024 * 1024));
    charge(&mut bytes, graph)?;
    for node in &graph.nodes {
        if node
            .derived_from
            .len()
            .saturating_add(node.derived_nodes.len())
            .saturating_add(assertions.len())
            .saturating_add(nodes.len())
            > 1000
        {
            return Err(error("E_CONTEXT_BUDGET"));
        }
        charge(&mut bytes, &assertions)?;
        charge(&mut bytes, &nodes)?;
    }
    for edge in &graph.edges {
        if edge.derivations.len() > 128
            || edge.derived_from.len().saturating_add(assertions.len()) > 1000
        {
            return Err(error("E_CONTEXT_BUDGET"));
        }
        charge(&mut bytes, &assertions)?;
        for group in &edge.derivations {
            if group.premises.len().saturating_add(assertions.len()) > 1000 {
                return Err(error("E_CONTEXT_BUDGET"));
            }
            charge(&mut bytes, &assertions)?;
            for assertion in &assertions {
                charge(
                    &mut bytes,
                    &GraphRef {
                        graph_id: assertion.graph_id.clone(),
                        revision: assertion.revision.clone(),
                    },
                )?;
            }
        }
    }
    let snapshots: Vec<_> = assertions
        .iter()
        .map(|a| GraphRef {
            graph_id: a.graph_id.clone(),
            revision: a.revision.clone(),
        })
        .collect();
    for node in &mut graph.nodes {
        node.derived_from.extend(assertions.iter().cloned());
        node.derived_from
            .sort_by(|a, b| assertion_label(a).cmp(&assertion_label(b)));
        node.derived_from.dedup();
        node.derived_nodes.extend(nodes.iter().cloned());
        node.derived_nodes
            .sort_by(|a, b| node_label(a).cmp(&node_label(b)));
        node.derived_nodes.dedup();
    }
    for edge in &mut graph.edges {
        edge.derived_from.extend(assertions.iter().cloned());
        edge.derived_from
            .sort_by(|a, b| assertion_label(a).cmp(&assertion_label(b)));
        edge.derived_from.dedup();
        for group in &mut edge.derivations {
            group.premises.extend(assertions.iter().cloned());
            group
                .premises
                .sort_by(|a, b| assertion_label(a).cmp(&assertion_label(b)));
            group.premises.dedup();
            group.input_snapshots.extend(snapshots.iter().cloned());
            group
                .input_snapshots
                .sort_by(|a, b| label(a).cmp(&label(b)));
            group.input_snapshots.dedup();
        }
    }
    Ok(())
}

/// Generated-result convenience: retain descriptor gates in every descriptive origin index.
pub fn protect_result_generated(result: &mut QueryResult) -> Result<(), Diagnostic> {
    protect_result_generated_bounded(result, 32 * 1024 * 1024)
}
pub fn protect_result_generated_bounded(
    result: &mut QueryResult,
    limit: usize,
) -> Result<(), Diagnostic> {
    protect_generated_bounded(&mut result.graph, limit)?;
    let Some(typing) = &result.graph.context_typing else {
        return Ok(());
    };
    let (assertions, _) = gates(typing)?;
    let mut bytes = Bytes(limit.min(32 * 1024 * 1024));
    charge(&mut bytes, result)?;
    for edge in &result.graph.edges {
        charge(&mut bytes, &edge.derived_from)?;
    }
    charge(&mut bytes, &assertions)?;
    for assertion in &assertions {
        charge(
            &mut bytes,
            &GraphRef {
                graph_id: assertion.graph_id.clone(),
                revision: assertion.revision.clone(),
            },
        )?;
    }
    for edge in &result.graph.edges {
        let origins = result.edge_origins.entry(edge.id.clone()).or_default();
        origins.extend(edge.derived_from.iter().cloned());
        origins.sort_by(|a, b| assertion_label(a).cmp(&assertion_label(b)));
        origins.dedup();
    }
    result.provenance.extend(assertions.iter().cloned());
    result
        .provenance
        .sort_by(|a, b| assertion_label(a).cmp(&assertion_label(b)));
    result.provenance.dedup();
    result
        .input_snapshots
        .extend(assertions.into_iter().map(|a| GraphRef {
            graph_id: a.graph_id,
            revision: a.revision,
        }));
    result
        .input_snapshots
        .sort_by(|a, b| label(a).cmp(&label(b)));
    result.input_snapshots.dedup();
    validate_result(result)
}
