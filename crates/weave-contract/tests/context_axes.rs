use serde_json::{json, Value};
use weave_contract::context_axes::*;

fn definition() -> Value {
    json!({"schema":{"reference":{"id":"world","revision":"1"},"axes":{
        "active":{"kind":"boolean"},"count":{"kind":"integer"},
        "name":{"kind":"string"},"load":{"kind":"decimal"},
        "jurisdiction":{"kind":"enum","members":["FI","EE"]},
        "distance":{"kind":"quantity","unit":{"dimension_id":"length","unit_id":"metre","revision":"1"}}
    }},"values":{"active":false,"count":9007199254740993_i64,"name":"baseline","load":"0.75","jurisdiction":"EE","distance":{"amount":"1.25","unit":{"dimension_id":"length","unit_id":"metre","revision":"1"}}}})
}
fn decode(value: &Value) -> Result<ContextDefinition, ContextError> {
    ContextDefinition::from_json(&serde_json::to_vec(value).unwrap())
}
#[test]
fn exact_total_assignments_preserve_nominal_values_and_canonical_order() {
    let original = decode(&definition()).unwrap();
    assert_eq!(original.values["count"].as_i64(), Some(9007199254740993));
    let canonical = original.canonical_bytes().unwrap();
    let roundtrip = ContextDefinition::from_json(&canonical).unwrap();
    assert_eq!(canonical, roundtrip.canonical_bytes().unwrap());
    assert_eq!(original.fingerprint(), roundtrip.fingerprint());
    let mut reordered = definition();
    reordered["schema"]["axes"]["jurisdiction"]["members"] = json!(["EE", "FI"]);
    assert_eq!(
        decode(&reordered).unwrap().fingerprint(),
        original.fingerprint()
    );
    reordered["values"]["jurisdiction"] = json!("FI");
    assert_ne!(
        decode(&reordered).unwrap().fingerprint(),
        original.fingerprint()
    );
}
#[test]
fn no_missing_extra_null_float_or_implicit_exact_value_conversions() {
    for (axis, wrong) in [
        ("active", json!(1)),
        ("count", json!(1.0)),
        ("count", json!(u64::MAX)),
        ("name", Value::Null),
        ("load", json!(0.75)),
        ("load", json!("0.750")),
        ("jurisdiction", json!("ee")),
    ] {
        let mut value = definition();
        value["values"][axis] = wrong;
        assert_eq!(decode(&value), Err(ContextError::Assignment), "{axis}");
    }
    let mut value = definition();
    value["values"].as_object_mut().unwrap().remove("active");
    assert_eq!(decode(&value), Err(ContextError::Assignment));
    value["values"]["unknown"] = json!(false);
    assert_eq!(decode(&value), Err(ContextError::Assignment));
    let mut value = definition();
    value["values"]["distance"]["unit"]["revision"] = json!("2");
    assert_eq!(decode(&value), Err(ContextError::Assignment));
    value["values"]["distance"]["unit"]["revision"] = json!("1");
    value["values"]["distance"]["amount"] = json!(1.25);
    assert_eq!(decode(&value), Err(ContextError::Assignment));
}
#[test]
fn duplicate_members_are_rejected_at_every_json_depth() {
    let text = serde_json::to_string(&definition()).unwrap();
    for (from, to) in [
        ("\"active\":false", "\"active\":false,\"active\":true"),
        ("\"id\":\"world\"", "\"id\":\"world\",\"id\":\"other\""),
        (
            "\"amount\":\"1.25\"",
            "\"amount\":\"1.25\",\"amount\":\"2\"",
        ),
        (
            "\"kind\":\"boolean\"",
            "\"kind\":\"boolean\",\"kind\":\"string\"",
        ),
        (
            "\"active\":{\"kind\":\"boolean\"}",
            "\"active\":{\"kind\":\"boolean\"},\"active\":{\"kind\":\"integer\"}",
        ),
    ] {
        assert!(text.contains(from));
        assert_eq!(
            ContextDefinition::from_json(text.replace(from, to).as_bytes()),
            Err(ContextError::Syntax)
        );
    }
    let top = format!(
        "{{\"schema\":{},\"schema\":{},\"values\":{}}}",
        definition()["schema"],
        definition()["schema"],
        definition()["values"]
    );
    assert_eq!(
        ContextDefinition::from_json(top.as_bytes()),
        Err(ContextError::Syntax)
    );
    let escaped = text.replace("\"active\":false", r#""active":false,"\u0061ctive":true"#);
    assert_eq!(
        ContextDefinition::from_json(escaped.as_bytes()),
        Err(ContextError::Syntax)
    );
}
#[test]
fn schema_labels_cannot_change_meaning_and_conflicts_do_not_replace_prior() {
    let original = decode(&definition()).unwrap();
    let mut registry = ContextSchemas::default();
    let first = registry.register(&original.schema).unwrap();
    let mut reordered = original.schema.clone();
    if let ContextAxisType::Enum { members } = reordered.axes.get_mut("jurisdiction").unwrap() {
        members.reverse();
    }
    assert_eq!(registry.register(&reordered).unwrap(), first);
    for changed in [
        ContextAxisType::String,
        ContextAxisType::Enum {
            members: vec!["EE".into()],
        },
    ] {
        let mut schema = original.schema.clone();
        schema.axes.insert("jurisdiction".into(), changed);
        assert_eq!(
            registry.register(&schema),
            Err(ContextError::SchemaConflict)
        );
        assert_eq!(registry.register(&original.schema).unwrap(), first);
    }
    let mut changed = original.schema.clone();
    changed.reference.revision = "2".into();
    assert_ne!(registry.register(&changed).unwrap(), first);
}
#[test]
fn invalid_descriptors_unknown_fields_and_duplicate_enumerations_fail() {
    for (path, value) in [
        (vec!["schema", "reference", "id"], json!("")),
        (vec!["schema", "reference", "revision"], json!("r\n")),
        (
            vec!["schema", "axes", "jurisdiction", "members"],
            json!(["EE", "EE"]),
        ),
        (
            vec!["schema", "axes", "jurisdiction", "members"],
            json!([""]),
        ),
    ] {
        let mut d = definition();
        let mut node = &mut d;
        for key in &path[..path.len() - 1] {
            node = &mut node[*key];
        }
        node[path[path.len() - 1]] = value;
        assert_eq!(decode(&d), Err(ContextError::Schema));
    }
    for field in ["unknown", "permission"] {
        let mut d = definition();
        d[field] = json!(true);
        assert_eq!(decode(&d), Err(ContextError::Syntax));
        let mut d = definition();
        d["schema"][field] = json!(true);
        assert_eq!(decode(&d), Err(ContextError::Syntax));
        let mut d = definition();
        d["schema"]["axes"]["load"][field] = json!(true);
        assert_eq!(decode(&d), Err(ContextError::Syntax));
    }
    let mut d = definition();
    d["schema"]["axes"]["load"]["kind"] = json!("float");
    assert_eq!(decode(&d), Err(ContextError::Syntax));
}
#[test]
fn budgets_precede_cloning_and_nested_input_traversal() {
    assert_eq!(
        ContextDefinition::from_json(&vec![b' '; MAX_BYTES + 1]),
        Err(ContextError::Budget)
    );
    let mut d = decode(&definition()).unwrap();
    d.values.insert("name".into(), json!("x".repeat(4097)));
    assert_eq!(d.canonical_bytes(), Err(ContextError::Assignment));
    d.values = (0..33).map(|i| (i.to_string(), Value::Null)).collect();
    assert_eq!(d.canonical_bytes(), Err(ContextError::Budget));
    let mut schema = decode(&definition()).unwrap().schema;
    schema.axes = (0..33)
        .map(|i| (i.to_string(), ContextAxisType::String))
        .collect();
    assert_eq!(schema.fingerprint(), Err(ContextError::Budget));
    schema.axes.clear();
    schema.axes.insert(
        "enum".into(),
        ContextAxisType::Enum {
            members: (0..129).map(|i| i.to_string()).collect(),
        },
    );
    assert_eq!(schema.canonical_bytes(), Err(ContextError::Budget));
    let mut d = decode(&definition()).unwrap();
    d.schema.axes = (0..32)
        .map(|i| (i.to_string(), ContextAxisType::String))
        .collect();
    d.values = (0..32)
        .map(|i| (i.to_string(), json!("x".repeat(4096))))
        .collect();
    assert_eq!(d.canonical_bytes(), Err(ContextError::Budget));
    let nested = format!("{}0{}", "[".repeat(20), "]".repeat(20));
    assert_eq!(
        ContextDefinition::from_json(nested.as_bytes()),
        Err(ContextError::Syntax)
    );
}
#[test]
fn schema_registry_has_a_finite_capacity_and_existing_labels_still_retry() {
    let mut schema = decode(&definition()).unwrap().schema;
    let mut registry = ContextSchemas::default();
    for i in 0..1024 {
        schema.reference.id = i.to_string();
        registry.register(&schema).unwrap();
    }
    registry.register(&schema).unwrap();
    schema.reference.id = "overflow".into();
    assert_eq!(registry.register(&schema), Err(ContextError::Budget));
}
#[test]
fn malformed_small_inputs_do_not_panic_and_trailing_data_is_rejected() {
    let source = serde_json::to_vec(&definition()).unwrap();
    for end in 0..source.len() {
        let _ = ContextDefinition::from_json(&source[..end]);
    }
    let mut trailing = source.clone();
    trailing.extend_from_slice(b" null");
    assert_eq!(
        ContextDefinition::from_json(&trailing),
        Err(ContextError::Syntax)
    );
    let schema =
        ContextSchema::from_json(&serde_json::to_vec(&definition()["schema"]).unwrap()).unwrap();
    assert_eq!(schema, decode(&definition()).unwrap().schema);
}

#[test]
fn embedded_schema_keeps_duplicate_checks_and_its_own_byte_budget() {
    #[derive(serde::Deserialize)]
    struct Envelope {
        schema: ContextSchema,
    }
    let schema = definition()["schema"].to_string();
    let valid = format!("{{\"schema\":{schema}}}");
    assert!(serde_json::from_str::<Envelope>(&valid)
        .unwrap()
        .schema
        .validate()
        .is_ok());
    let duplicate = valid.replace("\"id\":\"world\"", "\"id\":\"world\",\"id\":\"changed\"");
    assert!(serde_json::from_str::<Envelope>(&duplicate).is_err());
    let mut large = definition()["schema"].clone();
    large["axes"] = json!({});
    for i in 0..32 {
        large["axes"][format!("a{i}")] = json!({"kind":"enum","members":(0..128).map(|j|format!("member-{j:03}-012345678901234567890123456789")).collect::<Vec<_>>()});
    }
    let embedded = json!({"schema":large}).to_string();
    assert!(embedded.len() > MAX_BYTES);
    assert!(serde_json::from_str::<Envelope>(&embedded).is_err());
}
