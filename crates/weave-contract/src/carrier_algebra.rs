//! Bounded restriction algebra. Composition never authorizes references.
//! Existing stored Edge/Assertion derivations retain their historical validators.
use crate::{AssertionRef, Derivation, Diagnostic, GraphRef, NodeRef};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{self, Write};

type Result<T> = std::result::Result<T, Diagnostic>;
const MAX_GROUPS: usize = 128;
const MAX_REFS: usize = 1000;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_DEPTH: usize = 32;

/// Flat global AND gates; deliberately nonrecursive.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RefSet {
    pub assertions: Vec<AssertionRef>,
    pub nodes: Vec<NodeRef>,
    pub snapshots: Vec<GraphRef>,
}
pub type Alternative = Derivation;
/// all(flat) AND any(alternatives); empty alternatives mean no extra condition.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Carrier {
    pub flat: RefSet,
    pub alternatives: Vec<Alternative>,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub groups: usize,
    pub references: usize,
    pub bytes: usize,
    pub work: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            groups: MAX_GROUPS,
            references: MAX_REFS,
            bytes: MAX_BYTES,
            work: 100_000,
        }
    }
}
/// Share one budget across a caller's composition. Charges input scans and
/// prospective copies before allocation; conservative exhaustion is explicit.
pub struct Budget {
    limits: Limits,
    bytes: usize,
    work: usize,
}
impl Budget {
    pub fn new(limits: Limits) -> Self {
        let limits = Limits {
            groups: limits.groups.min(MAX_GROUPS),
            references: limits.references.min(MAX_REFS),
            bytes: limits.bytes.min(MAX_BYTES),
            work: limits.work.min(1_000_000),
        };
        Self {
            bytes: limits.bytes,
            work: limits.work,
            limits,
        }
    }
    fn step(&mut self) -> Result<()> {
        self.work = self.work.checked_sub(1).ok_or_else(limit)?;
        Ok(())
    }
    fn charge(&mut self, value: &impl Serialize) -> Result<()> {
        self.step()?;
        serde_json::to_writer(self, value).map_err(|_| limit())
    }
}
impl Write for Budget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_sub(bytes.len())
            .ok_or_else(|| io::Error::other("carrier budget"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn error(code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: message.into(),
    }
}
fn limit() -> Diagnostic {
    error(
        "E_INFLUENCE_BUDGET",
        "Alternative influence budget exceeded",
    )
}
fn id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512
}
fn graph_key(r: &GraphRef) -> (&str, &str) {
    (&r.graph_id, &r.revision)
}
fn refs_count(flat: &RefSet) -> usize {
    flat.assertions
        .len()
        .saturating_add(flat.nodes.len())
        .saturating_add(flat.snapshots.len())
}
fn group_count(group: &Alternative) -> usize {
    group
        .premises
        .len()
        .saturating_add(group.node_premises.len())
        .saturating_add(group.snapshot_premises.len())
}
fn flat_empty(flat: &RefSet) -> bool {
    refs_count(flat) == 0
}
fn unconditional(value: &Carrier) -> bool {
    flat_empty(&value.flat) && value.alternatives.is_empty()
}
fn value_depth(value: &serde_json::Value, depth: usize, budget: &mut Budget) -> Result<()> {
    budget.step()?;
    if depth > MAX_DEPTH {
        return Err(limit());
    }
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                value_depth(value, depth + 1, budget)?;
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                value_depth(value, depth + 1, budget)?;
            }
        }
        _ => {}
    }
    Ok(())
}
/// New carrier profile only. Never use this to tighten legacy graph validators.
pub fn validate(value: &Carrier, budget: &mut Budget) -> Result<()> {
    if value.alternatives.len() > budget.limits.groups {
        return Err(limit());
    }
    let mut count = refs_count(&value.flat);
    crate::influence::validate_record_refs(
        &value.flat.assertions,
        &value.flat.nodes,
        &value.flat.snapshots,
    )?;
    for group in &value.alternatives {
        budget.step()?;
        let n = group_count(group);
        if n == 0 {
            return Err(error(
                "E_INFLUENCE_EMPTY_GROUP",
                "New alternative groups need explicit restriction premises",
            ));
        }
        count = count.checked_add(n).ok_or_else(limit)?;
        if count > budget.limits.references {
            return Err(limit());
        }
        let d = &group;
        if !id(&d.operator) {
            return Err(error("E_INFLUENCE", "A bounded operator label is required"));
        }
        crate::influence::validate_record_refs(
            &d.premises,
            &d.node_premises,
            &group.snapshot_premises,
        )?;
        if d.input_snapshots.len() > MAX_REFS
            || d.input_snapshots
                .iter()
                .any(|r| !id(&r.graph_id) || !id(&r.revision))
        {
            return Err(error("E_INFLUENCE", "Invalid descriptive snapshot index"));
        }
        for value in d.parameters.values() {
            value_depth(value, 0, budget)?;
        }
    }
    if count > budget.limits.references {
        return Err(limit());
    }
    budget.charge(value)
}
fn normalize_group(group: &mut Alternative) {
    group.premises.sort_by(|a, b| {
        crate::influence::assertion_key(a).cmp(&crate::influence::assertion_key(b))
    });
    group.premises.dedup();
    group
        .node_premises
        .sort_by(|a, b| crate::influence::node_key(a).cmp(&crate::influence::node_key(b)));
    group.node_premises.dedup();
    group
        .snapshot_premises
        .sort_by(|a, b| graph_key(a).cmp(&graph_key(b)));
    group.snapshot_premises.dedup();
    group
        .input_snapshots
        .sort_by(|a, b| graph_key(a).cmp(&graph_key(b)));
    group.input_snapshots.dedup();
}
/// Canonicalize a new result; callers must not rewrite immutable stored records.
pub fn canonicalize(value: &Carrier, budget: &mut Budget) -> Result<Carrier> {
    validate(value, budget)?;
    budget.charge(value)?;
    let mut out = value.clone();
    normalize_refs(&mut out.flat);
    let mut groups = BTreeMap::new();
    for mut group in out.alternatives {
        budget.step()?;
        normalize_group(&mut group);
        budget.charge(&group)?; // prospective canonical key allocation
        let key = serde_json::to_vec(&group).expect("carrier serialization");
        groups.entry(key).or_insert(group);
    }
    out.alternatives = groups.into_values().collect();
    Ok(out)
}
fn normalize_refs(value: &mut RefSet) {
    value.assertions.sort_by(|a, b| {
        crate::influence::assertion_key(a).cmp(&crate::influence::assertion_key(b))
    });
    value.assertions.dedup();
    value
        .nodes
        .sort_by(|a, b| crate::influence::node_key(a).cmp(&crate::influence::node_key(b)));
    value.nodes.dedup();
    crate::influence::canonicalize_snapshots(&mut value.snapshots);
}
fn combine_refs(a: &RefSet, b: &RefSet, budget: &mut Budget) -> Result<RefSet> {
    if refs_count(a).saturating_add(refs_count(b)) > budget.limits.references {
        return Err(limit());
    }
    budget.charge(&(a, b))?;
    let mut out = a.clone();
    out.assertions.extend(b.assertions.iter().cloned());
    out.nodes.extend(b.nodes.iter().cloned());
    out.snapshots.extend(b.snapshots.iter().cloned());
    normalize_refs(&mut out);
    Ok(out)
}
fn group_refs(group: &Alternative, budget: &mut Budget) -> Result<RefSet> {
    budget.charge(group)?;
    Ok(RefSet {
        assertions: group.premises.clone(),
        nodes: group.node_premises.clone(),
        snapshots: group.snapshot_premises.clone(),
    })
}
fn make_group(
    operator: &str,
    gates: RefSet,
    parents: &impl Serialize,
    budget: &mut Budget,
) -> Result<Alternative> {
    if flat_empty(&gates) {
        return Err(error(
            "E_INFLUENCE_EMPTY_GROUP",
            "No unconditional group encoding is defined",
        ));
    }
    // Authoritative premises, descriptive indexes and explanation parameters each
    // retain data. Charge their copies independently before constructing them.
    budget.charge(&(&gates, &gates, &gates, parents))?;
    let mut snapshots: Vec<_> = gates
        .assertions
        .iter()
        .map(|r| GraphRef {
            graph_id: r.graph_id.clone(),
            revision: r.revision.clone(),
        })
        .chain(gates.nodes.iter().map(|r| GraphRef {
            graph_id: r.graph_id.clone(),
            revision: r.revision.clone(),
        }))
        .chain(gates.snapshots.iter().cloned())
        .collect();
    snapshots.sort_by(|a, b| graph_key(a).cmp(&graph_key(b)));
    snapshots.dedup();
    Ok(Derivation {
        operator: operator.into(),
        premises: gates.assertions,
        node_premises: gates.nodes,
        parameters: BTreeMap::from([(
            "inputs".into(),
            serde_json::to_value(parents).expect("carrier serialization"),
        )]),
        input_snapshots: snapshots,
        snapshot_premises: gates.snapshots,
    })
}
/// Whole graph Union uses this conjunction, never proof disjunction.
pub fn conjunction(a: &Carrier, b: &Carrier, budget: &mut Budget) -> Result<Carrier> {
    let a = canonicalize(a, budget)?;
    let b = canonicalize(b, budget)?;
    if a == b {
        return Ok(a);
    }
    let flat = combine_refs(&a.flat, &b.flat, budget)?;
    let alternatives = if a.alternatives.is_empty() {
        budget.charge(&b.alternatives)?;
        b.alternatives
    } else if b.alternatives.is_empty() {
        budget.charge(&a.alternatives)?;
        a.alternatives
    } else {
        if a.alternatives
            .len()
            .checked_mul(b.alternatives.len())
            .is_none_or(|n| n > budget.limits.groups)
        {
            return Err(limit());
        }
        let left_refs: usize = a.alternatives.iter().map(group_count).sum();
        let right_refs: usize = b.alternatives.iter().map(group_count).sum();
        let total = left_refs
            .checked_mul(b.alternatives.len())
            .and_then(|n| {
                right_refs
                    .checked_mul(a.alternatives.len())
                    .and_then(|m| n.checked_add(m))
            })
            .and_then(|n| n.checked_add(refs_count(&flat)))
            .ok_or_else(limit)?;
        if total > budget.limits.references {
            return Err(limit());
        }
        let mut groups = Vec::new();
        let mut raw = refs_count(&flat);
        for x in &a.alternatives {
            for y in &b.alternatives {
                budget.step()?;
                raw = raw
                    .checked_add(group_count(x))
                    .and_then(|n| n.checked_add(group_count(y)))
                    .ok_or_else(limit)?;
                if raw > budget.limits.references {
                    return Err(limit());
                }
                let xr = group_refs(x, budget)?;
                let yr = group_refs(y, budget)?;
                let gates = combine_refs(&xr, &yr, budget)?;
                groups.push(make_group(
                    "weave:influence:and/v1",
                    gates,
                    &(x, y),
                    budget,
                )?);
            }
        }
        groups
    };
    canonicalize(&Carrier { flat, alternatives }, budget)
}
fn branches(value: &Carrier, budget: &mut Budget) -> Result<Vec<Alternative>> {
    let raw = refs_count(&value.flat)
        .checked_mul(value.alternatives.len().max(1))
        .and_then(|n| n.checked_add(value.alternatives.iter().map(group_count).sum::<usize>()))
        .ok_or_else(limit)?;
    if raw > budget.limits.references {
        return Err(limit());
    }
    if value.alternatives.is_empty() {
        budget.charge(&value.flat)?;
        return Ok(vec![make_group(
            "weave:influence:flat/v1",
            value.flat.clone(),
            &value.flat,
            budget,
        )?]);
    }
    let mut out = Vec::new();
    for group in &value.alternatives {
        let refs = group_refs(group, budget)?;
        let gates = combine_refs(&value.flat, &refs, budget)?;
        out.push(make_group(
            "weave:influence:branch/v1",
            gates,
            &(&value.flat, group),
            budget,
        )?);
    }
    Ok(out)
}
/// Distribute global gates into each existing branch without interpreting descriptive pins.
pub fn distribute(branch: &Carrier, flat: &Carrier, budget: &mut Budget) -> Result<Carrier> {
    let joined = conjunction(branch, flat, budget)?;
    let alternatives = branches(&joined, budget)?;
    canonicalize(
        &Carrier {
            flat: RefSet::default(),
            alternatives,
        },
        budget,
    )
}
/// Restricted alternatives for one identical value only. There is deliberately
/// no true OR restricted simplification: that would discard explanation traces.
pub fn disjunction(a: &Carrier, b: &Carrier, budget: &mut Budget) -> Result<Carrier> {
    let a = canonicalize(a, budget)?;
    let b = canonicalize(b, budget)?;
    if unconditional(&a) || unconditional(&b) {
        if unconditional(&a) && unconditional(&b) {
            return Ok(Carrier::default());
        }
        return Err(error(
            "E_INFLUENCE_DISJUNCTION_PROFILE",
            "Unconditional alternative requires a separately retained explanation profile",
        ));
    }
    let count = a
        .alternatives
        .len()
        .max(1)
        .checked_add(b.alternatives.len().max(1))
        .ok_or_else(limit)?;
    if count > budget.limits.groups {
        return Err(limit());
    }
    let expanded_refs = |value: &Carrier| -> Option<usize> {
        refs_count(&value.flat)
            .checked_mul(value.alternatives.len().max(1))
            .and_then(|n| n.checked_add(value.alternatives.iter().map(group_count).sum::<usize>()))
    };
    let total = expanded_refs(&a)
        .and_then(|n| expanded_refs(&b).and_then(|m| n.checked_add(m)))
        .ok_or_else(limit)?;
    if total > budget.limits.references {
        return Err(limit());
    }
    let mut alternatives = branches(&a, budget)?;
    alternatives.extend(branches(&b, budget)?);
    canonicalize(
        &Carrier {
            flat: RefSet::default(),
            alternatives,
        },
        budget,
    )
}

/// Copy a borrowed record carrier only after its counts and serialized size are charged.
pub fn from_parts(
    assertions: &[AssertionRef],
    nodes: &[NodeRef],
    snapshots: &[GraphRef],
    groups: &[Derivation],
    budget: &mut Budget,
) -> Result<Carrier> {
    if groups.len() > budget.limits.groups
        || assertions
            .len()
            .saturating_add(nodes.len())
            .saturating_add(snapshots.len())
            .saturating_add(
                groups
                    .iter()
                    .map(|g| {
                        g.premises
                            .len()
                            .saturating_add(g.node_premises.len())
                            .saturating_add(g.snapshot_premises.len())
                    })
                    .sum::<usize>(),
            )
            > budget.limits.references
    {
        return Err(limit());
    }
    budget.charge(&(assertions, nodes, snapshots, groups))?;
    let out = Carrier {
        flat: RefSet {
            assertions: assertions.to_vec(),
            nodes: nodes.to_vec(),
            snapshots: snapshots.to_vec(),
        },
        alternatives: groups.to_vec(),
    };
    validate(&out, budget)?;
    Ok(out)
}
pub fn from_influence(value: &crate::GraphInfluence, budget: &mut Budget) -> Result<Carrier> {
    from_parts(
        &value.assertions,
        &value.nodes,
        &value.snapshots,
        &value.derivations,
        budget,
    )
}
pub fn into_influence(value: Carrier) -> crate::GraphInfluence {
    crate::GraphInfluence {
        assertions: value.flat.assertions,
        nodes: value.flat.nodes,
        snapshots: value.flat.snapshots,
        derivations: value.alternatives,
    }
}
/// All assertion leaves for descriptive indexes, never a conjunctive carrier.
pub fn assertion_index(value: &crate::GraphInfluence) -> Vec<AssertionRef> {
    let mut refs: Vec<_> = value
        .assertions
        .iter()
        .chain(value.derivations.iter().flat_map(|g| &g.premises))
        .cloned()
        .collect();
    refs.sort_by(|a, b| {
        crate::influence::assertion_key(a).cmp(&crate::influence::assertion_key(b))
    });
    refs.dedup();
    refs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssertionRef, NodeRef};
    fn budget() -> Budget {
        Budget::new(Limits::default())
    }
    fn assertion(id: &str) -> AssertionRef {
        AssertionRef {
            graph_id: "G".into(),
            revision: "r".into(),
            assertion_id: id.into(),
        }
    }
    fn refs(bits: u8) -> RefSet {
        let mut out = RefSet::default();
        if bits & 1 != 0 {
            out.assertions.push(assertion("A"));
        }
        if bits & 2 != 0 {
            out.nodes.push(NodeRef {
                graph_id: "G".into(),
                revision: "r".into(),
                node_id: "B".into(),
            });
        }
        if bits & 4 != 0 {
            out.snapshots.push(GraphRef {
                graph_id: "S".into(),
                revision: "r".into(),
            });
        }
        if bits & 8 != 0 {
            out.assertions.push(assertion("D"));
        }
        out
    }
    fn alt(bits: u8, label: &str) -> Alternative {
        let gates = refs(bits);
        Derivation {
            operator: label.into(),
            premises: gates.assertions,
            node_premises: gates.nodes,
            parameters: BTreeMap::new(),
            input_snapshots: vec![GraphRef {
                graph_id: "descriptive-only".into(),
                revision: "not-authority".into(),
            }],
            snapshot_premises: gates.snapshots,
        }
    }
    fn carrier(flat: u8, groups: &[u8]) -> Carrier {
        Carrier {
            flat: refs(flat),
            alternatives: groups
                .iter()
                .enumerate()
                .map(|(i, b)| alt(*b, &format!("proof{i}")))
                .collect(),
        }
    }
    // Independent evaluator: resolves each typed atom from an exhaustive assignment.
    // It does not call the implementation's conjunction/disjunction/ref-union helpers.
    fn assertions_ok(refs: &[AssertionRef], mask: u8) -> bool {
        refs.iter().all(|r| match r.assertion_id.as_str() {
            "A" => mask & 1 != 0,
            "D" => mask & 8 != 0,
            _ => false,
        })
    }
    fn nodes_ok(refs: &[NodeRef], mask: u8) -> bool {
        refs.iter().all(|r| r.node_id == "B" && mask & 2 != 0)
    }
    fn snapshots_ok(refs: &[GraphRef], mask: u8) -> bool {
        refs.iter().all(|r| r.graph_id == "S" && mask & 4 != 0)
    }
    fn evaluate(value: &Carrier, mask: u8) -> bool {
        let flat = &value.flat;
        assertions_ok(&flat.assertions, mask)
            && nodes_ok(&flat.nodes, mask)
            && snapshots_ok(&flat.snapshots, mask)
            && (value.alternatives.is_empty()
                || value.alternatives.iter().any(|g| {
                    assertions_ok(&g.premises, mask)
                        && nodes_ok(&g.node_premises, mask)
                        && snapshots_ok(&g.snapshot_premises, mask)
                }))
    }
    #[test]
    fn exhaustive_four_atom_truth_table_for_and_and_supported_or() {
        let forms = [
            carrier(0, &[]),
            carrier(1, &[]),
            carrier(2, &[]),
            carrier(4, &[]),
            carrier(0, &[1, 2]),
            carrier(4, &[1, 2]),
            carrier(0, &[3, 4]),
            carrier(8, &[5, 2]),
        ];
        let mut checks = 0;
        for (i, a) in forms.iter().enumerate() {
            for (j, b) in forms.iter().enumerate() {
                let and = conjunction(a, b, &mut budget()).unwrap();
                let or = disjunction(a, b, &mut budget());
                for mask in 0..16 {
                    let av = evaluate(a, mask);
                    let bv = evaluate(b, mask);
                    assert_eq!(evaluate(&and, mask), av && bv, "AND {i},{j},{mask}");
                    checks += 1;
                    if i == 0 || j == 0 {
                        if i == j {
                            assert!(evaluate(or.as_ref().unwrap(), mask));
                        } else {
                            assert_eq!(
                                or.as_ref().unwrap_err().code,
                                "E_INFLUENCE_DISJUNCTION_PROFILE"
                            );
                        }
                    } else {
                        assert_eq!(
                            evaluate(or.as_ref().unwrap(), mask),
                            av || bv,
                            "OR {i},{j},{mask}"
                        );
                        checks += 1;
                    }
                }
            }
        }
        assert_eq!(checks, 1808);
    }
    #[test]
    fn descriptive_pins_do_not_authorize_and_snapshot_only_group_is_explicit() {
        let mut value = carrier(0, &[1]);
        assert!(!evaluate(&value, 0));
        assert!(evaluate(&value, 1));
        value.alternatives[0].premises.clear();
        assert_eq!(
            validate(&value, &mut budget()).unwrap_err().code,
            "E_INFLUENCE_EMPTY_GROUP"
        );
        value.alternatives[0].snapshot_premises = refs(4).snapshots;
        validate(&value, &mut budget()).unwrap();
        assert!(evaluate(&value, 4));
        assert!(!evaluate(&value, 1));
    }
    #[test]
    fn duplicate_groups_deduplicate_only_complete_explanations_and_raw_limits_apply() {
        let a = alt(1, "first");
        let b = alt(1, "other-explanation");
        let value = Carrier {
            flat: RefSet::default(),
            alternatives: vec![a.clone(), a.clone(), b],
        };
        let out = canonicalize(&value, &mut budget()).unwrap();
        assert_eq!(out.alternatives.len(), 2);
        assert_eq!(canonicalize(&out, &mut budget()).unwrap(), out);
        assert_eq!(conjunction(&out, &out, &mut budget()).unwrap(), out);
        let excessive = Carrier {
            flat: RefSet::default(),
            alternatives: vec![a; 129],
        };
        assert_eq!(
            canonicalize(&excessive, &mut budget()).unwrap_err().code,
            "E_INFLUENCE_BUDGET"
        );
        let excessive = Carrier {
            flat: RefSet {
                assertions: vec![assertion("A"); 1001],
                ..RefSet::default()
            },
            alternatives: vec![],
        };
        assert!(canonicalize(&excessive, &mut budget()).is_err());
    }
    #[test]
    fn bounded_product_and_repeated_flat_expansion_reject_before_retention() {
        let groups = |count| Carrier {
            flat: RefSet::default(),
            alternatives: (0..count)
                .map(|i| alt(1, &format!("different{i}")))
                .collect(),
        };
        let mut other = groups(12);
        for g in &mut other.alternatives {
            g.operator = format!("other-{}", g.operator);
        }
        assert_eq!(
            conjunction(&groups(12), &other, &mut budget())
                .unwrap_err()
                .code,
            "E_INFLUENCE_BUDGET"
        );
        let a = groups(8);
        let b = groups(16);
        assert_eq!(
            conjunction(&a, &b, &mut budget())
                .unwrap()
                .alternatives
                .len(),
            128
        );
        let mut repeated = groups(64);
        repeated.flat.assertions = vec![assertion("A"); 16];
        let b = carrier(2, &[]);
        // Canonicalization deduplicates the repeated flat atom; use distinct pins
        // to test actual repeated expansion rather than relying on input spelling.
        repeated.flat.assertions = (0..16).map(|i| assertion(&format!("a{i}"))).collect();
        assert_eq!(
            disjunction(&repeated, &b, &mut budget()).unwrap_err().code,
            "E_INFLUENCE_BUDGET"
        );
        let mut shared = Budget::new(Limits {
            work: 4,
            ..Limits::default()
        });
        assert!(conjunction(&carrier(1, &[]), &carrier(2, &[]), &mut shared).is_err());
        let mut bytes = Budget::new(Limits {
            bytes: 10,
            ..Limits::default()
        });
        assert!(canonicalize(&carrier(1, &[]), &mut bytes).is_err());
    }
    #[test]
    fn unconditional_disjunction_never_silently_erases_parent_trace() {
        let mut proof = carrier(0, &[1]);
        proof.alternatives[0]
            .parameters
            .insert("important".into(), serde_json::json!({"source":"original"}));
        let original = proof.clone();
        assert_eq!(
            disjunction(&Carrier::default(), &proof, &mut budget())
                .unwrap_err()
                .code,
            "E_INFLUENCE_DISJUNCTION_PROFILE"
        );
        assert_eq!(proof, original);
        let out = conjunction(&Carrier::default(), &proof, &mut budget()).unwrap();
        assert_eq!(
            out.alternatives[0].parameters,
            proof.alternatives[0].parameters
        );
    }
    #[test]
    fn new_validator_does_not_reinterpret_legacy_empty_derivation_lists_or_groups() {
        let legacy:crate::GraphData=serde_json::from_value(serde_json::json!({"nodes":[],"edges":[{"id":"e","predicate":"p","from":"a","to":"b","valid_time":{"start":0},"derivations":[{"operator":"legacy","premises":[]}]}]})).unwrap();
        // This old portable validator historically permits this payload; native
        // commit/Reason/Explain perform stricter checks. Do not silently tighten it.
        crate::influence::validate_graph(&legacy).unwrap();
        validate(&Carrier::default(), &mut budget()).unwrap();
        let new = Carrier {
            flat: RefSet::default(),
            alternatives: vec![legacy.edges[0].derivations[0].clone()],
        };
        assert_eq!(
            validate(&new, &mut budget()).unwrap_err().code,
            "E_INFLUENCE_EMPTY_GROUP"
        );
    }
    #[test]
    fn empty_value_to_scalar_path_keeps_or_guard_under_other_flat_restrictions() {
        // Model an empty metadata target carrying A|B and a later scalar consuming
        // it under D. The same result-local guard must be copied onto that scalar.
        let path = carrier(0, &[1, 2]);
        let parent = carrier(8, &[]);
        let result = conjunction(&path, &parent, &mut budget()).unwrap();
        assert!(evaluate(&result, 1 | 8));
        assert!(evaluate(&result, 2 | 8));
        assert!(!evaluate(&result, 8));
        assert!(!evaluate(&result, 1 | 2));
        let record_guard = result.clone(); // envelope removed, local guard survives
        for mask in 0..16 {
            assert_eq!(
                evaluate(&record_guard, mask),
                mask & 8 != 0 && (mask & 1 != 0 || mask & 2 != 0)
            );
        }
    }
}
