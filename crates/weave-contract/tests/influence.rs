use serde_json::json;
use weave_contract::*;
fn ctx() -> AlgebraContext {
    AlgebraContext {
        principal: "alice".into(),
        max_objects: 1000,
        max_output_bytes: 1024 * 1024,
    }
}
fn input() -> QueryResult {
    serde_json::from_value(json!({"version":VERSION,"graph":{"nodes":[{"id":"n","entity_id":"n","space_id":"s"}],"edges":[{"id":"p","predicate":"p","from":"n","to":"n","valid_time":{"start":0,"end":10},"derivations":[
        {"operator":"first","premises":[],"node_premises":[{"graph_id":"a","revision":"r","node_id":"n"}]},
        {"operator":"second","premises":[],"node_premises":[{"graph_id":"b","revision":"r","node_id":"n"}]}]}]},
        "snapshots":{},"input_snapshots":[],"coverage":"complete","diagnostics":[],"provenance":[],"metadata_graphs":[],"edge_origins":{"p":[]},"node_origins":{"n":[]}
    })).unwrap()
}
fn rules() -> RuleSet {
    let atom = |predicate: &str| RuleAtom {
        predicate: predicate.into(),
        from: RuleTerm::Variable { name: "x".into() },
        to: RuleTerm::Variable { name: "y".into() },
        polarity: Polarity::Positive,
    };
    RuleSet {
        id: "test".into(),
        revision: "1".into(),
        rules: vec![Rule {
            id: "step".into(),
            head: atom("q"),
            body: vec![atom("p")],
            allow_cross_space: false,
        }],
    }
}
#[test]
fn node_only_alternatives_remain_distinct_in_rules_support_and_explanation() {
    let source = input();
    let budget = RuleBudget {
        max_steps: 10000,
        max_rounds: 10,
        max_facts: 100,
        max_derivations: 128,
    };
    let out = rules::reason(source.clone(), &rules(), &budget, &ctx()).unwrap();
    let derived = out.graph.edges.iter().find(|e| e.predicate == "q").unwrap();
    assert_eq!(derived.derivations.len(), 2);
    assert!(derived
        .derivations
        .iter()
        .all(|g| g.premises.is_empty() && g.node_premises.len() == 1));
    assert_ne!(
        derived.derivations[0].node_premises,
        derived.derivations[1].node_premises
    );
    let repeated = rules::reason(out.clone(), &rules(), &budget, &ctx()).unwrap();
    assert_eq!(out.graph, repeated.graph);
    let entity = EntitySpace {
        entity_id: "n".into(),
        space_id: "s".into(),
    };
    let support = algebra::support(source.clone(), "p", &entity, &entity, 5, &ctx()).unwrap();
    assert_eq!(support.graph.edges[0].derivations.len(), 2);
    assert_eq!(support.graph.nodes[0].derived_nodes.len(), 2);
    let explained = identity::explain(&source, &ctx()).unwrap();
    let groups: Vec<_> = explained
        .graph
        .nodes
        .iter()
        .filter(|n| n.properties["kind"] == "derivation")
        .collect();
    assert_eq!(groups.len(), 2);
    assert!(groups.iter().all(|n| n.derived_nodes.len() == 1));
    assert_ne!(groups[0].derived_nodes, groups[1].derived_nodes);
    assert_eq!(
        explained
            .graph
            .nodes
            .iter()
            .filter(|n| n.properties["kind"] == "node_premise")
            .count(),
        2
    );
    assert!(explained
        .graph
        .edges
        .iter()
        .all(|e| e.derivations[0].node_premises.len() == 1));
}
#[test]
fn generated_protection_checks_profile_and_exact_final_byte_bound() {
    let mut source = input();
    source.graph.influence = Some(GraphInfluence {
        assertions: vec![],
        nodes: vec![NodeRef {
            graph_id: "secret".into(),
            revision: "r".into(),
            node_id: "n".into(),
        }],
    });
    let mut protected = source.clone();
    influence::protect_generated_result(&mut protected, 1024 * 1024).unwrap();
    assert!(protected
        .graph
        .edges
        .iter()
        .flat_map(|e| &e.derivations)
        .all(|g| g.input_snapshots.iter().any(|p| p.graph_id == "secret")));
    let exact = serde_json::to_vec(&protected).unwrap().len();
    assert!(influence::protect_generated_result(&mut source.clone(), exact - 1).is_err());
    let mut explicit = source;
    explicit.graph.profile = GraphProfile::Explicit;
    assert_eq!(
        influence::protect_generated_result(&mut explicit, 1024 * 1024)
            .unwrap_err()
            .code,
        "E_INFLUENCE_PROFILE"
    );
}
