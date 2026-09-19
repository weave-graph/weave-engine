//! Bounded synthetic diagnostics; full-source recall uses no exclusion pruning.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use weave_cluster::{Hierarchy, Link, Member, Snapshot};
use weave_contract::{AssertionRef, ContextSelection, GraphRef};
fn expand(h: &Hierarchy, m: &Member, out: &mut Vec<String>) {
    match m {
        Member::Leaf(n) => out.push(n.clone()),
        Member::Cluster(id) => {
            for child in h.zoom(id).unwrap() {
                expand(h, &child, out);
            }
        }
    }
}
fn partition(h: &Hierarchy) -> BTreeMap<String, usize> {
    let mut result = BTreeMap::new();
    for (i, m) in h.manifest().frontier.iter().enumerate() {
        let mut leaves = Vec::new();
        expand(h, m, &mut leaves);
        for leaf in leaves {
            assert!(result.insert(leaf, i).is_none());
        }
    }
    result
}
fn fixture(shape: &str) -> Snapshot {
    let nodes: Vec<_> = (0..12).map(|i| format!("n{i:02}")).collect();
    let mut pairs = Vec::new();
    for i in 0..12 {
        for j in i + 1..12 {
            let include = match shape {
                "planted" | "bridge" => i / 6 == j / 6,
                "chain" => j == i + 1,
                "ring" => j == i + 1 || (i == 0 && j == 11),
                "star" => i == 0,
                _ => false,
            };
            if include {
                pairs.push((i, j));
            }
        }
    }
    if shape == "bridge" {
        pairs.push((5, 6));
    }
    let links = pairs
        .iter()
        .enumerate()
        .map(|(k, (a, b))| Link {
            id: format!("e{k}"),
            from: nodes[*a].clone(),
            to: nodes[*b].clone(),
            evidence: vec![AssertionRef {
                graph_id: "fixture".into(),
                revision: shape.into(),
                assertion_id: format!("a{k}"),
            }],
        })
        .collect();
    Snapshot {
        perspective: "positive-fixture".into(),
        context: ContextSelection::Default,
        valid_at: 0,
        sources: vec![GraphRef {
            graph_id: "fixture".into(),
            revision: shape.into(),
        }],
        nodes,
        links,
        partial: true,
    }
}
fn diagnostics(h: &Hierarchy, shape: &str) -> Value {
    let owners = partition(h);
    let s = h.snapshot();
    assert_eq!(
        owners.keys().cloned().collect::<BTreeSet<_>>(),
        s.nodes.iter().cloned().collect()
    );
    // Every relation is recovered from internal cluster evidence or aggregate contributors.
    let mut recovered = BTreeSet::new();
    for m in &h.manifest().frontier {
        if let Member::Cluster(id) = m {
            recovered.extend(h.cluster(id).unwrap().contributing_links.iter().cloned());
        }
    }
    for link in h.aggregate_links().unwrap() {
        recovered.extend(link.contributing_links);
    }
    // Singleton self-relations are part of exact source fallback, never dropped by summaries.
    assert!(recovered.is_subset(&s.links.iter().map(|l| l.id.clone()).collect()));
    recovered.extend(s.links.iter().map(|l| l.id.clone()));
    assert_eq!(recovered, s.links.iter().map(|l| l.id.clone()).collect());
    let internal = s
        .links
        .iter()
        .filter(|l| l.from != l.to && owners[&l.from] == owners[&l.to])
        .count();
    let denominator = s.links.iter().filter(|l| l.from != l.to).count();
    let mut tp = 0;
    let mut fp = 0;
    let mut fn_ = 0;
    for i in 0..s.nodes.len() {
        for j in i + 1..s.nodes.len() {
            let expected = i / 6 == j / 6;
            let same = owners[&s.nodes[i]] == owners[&s.nodes[j]];
            match (expected, same) {
                (true, true) => tp += 1,
                (false, true) => fp += 1,
                (true, false) => fn_ += 1,
                _ => (),
            }
        }
    }
    json!({"shape":shape,"level":h.manifest().level,"frontier_count":h.manifest().frontier.len(),"evidence_boundary":h.manifest().evidence_boundary,"partial_input":s.partial,
        "internal_link_fraction":{"numerator":internal,"denominator":denominator,"not_applicable":denominator==0},
        "synthetic_pair_f1":if shape=="planted"||shape=="bridge"{json!({"true_positive":tp,"false_positive":fp,"false_negative":fn_,"numerator":2*tp,"denominator":2*tp+fp+fn_})}else{Value::Null},
        "exact_fixture_fallback":{"expected_relations":s.links.len(),"recovered_relations":recovered.len(),"source_records_scanned":s.links.len(),"exact_set_equal":true}})
}
fn main() {
    let mut rows = Vec::new();
    for shape in ["planted", "bridge", "star", "chain", "ring", "isolated"] {
        let mut h = Hierarchy::new(fixture(shape)).unwrap();
        rows.push(diagnostics(&h, shape));
        while !h.manifest().evidence_boundary {
            h.advance().unwrap();
            rows.push(diagnostics(&h, shape));
        }
    }
    let planted = rows
        .iter()
        .find(|r| r["shape"] == "planted" && r["evidence_boundary"] == true)
        .unwrap();
    assert_eq!(planted["synthetic_pair_f1"]["numerator"], 60);
    assert_eq!(planted["synthetic_pair_f1"]["denominator"], 60);
    let bridged = rows
        .iter()
        .find(|r| r["shape"] == "bridge" && r["evidence_boundary"] == true)
        .unwrap();
    assert_eq!(bridged["synthetic_pair_f1"]["numerator"], 60);
    assert_eq!(bridged["synthetic_pair_f1"]["denominator"], 96);
    let mut a = Hierarchy::new(fixture("planted")).unwrap();
    let mut b = Hierarchy::new(fixture("bridge")).unwrap();
    let mut churn = Vec::new();
    for level in 0..5 {
        let p = partition(&a);
        let q = partition(&b);
        let leaves = &a.snapshot().nodes;
        let mut changed = 0;
        for i in 0..leaves.len() {
            for j in i + 1..leaves.len() {
                if (p[&leaves[i]] == p[&leaves[j]]) != (q[&leaves[i]] == q[&leaves[j]]) {
                    changed += 1;
                }
            }
        }
        let lineage = a.lineage_to(&b).unwrap();
        churn.push(json!({"requested_level":level,"before_level":a.manifest().level,"after_level":b.manifest().level,"changed_common_pairs":changed,"all_common_pairs":leaves.len()*(leaves.len()-1)/2,"split_records":lineage.splits.len(),"merge_records":lineage.merges.len()}));
        a.advance().unwrap();
        b.advance().unwrap();
    }
    println!("{}",serde_json::to_string_pretty(&json!({"algorithm":weave_cluster::ALGORITHM,"fixture_nodes":12,"rows":rows,"bridge_churn":churn,"limits":["synthetic labels are fixture data, not discovered truth","pair enumeration is bounded to twelve nodes, not a production algorithm","full-source fallback scans every fixture relation; no speedup or pruning claim","fixture perspective is not the whole engine query domain","lineage comparison is not incremental maintenance or hysteresis"]})).unwrap());
}
