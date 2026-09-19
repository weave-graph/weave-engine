use std::collections::BTreeSet;
use weave_cluster::*;
use weave_contract::{AssertionRef, ContextSelection, GraphRef};

fn graph(n: usize, pairs: &[(usize, usize)]) -> Snapshot {
    Snapshot {
        perspective: "topology".into(),
        context: ContextSelection::Default,
        valid_at: 10,
        sources: vec![GraphRef {
            graph_id: "g".into(),
            revision: "r".into(),
        }],
        nodes: (0..n).map(|i| format!("node-{i:04}")).collect(),
        links: pairs
            .iter()
            .enumerate()
            .map(|(i, (a, b))| Link {
                id: format!("link-{i}"),
                from: format!("node-{a:04}"),
                to: format!("node-{b:04}"),
                evidence: vec![AssertionRef {
                    graph_id: "g".into(),
                    revision: "r".into(),
                    assertion_id: format!("claim-{i}"),
                }],
            })
            .collect(),
        partial: false,
    }
}
fn finish(h: &mut Hierarchy) -> Manifest {
    loop {
        let m = h.advance().unwrap();
        if m.evidence_boundary {
            return m;
        }
    }
}
fn collect(h: &Hierarchy, m: &Member) -> BTreeSet<String> {
    match m {
        Member::Leaf(id) => BTreeSet::from([id.clone()]),
        Member::Cluster(id) => h
            .zoom(id)
            .unwrap()
            .iter()
            .flat_map(|c| collect(h, c))
            .collect(),
    }
}

#[test]
fn lazy_recursive_navigation_preserves_exact_leaf_and_evidence_sets() {
    let source = graph(65, &(0..63).map(|i| (i, i + 1)).collect::<Vec<_>>());
    let original = source.clone();
    let mut h = Hierarchy::new(source).unwrap();
    assert_eq!(h.manifest().level, 0);
    assert_eq!(h.manifest().frontier.len(), 65);
    let first = h.advance().unwrap();
    assert_eq!(first.frontier.len(), 33);
    assert!(!first.evidence_boundary);
    let result = finish(&mut h);
    assert!(result.level >= 6);
    // Disconnected node remains evidence, not invented into an unrelated aggregate.
    assert_eq!(result.frontier.len(), 2);
    let leaves: BTreeSet<_> = result
        .frontier
        .iter()
        .flat_map(|c| collect(&h, c))
        .collect();
    assert_eq!(leaves, original.nodes.iter().cloned().collect());
    let aggregate = result
        .frontier
        .iter()
        .find_map(|m| match m {
            Member::Cluster(id) => Some(id),
            _ => None,
        })
        .unwrap();
    let mut all = Vec::new();
    let mut offset = 0;
    loop {
        let page = h.evidence(aggregate, offset, 7).unwrap();
        all.extend(page.items);
        match page.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }
    assert_eq!(
        all.into_iter().collect::<BTreeSet<_>>(),
        original.links.iter().map(|e| e.id.clone()).collect()
    );
    let mut canonical = original;
    canonical.links.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(h.snapshot(), &canonical);
    assert_eq!(h.advance().unwrap(), result);
}

#[test]
fn deterministic_order_and_private_snapshot_revision_do_not_change_layout() {
    let source = graph(8, &[(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7)]);
    let mut reordered = source.clone();
    reordered.nodes.reverse();
    reordered.links.reverse();
    let mut a = Hierarchy::new(source.clone()).unwrap();
    let mut b = Hierarchy::new(reordered).unwrap();
    assert_eq!(finish(&mut a), finish(&mut b));
    // Host visibility filtering produced the same visible facts in a later source revision.
    // This tests layout invariance; source provenance intentionally still names the new pin.
    let mut changed = source;
    changed.sources[0].revision = "private-only-revision".into();
    for link in &mut changed.links {
        link.evidence[0].revision = "private-only-revision".into();
    }
    let mut c = Hierarchy::new(changed).unwrap();
    assert_eq!(a.manifest(), finish(&mut c));
    assert_ne!(a.snapshot().sources, c.snapshot().sources);
}

#[test]
fn directed_aggregate_links_are_existential_and_keep_all_contributions() {
    let mut h = Hierarchy::new(graph(4, &[(0, 1), (2, 3), (1, 2), (2, 1), (0, 3)])).unwrap();
    h.advance().unwrap();
    let links = h.aggregate_links().unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(
        links
            .iter()
            .map(|e| e.contributing_links.len())
            .sum::<usize>(),
        3
    );
    assert_eq!(links[0].from, links[1].to);
    assert_eq!(links[0].to, links[1].from);
}

#[test]
fn separate_perspectives_overlap_without_membership_cycles() {
    let mut source = graph(3, &[(0, 1)]);
    let mut a = Hierarchy::new(source.clone()).unwrap();
    source.links[0].to = "node-0002".into();
    source.perspective = "other-relation".into();
    let mut b = Hierarchy::new(source).unwrap();
    let ma = finish(&mut a);
    let mb = finish(&mut b);
    let ca = ma
        .frontier
        .iter()
        .find_map(|m| {
            if let Member::Cluster(id) = m {
                Some(id)
            } else {
                None
            }
        })
        .unwrap();
    let cb = mb
        .frontier
        .iter()
        .find_map(|m| {
            if let Member::Cluster(id) = m {
                Some(id)
            } else {
                None
            }
        })
        .unwrap();
    assert_ne!(ca, cb);
    assert_eq!(
        a.cluster(ca)
            .unwrap()
            .leaves
            .iter()
            .filter(|n| b.cluster(cb).unwrap().leaves.contains(n))
            .count(),
        1
    );
    for id in [ca] {
        for child in a.zoom(id).unwrap() {
            assert!(matches!(child, Member::Leaf(_)));
        }
    }
}

#[test]
fn budgets_and_bad_input_never_appear_as_complete_expansion() {
    let source = graph(3, &[(0, 1), (1, 2)]);
    let mut h = Hierarchy::with_budget(source.clone(), 8).unwrap();
    let before = h.manifest();
    assert_eq!(h.advance().unwrap_err(), Error("E_CLUSTER_BUDGET"));
    assert_eq!(h.manifest().frontier, before.frontier);
    assert!(!h.manifest().evidence_boundary);
    let mut bad = source.clone();
    bad.nodes.push(bad.nodes[0].clone());
    assert!(matches!(Hierarchy::new(bad), Err(Error("E_CLUSTER_INPUT"))));
    let mut bad = source.clone();
    bad.links[0].evidence[0].revision = "missing".into();
    assert!(matches!(Hierarchy::new(bad), Err(Error("E_CLUSTER_INPUT"))));
    let mut partial = source;
    partial.partial = true;
    let mut h = Hierarchy::new(partial).unwrap();
    let m = finish(&mut h);
    assert!(m.partial_input && m.approximate_navigation && m.evidence_boundary);
    let id = match &m.frontier[0] {
        Member::Cluster(id) => id,
        _ => panic!(),
    };
    assert_eq!(
        h.evidence(id, usize::MAX, 1).unwrap_err(),
        Error("E_CLUSTER_CURSOR")
    );
    assert_eq!(h.evidence(id, 0, 0).unwrap_err(), Error("E_CLUSTER_BUDGET"));
}
