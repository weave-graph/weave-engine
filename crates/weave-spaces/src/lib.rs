//! Pure calculations over host-authorized evidence. This crate does not authenticate
//! callers or read graph storage. Hosts must resolve and authorize every supplied
//! descriptor, sample and transform before calling, and recheck before publication.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use weave_contract::Interval;

const MAX_DIMENSIONS: usize = 4096;
const MAX_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub &'static str);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

/// Unlike legacy reader lists, an empty Principals set means nobody, never public.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "principals",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Visibility {
    Public,
    Principals(BTreeSet<String>),
}
impl Visibility {
    pub fn permits(&self, principal: &str) -> bool {
        match self {
            Self::Public => true,
            Self::Principals(p) => p.contains(principal),
        }
    }
    fn intersection(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Public, value) | (value, Self::Public) => value.clone(),
            (Self::Principals(a), Self::Principals(b)) => {
                Self::Principals(a.intersection(b).cloned().collect())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub graph_id: String,
    pub revision: String,
    pub object_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Evidence<T> {
    pub value: T,
    pub valid_time: Interval,
    pub visibility: Visibility,
    pub sources: BTreeSet<Source>,
}
impl<T: Serialize> Evidence<T> {
    fn validate(&self, principal: &str, at: i64) -> Result<()> {
        // Authorization precedes data-dependent validation and size diagnostics.
        if !self.visibility.permits(principal) {
            return Err(Error("E_UNAVAILABLE"));
        }
        if !self.valid_time.valid() || !self.valid_time.contains(at) {
            return Err(Error("E_TIME"));
        }
        if self.sources.is_empty()
            || self.sources.len() > 256
            || self
                .sources
                .iter()
                .any(|s| s.graph_id.is_empty() || s.revision.is_empty() || s.object_id.is_empty())
        {
            return Err(Error("E_SOURCE"));
        }
        bounded(self)?;
        Ok(())
    }
}

// Streaming size check avoids materializing a second copy before enforcing limits.
fn bounded(value: &impl Serialize) -> Result<()> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            if self.0 > MAX_BYTES {
                return Err(std::io::Error::other("budget"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter(0), value).map_err(|_| Error("E_BUDGET"))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Metre,
    Centimetre,
    Millimetre,
}
impl Unit {
    fn metres(self) -> f64 {
        match self {
            Self::Metre => 1.0,
            Self::Centimetre => 0.01,
            Self::Millimetre => 0.001,
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Euclidean,
    CosineDistance,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Geometry {
    Physical3d {
        frame: String,
        unit: Unit,
    },
    Embedding {
        encoder: String,
        preprocessing: String,
        dimensions: usize,
        metric: Metric,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Space {
    pub id: String,
    pub revision: String,
    pub geometry: Geometry,
}
impl Space {
    /// Validate this geometry descriptor without reading data or asserting authority.
    /// Callers remain responsible for input allocation and transport byte bounds.
    pub fn validate(&self) -> Result<()> {
        self.dimensions().map(|_| ())
    }
    fn dimensions(&self) -> Result<usize> {
        if self.id.is_empty() || self.revision.is_empty() {
            return Err(Error("E_SPACE"));
        }
        match &self.geometry {
            Geometry::Physical3d { frame, .. } if !frame.is_empty() => Ok(3),
            Geometry::Embedding {
                encoder,
                preprocessing,
                dimensions,
                ..
            } if !encoder.is_empty()
                && !preprocessing.is_empty()
                && (1..=MAX_DIMENSIONS).contains(dimensions) =>
            {
                Ok(*dimensions)
            }
            _ => Err(Error("E_SPACE")),
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VectorRole {
    Position,
    Direction,
    Embedding,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Coordinates {
    pub space: Space,
    pub role: VectorRole,
    pub values: Vec<f64>,
}
impl Coordinates {
    /// Validate dimensions, finite components and role compatibility only.
    /// This does not authenticate the coordinates or construct trusted evidence.
    pub fn validate(&self) -> Result<()> {
        if self.values.len() != self.space.dimensions()?
            || self.values.iter().any(|v| !v.is_finite())
        {
            return Err(Error("E_COORDINATES"));
        }
        match (&self.space.geometry, self.role) {
            (Geometry::Physical3d { .. }, VectorRole::Position | VectorRole::Direction)
            | (Geometry::Embedding { .. }, VectorRole::Embedding) => Ok(()),
            _ => Err(Error("E_VECTOR_ROLE")),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Measurement {
    pub value: f64,
    pub unit: Option<Unit>,
    pub metric: Metric,
    pub space: Space,
}

fn derive<A, B, T>(value: T, a: &Evidence<A>, b: &Evidence<B>) -> Evidence<T> {
    Evidence {
        value,
        valid_time: Interval {
            start: a.valid_time.start.max(b.valid_time.start),
            end: match (a.valid_time.end, b.valid_time.end) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
        },
        visibility: a.visibility.intersection(&b.visibility),
        sources: a.sources.union(&b.sources).cloned().collect(),
    }
}

/// Exact descriptor equality is required, including encoder/preprocessing/revision.
/// Direction annotations are deliberately excluded from point distance.
pub fn distance(
    principal: &str,
    at: i64,
    a: &Evidence<Coordinates>,
    b: &Evidence<Coordinates>,
) -> Result<Evidence<Measurement>> {
    // Check both disclosures first so unauthorized descriptors do not affect errors.
    if !a.visibility.permits(principal) || !b.visibility.permits(principal) {
        return Err(Error("E_UNAVAILABLE"));
    }
    a.validate(principal, at)?;
    b.validate(principal, at)?;
    a.value.validate()?;
    b.value.validate()?;
    if a.value.space != b.value.space {
        return Err(Error("E_INCOMPATIBLE_SPACE"));
    }
    if a.value.role != b.value.role || a.value.role == VectorRole::Direction {
        return Err(Error("E_VECTOR_ROLE"));
    }
    let (metric, unit) = match a.value.space.geometry {
        Geometry::Physical3d { unit, .. } => (Metric::Euclidean, Some(unit)),
        Geometry::Embedding { metric, .. } => (metric, None),
    };
    let x = &a.value.values;
    let y = &b.value.values;
    let value = match metric {
        Metric::Euclidean => x.iter().zip(y).fold(0.0_f64, |n, (x, y)| n.hypot(x - y)),
        Metric::CosineDistance => {
            // Scale before normalization: even finite values can have overflowing norm.
            let xs = x.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
            let ys = y.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
            if xs == 0.0 || ys == 0.0 {
                return Err(Error("E_ZERO_VECTOR"));
            }
            let xn = x.iter().fold(0.0_f64, |n, v| n.hypot(v / xs));
            let yn = y.iter().fold(0.0_f64, |n, v| n.hypot(v / ys));
            let cosine: f64 = x
                .iter()
                .zip(y)
                .map(|(x, y)| (x / xs / xn) * (y / ys / yn))
                .sum();
            1.0 - cosine.clamp(-1.0, 1.0)
        }
    };
    if !value.is_finite() {
        return Err(Error("E_NUMERIC_RANGE"));
    }
    let output = derive(
        Measurement {
            value,
            unit,
            metric,
            space: a.value.space.clone(),
        },
        a,
        b,
    );
    if output.sources.len() > 256 {
        return Err(Error("E_BUDGET"));
    }
    bounded(&output)?;
    Ok(output)
}

/// Explicit directed rigid transform. Rotation is orthonormal and right-handed.
/// Translation uses target units; direction vectors are rotated/scaled, not translated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RigidTransform {
    pub from: Space,
    pub to: Space,
    pub rotation: [[f64; 3]; 3],
    pub translation: [f64; 3],
}
impl RigidTransform {
    fn validate(&self) -> Result<(Unit, Unit)> {
        self.from.dimensions()?;
        self.to.dimensions()?;
        let (Geometry::Physical3d { unit: from, .. }, Geometry::Physical3d { unit: to, .. }) =
            (&self.from.geometry, &self.to.geometry)
        else {
            return Err(Error("E_TRANSFORM_DOMAIN"));
        };
        if self
            .rotation
            .iter()
            .flatten()
            .chain(self.translation.iter())
            .any(|n| !n.is_finite())
        {
            return Err(Error("E_TRANSFORM"));
        }
        let r = &self.rotation;
        for i in 0..3 {
            for j in 0..3 {
                let dot: f64 = (0..3).map(|k| r[i][k] * r[j][k]).sum();
                if (dot - if i == j { 1.0 } else { 0.0 }).abs() > 1e-10 {
                    return Err(Error("E_TRANSFORM"));
                }
            }
        }
        let det = r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
            - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
            + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0]);
        if (det - 1.0).abs() > 1e-10 {
            return Err(Error("E_TRANSFORM"));
        }
        Ok((*from, *to))
    }
}
pub fn transform(
    principal: &str,
    at: i64,
    input: &Evidence<Coordinates>,
    mapping: &Evidence<RigidTransform>,
) -> Result<Evidence<Coordinates>> {
    if !input.visibility.permits(principal) || !mapping.visibility.permits(principal) {
        return Err(Error("E_UNAVAILABLE"));
    }
    input.validate(principal, at)?;
    mapping.validate(principal, at)?;
    input.value.validate()?;
    let (from, to) = mapping.value.validate()?;
    if input.value.space != mapping.value.from {
        return Err(Error("E_TRANSFORM_DOMAIN"));
    }
    let values = (0..3)
        .map(|i| {
            let rotated: f64 = (0..3)
                .map(|j| mapping.value.rotation[i][j] * input.value.values[j])
                .sum();
            rotated * (from.metres() / to.metres())
                + if input.value.role == VectorRole::Position {
                    mapping.value.translation[i]
                } else {
                    0.0
                }
        })
        .collect();
    let output = derive(
        Coordinates {
            space: mapping.value.to.clone(),
            role: input.value.role,
            values,
        },
        input,
        mapping,
    );
    output.value.validate()?;
    if output.sources.len() > 256 {
        return Err(Error("E_BUDGET"));
    }
    bounded(&output)?;
    Ok(output)
}

/// Display-only type deliberately cannot be supplied to `distance`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NavigationProjection {
    pub source_space: Space,
    pub projection_revision: String,
    pub algorithm: String,
    pub axes: [usize; 3],
    pub values: [f64; 3],
    pub approximate: bool,
}
pub fn project_axes(
    principal: &str,
    at: i64,
    input: &Evidence<Coordinates>,
    axes: [usize; 3],
    projection_revision: &str,
) -> Result<Evidence<NavigationProjection>> {
    input.validate(principal, at)?;
    input.value.validate()?;
    if projection_revision.len() > 1024 {
        return Err(Error("E_BUDGET"));
    }
    if input.value.role != VectorRole::Embedding
        || projection_revision.is_empty()
        || axes.iter().any(|i| *i >= input.value.values.len())
        || axes.iter().collect::<BTreeSet<_>>().len() != 3
    {
        return Err(Error("E_PROJECTION"));
    }
    let output = Evidence {
        value: NavigationProjection {
            source_space: input.value.space.clone(),
            projection_revision: projection_revision.into(),
            algorithm: "axis-selection/1".into(),
            axes,
            values: axes.map(|i| input.value.values[i]),
            approximate: true,
        },
        valid_time: input.valid_time.clone(),
        visibility: input.visibility.clone(),
        sources: input.sources.clone(),
    };
    bounded(&output)?;
    Ok(output)
}
