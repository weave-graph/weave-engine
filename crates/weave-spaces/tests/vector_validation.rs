//! Public pure validation requires no Evidence wrapper or fabricated authority.
use weave_spaces::{Coordinates, Geometry, Metric, Space, Unit, VectorRole};
fn physical() -> Space {
    Space {
        id: "building".into(),
        revision: "1".into(),
        geometry: Geometry::Physical3d {
            frame: "A".into(),
            unit: Unit::Metre,
        },
    }
}
#[test]
fn pure_descriptor_and_coordinate_validation_preserves_wire() {
    let space = physical();
    assert!(space.validate().is_ok());
    let value = Coordinates {
        space,
        role: VectorRole::Position,
        values: vec![0.0, -0.0, f64::MAX],
    };
    let before = serde_json::to_value(&value).unwrap();
    assert!(value.validate().is_ok());
    assert_eq!(serde_json::to_value(&value).unwrap(), before);
    let mut bad = value.clone();
    bad.values[2] = f64::INFINITY;
    assert_eq!(bad.validate().unwrap_err().0, "E_COORDINATES");
    bad.values[2] = f64::NAN;
    assert_eq!(bad.validate().unwrap_err().0, "E_COORDINATES");
    bad.values = vec![0.0, 1.0];
    assert_eq!(bad.validate().unwrap_err().0, "E_COORDINATES");
    bad.values = vec![0.0; 3];
    bad.role = VectorRole::Embedding;
    assert_eq!(bad.validate().unwrap_err().0, "E_VECTOR_ROLE");
}
#[test]
fn pure_descriptor_validation_rejects_invalid_shapes_and_roles() {
    for dimensions in [0, 4097] {
        let space = Space {
            id: "embedding".into(),
            revision: "1".into(),
            geometry: Geometry::Embedding {
                encoder: "v1".into(),
                preprocessing: "tokens".into(),
                dimensions,
                metric: Metric::CosineDistance,
            },
        };
        assert_eq!(space.validate().unwrap_err().0, "E_SPACE");
    }
    let mut space = physical();
    space.id.clear();
    assert_eq!(space.validate().unwrap_err().0, "E_SPACE");
    let mut space = physical();
    space.revision.clear();
    assert_eq!(space.validate().unwrap_err().0, "E_SPACE");
    let space = Space {
        id: "embedding".into(),
        revision: "1".into(),
        geometry: Geometry::Embedding {
            encoder: "v1".into(),
            preprocessing: "tokens".into(),
            dimensions: 4096,
            metric: Metric::Euclidean,
        },
    };
    assert!(space.validate().is_ok());
    let values = Coordinates {
        space,
        role: VectorRole::Embedding,
        values: vec![0.0; 4096],
    };
    assert!(values.validate().is_ok());
}
