use super::*;
pub(crate) fn joined_schema(
    left: &GraphData,
    right: &GraphData,
    predicate: &str,
) -> Result<Option<GraphSchema>> {
    match (&left.schema, &right.schema) {
        (None, None) => Ok(None),
        (Some(l), Some(r)) => {
            if l.id == r.id && l.revision == r.revision && l != r {
                return Err(err(
                    "E_SCHEMA_REVISION",
                    "same schema revision has different descriptors",
                ));
            }
            Ok(Some(GraphSchema {
                id: format!(
                    "derived-schema:{:x}",
                    Sha256::digest(serde_json::to_vec(&(
                        "weave-schema-join-v1",
                        l,
                        r,
                        predicate
                    ))?)
                ),
                revision: "1".into(),
                nodes: BTreeMap::new(),
                edges: BTreeMap::new(),
            }))
        }
        _ => Err(err(
            "E_SCHEMA_JOIN",
            "typed and untyped graphs require an explicit schema mapping before join",
        )),
    }
}
pub(crate) fn type_join_node(
    node: &mut Node,
    source: &GraphSchema,
    output: &mut GraphSchema,
    bytes: &mut usize,
) -> Result<()> {
    let original = node
        .type_id
        .as_ref()
        .ok_or_else(|| err("E_SCHEMA_TYPE", "typed source node has no type"))?;
    let definition = source
        .nodes
        .get(original)
        .ok_or_else(|| err("E_SCHEMA_TYPE", "typed source node type is unresolved"))?;
    let name = format!(
        "node:{:x}",
        Sha256::digest(serde_json::to_vec(&(source, original))?)
    );
    if !output.nodes.contains_key(&name) {
        *bytes += json_size(
            &(&name, definition),
            MATERIALIZED_LIMIT.saturating_sub(*bytes),
        )?;
        output.nodes.insert(name.clone(), definition.clone());
    }
    node.type_id = Some(name);
    Ok(())
}
pub(crate) fn type_join_edge(
    from: &Node,
    to: &Node,
    output: &mut GraphSchema,
    bytes: &mut usize,
) -> Result<String> {
    let from_type = from
        .type_id
        .clone()
        .ok_or_else(|| err("E_SCHEMA_TYPE", "derived source node type unavailable"))?;
    let to_type = to
        .type_id
        .clone()
        .ok_or_else(|| err("E_SCHEMA_TYPE", "derived target node type unavailable"))?;
    let definition = EdgeSchema {
        from_type,
        to_type,
        properties: BTreeMap::new(),
        allow_cross_space: from.space_id != to.space_id,
        allow_extra_properties: false,
    };
    let name = format!(
        "edge:{:x}",
        Sha256::digest(serde_json::to_vec(&definition)?)
    );
    if !output.edges.contains_key(&name) {
        *bytes += json_size(
            &(&name, &definition),
            MATERIALIZED_LIMIT.saturating_sub(*bytes),
        )?;
        output.edges.insert(name.clone(), definition);
    }
    Ok(name)
}
