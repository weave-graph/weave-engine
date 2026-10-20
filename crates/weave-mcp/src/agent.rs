//! The B4 agent tools that are not one graph expression (#40):
//! `assert_fact`, `retract_fact` and `describe_schema`.
//!
//! This module only turns arguments into contract programs and summarizes
//! results. It holds no authority: the host (`weave-service`) runs every
//! program as the authenticated principal, mints the record IDs, and the
//! engine authorizes and records the writer.
//!
//! **Data/code boundary for writes.** The argument types are closed
//! (`deny_unknown_fields`) and have no field for a writer, a recorded
//! time, `on_behalf_of`, a grant, a derivation gate or a record ID: the
//! writer comes from the credential, IDs from the host, and the visibility
//! of every written record is stated explicitly. Property values are JSON
//! data and never become program structure.
use crate::{check_arguments, codes, failure};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use weave_contract::embedding::SpaceRef;
use weave_contract::facts::{Fact, FactChange, FactTarget};
use weave_contract::provenance::{ClaimedProvenance, EvidenceRef, Method};
use weave_contract::traversal::ChangesPlan;
use weave_contract::{Command, GraphExpression, Interval, Polarity, Program, VERSION};
use weave_engine::Error;

/// Most facts one `assert_fact` or `retract_fact` call writes.
pub const MAX_FACTS_PER_CALL: usize = 256;
/// Longest `ref` label, in bytes.
pub const MAX_REF_BYTES: usize = 64;
/// Most visible facts `describe_schema` summarizes; more fails `E_BUDGET`.
pub const MAX_DESCRIBED_FACTS: u32 = 200_000;
/// Most entries in each list of a `describe_schema` result. Longer lists
/// keep the most frequent entries and report how many were omitted.
pub const MAX_DESCRIBED_ENTRIES: usize = 1_000;

fn main_branch() -> String {
    "main".into()
}

fn parse<T: DeserializeOwned>(tool: &str, arguments: Value) -> Result<T, Error> {
    check_arguments(&arguments)?;
    serde_json::from_value(arguments)
        .map_err(|error| failure(codes::INPUT, format!("invalid {tool} arguments: {error}")))
}

// ------------------------------------------------------------- assert_fact

/// How the facts were produced: the claimed part of their provenance. The
/// writer and recorded time are not claims; the engine records them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub method: Method,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
}

/// Who may read every fact of the call. Required: an agent write is never
/// public by omission.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Visibility {
    #[serde(default)]
    pub public: bool,
    #[serde(default)]
    pub readers: Vec<String>,
    #[serde(default)]
    pub reader_groups: Vec<String>,
}

/// A label naming a fact of the same call.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalRef {
    #[serde(rename = "ref")]
    pub label: String,
}

/// An edge endpoint: an existing node ID, or a node of this call.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum NodeTarget {
    Id(String),
    Ref(LocalRef),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorKind {
    Node,
    Edge,
    Entity,
}

/// What an attachment or embedding is about: `id` names an existing
/// record (or entity), `ref` a fact of this call.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub kind: AnchorKind,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "ref")]
    pub label: Option<String>,
}

/// One fact to assert, without an ID: the host mints it.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NewFact {
    Node {
        #[serde(default, rename = "ref")]
        label: Option<String>,
        /// Defaults to the minted node ID: a new entity.
        #[serde(default)]
        entity_id: Option<String>,
        space_id: String,
        #[serde(default)]
        type_id: Option<String>,
        #[serde(default)]
        properties: Map<String, Value>,
        #[serde(default)]
        valid_time: Option<Interval>,
    },
    Edge {
        #[serde(default, rename = "ref")]
        label: Option<String>,
        predicate: String,
        from: NodeTarget,
        to: NodeTarget,
        valid_time: Interval,
        #[serde(default)]
        properties: Map<String, Value>,
        #[serde(default)]
        polarity: Polarity,
    },
    Attachment {
        #[serde(default, rename = "ref")]
        label: Option<String>,
        host: Anchor,
        key: String,
        value: Value,
        valid_time: Interval,
    },
    Embedding {
        #[serde(default, rename = "ref")]
        label: Option<String>,
        space: SpaceRef,
        subject: Anchor,
        vector: Vec<f32>,
        valid_time: Interval,
    },
}

impl NewFact {
    fn label(&self) -> Option<&str> {
        match self {
            NewFact::Node { label, .. }
            | NewFact::Edge { label, .. }
            | NewFact::Attachment { label, .. }
            | NewFact::Embedding { label, .. } => label.as_deref(),
        }
    }
    fn kind(&self) -> &'static str {
        match self {
            NewFact::Node { .. } => "node",
            NewFact::Edge { .. } => "edge",
            NewFact::Attachment { .. } => "attachment",
            NewFact::Embedding { .. } => "embedding",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssertArguments {
    pub graph_id: String,
    #[serde(default = "main_branch")]
    pub branch_id: String,
    pub provenance: Claim,
    pub visibility: Visibility,
    pub facts: Vec<NewFact>,
}

/// A write a tool call plans: one `write` command, and the facts it
/// asserts or retracts as `{ref?, kind, id}`.
#[derive(Debug, Clone)]
pub struct PlannedWrite {
    pub command: Command,
    pub facts: Vec<Value>,
}

impl PlannedWrite {
    /// The program that runs the write: exactly one `write` command.
    pub fn program(&self) -> Program {
        Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![self.command.clone()],
        }
    }
}

fn check_count(len: usize) -> Result<(), Error> {
    if len == 0 {
        return Err(failure(codes::INPUT, "facts must name at least one fact"));
    }
    if len > MAX_FACTS_PER_CALL {
        return Err(failure(
            "E_BUDGET",
            format!("one call writes at most {MAX_FACTS_PER_CALL} facts"),
        ));
    }
    Ok(())
}

/// Plan an `assert_fact` call. `mint` returns a fresh record ID for the
/// caller; every asserted fact gets one, so no argument chooses an ID.
pub fn plan_assert(
    arguments: Value,
    mut mint: impl FnMut() -> String,
) -> Result<PlannedWrite, Error> {
    let arguments: AssertArguments = parse("assert_fact", arguments)?;
    check_count(arguments.facts.len())?;
    let claim = arguments.provenance;
    if claim.method == Method::Unspecified {
        return Err(failure(
            codes::CLAIM,
            "provenance.method must state how the facts were produced (model, rule, import or human)",
        ));
    }
    let visibility = arguments.visibility;
    let restricted = !visibility.readers.is_empty() || !visibility.reader_groups.is_empty();
    if visibility.public == restricted {
        return Err(failure(
            codes::CLAIM,
            "visibility must name readers or reader_groups, or be exactly {\"public\": true}",
        ));
    }
    let claimed = serde_json::to_value(ClaimedProvenance {
        source: claim.source,
        method: claim.method,
        evidence: claim.evidence,
    })
    .expect("provenance serializes");

    // Mint every ID first, so refs may point forward or backward.
    let ids: Vec<String> = arguments.facts.iter().map(|_| mint()).collect();
    let mut labels: HashMap<&str, (usize, &'static str)> = HashMap::new();
    for (index, fact) in arguments.facts.iter().enumerate() {
        if let Some(label) = fact.label() {
            if label.is_empty() || label.len() > MAX_REF_BYTES {
                return Err(failure(
                    codes::INPUT,
                    format!("a ref label requires 1 to {MAX_REF_BYTES} bytes"),
                ));
            }
            if labels.insert(label, (index, fact.kind())).is_some() {
                return Err(failure(codes::INPUT, format!("ref {label:?} is repeated")));
            }
        }
    }
    let resolve = |label: &str, want: &str| -> Result<usize, Error> {
        match labels.get(label) {
            Some((index, kind)) if *kind == want => Ok(*index),
            Some(_) => Err(failure(
                codes::INPUT,
                format!("ref {label:?} does not name a {want} of this call"),
            )),
            None => Err(failure(
                codes::INPUT,
                format!("ref {label:?} names no fact of this call"),
            )),
        }
    };
    let entity_of = |index: usize| match &arguments.facts[index] {
        NewFact::Node { entity_id, .. } => entity_id.clone().unwrap_or_else(|| ids[index].clone()),
        _ => unreachable!("resolved as a node"),
    };
    let node = |target: &NodeTarget| -> Result<String, Error> {
        match target {
            NodeTarget::Id(id) => Ok(id.clone()),
            NodeTarget::Ref(local) => Ok(ids[resolve(&local.label, "node")?].clone()),
        }
    };
    let anchor = |anchor: &Anchor| -> Result<(AnchorKind, String), Error> {
        let id = match (&anchor.id, &anchor.label) {
            (Some(id), None) => id.clone(),
            (None, Some(label)) => match anchor.kind {
                AnchorKind::Node => ids[resolve(label, "node")?].clone(),
                AnchorKind::Edge => ids[resolve(label, "edge")?].clone(),
                AnchorKind::Entity => entity_of(resolve(label, "node")?),
            },
            _ => {
                return Err(failure(
                    codes::INPUT,
                    "a host or subject names exactly one of id and ref",
                ))
            }
        };
        Ok((anchor.kind, id))
    };

    let mut changes = Vec::with_capacity(arguments.facts.len());
    let mut planned = Vec::with_capacity(arguments.facts.len());
    for (index, fact) in arguments.facts.iter().enumerate() {
        let id = &ids[index];
        let mut record = match fact {
            NewFact::Node {
                entity_id,
                space_id,
                type_id,
                properties,
                valid_time,
                ..
            } => {
                let mut record = json!({"kind": "node", "id": id,
                    "entity_id": entity_id.clone().unwrap_or_else(|| id.clone()),
                    "space_id": space_id, "properties": properties});
                if let Some(type_id) = type_id {
                    record["type_id"] = json!(type_id);
                }
                if let Some(valid_time) = valid_time {
                    record["valid_time"] = json!(valid_time);
                }
                record
            }
            NewFact::Edge {
                predicate,
                from,
                to,
                valid_time,
                properties,
                polarity,
                ..
            } => json!({"kind": "edge", "id": id, "predicate": predicate,
                "from": node(from)?, "to": node(to)?, "valid_time": valid_time,
                "properties": properties, "polarity": polarity}),
            NewFact::Attachment {
                host,
                key,
                value,
                valid_time,
                ..
            } => {
                let (kind, host_id) = anchor(host)?;
                json!({"kind": "attachment", "id": id,
                    "host": {"kind": kind_name(kind), "id": host_id}, "key": key,
                    "value": {"kind": "literal", "value": value}, "valid_time": valid_time})
            }
            NewFact::Embedding {
                space,
                subject,
                vector,
                valid_time,
                ..
            } => {
                let (kind, subject_id) = anchor(subject)?;
                let subject = match kind {
                    AnchorKind::Node => json!({"kind": "node", "node_id": subject_id}),
                    AnchorKind::Edge => json!({"kind": "edge", "edge_id": subject_id}),
                    AnchorKind::Entity => json!({"kind": "entity", "entity_id": subject_id}),
                };
                json!({"kind": "embedding", "id": id, "space": space, "subject": subject,
                    "vector": vector, "valid_time": valid_time})
            }
        };
        record["provenance"] = claimed.clone();
        if restricted {
            record["readers"] = json!(visibility.readers);
            record["reader_groups"] = json!(visibility.reader_groups);
        }
        let record: Fact = serde_json::from_value(record).map_err(|error| {
            failure(
                codes::INPUT,
                format!("invalid fact {index} of assert_fact: {error}"),
            )
        })?;
        let mut entry = json!({"kind": fact_kind_name(&record), "id": id});
        if let Some(label) = fact.label() {
            entry["ref"] = json!(label);
        }
        planned.push(entry);
        changes.push(FactChange::Assert { fact: record });
    }
    Ok(PlannedWrite {
        command: Command::Write {
            graph_id: arguments.graph_id,
            branch_id: arguments.branch_id,
            expected_head: None,
            changes,
        },
        facts: planned,
    })
}

fn fact_kind_name(fact: &Fact) -> String {
    serde_json::to_value(fact.kind())
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

fn kind_name(kind: AnchorKind) -> &'static str {
    match kind {
        AnchorKind::Node => "node",
        AnchorKind::Edge => "edge",
        AnchorKind::Entity => "entity",
    }
}

// ------------------------------------------------------------ retract_fact

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetractArguments {
    pub graph_id: String,
    #[serde(default = "main_branch")]
    pub branch_id: String,
    pub facts: Vec<FactTarget>,
}

/// Plan a `retract_fact` call: one `write` with a `retract` per target.
pub fn plan_retract(arguments: Value) -> Result<PlannedWrite, Error> {
    let arguments: RetractArguments = parse("retract_fact", arguments)?;
    check_count(arguments.facts.len())?;
    let facts = arguments
        .facts
        .iter()
        .map(|target| json!({"kind": target.kind, "id": target.id}))
        .collect();
    let changes = arguments
        .facts
        .into_iter()
        .map(|target| FactChange::Retract {
            target,
            expected: None,
        })
        .collect();
    Ok(PlannedWrite {
        command: Command::Write {
            graph_id: arguments.graph_id,
            branch_id: arguments.branch_id,
            expected_head: None,
            changes,
        },
        facts,
    })
}

// --------------------------------------------------------- describe_schema

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DescribeArguments {
    #[serde(default)]
    pub graph_id: Option<String>,
    #[serde(default = "main_branch")]
    pub branch_id: String,
}

pub fn parse_describe(arguments: Value) -> Result<DescribeArguments, Error> {
    parse("describe_schema", arguments)
}

/// The read a graph description summarizes: every fact the caller can see
/// at the branch head, as the change set from the empty state. It runs as
/// the caller like any other read, so hidden facts never reach the summary.
pub fn describe_program(graph_id: &str, branch_id: &str) -> Program {
    let plan: ChangesPlan = serde_json::from_value(json!({
        "graph_id": graph_id, "branch_id": branch_id,
        "from": {"kind": "as_of", "unix_millis": 0}, "to": {"kind": "head"},
        "limit": MAX_DESCRIBED_FACTS,
    }))
    .expect("a valid changes plan");
    crate::program(GraphExpression::Changes(plan))
}

#[derive(Default)]
struct Tally {
    count: u64,
    keys: BTreeSet<String>,
    types: BTreeSet<&'static str>,
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// The most frequent entries first, then by name, capped.
fn ranked(
    tallies: BTreeMap<String, Tally>,
    name: &str,
    omitted: &mut Map<String, Value>,
    list: &str,
    extra: impl Fn(&str, &Tally) -> Map<String, Value>,
) -> Value {
    let mut entries: Vec<(String, Tally)> = tallies.into_iter().collect();
    entries.sort_by(|a, b| b.1.count.cmp(&a.1.count).then_with(|| a.0.cmp(&b.0)));
    if entries.len() > MAX_DESCRIBED_ENTRIES {
        omitted.insert(list.into(), json!(entries.len() - MAX_DESCRIBED_ENTRIES));
        entries.truncate(MAX_DESCRIBED_ENTRIES);
    }
    Value::Array(
        entries
            .into_iter()
            .map(|(key, tally)| {
                let mut entry = Map::new();
                entry.insert(name.into(), json!(key));
                entry.insert("count".into(), json!(tally.count));
                entry.extend(extra(&key, &tally));
                Value::Object(entry)
            })
            .collect(),
    )
}

/// Summarize the change set of [`describe_program`] (its first command
/// result) into a graph description. Everything in it comes from facts the
/// caller can see; `permission` and `reader_groups` are the caller's own
/// standing.
pub fn summarize(
    graph_id: &str,
    branch_id: &str,
    result: &Value,
    permission: Value,
    reader_groups: Value,
) -> Value {
    let changes = &result["changes"];
    let mut kinds: BTreeMap<String, u64> = BTreeMap::new();
    let mut spaces: BTreeMap<String, Tally> = BTreeMap::new();
    let mut types: BTreeMap<String, Tally> = BTreeMap::new();
    let mut node_properties: BTreeMap<String, Tally> = BTreeMap::new();
    let mut predicates: BTreeMap<String, Tally> = BTreeMap::new();
    let mut attachment_keys: BTreeMap<String, Tally> = BTreeMap::new();
    let mut embedding_spaces: BTreeMap<String, (Value, u64, Option<String>)> = BTreeMap::new();
    let mut first_node: Option<String> = None;
    let entries = changes["entries"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for entry in entries {
        let fact = &entry["after"];
        let Some(kind) = fact["kind"].as_str() else {
            continue;
        };
        *kinds.entry(kind.into()).or_default() += 1;
        match kind {
            "node" => {
                if first_node.is_none() {
                    first_node = fact["id"].as_str().map(String::from);
                }
                if let Some(space) = fact["space_id"].as_str() {
                    spaces.entry(space.into()).or_default().count += 1;
                }
                if let Some(type_id) = fact["type_id"].as_str() {
                    types.entry(type_id.into()).or_default().count += 1;
                }
                for (key, value) in fact["properties"].as_object().into_iter().flatten() {
                    let tally = node_properties.entry(key.clone()).or_default();
                    tally.count += 1;
                    tally.types.insert(json_type(value));
                }
            }
            "edge" => {
                if let Some(predicate) = fact["predicate"].as_str() {
                    let tally = predicates.entry(predicate.into()).or_default();
                    tally.count += 1;
                    for key in fact["properties"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .map(|(k, _)| k)
                    {
                        tally.keys.insert(key.clone());
                    }
                }
            }
            "attachment" => {
                if let Some(key) = fact["key"].as_str() {
                    let tally = attachment_keys.entry(key.into()).or_default();
                    tally.count += 1;
                    if let Some(host) = fact["host"]["kind"].as_str() {
                        tally.keys.insert(host.into());
                    }
                    let value = &fact["value"];
                    tally.types.insert(match value["kind"].as_str() {
                        Some("literal") => json_type(&value["value"]),
                        Some("graph") => "graph",
                        Some("live_graph") => "live_graph",
                        Some("object") => "object_ref",
                        _ => "other",
                    });
                }
            }
            "embedding_space" => {
                let key = format!(
                    "{}@{}",
                    fact["id"].as_str().unwrap_or_default(),
                    fact["revision"].as_str().unwrap_or_default()
                );
                let mut descriptor = fact.clone();
                if let Some(object) = descriptor.as_object_mut() {
                    object.remove("kind");
                }
                embedding_spaces
                    .entry(key)
                    .or_insert((Value::Null, 0, None))
                    .0 = descriptor;
            }
            "embedding" => {
                let key = format!(
                    "{}@{}",
                    fact["space"]["id"].as_str().unwrap_or_default(),
                    fact["space"]["revision"].as_str().unwrap_or_default()
                );
                let slot = embedding_spaces
                    .entry(key)
                    .or_insert((Value::Null, 0, None));
                slot.1 += 1;
                if slot.2.is_none() {
                    slot.2 = fact["id"].as_str().map(String::from);
                }
            }
            _ => {}
        }
    }
    let mut omitted = Map::new();
    let at = json!({"graph_id": graph_id, "branch_id": branch_id});
    let predicates = ranked(
        predicates,
        "predicate",
        &mut omitted,
        "predicates",
        |key, tally| {
            let mut arguments = at.clone();
            arguments["predicate"] = json!(key);
            let mut extra = Map::new();
            extra.insert("property_keys".into(), json!(tally.keys));
            extra.insert(
                "example".into(),
                json!({"tool": "query", "arguments": arguments}),
            );
            extra
        },
    );
    let typed = |_: &str, tally: &Tally| {
        let mut extra = Map::new();
        extra.insert("types".into(), json!(tally.types));
        extra
    };
    let node_properties = ranked(
        node_properties,
        "key",
        &mut omitted,
        "node_properties",
        typed,
    );
    let attachment_keys = ranked(
        attachment_keys,
        "key",
        &mut omitted,
        "attachment_keys",
        |_, tally| {
            let mut extra = Map::new();
            extra.insert("host_kinds".into(), json!(tally.keys));
            extra.insert("value_types".into(), json!(tally.types));
            extra
        },
    );
    let none = |_: &str, _: &Tally| Map::new();
    let node_spaces = ranked(spaces, "space_id", &mut omitted, "node_spaces", none);
    let node_types = ranked(types, "type_id", &mut omitted, "node_types", none);
    let mut described_spaces: Vec<Value> = embedding_spaces
        .into_values()
        // An embedding whose space descriptor is not visible is still
        // counted, under a space with an unknown descriptor.
        .map(|(descriptor, count, sample)| {
            let mut entry = json!({"space": descriptor, "embeddings": count});
            if let (Some(sample), Some(id), Some(revision)) = (
                sample,
                descriptor["id"].as_str(),
                descriptor["revision"].as_str(),
            ) {
                let mut arguments = at.clone();
                arguments["kind"] = json!("nearest");
                arguments["space"] = json!({"id": id, "revision": revision});
                arguments["query"] = json!({"kind": "embedding", "embedding_id": sample});
                arguments["k"] = json!(10);
                entry["example"] = json!({"tool": "hybrid_search", "arguments": arguments});
            }
            entry
        })
        .collect();
    if described_spaces.len() > MAX_DESCRIBED_ENTRIES {
        omitted.insert(
            "embedding_spaces".into(),
            json!(described_spaces.len() - MAX_DESCRIBED_ENTRIES),
        );
        described_spaces.truncate(MAX_DESCRIBED_ENTRIES);
    }
    let mut description = json!({
        "graph_id": graph_id,
        "branch_id": branch_id,
        "revision": changes["to_revision"],
        "coverage": result["coverage"],
        "permission": permission,
        "reader_groups": reader_groups,
        "facts": kinds,
        "predicates": predicates,
        "node_spaces": node_spaces,
        "node_types": node_types,
        "node_properties": node_properties,
        "attachment_keys": attachment_keys,
        "embedding_spaces": described_spaces,
        "omitted": omitted,
    });
    if let Some(seed) = first_node {
        let mut arguments = at.clone();
        arguments["seeds"] = json!([seed]);
        arguments["hops"] = json!(1);
        arguments["direction"] = json!("both");
        description["example"] = json!({"tool": "expand", "arguments": arguments});
    }
    description
}

// ------------------------------------------------------------------ schemas

fn interval_schema() -> Value {
    json!({
        "type": "object",
        "description": "Valid time: when this was true in the world (Unix ms); end absent means open",
        "properties": {"start": {"type": "integer"}, "end": {"type": ["integer", "null"]}},
        "required": ["start"],
        "additionalProperties": false,
    })
}

fn ref_schema() -> Value {
    json!({"type": "string", "minLength": 1, "maxLength": MAX_REF_BYTES,
        "description": "A label other facts of this call use to refer to this one"})
}

fn node_target_schema() -> Value {
    json!({"oneOf": [
        {"type": "string", "description": "An existing node ID"},
        {"type": "object", "properties": {"ref": {"type": "string"}}, "required": ["ref"],
         "additionalProperties": false, "description": "A node of this call"},
    ]})
}

fn anchor_schema() -> Value {
    json!({
        "type": "object",
        "description": "An existing record (id) or a fact of this call (ref); for an entity, ref names a node of this call and means its entity",
        "properties": {
            "kind": {"enum": ["node", "edge", "entity"]},
            "id": {"type": "string"},
            "ref": {"type": "string"},
        },
        "required": ["kind"],
        "additionalProperties": false,
    })
}

fn method_schema() -> Value {
    json!({"oneOf": [
        {"type": "object", "properties": {"kind": {"const": "model"}, "model": {"type": "string"}, "version": {"type": "string"}},
         "required": ["kind", "model", "version"], "additionalProperties": false},
        {"type": "object", "properties": {"kind": {"const": "rule"}, "rule_id": {"type": "string"}, "revision": {"type": "string"}},
         "required": ["kind", "rule_id", "revision"], "additionalProperties": false},
        {"type": "object", "properties": {"kind": {"const": "import"}, "importer": {"type": "string"}, "version": {"type": "string"}},
         "required": ["kind", "importer", "version"], "additionalProperties": false},
        {"type": "object", "properties": {"kind": {"const": "human"}}, "required": ["kind"], "additionalProperties": false},
    ]})
}

fn evidence_schema() -> Value {
    json!({
        "type": "array", "maxItems": 64,
        "description": "What the facts are based on: {kind: artifact, uri, digest} for a document, or {kind: node|assertion|embedding, graph_id, revision, <kind>_id} or {kind: graph, graph_id, revision} for exact facts",
        "items": {"type": "object", "properties": {"kind": {"enum": ["artifact", "node", "assertion", "embedding", "graph"]}}, "required": ["kind"]},
    })
}

/// `assert_fact` arguments.
pub fn assert_schema() -> Value {
    let label = ref_schema();
    json!({
        "type": "object",
        "properties": {
            "graph_id": {"type": "string"},
            "branch_id": {"type": "string", "default": "main"},
            "provenance": {
                "type": "object",
                "properties": {
                    "method": method_schema(),
                    "source": {"type": "string", "description": "A free label for the source, such as a document title; not an identity"},
                    "evidence": evidence_schema(),
                },
                "required": ["method"],
                "additionalProperties": false,
            },
            "visibility": {
                "type": "object",
                "description": "Who may read every fact of this call: reader_groups (groups you may name; see describe_schema) and/or readers (principal IDs), or exactly {public: true}",
                "properties": {
                    "public": {"type": "boolean"},
                    "readers": {"type": "array", "items": {"type": "string"}},
                    "reader_groups": {"type": "array", "items": {"type": "string"}},
                },
                "additionalProperties": false,
            },
            "facts": {
                "type": "array", "minItems": 1, "maxItems": MAX_FACTS_PER_CALL,
                "items": {"oneOf": [
                    {"type": "object", "properties": {
                        "kind": {"const": "node"}, "ref": label, "entity_id": {"type": "string", "description": "Defaults to the minted node ID"},
                        "space_id": {"type": "string"}, "type_id": {"type": "string"},
                        "properties": {"type": "object"}, "valid_time": interval_schema()},
                     "required": ["kind", "space_id"], "additionalProperties": false},
                    {"type": "object", "properties": {
                        "kind": {"const": "edge"}, "ref": label, "predicate": {"type": "string"},
                        "from": node_target_schema(), "to": node_target_schema(), "valid_time": interval_schema(),
                        "properties": {"type": "object"}, "polarity": {"enum": ["positive", "negative"], "default": "positive"}},
                     "required": ["kind", "predicate", "from", "to", "valid_time"], "additionalProperties": false},
                    {"type": "object", "properties": {
                        "kind": {"const": "attachment"}, "ref": label, "host": anchor_schema(), "key": {"type": "string"},
                        "value": {"description": "Any JSON value"}, "valid_time": interval_schema()},
                     "required": ["kind", "host", "key", "value", "valid_time"], "additionalProperties": false},
                    {"type": "object", "properties": {
                        "kind": {"const": "embedding"}, "ref": label,
                        "space": {"type": "object", "properties": {"id": {"type": "string"}, "revision": {"type": "string"}},
                                  "required": ["id", "revision"], "additionalProperties": false},
                        "subject": anchor_schema(), "vector": {"type": "array", "items": {"type": "number"}},
                        "valid_time": interval_schema()},
                     "required": ["kind", "space", "subject", "vector", "valid_time"], "additionalProperties": false},
                ]},
            },
        },
        "required": ["graph_id", "provenance", "visibility", "facts"],
        "additionalProperties": false,
    })
}

/// `retract_fact` arguments.
pub fn retract_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "graph_id": {"type": "string"},
            "branch_id": {"type": "string", "default": "main"},
            "facts": {
                "type": "array", "minItems": 1, "maxItems": MAX_FACTS_PER_CALL,
                "items": {"type": "object", "properties": {
                    "kind": {"enum": ["node", "edge", "structural_edge", "assertion", "attachment", "embedding"]},
                    "id": {"type": "string"}},
                 "required": ["kind", "id"], "additionalProperties": false},
            },
        },
        "required": ["graph_id", "facts"],
        "additionalProperties": false,
    })
}

/// `describe_schema` arguments.
pub fn describe_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "graph_id": {"type": "string", "description": "Describe this graph; absent lists the graphs you can see"},
            "branch_id": {"type": "string", "default": "main"},
        },
        "additionalProperties": false,
    })
}

// ----------------------------------------------------------------- compile

/// Longest source text (and module source) `compile` accepts, in bytes.
pub const MAX_SOURCE_BYTES: usize = 512 * 1024;
/// Most modules one `compile` call supplies.
pub const MAX_MODULES: usize = 64;
/// Most graphs whose embedding spaces one `compile` call's manifest lists.
pub const MAX_SPACE_GRAPHS: usize = 8;

/// A module the source imports, pinned by content as the compiler requires.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub id: String,
    pub revision: String,
    pub source: String,
}

/// `compile` arguments. There is no policy field: the host sets the policy.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileArguments {
    pub source: String,
    #[serde(default)]
    pub modules: Vec<Module>,
    #[serde(default)]
    pub entry_id: Option<String>,
    /// Graphs whose embedding spaces the manifest lists; `None` is the
    /// host's default.
    #[serde(default)]
    pub graphs: Option<Vec<String>>,
}

pub fn parse_compile(arguments: Value) -> Result<CompileArguments, Error> {
    crate::check_arguments_within(&arguments, MAX_SOURCE_BYTES)?;
    let arguments: CompileArguments = serde_json::from_value(arguments)
        .map_err(|error| failure(codes::INPUT, format!("invalid compile arguments: {error}")))?;
    if arguments.modules.len() > MAX_MODULES {
        return Err(failure(
            "E_BUDGET",
            format!("at most {MAX_MODULES} modules"),
        ));
    }
    if arguments
        .graphs
        .as_ref()
        .is_some_and(|g| g.len() > MAX_SPACE_GRAPHS)
    {
        return Err(failure(
            "E_BUDGET",
            format!("graphs names at most {MAX_SPACE_GRAPHS} graphs"),
        ));
    }
    if arguments
        .entry_id
        .as_ref()
        .is_some_and(|id| id.is_empty() || id.len() > MAX_REF_BYTES * 4)
    {
        return Err(failure(codes::INPUT, "entry_id requires 1 to 256 bytes"));
    }
    Ok(arguments)
}

/// The `weave-compiler-request/1` a `compile` call sends: the caller's
/// source with the host's policy (always `untrusted`, `read_only` as the
/// host decides) and the caller's visible manifest.
pub fn compile_request(arguments: &CompileArguments, read_only: bool, manifest: Value) -> Value {
    json!({
        "format": "weave-compiler-request/1",
        "entry_id": arguments.entry_id.as_deref().unwrap_or("agent"),
        "source": arguments.source,
        "modules": arguments.modules,
        "policy": {"untrusted": true, "read_only": read_only, "visible": manifest},
    })
}

/// The embedding space descriptors in a [`describe_program`] result, as a
/// `weave-visible/1` manifest lists them.
pub fn visible_spaces(result: &Value) -> Vec<Value> {
    result["changes"]["entries"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|entry| entry["after"]["kind"] == "embedding_space")
        .map(|entry| {
            let mut space = entry["after"].clone();
            if let Some(object) = space.as_object_mut() {
                object.remove("kind");
            }
            space
        })
        .collect()
}

/// `compile` arguments schema.
pub fn compile_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "source": {"type": "string", "maxLength": MAX_SOURCE_BYTES,
                "description": "Weave language source"},
            "modules": {"type": "array", "maxItems": MAX_MODULES, "items": {
                "type": "object",
                "properties": {"id": {"type": "string"}, "revision": {"type": "string"},
                    "source": {"type": "string"}},
                "required": ["id", "revision", "source"], "additionalProperties": false},
                "description": "Modules the source imports, pinned by content"},
            "entry_id": {"type": "string", "description": "A label for diagnostics (default agent)"},
            "graphs": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_SPACE_GRAPHS,
                "description": "Graphs whose embedding spaces to check; default: your first readable graphs"},
        },
        "required": ["source"],
        "additionalProperties": false,
    })
}
