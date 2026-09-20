//! Independent source/native boundary checks. Draft until the canonical 0.16 freeze.
use serde_json::json;
use weave_contract::view_registration::{
    seal_template, VIEW_TEMPLATE_FORMAT, VIEW_TEMPLATE_PROTOCOL,
};
use weave_contract::*;
use weave_engine::*;

fn host() -> HostContext {
    HostContext::new("alice", ["g".into(), "marker".into()])
}
fn query() -> GraphExpression {
    GraphExpression::Query {
        query: serde_json::from_value(json!({"graph_id":"g"})).unwrap(),
    }
}
fn program(version: &str, commands: Vec<Command>) -> Program {
    Program {
        version: version.into(),
        source_revisions: vec![],
        commands,
    }
}
fn write(e: &mut Engine, empty: bool) {
    let data = if empty {
        GraphData::default()
    } else {
        serde_json::from_value(json!({"nodes":[{"id":"n","entity_id":"n","space_id":"s"}]}))
            .unwrap()
    };
    let expected_head = e.head("g", "main").unwrap();
    e.execute(
        &program(
            VERSION,
            vec![Command::Commit {
                graph_id: "g".into(),
                branch_id: "main".into(),
                expected_head,
                data,
            }],
        ),
        &host(),
    )
    .unwrap();
}
fn template() -> CompiledViewTemplate {
    seal_template(CompiledViewTemplate {
        format: VIEW_TEMPLATE_FORMAT.into(),
        protocol: VIEW_TEMPLATE_PROTOCOL.into(),
        name: "RootActive".into(),
        revision: "1".into(),
        expression: query(),
        clock: ViewClock::Fixed,
        source_revisions: vec![SourceRevision {
            name: "root-module".into(),
            revision: "1".into(),
            digest: format!("sha256:{}", "a".repeat(64)),
        }],
        definition_digest: String::new(),
    })
    .unwrap()
}
fn current(id: &str, digest: &str) -> GraphExpression {
    GraphExpression::CurrentView {
        selection: CurrentViewSelection {
            view_id: id.into(),
            definition_digest: digest.into(),
            time: ViewReadTime::Fixed,
        },
    }
}
fn marker() -> Command {
    Command::Commit {
        graph_id: "marker".into(),
        branch_id: "main".into(),
        expected_head: None,
        data: GraphData::default(),
    }
}
fn nested(input: GraphExpression) -> GraphExpression {
    GraphExpression::Filter {
        input: Box::new(input),
        predicate: None,
        valid_at: None,
    }
}

#[test]
fn root_compiled_identity_survives_empty_refresh_restart_and_cannot_be_legacy_spoofed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("views.db");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, false);
    let t = template();
    let registered = e
        .register_compiled_view("compiled", &t, None, &host())
        .unwrap();
    assert_eq!(registered.result.source_revisions, t.source_revisions);
    e.enroll_incremental_view("compiled", &host()).unwrap();
    write(&mut e, true);
    let empty = e.refresh_view("compiled", None, &host()).unwrap();
    assert!(empty.result.graph.nodes.is_empty());
    assert_eq!(empty.result.source_revisions, t.source_revisions);
    e.register_view(
        &ViewDefinition {
            id: "legacy".into(),
            expression: t.expression.clone(),
            clock: t.clock.clone(),
        },
        None,
        &host(),
    )
    .unwrap();
    drop(e);
    let mut e = Engine::open(&path).unwrap();
    let before = e
        .read_view("compiled", None, ViewFreshness::RequireCurrent, &host())
        .unwrap();
    assert_eq!(before.result.source_revisions, t.source_revisions);
    assert!(e
        .execute(
            &program(
                VERSION,
                vec![Command::Evaluate {
                    value: current("compiled", &t.definition_digest)
                }]
            ),
            &host()
        )
        .is_ok());
    assert!(e
        .execute(
            &program(
                VERSION,
                vec![Command::Evaluate {
                    value: current("legacy", &t.definition_digest)
                }]
            ),
            &host()
        )
        .is_err());
    let mut changed = t.clone();
    changed.source_revisions[0].digest = format!("sha256:{}", "b".repeat(64));
    assert!(e
        .register_compiled_view("tampered", &changed, None, &host())
        .is_err());
    assert!(e
        .read_view("tampered", None, ViewFreshness::AllowStale, &host())
        .is_err());
    // A recomputed valid digest still cannot replace the immutable instance definition.
    let mut changed = t.clone();
    changed.revision = "2".into();
    changed
        .source_revisions
        .retain(|s| !s.name.starts_with("weave:view-template:"));
    let changed = seal_template(changed).unwrap();
    assert!(e
        .register_compiled_view("compiled", &changed, None, &host())
        .is_err());
    let after = e
        .read_view("compiled", None, ViewFreshness::RequireCurrent, &host())
        .unwrap();
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.result, before.result);
}

#[test]
fn root_old_wire_and_dynamic_view_registration_reject_nested_read_variants() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, false);
    let t = template();
    e.execute(
        &program("0.15.0", vec![Command::Evaluate { value: query() }]),
        &host(),
    )
    .unwrap();
    for value in [
        current("missing", &t.definition_digest),
        GraphExpression::AcceptedGraph {
            selection: AcceptedGraphSelection {
                view_id: "missing".into(),
                decision_id: "occurrence".into(),
            },
        },
    ] {
        let value = nested(value);
        let error = e
            .execute(
                &program(
                    "0.15.0",
                    vec![
                        marker(),
                        Command::Evaluate {
                            value: value.clone(),
                        },
                    ],
                ),
                &host(),
            )
            .unwrap_err();
        assert_eq!(error.code, "E_VERSION");
        assert_eq!(e.head("marker", "main").unwrap(), None);
        assert!(e
            .register_view(
                &ViewDefinition {
                    id: "recursive".into(),
                    expression: value,
                    clock: ViewClock::Fixed
                },
                None,
                &host()
            )
            .is_err());
        assert!(e
            .read_view("recursive", None, ViewFreshness::AllowStale, &host())
            .is_err());
    }
}

#[test]
fn root_stale_and_definition_mismatch_roll_back_prior_program_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("atomic.db");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, false);
    let t = template();
    e.register_compiled_view("live", &t, None, &host()).unwrap();
    let wrong = format!("sha256:{}", "0".repeat(64));
    assert!(e
        .execute(
            &program(
                VERSION,
                vec![
                    marker(),
                    Command::Evaluate {
                        value: current("live", &wrong)
                    }
                ]
            ),
            &host()
        )
        .is_err());
    assert_eq!(e.head("marker", "main").unwrap(), None);
    write(&mut e, true);
    let error = e
        .execute(
            &program(
                VERSION,
                vec![
                    marker(),
                    Command::Evaluate {
                        value: current("live", &t.definition_digest),
                    },
                ],
            ),
            &host(),
        )
        .unwrap_err();
    assert_eq!(error.code, "E_FRESHNESS");
    drop(e);
    let e = Engine::open(&path).unwrap();
    assert_eq!(e.head("marker", "main").unwrap(), None);
    assert_eq!(
        e.read_view("live", None, ViewFreshness::AllowStale, &host())
            .unwrap()
            .generation,
        1
    );
}
