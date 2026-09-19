use crate::decimal::{Decimal, DecimalError::*};
use weave_contract::{decimal, quantity};
fn d(text: &str) -> Decimal {
    text.parse().unwrap()
}

#[test]
fn decimal_canonical_encoding_preserves_values_beyond_binary64_integer_precision() {
    for (input, expected) in [
        ("+0012.3400e-1", "1.234"),
        ("-0.000e+128", "0"),
        ("100e-2", "1"),
        ("1e-18", "0.000000000000000001"),
        ("9007199254740993", "9007199254740993"),
        ("-999999999999999999", "-999999999999999999"),
    ] {
        let value = d(input);
        assert_eq!(value.to_string(), expected);
        let wire = serde_json::to_string(&value).unwrap();
        assert_eq!(wire, format!("\"{expected}\""));
        assert_eq!(serde_json::from_str::<Decimal>(&wire).unwrap(), value);
    }
    for wire in ["1.2", "\"1.20\"", "\"1e0\"", "\"-0\"", "\"+1\""] {
        assert!(serde_json::from_str::<Decimal>(wire).is_err());
    }
}
#[test]
fn decimal_parser_rejects_malformed_unbounded_and_unrepresentable_values() {
    for input in [
        "", " ", "1 ", ".1", "1.", "--1", "1e", "1e-", "NaN", "Infinity", "1_000", "１２", "1.2.3",
    ] {
        assert_eq!(input.parse::<Decimal>(), Err(Syntax), "{input}");
    }
    assert_eq!("1e129".parse::<Decimal>(), Err(InputLimit));
    assert_eq!("0".repeat(129).parse::<Decimal>(), Err(InputLimit));
    assert_eq!("1e-19".parse::<Decimal>(), Err(Scale));
    assert_eq!("1000000000000000000".parse::<Decimal>(), Err(Precision));
    assert_eq!("0.1234567890123456789".parse::<Decimal>(), Err(Precision));
}
#[test]
fn exact_arithmetic_never_implicitly_rounds() {
    assert_eq!(d("0.1").checked_add(d("0.2")), Ok(d("0.3")));
    assert_eq!(
        d("0.999999999999999999").checked_add(d("0.000000000000000001")),
        Ok(d("1"))
    );
    assert_eq!(
        d("999999999999999999").checked_sub(d("999999999999999999")),
        Ok(Decimal::ZERO)
    );
    assert_eq!(d("1.25").checked_mul(d("0.8")), Ok(d("1")));
    assert_eq!(
        d("999999999999999999").checked_mul(d("0.1")),
        Ok(d("99999999999999999.9"))
    );
    assert_eq!(
        d("999999999999999999").checked_add(d("0.1")),
        Err(Precision)
    );
    assert_eq!(d("999999999999999999").checked_mul(d("2")), Err(Precision));
    assert_eq!(d("0.000000000000000001").checked_mul(d("0.1")), Err(Scale));
}
#[test]
fn exact_division_reduces_before_testing_decimal_termination() {
    for (a, b, result) in [
        ("1", "8", "0.125"),
        ("3", "6", "0.5"),
        ("-10", "0.1", "-100"),
        ("-1", "-2", "0.5"),
        ("0.000000000000000001", "0.000000000000000001", "1"),
        ("100000000000000000", "100000000000000000", "1"),
        ("0", "3", "0"),
    ] {
        assert_eq!(d(a).checked_div(d(b)), Ok(d(result)));
    }
    assert_eq!(d("1").checked_div(d("3")), Err(NonTerminating));
    assert_eq!(d("0").checked_div(d("0")), Err(DivisionByZero));
    assert_eq!(d("1e-18").checked_div(d("2")), Err(Scale));
    assert_eq!(
        d("999999999999999999").checked_div(d("1e-18")),
        Err(Precision)
    );
}
#[test]
fn bounded_rational_oracle_covers_arithmetic_and_finite_division() {
    // Independent integer oracle over a two-decimal input grid.
    for a in -40_i128..=40 {
        for b in -40_i128..=40 {
            let x = d(&format!("{a}e-2"));
            let y = d(&format!("{b}e-2"));
            assert_eq!(x.checked_add(y), Ok(d(&format!("{}e-2", a + b))));
            assert_eq!(x.checked_sub(y), Ok(d(&format!("{}e-2", a - b))));
            assert_eq!(x.checked_mul(y), Ok(d(&format!("{}e-4", a * b))));
            if b != 0 {
                match x.checked_div(y) {
                    Ok(value) => assert_eq!(value.checked_mul(y), Ok(x)),
                    Err(NonTerminating) => {
                        let mut n = a.abs();
                        let mut q = b.abs();
                        while q != 0 {
                            (n, q) = (q, n % q);
                        }
                        let mut denominator = b.abs() / n;
                        while denominator % 2 == 0 {
                            denominator /= 2;
                        }
                        while denominator % 5 == 0 {
                            denominator /= 5;
                        }
                        assert_ne!(denominator, 1);
                    }
                    other => panic!("Unexpected division {a}/{b}: {other:?}"),
                }
            }
        }
    }
}

use quantity::{Quantity, QuantityError, RationalConversion, UnitDescriptor};
fn unit(dimension: &str, name: &str, revision: &str) -> UnitDescriptor {
    UnitDescriptor::new(dimension.into(), name.into(), revision.into()).unwrap()
}
#[test]
fn nominal_quantities_require_exact_descriptor_identity() {
    let metre = unit("length", "metre", "1");
    assert_eq!(metre.dimension_id(), "length");
    assert_eq!(metre.unit_id(), "metre");
    assert_eq!(metre.revision(), "1");
    let a = Quantity::new(d("1.2"), metre.clone());
    let b = Quantity::new(d("0.3"), metre.clone());
    assert_eq!(a.checked_add(&b).unwrap().amount(), d("1.5"));
    assert_eq!(a.checked_sub(&b).unwrap().amount(), d("0.9"));
    assert_eq!(a.checked_mul(d("2")).unwrap().amount(), d("2.4"));
    assert_eq!(a.checked_div(d("4")).unwrap().amount(), d("0.3"));
    assert_eq!(a.unit(), &metre);
    for incompatible in [
        unit("length", "centimetre", "1"),
        unit("length", "metre", "2"),
        unit("duration", "metre", "1"),
    ] {
        assert_eq!(
            a.checked_add(&Quantity::new(d("1"), incompatible)),
            Err(QuantityError::UnitMismatch)
        );
    }
    let wire = serde_json::to_string(&a).unwrap();
    assert_eq!(serde_json::from_str::<Quantity>(&wire).unwrap(), a);
}
#[test]
fn explicit_conversion_matches_exact_descriptors_and_preserves_decimal_precision() {
    let metre = unit("length", "metre", "1");
    let cm = unit("length", "centimetre", "1");
    let conversion = RationalConversion::new(metre.clone(), cm.clone(), d("100"), d("1")).unwrap();
    assert_eq!(conversion.source(), &metre);
    assert_eq!(conversion.target(), &cm);
    let output = Quantity::new(d("1.23"), metre.clone())
        .convert(&conversion)
        .unwrap();
    assert_eq!(output.amount(), d("123"));
    assert_eq!(output.unit(), &cm);
    assert_eq!(
        Quantity::new(d("1"), unit("length", "metre", "2")).convert(&conversion),
        Err(QuantityError::UnitMismatch)
    );
    let inverse = RationalConversion::new(cm, metre.clone(), d("1"), d("100")).unwrap();
    assert_eq!(
        output.convert(&inverse).unwrap(),
        Quantity::new(d("1.23"), metre)
    );
    let wire = serde_json::to_string(&conversion).unwrap();
    assert_eq!(
        serde_json::from_str::<RationalConversion>(&wire).unwrap(),
        conversion
    );
}
#[test]
fn rational_conversion_cancels_before_precision_check_and_never_rounds() {
    let m = unit("length", "m", "1");
    let n = unit("length", "n", "1");
    let third = RationalConversion::new(m.clone(), n.clone(), d("1"), d("3")).unwrap();
    assert_eq!(
        Quantity::new(d("3"), m.clone())
            .convert(&third)
            .unwrap()
            .amount(),
        d("1")
    );
    assert_eq!(
        Quantity::new(d("1"), m.clone()).convert(&third),
        Err(QuantityError::Decimal(NonTerminating))
    );
    let max = d("999999999999999999");
    let identity = RationalConversion::new(m.clone(), n.clone(), max, max).unwrap();
    assert_eq!(
        Quantity::new(max, m.clone())
            .convert(&identity)
            .unwrap()
            .amount(),
        max
    );
    for (numerator, denominator) in [("0", "1"), ("1", "0"), ("-1", "1"), ("1", "-1")] {
        assert_eq!(
            RationalConversion::new(m.clone(), n.clone(), d(numerator), d(denominator)),
            Err(QuantityError::InvalidFactor)
        );
    }
    assert_eq!(
        RationalConversion::new(m, unit("duration", "second", "1"), d("1"), d("1")),
        Err(QuantityError::DimensionMismatch)
    );
}
#[test]
fn quantity_wire_rejects_invalid_descriptors_affine_offsets_and_untyped_amounts() {
    let value = serde_json::json!({"amount":"1.2","unit":{"dimension_id":"length","unit_id":"metre","revision":"1"}});
    for bad in [serde_json::json!(1.2), serde_json::json!("1.20")] {
        let mut wire = value.clone();
        wire["amount"] = bad;
        assert!(serde_json::from_value::<Quantity>(wire).is_err());
    }
    for bad in ["".to_string(), "x".repeat(513), "bad\nunit".into()] {
        let mut wire = value.clone();
        wire["unit"]["unit_id"] = serde_json::json!(bad);
        assert!(serde_json::from_value::<Quantity>(wire).is_err());
    }
    let descriptor = value["unit"].clone();
    let offset = serde_json::json!({"from":descriptor,"to":descriptor,"numerator":"1","denominator":"1","offset":"273.15"});
    assert!(serde_json::from_value::<RationalConversion>(offset).is_err());
}
#[test]
fn rational_scaling_matches_independent_small_integer_oracle() {
    for amount in -12_i128..=12 {
        for numerator in 1..=12_i128 {
            for denominator in 1..=12_i128 {
                let expected = d(&format!("{}e-2", amount * numerator))
                    .checked_div(d(&denominator.to_string()));
                let actual = d(&format!("{amount}e-2"))
                    .checked_mul_ratio(d(&numerator.to_string()), d(&denominator.to_string()));
                assert_eq!(actual, expected, "{amount} * {numerator} / {denominator}");
            }
        }
    }
}

#[test]
fn numeric_schema_extends_without_changing_existing_descriptor_encoding() {
    use weave_contract::{PropertySchema, ScalarType};
    for (kind, name) in [
        (ScalarType::String, "string"),
        (ScalarType::Integer, "integer"),
        (ScalarType::Float, "float"),
        (ScalarType::Boolean, "boolean"),
    ] {
        let shape = PropertySchema {
            value_type: kind,
            required: true,
            nullable: false,
        };
        assert_eq!(
            serde_json::to_string(&shape).unwrap(),
            format!("{{\"value_type\":\"{name}\",\"required\":true,\"nullable\":false}}")
        );
    }
    let original = r#"{"id":"Old","revision":"1","nodes":{"N":{"properties":{"name":{"value_type":"string","required":true,"nullable":false}},"space_id":null,"allow_extra_properties":false}},"edges":{}}"#;
    let schema: weave_contract::GraphSchema = serde_json::from_str(original).unwrap();
    assert_eq!(serde_json::to_string(&schema).unwrap(), original);
}
#[test]
fn numeric_schema_requires_canonical_amounts_and_exact_unit_descriptors() {
    use serde_json::json;
    let unit = json!({"dimension_id":"length","unit_id":"metre","revision":"1"});
    let value = json!({"nodes":[{"id":"n","entity_id":"n","space_id":"s","type_id":"Measure","properties":{"exact":"9007199254740993","length":{"amount":"1.2","unit":unit}}}],"schema":{"id":"Measurements","revision":"1","nodes":{"Measure":{"properties":{"exact":{"value_type":"decimal","required":true},"length":{"value_type":{"quantity":unit},"required":true}}}},"edges":{}}});
    let graph: weave_contract::GraphData = serde_json::from_value(value.clone()).unwrap();
    assert!(weave_contract::validate_schema_graph(&graph).is_empty());
    for amount in [
        json!(9007199254740993_u64),
        json!("1.20"),
        json!("1e0"),
        json!("1e-19"),
    ] {
        let mut invalid = value.clone();
        invalid["nodes"][0]["properties"]["exact"] = amount;
        let graph = serde_json::from_value(invalid).unwrap();
        assert!(weave_contract::validate_schema_graph(&graph)
            .iter()
            .any(|d| d.code == "E_SCHEMA_PROPERTY_TYPE"));
    }
    let mut invalid = value;
    invalid["nodes"][0]["properties"]["length"]["unit"]["revision"] = json!("2");
    let graph = serde_json::from_value(invalid).unwrap();
    assert!(weave_contract::validate_schema_graph(&graph)
        .iter()
        .any(|d| d.code == "E_SCHEMA_PROPERTY_TYPE"));
}
