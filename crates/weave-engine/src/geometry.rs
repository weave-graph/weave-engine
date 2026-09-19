//! Geometry from authorized graph assertions; raw wire payloads never mint Evidence authority.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use weave_spaces::{
    Coordinates, Evidence, Measurement, NavigationProjection, RigidTransform, Source, Visibility,
};
const PROPERTY: &str = "weave.geometry";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Payload {
    Coordinates(Coordinates),
    RigidTransform(RigidTransform),
    Measurement(Measurement),
    NavigationProjection(NavigationProjection),
}
struct Operand {
    value: QueryResult,
    edge: Edge,
    payload: Payload,
    origins: Vec<AssertionRef>,
}
fn failure(error: weave_spaces::Error) -> Error {
    err(
        error.0,
        "geometry evidence or operation is unavailable or incompatible",
    )
}
impl Engine {
    fn geometry_operand(
        &self,
        operand: &GeometryOperand,
        at: i64,
        values: &BTreeMap<String, QueryResult>,
        host: &HostContext,
        depth: u32,
        budget: &mut usize,
    ) -> Result<Operand> {
        if !valid_id(&operand.assertion_id) {
            return Err(err("E_ID", "geometry assertion ID must be bounded"));
        }
        let value = self.expression(&operand.input, values, host, depth, budget)?;
        let edge = value
            .graph
            .edges
            .iter()
            .find(|e| {
                e.id == operand.assertion_id
                    && e.polarity == Polarity::Positive
                    && e.valid_time.contains(at)
            })
            .cloned()
            .ok_or_else(|| err("E_GEOMETRY_UNAVAILABLE", "geometry assertion unavailable"))?;
        context::ensure_consumable(
            value.selected_context.as_ref(),
            edge.assertion_context.as_ref(),
        )
        .map_err(|d| err(&d.code, &d.message))?;
        let from = value
            .graph
            .nodes
            .iter()
            .find(|n| n.id == edge.from)
            .ok_or_else(|| err("E_GEOMETRY_UNAVAILABLE", "geometry endpoint unavailable"))?;
        let to = value
            .graph
            .nodes
            .iter()
            .find(|n| n.id == edge.to)
            .ok_or_else(|| err("E_GEOMETRY_UNAVAILABLE", "geometry endpoint unavailable"))?;
        for node in [from, to] {
            if let Some(scope) = &node.context_scope {
                context::compatible_context(value.selected_context.as_ref(), Some(scope))
                    .map_err(|d| err(&d.code, &d.message))?;
            }
        }
        let raw = edge.assertion_properties.get(PROPERTY).ok_or_else(|| {
            err(
                "E_GEOMETRY_VALUE",
                "geometry payload missing from source assertion",
            )
        })?;
        json_size(raw, 1024 * 1024)?;
        let payload: Payload = serde_json::from_value(raw.clone())
            .map_err(|_| err("E_GEOMETRY_VALUE", "invalid typed geometry payload"))?;
        let anchored = match &payload {
            Payload::Coordinates(point) => point.space.id == from.space_id,
            Payload::RigidTransform(mapping) => {
                mapping.from.id == from.space_id && mapping.to.id == to.space_id
            }
            // Result kinds cannot be passed off as coordinate assertions.
            _ => true,
        };
        if !anchored {
            return Err(err(
                "E_GEOMETRY_SPACE",
                "geometry descriptor does not match its manifestation space",
            ));
        }
        let origins = value
            .edge_origins
            .get(&edge.id)
            .filter(|p| !p.is_empty())
            .cloned()
            .ok_or_else(|| {
                err(
                    "E_PROVENANCE",
                    "geometry assertion lacks engine-derived origins",
                )
            })?;
        Ok(Operand {
            value,
            edge,
            payload,
            origins,
        })
    }
    pub(crate) fn geometry(
        &self,
        operation: &GeometryOperation,
        at: i64,
        values: &BTreeMap<String, QueryResult>,
        host: &HostContext,
        depth: u32,
        budget: &mut usize,
    ) -> Result<QueryResult> {
        let (first, second, output, window, operator, parameters) = match operation {
            GeometryOperation::Distance { left, right } => {
                let left = self.geometry_operand(left, at, values, host, depth, budget)?;
                let right = self.geometry_operand(right, at, values, host, depth, budget)?;
                context::compatible_context(
                    left.value.selected_context.as_ref(),
                    right.value.selected_context.as_ref(),
                )
                .map_err(|d| err(&d.code, &d.message))?;
                let (Payload::Coordinates(a), Payload::Coordinates(b)) =
                    (&left.payload, &right.payload)
                else {
                    return Err(err("E_GEOMETRY_KIND","distance requires coordinate claims; display projections and measurements are not metric inputs"));
                };
                let output = weave_spaces::distance(
                    &host.principal,
                    at,
                    &evidence(a.clone(), &left, host),
                    &evidence(b.clone(), &right, host),
                )
                .map_err(failure)?;
                (
                    left,
                    Some(right),
                    Payload::Measurement(output.value),
                    output.valid_time,
                    "weave:geometry-distance:v1",
                    BTreeMap::from([("valid_at".into(), json!(at))]),
                )
            }
            GeometryOperation::Transform { input, mapping } => {
                let input = self.geometry_operand(input, at, values, host, depth, budget)?;
                let mapping = self.geometry_operand(mapping, at, values, host, depth, budget)?;
                context::compatible_context(
                    input.value.selected_context.as_ref(),
                    mapping.value.selected_context.as_ref(),
                )
                .map_err(|d| err(&d.code, &d.message))?;
                let (Payload::Coordinates(a), Payload::RigidTransform(b)) =
                    (&input.payload, &mapping.payload)
                else {
                    return Err(err(
                        "E_GEOMETRY_KIND",
                        "transform requires coordinates and an explicit rigid mapping assertion",
                    ));
                };
                let output = weave_spaces::transform(
                    &host.principal,
                    at,
                    &evidence(a.clone(), &input, host),
                    &evidence(b.clone(), &mapping, host),
                )
                .map_err(failure)?;
                (
                    input,
                    Some(mapping),
                    Payload::Coordinates(output.value),
                    output.valid_time,
                    "weave:geometry-transform:v1",
                    BTreeMap::from([("valid_at".into(), json!(at))]),
                )
            }
            GeometryOperation::ProjectAxes {
                input,
                axes,
                projection_revision,
            } => {
                if !valid_id(projection_revision) {
                    return Err(err("E_ID", "projection revision must be bounded"));
                }
                let input = self.geometry_operand(input, at, values, host, depth, budget)?;
                let Payload::Coordinates(a) = &input.payload else {
                    return Err(err(
                        "E_GEOMETRY_KIND",
                        "projection requires original embedding coordinates",
                    ));
                };
                let output = weave_spaces::project_axes(
                    &host.principal,
                    at,
                    &evidence(a.clone(), &input, host),
                    *axes,
                    projection_revision,
                )
                .map_err(failure)?;
                (
                    input,
                    None,
                    Payload::NavigationProjection(output.value),
                    output.valid_time,
                    "weave:geometry-project-axes:v1",
                    BTreeMap::from([
                        ("valid_at".into(), json!(at)),
                        ("axes".into(), json!(axes)),
                        ("projection_revision".into(), json!(projection_revision)),
                    ]),
                )
            }
        };
        geometry_result(first, second, output, window, operator, parameters, host)
    }
}
fn evidence<T>(value: T, operand: &Operand, host: &HostContext) -> Evidence<T> {
    Evidence {
        value,
        valid_time: operand.edge.valid_time.clone(),
        visibility: Visibility::Principals(BTreeSet::from([host.principal.clone()])),
        sources: operand
            .origins
            .iter()
            .map(|p| Source {
                graph_id: p.graph_id.clone(),
                revision: p.revision.clone(),
                object_id: p.assertion_id.clone(),
            })
            .collect(),
    }
}
fn geometry_result(
    first: Operand,
    second: Option<Operand>,
    payload: Payload,
    window: Interval,
    operator: &str,
    mut parameters: BTreeMap<String, Value>,
    host: &HostContext,
) -> Result<QueryResult> {
    let context = match &second {
        Some(other) => context::compatible_context(
            first.value.selected_context.as_ref(),
            other.value.selected_context.as_ref(),
        )
        .map_err(|d| err(&d.code, &d.message))?,
        None => first.value.selected_context.clone(),
    };
    parameters.insert("context".into(), json!(context.clone().unwrap_or_default()));
    parameters.insert(
        "reproducibility".into(),
        json!("finite-binary64-tolerance; not bitwise-cross-platform"),
    );
    let ctx = algebra_context(host);
    let derivations = if let Some(other) = &second {
        algebra::combine_derivations(
            &first.edge,
            &first.origins,
            &other.edge,
            &other.origins,
            operator,
            parameters.clone(),
            &[],
            &ctx,
        )
        .map_err(|d| err(&d.code, &d.message))?
    } else {
        let parents = algebra::edge_alternatives(&first.edge, &first.origins)
            .map_err(|d| err(&d.code, &d.message))?;
        let mut out = Vec::new();
        let mut size = 0;
        for parent in parents {
            let mut fields = parameters.clone();
            fields.insert("inputs".into(), json!([parent]));
            let mut snapshots = Vec::new();
            for p in &parent.premises {
                let r = GraphRef {
                    graph_id: p.graph_id.clone(),
                    revision: p.revision.clone(),
                };
                if !snapshots.contains(&r) {
                    snapshots.push(r);
                }
            }
            for p in &parent.node_premises {
                let pin = GraphRef {
                    graph_id: p.graph_id.clone(),
                    revision: p.revision.clone(),
                };
                if !snapshots.contains(&pin) {
                    snapshots.push(pin);
                }
            }
            let group = Derivation {
                node_premises: parent.node_premises.clone(),
                operator: operator.into(),
                premises: parent.premises,
                parameters: fields,
                input_snapshots: snapshots,
            };
            size += json_size(&group, MATERIALIZED_LIMIT.saturating_sub(size))?;
            out.push(group);
        }
        out
    };
    let mut origins = Vec::new();
    for p in derivations.iter().flat_map(|d| &d.premises) {
        if !origins.contains(p) {
            origins.push(p.clone());
        }
    }
    let node_gates = derivations
        .iter()
        .flat_map(|d| d.node_premises.iter().cloned())
        .collect::<Vec<_>>();
    let mut node_gates = GraphInfluence {
        assertions: origins.clone(),
        nodes: node_gates,
    };
    weave_contract::influence::canonicalize(&mut node_gates);
    weave_contract::influence::validate(&node_gates).map_err(|d| err(&d.code, &d.message))?;
    let encoded = serde_json::to_value(&payload)?;
    let identity = format!(
        "geometry:{:x}",
        Sha256::digest(serde_json::to_vec(&(
            operator,
            &encoded,
            &derivations,
            &context
        ))?)
    );
    let (kind, approximate, scalar, space) = match &payload {
        Payload::Coordinates(p) => ("coordinates", false, Value::Null, p.space.id.clone()),
        Payload::Measurement(m) => (
            "measurement",
            false,
            json!(m.value),
            "weave:geometry-results".into(),
        ),
        Payload::NavigationProjection(_) => (
            "navigation_projection",
            true,
            Value::Null,
            "weave:navigation".into(),
        ),
        Payload::RigidTransform(_) => {
            return Err(err(
                "E_GEOMETRY_KIND",
                "mapping is not a derived operation result",
            ))
        }
    };
    let node = Node {
        derived_nodes: node_gates.nodes,
        derived_from: origins.clone(),
        context_scope: Some(context.clone().unwrap_or_default()),
        id: "result".into(),
        entity_id: identity,
        space_id: space,
        type_id: Some("GeometryResult".into()),
        properties: BTreeMap::from([
            ("kind".into(), json!(kind)),
            ("approximate".into(), json!(approximate)),
            ("value".into(), scalar),
        ]),
        metadata: vec![],
        readers: vec![host.principal.clone()],
    };
    let edge = Edge {
        derived_nodes: vec![],
        structural_ref: None,
        assertion_source: None,
        assertion_context: context
            .as_ref()
            .and_then(ContextSelection::reference)
            .cloned(),
        assertion_properties: BTreeMap::from([(PROPERTY.into(), encoded)]),
        id: "value".into(),
        type_id: Some("GeometryEvidence".into()),
        predicate: "weave:geometry:value".into(),
        from: "result".into(),
        to: "result".into(),
        valid_time: window,
        polarity: Polarity::Positive,
        properties: BTreeMap::new(),
        metadata: vec![],
        readers: vec![host.principal.clone()],
        derived_from: origins.clone(),
        derivations,
    };
    let typing = match &second {
        Some(other) => context_typing::merge(
            first.value.graph.context_typing.as_ref(),
            other.value.graph.context_typing.as_ref(),
        ),
        None => Ok(first.value.graph.context_typing.clone()),
    }
    .map_err(|d| err(&d.code, &d.message))?;
    let influence = weave_contract::influence::merge(
        first.value.graph.influence.as_ref(),
        second
            .as_ref()
            .and_then(|other| other.value.graph.influence.as_ref()),
    )
    .map_err(|d| err(&d.code, &d.message))?;
    let mut value = first.value;
    if let Some(other) = second {
        value.source_revisions =
            algebra::merge_source_revisions(&value.source_revisions, &other.value.source_revisions)
                .map_err(|d| err(&d.code, &d.message))?;
        for r in other.value.input_snapshots {
            if !value.input_snapshots.contains(&r) {
                value.input_snapshots.push(r);
            }
        }
        for (g, r) in other.value.snapshots {
            value.snapshots.entry(g).or_insert(r);
        }
        if other.value.coverage == Coverage::Partial {
            value.coverage = Coverage::Partial;
        }
        for d in other.value.diagnostics {
            if !value.diagnostics.contains(&d) {
                value.diagnostics.push(d);
            }
        }
    }
    value.version = VERSION.into();
    value.selected_context = context;
    value.graph = GraphData {
        influence,
        context_typing: typing,
        schema: Some(result_schema()),
        nodes: vec![node],
        edges: vec![edge],
        ..GraphData::default()
    };
    value.provenance = origins.clone();
    value.edge_origins = BTreeMap::from([("value".into(), origins)]);
    // A calculation result is a synthetic value, not a newly asserted identity counterpart.
    value.node_origins = BTreeMap::from([("result".into(), vec![])]);
    value.attachment_origins.clear();
    value.metadata_graphs.clear();
    context_typing::protect_result_generated(&mut value).map_err(|d| err(&d.code, &d.message))?;
    weave_contract::influence::protect_generated_result(&mut value, MATERIALIZED_LIMIT)
        .map_err(|d| err(&d.code, &d.message))?;
    validate_graph(&value.graph)?;
    json_size(&value, MATERIALIZED_LIMIT)?;
    Ok(value)
}
fn result_schema() -> GraphSchema {
    let property = |value_type, nullable| PropertySchema {
        value_type,
        required: true,
        nullable,
    };
    GraphSchema {
        id: "weave:geometry-result".into(),
        revision: "1".into(),
        nodes: BTreeMap::from([(
            "GeometryResult".into(),
            NodeSchema {
                properties: BTreeMap::from([
                    ("kind".into(), property(ScalarType::String, false)),
                    ("approximate".into(), property(ScalarType::Boolean, false)),
                    ("value".into(), property(ScalarType::Float, true)),
                ]),
                space_id: None,
                allow_extra_properties: false,
            },
        )]),
        edges: BTreeMap::from([(
            "GeometryEvidence".into(),
            EdgeSchema {
                from_type: "GeometryResult".into(),
                to_type: "GeometryResult".into(),
                properties: BTreeMap::new(),
                allow_cross_space: false,
                allow_extra_properties: false,
            },
        )]),
    }
}
