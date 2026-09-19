//! Geometry operators select graph assertions; plans never provide trusted Evidence wrappers.
use crate::GraphExpression;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryOperand {
    pub input: Box<GraphExpression>,
    pub assertion_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GeometryOperation {
    Distance {
        left: GeometryOperand,
        right: GeometryOperand,
    },
    Transform {
        input: GeometryOperand,
        mapping: GeometryOperand,
    },
    ProjectAxes {
        input: GeometryOperand,
        axes: [usize; 3],
        projection_revision: String,
    },
}
impl GeometryOperation {
    pub fn inputs(&self) -> Vec<&GraphExpression> {
        match self {
            Self::Distance { left, right } => vec![&left.input, &right.input],
            Self::Transform { input, mapping } => vec![&input.input, &mapping.input],
            Self::ProjectAxes { input, .. } => vec![&input.input],
        }
    }
    pub fn inputs_mut(&mut self) -> Vec<&mut GraphExpression> {
        match self {
            Self::Distance { left, right } => vec![&mut left.input, &mut right.input],
            Self::Transform { input, mapping } => vec![&mut input.input, &mut mapping.input],
            Self::ProjectAxes { input, .. } => vec![&mut input.input],
        }
    }
}
