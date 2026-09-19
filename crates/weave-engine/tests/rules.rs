use serde_json::json;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("alice", ["g".into(), "derived".into()])
}
fn graph() -> GraphData {
    serde_json::from_value(json!({"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"},{"id":"c","entity_id":"C","space_id":"s"}],"edges":[{"id":"ab","predicate":"link","from":"a","to":"b","valid_time":{"start":0,"end":10}},{"id":"bc","predicate":"link","from":"b","to":"c","valid_time":{"start":5,"end":20}}]})).unwrap()
}
fn term(name: &str) -> RuleTerm {
    RuleTerm::Variable { name: name.into() }
}
fn atom(p: &str, a: &str, b: &str) -> RuleAtom {
    RuleAtom {
        predicate: p.into(),
        from: term(a),
        to: term(b),
        polarity: Polarity::Positive,
    }
}
fn set() -> RuleSet {
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
fn expression() -> GraphExpression {
    GraphExpression::Reason {
        input: Box::new(GraphExpression::Query {
            query: serde_json::from_value(json!({"graph_id":"g"})).unwrap(),
        }),
        rules: set(),
    }
}
fn commit(data: GraphData, head: Option<String>) -> Command {
    Command::Commit {
        graph_id: "g".into(),
        branch_id: "main".into(),
        expected_head: head,
        data,
    }
}
fn program(commands: Vec<Command>) -> Program {
    Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands,
    }
}
fn result(outputs: Vec<CommandResult>) -> QueryResult {
    match outputs.into_iter().last().unwrap() {
        CommandResult::Queried { result } => *result,
        _ => panic!("expected query"),
    }
}
#[test]
fn executable_reason_preserves_temporal_leaf_proof_and_restrictions_on_repersistence() {
    let mut e = Engine::memory().unwrap();
    let mut data = graph();
    data.edges[1].readers = vec!["alice".into()];
    let out = result(
        e.execute(
            &program(vec![
                commit(data, None),
                Command::Evaluate {
                    value: expression(),
                },
            ]),
            &host(),
        )
        .unwrap(),
    );
    let ac = out
        .graph
        .edges
        .iter()
        .find(|e| e.predicate == "reach" && e.from == "a" && e.to == "c")
        .unwrap();
    assert_eq!(
        ac.valid_time,
        Interval {
            start: 5,
            end: Some(10)
        }
    );
    assert_eq!(ac.derivations[0].premises.len(), 2);
    assert!(ac.readers == ["alice"]);
    assert!(out
        .source_revisions
        .iter()
        .any(|s| s.name == "reachability"));
    assert_eq!(e.event_count().unwrap(), 1);
    let bob = result(
        e.execute(
            &program(vec![Command::Evaluate {
                value: expression(),
            }]),
            &HostContext::new("bob", []),
        )
        .unwrap(),
    );
    assert!(!bob.graph.edges.iter().any(|e| e.from == "a" && e.to == "c"));
    let mut repersist = out.graph;
    for node in &mut repersist.nodes {
        node.readers.clear();
    }
    for edge in &mut repersist.edges {
        edge.readers.clear();
    }
    e.execute(
        &program(vec![Command::Commit {
            graph_id: "derived".into(),
            branch_id: "main".into(),
            expected_head: None,
            data: repersist,
        }]),
        &host(),
    )
    .unwrap();
    let q: QueryPlan = serde_json::from_value(json!({"graph_id":"derived"})).unwrap();
    assert!(!e
        .query(&q, &HostContext::new("bob", []))
        .unwrap()
        .graph
        .edges
        .iter()
        .any(|e| e.from == "a" && e.to == "c"));
}
#[test]
fn nested_rules_reject_legacy_versions_and_unsafe_module_rolls_back_prior_commit() {
    let mut e = Engine::memory().unwrap();
    let mut p = program(vec![
        commit(graph(), None),
        Command::Evaluate {
            value: GraphExpression::Filter {
                input: Box::new(expression()),
                predicate: Some("reach".into()),
                valid_at: None,
            },
        },
    ]);
    p.version = "0.6.0".into();
    assert_eq!(e.execute(&p, &host()).unwrap_err().code, "E_VERSION");
    assert_eq!(e.event_count().unwrap(), 0);
    p.version = VERSION.into();
    let mut invalid = set();
    invalid.rules[0].head.to = term("unbound");
    p.commands[1] = Command::Evaluate {
        value: GraphExpression::Reason {
            input: Box::new(GraphExpression::Query {
                query: serde_json::from_value(json!({"graph_id":"g"})).unwrap(),
            }),
            rules: invalid,
        },
    };
    assert_eq!(e.execute(&p, &host()).unwrap_err().code, "E_RULE_RANGE");
    assert_eq!(e.event_count().unwrap(), 0);
    assert_eq!(e.head("g", "main").unwrap(), None);
    // Version 0.6 source manifests remain valid after the additive 0.7 operator.
    let mut old = program(vec![commit(graph(), None)]);
    old.version = "0.6.0".into();
    old.source_revisions.push(SourceRevision {
        name: "prior".into(),
        revision: "1".into(),
        digest: "prior-digest".into(),
    });
    assert!(e.execute(&old, &host()).is_ok());
}
#[test]
fn rule_host_budget_failure_rolls_back_whole_program() {
    let mut e = Engine::memory().unwrap();
    let mut d = graph();
    d.nodes.clear();
    d.edges.clear();
    for i in 0..401 {
        d.nodes.push(
            serde_json::from_value(
                json!({"id":format!("n{i}"),"entity_id":format!("e{i}"),"space_id":"s"}),
            )
            .unwrap(),
        );
    }
    for i in 0..400 {
        d.edges.push(serde_json::from_value(json!({"id":format!("edge{i}"),"predicate":"link","from":format!("n{i}"),"to":format!("n{}",i+1),"valid_time":{"start":0}})).unwrap());
    }
    let p = program(vec![
        commit(d, None),
        Command::Evaluate {
            value: expression(),
        },
    ]);
    assert_eq!(e.execute(&p, &host()).unwrap_err().code, "E_RULE_BUDGET");
    assert_eq!(e.event_count().unwrap(), 0);
    assert_eq!(e.head("g", "main").unwrap(), None);
}
#[test]
fn clocked_rule_view_retracts_expired_support_and_matches_fresh_recomputation() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("rules.db");
    let mut e = Engine::open(&db).unwrap();
    e.execute(&program(vec![commit(graph(), None)]), &host())
        .unwrap();
    let definition = ViewDefinition {
        id: "reach-view".into(),
        expression: expression(),
        clock: ViewClock::Tick,
    };
    let first = e.register_view(&definition, Some(6), &host()).unwrap();
    assert!(first
        .result
        .graph
        .edges
        .iter()
        .any(|e| e.from == "a" && e.to == "c"));
    drop(e);
    let mut e = Engine::open(&db).unwrap();
    let second = e.refresh_view("reach-view", Some(10), &host()).unwrap();
    assert!(!second
        .result
        .graph
        .edges
        .iter()
        .any(|e| e.from == "a" && e.to == "c"));
    let GraphExpression::Reason { input, rules } = expression() else {
        unreachable!()
    };
    let GraphExpression::Query { mut query } = *input else {
        unreachable!()
    };
    query.valid_at = Some(10);
    let expected = result(
        e.execute(
            &program(vec![Command::Evaluate {
                value: GraphExpression::Reason {
                    input: Box::new(GraphExpression::Query { query }),
                    rules,
                },
            }]),
            &host(),
        )
        .unwrap(),
    );
    assert_eq!(second.result, expected);
    assert_eq!(e.event_count().unwrap(), 1);
}
#[test]
fn client_source_manifest_cannot_equivocate_about_executed_rule_module() {
    let mut e = Engine::memory().unwrap();
    let mut p = program(vec![
        commit(graph(), None),
        Command::Bind {
            name: "value".into(),
            value: expression(),
        },
    ]);
    p.source_revisions.push(SourceRevision {
        name: "reachability".into(),
        revision: "1".into(),
        digest: "forged".into(),
    });
    assert_eq!(
        e.execute(&p, &host()).unwrap_err().code,
        "E_SOURCE_REVISION"
    );
    assert_eq!(e.event_count().unwrap(), 0);
    assert_eq!(e.head("g", "main").unwrap(), None);
}
