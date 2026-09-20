use serde_json::json;
use weave_contract::{view_registration::*, *};
fn draft() -> CompiledViewTemplate {
    serde_json::from_value(json!({"format":VIEW_TEMPLATE_FORMAT,"protocol":VIEW_TEMPLATE_PROTOCOL,"name":"Active","revision":"1","expression":{"kind":"query","query":{"graph_id":"Fleet","branch_id":"main","include_metadata":false}},"clock":"tick","source_revisions":[],"definition_digest":""})).unwrap()
}
#[test]
fn canonical_recipe_identity_is_stable_and_sensitive_to_sources_and_semantics() {
    let sealed = seal_template(draft()).unwrap();
    validate_template(&sealed).unwrap();
    assert_eq!(seal_template(sealed.clone()).unwrap(), sealed);
    assert_eq!(sealed.source_revisions.len(), 1);
    assert!(sealed.source_revisions[0]
        .name
        .starts_with("weave:view-template:"));
    for changed in ["predicate", "clock", "revision", "source"] {
        let mut next = draft();
        match changed {
            "predicate" => {
                if let GraphExpression::Query { query } = &mut next.expression {
                    query.predicate = Some("changed".into());
                }
            }
            "clock" => next.clock = ViewClock::Fixed,
            "revision" => next.revision = "2".into(),
            _ => next.source_revisions.push(SourceRevision {
                name: "module:X".into(),
                revision: "1".into(),
                digest: format!("sha256:{}", "a".repeat(64)),
            }),
        }
        assert_ne!(
            seal_template(next).unwrap().definition_digest,
            sealed.definition_digest
        );
    }
}
#[test]
fn tampering_conflicts_captures_and_excessive_recipes_reject() {
    let sealed = seal_template(draft()).unwrap();
    let mut changed = sealed.clone();
    changed.source_revisions.clear();
    assert_eq!(
        validate_template(&changed).unwrap_err().code,
        "E_VIEW_TEMPLATE"
    );
    let mut changed = sealed.clone();
    changed.source_revisions[0].digest = format!("sha256:{}", "f".repeat(64));
    assert_eq!(
        validate_template(&changed).unwrap_err().code,
        "E_SOURCE_REVISION"
    );
    let mut changed = draft();
    changed.expression = GraphExpression::CurrentView {
        selection: CurrentViewSelection {
            view_id: "other".into(),
            definition_digest: sealed.definition_digest,
            time: ViewReadTime::Fixed,
        },
    };
    assert_eq!(seal_template(changed).unwrap_err().code, "E_VIEW_TEMPLATE");
    let mut changed = draft();
    for _ in 0..33 {
        changed.expression = GraphExpression::Filter {
            input: Box::new(changed.expression),
            predicate: None,
            valid_at: None,
        };
    }
    assert_eq!(seal_template(changed).unwrap_err().code, "E_BUDGET");
    let mut changed = draft();
    changed.name = "x".repeat(1024 * 1024);
    assert!(seal_template(changed).is_err());
}
#[test]
fn wire_requires_exact_occurrence_definition_and_explicit_clock() {
    assert!(serde_json::from_value::<AcceptedGraphSelection>(json!({"view_id":"v"})).is_err());
    assert!(serde_json::from_value::<CurrentViewSelection>(
        json!({"view_id":"v","definition_digest":"d"})
    )
    .is_err());
    assert!(serde_json::from_value::<ViewReadTime>(json!({"kind":"fixed","valid_at":1})).is_err());
    assert_eq!(
        serde_json::to_string(&ViewClock::Fixed).unwrap(),
        "\"fixed\""
    );
    assert_eq!(
        serde_json::to_value(ViewReadTime::Tick { valid_at: 7 }).unwrap(),
        json!({"kind":"tick","valid_at":7})
    );
}
