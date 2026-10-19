//! Rows to facts. Every fact ID derives from the table's key column, so the
//! same row always maps to the same fact: that ID is the idempotency key.
use crate::schema::{self, End, Prop, Table, Ty, IMPORTER, NOTE_KEY, SCHEMA_REVISION, SPACE};
use crate::series::SeriesRef;
use crate::table::{parse_time, Batch, Row};
use crate::{error, Result};
use serde_json::{json, Map, Value};
use weave_contract::facts::Fact;
use weave_contract::provenance::{ClaimedProvenance, EvidenceRef, Method};

/// Most bytes of a row key (fact IDs hold at most 512).
pub const MAX_KEY_BYTES: usize = 256;

/// The node ID of a node type and key (`unit:SN-000001`).
pub fn node_id(node: &schema::NodeType, key: &str) -> String {
    format!("{}:{key}", node.prefix)
}

/// The claimed method of every imported fact.
pub fn import_method() -> Method {
    Method::Import {
        importer: IMPORTER.into(),
        version: SCHEMA_REVISION.into(),
    }
}

struct Ctx<'a> {
    table: &'a Table,
    row: &'a Row,
    key: &'a str,
}

impl Ctx<'_> {
    fn fail(&self, message: &str) -> weave_engine::Error {
        error(
            "E_HW_ROW",
            &format!("table {} row {:?}: {message}", self.table.name, self.key),
        )
    }
    fn get(&self, column: &str) -> Option<&str> {
        self.row
            .get(column)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
    fn need(&self, column: &str) -> Result<&str> {
        self.get(column)
            .ok_or_else(|| self.fail(&format!("column {column} is required")))
    }
    fn time(&self, column: &str) -> Result<Option<i64>> {
        self.get(column)
            .map(|v| parse_time(v).map_err(|e| self.fail(&e.message)))
            .transpose()
    }
    fn key_of(&self, column: &str) -> Result<&str> {
        let value = self.need(column)?;
        check_key(value).map_err(|m| self.fail(&format!("column {column}: {m}")))?;
        Ok(value)
    }
    fn interval(&self, start: Option<&str>, required: bool, end: Option<&str>) -> Result<Value> {
        let s = match start {
            Some(column) if required => Some(
                self.time(column)?
                    .ok_or_else(|| self.fail(&format!("column {column} is required")))?,
            ),
            Some(column) => self.time(column)?,
            None => None,
        }
        .unwrap_or(0);
        let e = match end {
            Some(column) => self.time(column)?,
            None => None,
        };
        if e.is_some_and(|e| e <= s) {
            return Err(self.fail("valid time end must exceed its start"));
        }
        Ok(json!({"start": s, "end": e}))
    }
    fn value(&self, prop: &Prop) -> Result<Option<Value>> {
        let Some(text) = self.get(prop.column) else {
            if prop.required {
                return Err(self.fail(&format!("column {} is required", prop.column)));
            }
            return Ok(None);
        };
        let bad = || {
            self.fail(&format!(
                "column {} needs a {}",
                prop.column,
                prop.ty.name()
            ))
        };
        Ok(Some(match prop.ty {
            Ty::Str => Value::String(text.to_string()),
            Ty::Int => text.parse::<i64>().map_err(|_| bad())?.into(),
            Ty::Float => {
                let f = text.parse::<f64>().map_err(|_| bad())?;
                if !f.is_finite() {
                    return Err(bad());
                }
                f.into()
            }
            Ty::Bool => match text {
                "true" => true.into(),
                "false" => false.into(),
                _ => return Err(bad()),
            },
            Ty::Time => self.time(prop.column)?.ok_or_else(bad)?.into(),
        }))
    }
    fn props(&self, props: &[Prop]) -> Result<Map<String, Value>> {
        let mut out = Map::new();
        for prop in props {
            if let Some(value) = self.value(prop)? {
                out.insert(prop.column.into(), value);
            }
        }
        Ok(out)
    }
    fn readers(&self) -> Vec<String> {
        let mut readers: Vec<String> = self
            .get("readers")
            .unwrap_or("")
            .split(';')
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_string)
            .collect();
        readers.sort();
        readers.dedup();
        readers
    }
    fn provenance(&self, batch_source: Option<&str>) -> Result<ClaimedProvenance> {
        let source = self
            .get("source")
            .or(batch_source)
            .unwrap_or(self.table.default_source);
        let evidence = match (self.get("evidence_uri"), self.get("evidence_digest")) {
            (None, None) => vec![],
            (Some(uri), Some(digest)) => vec![EvidenceRef::Artifact {
                uri: uri.into(),
                digest: digest.into(),
            }],
            _ => return Err(self.fail("evidence_uri and evidence_digest go together")),
        };
        let claim = ClaimedProvenance {
            source: Some(source.into()),
            method: import_method(),
            evidence,
        };
        claim
            .validate()
            .map_err(|e| self.fail(&format!("{}: {}", e.code, e.message)))?;
        Ok(claim)
    }
}

fn check_key(key: &str) -> std::result::Result<(), &'static str> {
    if key.is_empty() || key.len() > MAX_KEY_BYTES {
        return Err("a key needs 1 to 256 bytes");
    }
    if key.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("a key must not contain whitespace or control characters");
    }
    Ok(())
}

fn fact(value: Value) -> Result<Fact> {
    serde_json::from_value(value).map_err(|e| error("E_HW_ROW", &format!("invalid fact: {e}")))
}

/// The facts one row of `table` produces.
pub fn row_facts(table: &Table, row: &Row, batch_source: Option<&str>) -> Result<Vec<Fact>> {
    let columns = table.columns();
    if let Some(column) = row.keys().find(|c| !columns.contains(&c.as_str())) {
        return Err(error(
            "E_HW_INPUT",
            &format!("table {} has no column {column:?}", table.name),
        ));
    }
    let key = row.get(table.key).map(String::as_str).unwrap_or("");
    let ctx = Ctx { table, row, key };
    let key = ctx.key_of(table.key)?;
    let readers = ctx.readers();
    let provenance = ctx.provenance(batch_source)?;
    let mut out = Vec::new();
    let mut own_valid = json!({"start": 0, "end": null});
    let own_id = table.node.map(|n| node_id(&n.node, key));
    if let Some(map) = &table.node {
        let id = own_id.clone().expect("node table has a node ID");
        let mut node = json!({
            "kind": "node",
            "id": id,
            "entity_id": id,
            "space_id": SPACE,
            "type_id": map.node.type_id,
            "properties": ctx.props(map.node.props)?,
            "provenance": provenance,
            "readers": readers,
        });
        if let Some((start, end)) = map.valid {
            own_valid = ctx.interval(Some(start), true, Some(end))?;
            node["valid_time"] = own_valid.clone();
        }
        out.push(fact(node)?);
    }
    for edge in table.edges {
        if edge.when.is_some_and(|c| ctx.get(c).is_none()) {
            continue;
        }
        let end_id = |end: End| -> Result<String> {
            match end {
                End::Row => Ok(own_id.clone().expect("row endpoint needs a node table")),
                End::Column(column, node) => Ok(node_id(&node, ctx.key_of(column)?)),
            }
        };
        let relation = schema::relation(edge.predicate);
        out.push(fact(json!({
            "kind": "edge",
            "id": format!("{}:{key}", edge.id_prefix),
            "type_id": edge.predicate,
            "predicate": edge.predicate,
            "from": end_id(edge.from)?,
            "to": end_id(edge.to)?,
            "valid_time": ctx.interval(edge.start, edge.start_required, edge.end)?,
            "properties": ctx.props(relation.props)?,
            "provenance": provenance,
            "readers": readers,
        }))?);
    }
    if let Some(column) = table.note {
        if let Some(text) = ctx.get(column) {
            let host = own_id.clone().expect("note table has a node");
            out.push(fact(json!({
                "kind": "attachment",
                "id": format!("note:{host}"),
                "host": {"kind": "node", "id": host},
                "key": NOTE_KEY,
                "value": {"kind": "literal", "value": text},
                "valid_time": own_valid,
                "provenance": provenance,
                "readers": readers,
            }))?);
        }
    }
    if table.series {
        let start = ctx
            .time("start")?
            .ok_or_else(|| ctx.fail("column start is required"))?;
        let end = ctx
            .time("end")?
            .ok_or_else(|| ctx.fail("column end is required"))?;
        let series = SeriesRef {
            store_uri: ctx.need("store_uri")?.into(),
            series_id: ctx.need("series_id")?.into(),
            start,
            end,
            digest: ctx.need("digest")?.into(),
        };
        let sensor = node_id(&schema::SENSOR, ctx.key_of("sensor_id")?);
        let attachment = series
            .to_attachment(&format!("series:{key}"), &sensor, readers, Some(provenance))
            .map_err(|e| ctx.fail(&e.message))?;
        out.push(Fact::Attachment(attachment));
    }
    Ok(out)
}

fn rank(fact: &Fact) -> u8 {
    match fact {
        Fact::Node(_) => 0,
        Fact::Edge(_) | Fact::StructuralEdge(_) => 1,
        Fact::Assertion(_) => 2,
        Fact::Attachment(_) => 3,
    }
}

/// The facts a batch produces: nodes first, then edges, then attachments,
/// each in table order and then row order. A fact ID produced twice fails
/// with `E_HW_ROW` unless both rows produce the identical fact.
pub fn batch_facts(batch: &Batch) -> Result<Vec<Fact>> {
    for name in batch.tables.keys() {
        if schema::table(name).is_none() {
            return Err(error("E_HW_INPUT", &format!("unknown table {name:?}")));
        }
    }
    let mut facts = Vec::new();
    for table in schema::TABLES {
        for row in batch.tables.get(table.name).into_iter().flatten() {
            facts.extend(row_facts(table, row, batch.source.as_deref())?);
        }
    }
    facts.sort_by_key(rank);
    let mut seen: std::collections::HashMap<(u8, String), usize> = Default::default();
    let mut out: Vec<Fact> = Vec::with_capacity(facts.len());
    for fact in facts {
        let namespace = if matches!(fact, Fact::Node(_)) { 0 } else { 1 };
        match seen.get(&(namespace, fact.id().to_string())) {
            Some(&at) if out[at] == fact => {}
            Some(_) => {
                return Err(error(
                    "E_HW_ROW",
                    &format!("two rows produce different facts with ID {:?}", fact.id()),
                ))
            }
            None => {
                seen.insert((namespace, fact.id().to_string()), out.len());
                out.push(fact);
            }
        }
    }
    Ok(out)
}
