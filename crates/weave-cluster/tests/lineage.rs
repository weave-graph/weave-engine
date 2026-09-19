use weave_cluster::{Hierarchy, Link, Member, Snapshot};
use weave_contract::{AssertionRef, ContextSelection, GraphRef};
fn hierarchy(nodes: &[&str], pairs: &[(&str, &str)], revision: &str) -> Hierarchy {
    let reference = GraphRef {
        graph_id: "g".into(),
        revision: revision.into(),
    };
    let mut h = Hierarchy::new(Snapshot {
        perspective: "relations".into(),
        context: ContextSelection::Default,
        valid_at: 0,
        sources: vec![reference.clone()],
        nodes: nodes.iter().map(|s| s.to_string()).collect(),
        links: pairs
            .iter()
            .enumerate()
            .map(|(i, (a, b))| Link {
                id: format!("e{i}"),
                from: a.to_string(),
                to: b.to_string(),
                evidence: vec![AssertionRef {
                    graph_id: reference.graph_id.clone(),
                    revision: revision.into(),
                    assertion_id: format!("a{i}"),
                }],
            })
            .collect(),
        partial: true,
    })
    .unwrap();
    h.advance().unwrap();
    h
}
#[test]
fn crossing_partitions_record_both_split_and_merge_without_equating_identities() {
    let a = hierarchy(&["a", "b", "c", "d"], &[("a", "b"), ("c", "d")], "1");
    let b = hierarchy(&["a", "b", "c", "d"], &[("a", "c"), ("b", "d")], "2");
    let l = a.lineage_to(&b).unwrap();
    assert_eq!(l.overlaps.len(), 4);
    assert!(l.overlaps.iter().all(|x| x.shared_leaves == 1));
    assert_eq!(l.splits.len(), 2);
    assert_eq!(l.merges.len(), 2);
    assert!(l.retired.is_empty() && l.created.is_empty());
    assert!(l.partial_input);
    assert_eq!(l.before_level, 1);
    assert_eq!(l.after_level, 1);
    assert_eq!(
        serde_json::to_vec(&l).unwrap(),
        serde_json::to_vec(&a.lineage_to(&b).unwrap()).unwrap()
    );
}
#[test]
fn unchanged_membership_retains_ids_but_new_evidence_has_new_record_revisions() {
    let a = hierarchy(&["a", "b", "island"], &[("a", "b")], "1");
    let b = hierarchy(&["a", "b", "island"], &[("a", "b")], "2");
    let l = a.lineage_to(&b).unwrap();
    assert_eq!(l.overlaps.len(), 2);
    assert!(l.splits.is_empty() && l.merges.is_empty());
    for x in &l.overlaps {
        assert_eq!(x.before.member, x.after.member);
        assert_ne!(x.before.revision, x.after.revision);
    }
    let same = a.lineage_to(&a).unwrap();
    assert!(same.overlaps.iter().all(|x| x.before == x.after));
}
#[test]
fn added_removed_and_isolated_leaves_are_not_silently_dropped() {
    let a = hierarchy(&["a", "b", "gone"], &[("a", "b")], "1");
    let b = hierarchy(&["a", "new"], &[], "2");
    let l = a.lineage_to(&b).unwrap();
    assert_eq!(l.removed_leaves, vec!["b", "gone"]);
    assert_eq!(l.added_leaves, vec!["new"]);
    assert_eq!(l.retired.len(), 1);
    assert_eq!(l.retired[0].member, Member::Leaf("gone".into()));
    assert_eq!(l.created.len(), 1);
    assert_eq!(l.created[0].member, Member::Leaf("new".into()));
    assert_eq!(l.overlaps[0].shared_leaves, 1);
}
#[test]
fn lineage_rejects_cross_context_perspective_and_source_domains() {
    let a = hierarchy(&["a"], &[], "1");
    for change in 0..3 {
        let mut s = a.snapshot().clone();
        match change {
            0 => s.perspective = "other".into(),
            1 => {
                s.context = ContextSelection::Pinned {
                    reference: GraphRef {
                        graph_id: "world".into(),
                        revision: "1".into(),
                    },
                }
            }
            _ => s.sources[0].graph_id = "other".into(),
        }
        let b = Hierarchy::new(s).unwrap();
        assert_eq!(a.lineage_to(&b).unwrap_err().0, "E_CLUSTER_SCOPE");
    }
}
#[test]
fn ambiguous_multi_graph_domains_and_oversized_lineage_reject_without_mutation() {
    let one = hierarchy(&["a"], &[], "1");
    let mut source = one.snapshot().clone();
    source.sources.push(GraphRef {
        graph_id: "other".into(),
        revision: "1".into(),
    });
    let many = Hierarchy::new(source).unwrap();
    assert_eq!(many.lineage_to(&many).unwrap_err().0, "E_CLUSTER_SCOPE");
    let make = |prefix: &str| {
        let mut source = one.snapshot().clone();
        source.nodes = (0..10000)
            .map(|i| format!("{prefix}{i:05}{}", "x".repeat(1000)))
            .collect();
        Hierarchy::new(source).unwrap()
    };
    let a = make("old");
    let b = make("new");
    let prior = a.manifest();
    assert_eq!(a.lineage_to(&b).unwrap_err().0, "E_CLUSTER_BUDGET");
    assert_eq!(a.manifest(), prior);
}
