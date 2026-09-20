use serde_json::json;
use weave_contract::{handler_registration::*, *};
fn draft() -> CompiledHandlerTemplate {
    serde_json::from_value(json!({"format":HANDLER_TEMPLATE_FORMAT,"protocol":HANDLER_TEMPLATE_PROTOCOL,"name":"Diagnose","revision":"1","input":{"graph_id":"Input","branch_id":"main","metadata_depth":4},"event_types":["graph.committed","graph.accepted"],"recipe":{"bindings":[],"output":"$event"},"output_slot":"warnings","source_revisions":[],"definition_digest":""})).unwrap()
}
fn reference() -> GraphExpression {
    GraphExpression::Reference {
        name: HANDLER_EVENT_BINDING.into(),
    }
}
#[test]
fn sealed_identity_has_fixed_framing_and_canonical_events() {
    let mut historical = draft();
    historical.protocol = "0.18.0".into();
    let value = seal_handler_template(historical).unwrap();
    validate_handler_template(&value).unwrap();
    assert_eq!(seal_handler_template(value.clone()).unwrap(), value);
    assert_eq!(
        value.event_types,
        vec![
            HandlerEventType::GraphAccepted,
            HandlerEventType::GraphCommitted
        ]
    );
    assert_eq!(value.source_revisions.len(), 1);
    assert!(value.source_revisions[0]
        .name
        .starts_with("weave:handler-template:"));
    assert_eq!(
        value.definition_digest,
        "sha256:04ff25f55c9d3c272cc0381f69bc3c8dd1325a4ba445667471fc7e672ee87849"
    );
    assert_eq!(
        value.source_revisions[0].digest,
        "sha256:bdc9d1cde698a8adae7b8300682e39b6af8fda26d138b79f72e3d620a1d1a2da"
    );
    let mut another = draft();
    another.protocol = "0.18.0".into();
    another.output_slot = "other".into();
    assert_ne!(
        seal_handler_template(another).unwrap().definition_digest,
        value.definition_digest
    );
    let mut changed = value.clone();
    changed.input.metadata_depth = 3;
    assert!(validate_handler_template(&changed).is_err());
}
#[test]
fn old_handler_protocols_and_unknown_fields_are_rejected() {
    for version in ["0.1.0", "0.16.0", "0.17.0"] {
        let mut value = draft();
        value.protocol = version.into();
        assert_eq!(
            seal_handler_template(value).unwrap_err().code,
            "E_HANDLER_VERSION"
        );
    }
    let mut value = serde_json::to_value(draft()).unwrap();
    value["authority"] = json!("grant");
    assert!(serde_json::from_value::<CompiledHandlerTemplate>(value).is_err());
}
#[test]
fn bindings_are_acyclic_and_every_unused_operand_is_checked() {
    let mut value = draft();
    value.recipe.bindings.push(HandlerBinding {
        name: "first".into(),
        value: GraphExpression::Reference {
            name: "future".into(),
        },
    });
    assert_eq!(
        seal_handler_template(value).unwrap_err().code,
        "E_HANDLER_BINDING"
    );
    for field in ["$event", "$secret"] {
        let mut value = draft();
        value.recipe.bindings.push(HandlerBinding {
            name: field.into(),
            value: reference(),
        });
        assert!(seal_handler_template(value).is_err());
    }
    let denied = GraphExpression::Query {
        query: serde_json::from_value(json!({"graph_id":"hidden"})).unwrap(),
    };
    let nested = GraphExpression::Geometry {
        operation: GeometryOperation::Distance {
            left: GeometryOperand {
                input: Box::new(reference()),
                assertion_id: "a".into(),
            },
            right: GeometryOperand {
                input: Box::new(denied),
                assertion_id: "b".into(),
            },
        },
        valid_at: 0,
    };
    let mut value = draft();
    value.recipe.bindings.push(HandlerBinding {
        name: "unused".into(),
        value: nested,
    });
    assert_eq!(
        seal_handler_template(value).unwrap_err().code,
        "E_HANDLER_READ"
    );
}
#[test]
fn borrowed_validation_bounds_depth_total_work_and_decode_lists() {
    let mut value = draft();
    let mut expr = reference();
    for _ in 0..34 {
        expr = GraphExpression::Explain {
            input: Box::new(expr),
        };
    }
    value.recipe.bindings.push(HandlerBinding {
        name: "deep".into(),
        value: expr,
    });
    assert_eq!(seal_handler_template(value).unwrap_err().code, "E_BUDGET");
    let mut value = draft();
    for n in 0..251 {
        let mut expr = reference();
        for _ in 0..3 {
            expr = GraphExpression::Explain {
                input: Box::new(expr),
            };
        }
        value.recipe.bindings.push(HandlerBinding {
            name: format!("v{n}"),
            value: expr,
        });
    }
    assert_eq!(seal_handler_template(value).unwrap_err().code, "E_BUDGET");
    let mut value = serde_json::to_value(draft()).unwrap();
    value["recipe"]["bindings"] = json!(vec![
        json!({"name":"v","value":{"kind":"reference","name":"$event"}});
        257
    ]);
    assert!(serde_json::from_value::<CompiledHandlerTemplate>(value).is_err());
}
#[test]
fn source_conflicts_and_changed_recipe_cannot_keep_old_identity() {
    let mut value = draft();
    value.source_revisions = vec![
        SourceRevision {
            name: "module:X".into(),
            revision: "1".into(),
            digest: format!("sha256:{}", "a".repeat(64)),
        },
        SourceRevision {
            name: "module:X".into(),
            revision: "1".into(),
            digest: format!("sha256:{}", "b".repeat(64)),
        },
    ];
    assert_eq!(
        seal_handler_template(value).unwrap_err().code,
        "E_SOURCE_REVISION"
    );
    let mut sealed = seal_handler_template(draft()).unwrap();
    sealed.definition_digest = format!("sha256:{}", "0".repeat(64));
    assert_eq!(
        validate_handler_template(&sealed).unwrap_err().code,
        "E_HANDLER_TEMPLATE"
    );
}
#[test]
fn prior_view_protocols_preserve_sealed_source_and_definition_bytes() {
    for version in ["0.16.0", "0.17.0", "0.18.0"] {
        let value:CompiledViewTemplate=serde_json::from_value(json!({"format":"weave-view-registration/1","protocol":version,"name":"Old","revision":"1","expression":{"kind":"query","query":{"graph_id":"Input"}},"clock":"fixed","source_revisions":[],"definition_digest":""})).unwrap();
        let sealed = view_registration::seal_template(value).unwrap();
        let bytes = serde_json::to_vec(&sealed).unwrap();
        view_registration::validate_template(&sealed).unwrap();
        assert_eq!(
            serde_json::to_vec(&view_registration::seal_template(sealed).unwrap()).unwrap(),
            bytes
        );
    }
}
#[test]
fn sealed_nested_reason_sources_are_derived_and_cannot_be_spoofed() {
    let mut value = draft();
    let set: RuleSet =
        serde_json::from_value(json!({"id":"rules:local","revision":"1","rules":[]})).unwrap();
    value.recipe.bindings.push(HandlerBinding {
        name: "unused".into(),
        value: GraphExpression::Filter {
            input: Box::new(GraphExpression::Reason {
                input: Box::new(reference()),
                rules: set.clone(),
            }),
            predicate: None,
            valid_at: None,
        },
    });
    let sealed = seal_handler_template(value.clone()).unwrap();
    let source = SourceRevision {
        name: set.id,
        revision: set.revision,
        digest: identity::source_fingerprint(
            &serde_json::from_value::<RuleSet>(
                json!({"id":"rules:local","revision":"1","rules":[]}),
            )
            .unwrap(),
        )
        .unwrap(),
    };
    assert!(sealed.source_revisions.contains(&source));
    let mut omitted = sealed.clone();
    omitted.source_revisions.retain(|s| s.name != source.name);
    omitted.definition_digest = handler_definition_digest(&omitted).unwrap();
    assert!(validate_handler_template(&omitted).is_err());
    value.source_revisions.push(SourceRevision {
        digest: format!("sha256:{}", "0".repeat(64)),
        ..source
    });
    assert_eq!(
        seal_handler_template(value).unwrap_err().code,
        "E_SOURCE_REVISION"
    );
}

#[test]
fn temporal_recipe_requires_new_profile_without_resealing_old_artifacts() {
    let current = seal_handler_template(draft()).unwrap();
    assert_eq!(current.protocol, "0.19.0");
    assert_eq!(
        current.definition_digest,
        "sha256:b23292b6476c3541b3c53c8a266bc08e82e0dc9ddbf3cb25ac37bccd0a92f91d"
    );
    let mut temporal = draft();
    temporal.recipe.bindings.push(HandlerBinding {
        name: "clipped".into(),
        value: GraphExpression::Window {
            input: Box::new(reference()),
            window: Interval {
                start: 0,
                end: Some(1),
            },
        },
    });
    temporal.recipe.output = "clipped".into();
    validate_handler_template(&seal_handler_template(temporal.clone()).unwrap()).unwrap();
    temporal.protocol = "0.18.0".into();
    assert_eq!(
        seal_handler_template(temporal).unwrap_err().code,
        "E_HANDLER_VERSION"
    );
}
