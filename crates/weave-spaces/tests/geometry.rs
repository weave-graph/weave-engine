use std::collections::BTreeSet;
use weave_contract::Interval;
use weave_spaces::*;

fn evidence<T>(value: T, id: &str) -> Evidence<T> {
    Evidence {
        value,
        valid_time: Interval {
            start: 0,
            end: Some(20),
        },
        visibility: Visibility::Public,
        sources: [Source {
            graph_id: "survey".into(),
            revision: "revision-1".into(),
            object_id: id.into(),
        }]
        .into(),
    }
}
fn physical(unit: Unit, frame: &str) -> Space {
    Space {
        id: "site".into(),
        revision: "v1".into(),
        geometry: Geometry::Physical3d {
            frame: frame.into(),
            unit,
        },
    }
}
fn point(values: Vec<f64>) -> Evidence<Coordinates> {
    evidence(
        Coordinates {
            space: physical(Unit::Metre, "survey-origin"),
            role: VectorRole::Position,
            values,
        },
        "point-a",
    )
}
fn embedding(values: Vec<f64>, metric: Metric) -> Evidence<Coordinates> {
    evidence(
        Coordinates {
            space: Space {
                id: "semantic".into(),
                revision: "encoder-v1".into(),
                geometry: Geometry::Embedding {
                    encoder: "model-v1".into(),
                    preprocessing: "tokens-v1".into(),
                    dimensions: values.len(),
                    metric,
                },
            },
            role: VectorRole::Embedding,
            values,
        },
        "embedding-a",
    )
}

#[test]
fn distances_require_exact_descriptors_and_preserve_time_and_all_restrictions() {
    let mut a = point(vec![0., 0., 0.]);
    a.visibility = Visibility::Principals(["alice".into(), "bob".into()].into());
    let mut b = point(vec![3., 4., 0.]);
    b.sources = evidence((), "point-b").sources;
    b.valid_time = Interval {
        start: 5,
        end: Some(10),
    };
    b.visibility = Visibility::Principals(["alice".into(), "charlie".into()].into());
    let result = distance("alice", 5, &a, &b).unwrap();
    assert_eq!(result.value.value, 5.);
    assert_eq!(result.value.unit, Some(Unit::Metre));
    assert_eq!(result.valid_time, b.valid_time);
    assert_eq!(
        result.visibility,
        Visibility::Principals(["alice".into()].into())
    );
    assert_eq!(result.sources.len(), 2);
    assert_eq!(distance("bob", 5, &a, &b).unwrap_err().0, "E_UNAVAILABLE");
    assert_eq!(distance("alice", 10, &a, &b).unwrap_err().0, "E_TIME");
    b.value.space = physical(Unit::Centimetre, "survey-origin");
    assert_eq!(
        distance("alice", 5, &a, &b).unwrap_err().0,
        "E_INCOMPATIBLE_SPACE"
    );
    b.value.space = physical(Unit::Metre, "other-origin");
    assert_eq!(
        distance("alice", 5, &a, &b).unwrap_err().0,
        "E_INCOMPATIBLE_SPACE"
    );
}

#[test]
fn equal_length_embeddings_do_not_establish_compatibility_and_zero_cosine_is_undefined() {
    let a = embedding(vec![1., 0., 0.], Metric::CosineDistance);
    let mut b = embedding(vec![0., 1., 0.], Metric::CosineDistance);
    assert_eq!(distance("alice", 0, &a, &b).unwrap().value.value, 1.);
    b.value.space.revision = "encoder-v2".into();
    assert_eq!(
        distance("alice", 0, &a, &b).unwrap_err().0,
        "E_INCOMPATIBLE_SPACE"
    );
    b.value.space = a.value.space.clone();
    if let Geometry::Embedding { preprocessing, .. } = &mut b.value.space.geometry {
        *preprocessing = "different-tokenizer".into();
    }
    assert_eq!(
        distance("alice", 0, &a, &b).unwrap_err().0,
        "E_INCOMPATIBLE_SPACE"
    );
    b.value.space = a.value.space.clone();
    b.value.values.fill(0.);
    assert_eq!(distance("alice", 0, &a, &b).unwrap_err().0, "E_ZERO_VECTOR");
    let huge = embedding(vec![1e308, 1e308, 1e308], Metric::CosineDistance);
    assert!(
        distance("alice", 0, &huge, &huge)
            .unwrap()
            .value
            .value
            .abs()
            < 1e-14
    );
}

#[test]
fn directed_transform_converts_units_and_never_translates_a_direction() {
    let mut input = point(vec![1., 2., 3.]);
    let mut mapping = evidence(
        RigidTransform {
            from: input.value.space.clone(),
            to: physical(Unit::Centimetre, "other-frame"),
            rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
            translation: [10., 20., 30.],
        },
        "calibration-v1",
    );
    mapping.visibility = Visibility::Principals(["alice".into()].into());
    let result = transform("alice", 0, &input, &mapping).unwrap();
    assert_eq!(result.value.values, vec![-190., 120., 330.]);
    assert_eq!(result.visibility, mapping.visibility);
    assert_eq!(result.sources.len(), 2);
    input.value.role = VectorRole::Direction;
    assert_eq!(
        transform("alice", 0, &input, &mapping)
            .unwrap()
            .value
            .values,
        vec![-200., 100., 300.]
    );
    assert_eq!(
        distance("alice", 0, &input, &input).unwrap_err().0,
        "E_VECTOR_ROLE"
    );
    assert_eq!(
        transform("bob", 0, &input, &mapping).unwrap_err().0,
        "E_UNAVAILABLE"
    );
    let mut reverse = mapping.clone();
    std::mem::swap(&mut reverse.value.from, &mut reverse.value.to);
    assert_eq!(
        transform("alice", 0, &input, &reverse).unwrap_err().0,
        "E_TRANSFORM_DOMAIN"
    );
    mapping.value.rotation[0][0] = 1.;
    assert_eq!(
        transform("alice", 0, &input, &mapping).unwrap_err().0,
        "E_TRANSFORM"
    );
}

#[test]
fn projection_retains_source_space_and_is_explicitly_approximate() {
    let mut input = embedding(vec![1., 2., 3., 4.], Metric::Euclidean);
    input.visibility = Visibility::Principals(["alice".into()].into());
    let result = project_axes("alice", 0, &input, [3, 1, 0], "projection-v1").unwrap();
    assert_eq!(result.value.values, [4., 2., 1.]);
    assert_eq!(result.value.source_space, input.value.space);
    assert_eq!(result.sources, input.sources);
    assert_eq!(result.visibility, input.visibility);
    assert!(result.value.approximate);
    // Identical display coordinates can hide arbitrarily different original values.
    let mut far = input.clone();
    far.value.values[3] += 100.;
    assert_eq!(
        project_axes("alice", 0, &input, [0, 1, 2], "p1")
            .unwrap()
            .value
            .values,
        project_axes("alice", 0, &far, [0, 1, 2], "p1")
            .unwrap()
            .value
            .values
    );
    assert_eq!(
        distance("alice", 0, &input, &far).unwrap().value.value,
        100.
    );
    assert_eq!(
        project_axes("alice", 0, &input, [0, 0, 1], "p1")
            .unwrap_err()
            .0,
        "E_PROJECTION"
    );
    assert_eq!(
        project_axes("bob", 0, &input, [0, 1, 9], "p1")
            .unwrap_err()
            .0,
        "E_UNAVAILABLE"
    );
}

#[test]
fn malformed_and_unbounded_inputs_fail_without_nan_or_implicit_public_release() {
    let a = point(vec![0., 0., 0.]);
    let mut b = a.clone();
    b.value.values[0] = f64::NAN;
    assert_eq!(distance("alice", 0, &a, &b).unwrap_err().0, "E_COORDINATES");
    b = point(vec![1e308, 0., 0.]);
    let opposite = point(vec![-1e308, 0., 0.]);
    assert_eq!(
        distance("alice", 0, &opposite, &b).unwrap_err().0,
        "E_NUMERIC_RANGE"
    );
    b.visibility = Visibility::Principals(BTreeSet::new());
    assert_eq!(distance("alice", 0, &a, &b).unwrap_err().0, "E_UNAVAILABLE");
    b = embedding(vec![0.; 4097], Metric::Euclidean);
    assert_eq!(distance("alice", 0, &b, &b).unwrap_err().0, "E_SPACE");
    b = a.clone();
    b.sources = [Source {
        graph_id: "x".repeat(1_048_577),
        revision: "v1".into(),
        object_id: "n1".into(),
    }]
    .into();
    assert_eq!(distance("alice", 0, &a, &b).unwrap_err().0, "E_BUDGET");
    b.visibility = Visibility::Principals(["bob".into()].into());
    assert_eq!(distance("alice", 0, &a, &b).unwrap_err().0, "E_UNAVAILABLE");
}
