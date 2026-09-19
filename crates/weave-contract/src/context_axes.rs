//! Proposed portable typed-context profile. No graph reads, authority or wire integration.
//! Decode through `ContextDefinition::from_json` to reject duplicate JSON members.
use crate::decimal::Decimal;
use crate::quantity::{Quantity, UnitDescriptor};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Write;

pub const MAX_BYTES: usize = 64 * 1024;
pub const MAX_AXES: usize = 32;
pub const MAX_ENUM: usize = 128;
const MAX_SCHEMAS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextError {
    Budget,
    Syntax,
    Schema,
    Assignment,
    SchemaConflict,
}
impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ContextError {}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct ContextSchemaRef {
    pub id: String,
    pub revision: String,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextAxisType {
    Boolean,
    Integer,
    String,
    Decimal,
    Quantity { unit: UnitDescriptor },
    Enum { members: Vec<String> },
}
// Custom Deserialize below preserves duplicate checks and a cumulative embedded byte budget.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ContextSchema {
    pub reference: ContextSchemaRef,
    pub axes: BTreeMap<String, ContextAxisType>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ContextDefinition {
    pub schema: ContextSchema,
    pub values: BTreeMap<String, Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaWire {
    reference: ContextSchemaRef,
    axes: BTreeMap<String, AxisWire>,
}
// Empty struct variants reject extra fields; serde unit variants ignore them.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum AxisWire {
    Boolean {},
    Integer {},
    String {},
    Decimal {},
    Quantity { unit: UnitDescriptor },
    Enum { members: Vec<String> },
}
impl From<AxisWire> for ContextAxisType {
    fn from(value: AxisWire) -> Self {
        match value {
            AxisWire::Boolean {} => Self::Boolean,
            AxisWire::Integer {} => Self::Integer,
            AxisWire::String {} => Self::String,
            AxisWire::Decimal {} => Self::Decimal,
            AxisWire::Quantity { unit } => Self::Quantity { unit },
            AxisWire::Enum { members } => Self::Enum { members },
        }
    }
}
impl From<SchemaWire> for ContextSchema {
    fn from(value: SchemaWire) -> Self {
        Self {
            reference: value.reference,
            axes: value
                .axes
                .into_iter()
                .map(|(name, kind)| (name, kind.into()))
                .collect(),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinitionWire {
    schema: SchemaWire,
    values: BTreeMap<String, Value>,
}
fn identifier(s: &str) -> bool {
    !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control)
}
struct Limited(Vec<u8>);
impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_BYTES {
            return Err(std::io::Error::other("context byte budget"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encoded(value: &impl Serialize) -> Result<Vec<u8>, ContextError> {
    let mut out = Limited(Vec::new());
    serde_json::to_writer(&mut out, value).map_err(|_| ContextError::Budget)?;
    Ok(out.0)
}
impl ContextSchema {
    pub fn from_json(bytes: &[u8]) -> Result<Self, ContextError> {
        let wire: SchemaWire =
            serde_json::from_value(strict_json(bytes)?).map_err(|_| ContextError::Syntax)?;
        let schema: Self = wire.into();
        schema.validate()?;
        Ok(schema)
    }
    pub fn validate(&self) -> Result<(), ContextError> {
        // Outer counts precede iteration/encoding/cloning.
        if self.axes.is_empty() || self.axes.len() > MAX_AXES {
            return Err(ContextError::Budget);
        }
        if !identifier(&self.reference.id) || !identifier(&self.reference.revision) {
            return Err(ContextError::Schema);
        }
        for (name, kind) in &self.axes {
            if !identifier(name) {
                return Err(ContextError::Schema);
            }
            if let ContextAxisType::Enum { members } = kind {
                if members.is_empty() || members.len() > MAX_ENUM {
                    return Err(ContextError::Budget);
                }
                let mut seen = BTreeSet::new();
                for member in members {
                    if !identifier(member) || !seen.insert(member) {
                        return Err(ContextError::Schema);
                    }
                }
            }
        }
        encoded(self)?;
        Ok(())
    }
    /// Validate one borrowed assignment without cloning a potentially invalid value.
    pub fn validate_value(&self, axis: &str, value: &Value) -> Result<(), ContextError> {
        self.validate()?;
        validate_axis(self.axes.get(axis).ok_or(ContextError::Assignment)?, value)
    }
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ContextError> {
        self.validate()?;
        let mut canonical = self.clone(); // Full bounded validation precedes clone.
        for kind in canonical.axes.values_mut() {
            if let ContextAxisType::Enum { members } = kind {
                members.sort();
            }
        }
        encoded(&canonical)
    }
    pub fn fingerprint(&self) -> Result<String, ContextError> {
        Ok(fingerprint(
            "weave-context-schema-v1",
            &self.canonical_bytes()?,
        ))
    }
}
impl ContextDefinition {
    pub fn from_json(bytes: &[u8]) -> Result<Self, ContextError> {
        let wire: DefinitionWire =
            serde_json::from_value(strict_json(bytes)?).map_err(|_| ContextError::Syntax)?;
        let definition = Self {
            schema: wire.schema.into(),
            values: wire.values,
        };
        definition.validate()?;
        Ok(definition)
    }
    pub fn validate(&self) -> Result<(), ContextError> {
        if self.values.len() > MAX_AXES {
            return Err(ContextError::Budget);
        }
        self.schema.validate()?;
        if self.values.len() != self.schema.axes.len() {
            return Err(ContextError::Assignment);
        }
        for (name, kind) in &self.schema.axes {
            let value = self.values.get(name).ok_or(ContextError::Assignment)?;
            validate_axis(kind, value)?;
        }
        encoded(self)?;
        Ok(())
    }
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ContextError> {
        self.validate()?;
        let mut canonical = self.clone();
        for kind in canonical.schema.axes.values_mut() {
            if let ContextAxisType::Enum { members } = kind {
                members.sort();
            }
        }
        encoded(&canonical)
    }
    pub fn fingerprint(&self) -> Result<String, ContextError> {
        Ok(fingerprint(
            "weave-context-definition-v1",
            &self.canonical_bytes()?,
        ))
    }
}
fn validate_axis(kind: &ContextAxisType, value: &Value) -> Result<(), ContextError> {
    let valid = match kind {
        ContextAxisType::Boolean => value.is_boolean(),
        ContextAxisType::Integer => value.as_i64().is_some(),
        ContextAxisType::String => value.as_str().is_some_and(|s| s.len() <= 4096),
        ContextAxisType::Decimal => Decimal::deserialize(value).is_ok(),
        ContextAxisType::Quantity { unit } => {
            // Validate bounded shape before a borrowed Deserialize can allocate strings.
            quantity_shape(value) && Quantity::deserialize(value).is_ok_and(|q| q.unit() == unit)
        }
        ContextAxisType::Enum { members } => value
            .as_str()
            .is_some_and(|s| members.iter().any(|m| m == s)),
    };
    if valid {
        Ok(())
    } else {
        Err(ContextError::Assignment)
    }
}
fn quantity_shape(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.len() != 2
        || object
            .get("amount")
            .is_none_or(|v| v.as_str().is_none_or(|s| s.len() > 128))
    {
        return false;
    }
    let Some(unit) = object.get("unit").and_then(Value::as_object) else {
        return false;
    };
    unit.len() == 3
        && ["dimension_id", "unit_id", "revision"].iter().all(|key| {
            unit.get(*key)
                .and_then(Value::as_str)
                .is_some_and(identifier)
        })
}
fn fingerprint(domain: &str, bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(domain.as_bytes());
    hash.update([0]);
    hash.update(bytes);
    format!("{hash:x}", hash = hash.finalize())
}
/// Bounded local conflict detection, not a trusted global schema registry.
#[derive(Default)]
pub struct ContextSchemas(BTreeMap<ContextSchemaRef, String>);
impl ContextSchemas {
    pub fn register(&mut self, schema: &ContextSchema) -> Result<String, ContextError> {
        let fingerprint = schema.fingerprint()?;
        if let Some(prior) = self.0.get(&schema.reference) {
            if prior != &fingerprint {
                return Err(ContextError::SchemaConflict);
            }
        } else {
            if self.0.len() >= MAX_SCHEMAS {
                return Err(ContextError::Budget);
            }
            self.0.insert(schema.reference.clone(), fingerprint.clone());
        }
        Ok(fingerprint)
    }
}

// The schema can occur inside a larger Program; its own decoder must charge bytes
// independently of the outer Program bound. Objects preserve duplicate members until checked.
struct Strict<'a> {
    depth: usize,
    remaining: &'a mut usize,
}
fn charge<E: serde::de::Error>(remaining: &mut usize, bytes: usize) -> Result<(), E> {
    *remaining = remaining
        .checked_sub(bytes)
        .ok_or_else(|| E::custom("context byte budget"))?;
    Ok(())
}
fn charge_value<E: serde::de::Error>(
    remaining: &mut usize,
    value: &impl Serialize,
) -> Result<(), E> {
    struct Counter<'a>(&'a mut usize);
    impl Write for Counter<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            *self.0 = self
                .0
                .checked_sub(bytes.len())
                .ok_or_else(|| std::io::Error::other("context byte budget"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter(remaining), value).map_err(|_| E::custom("context byte budget"))
}
impl<'de> DeserializeSeed<'de> for Strict<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        if self.depth > 8 {
            return Err(serde::de::Error::custom("context depth budget"));
        }
        d.deserialize_any(self)
    }
}
struct Key<'a>(&'a mut usize);
impl<'de> DeserializeSeed<'de> for Key<'_> {
    type Value = String;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<String, D::Error> {
        d.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for Key<'_> {
    type Value = String;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded context key")
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<String, E> {
        if value.len() > 512 {
            return Err(E::custom("context key budget"));
        }
        charge_value(self.0, &value)?;
        Ok(value.into())
    }
    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<String, E> {
        if value.len() > 512 {
            return Err(E::custom("context key budget"));
        }
        charge_value(self.0, &value)?;
        Ok(value)
    }
}
impl<'de> Visitor<'de> for Strict<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded duplicate-free context JSON")
    }
    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Value, E> {
        charge_value(self.remaining, &value)?;
        Ok(value.into())
    }
    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Value, E> {
        charge_value(self.remaining, &value)?;
        Ok(value.into())
    }
    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Value, E> {
        charge_value(self.remaining, &value)?;
        Ok(value.into())
    }
    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Value, E> {
        let number =
            serde_json::Number::from_f64(value).ok_or_else(|| E::custom("nonfinite number"))?;
        charge_value(self.remaining, &number)?;
        Ok(Value::Number(number))
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        charge(self.remaining, 4)?;
        Ok(Value::Null)
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Value, E> {
        if value.len() > 4096 {
            return Err(E::custom("context string budget"));
        }
        charge_value(self.remaining, &value)?;
        Ok(value.into())
    }
    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Value, E> {
        if value.len() > 4096 {
            return Err(E::custom("context string budget"));
        }
        charge_value(self.remaining, &value)?;
        Ok(Value::String(value))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        charge(self.remaining, 2)?;
        let mut object = Map::new();
        while let Some(key) = access.next_key_seed(Key(self.remaining))? {
            if object.len() >= MAX_ENUM {
                return Err(serde::de::Error::custom("context map budget"));
            }
            if object.contains_key(&key) {
                return Err(serde::de::Error::custom("duplicate context member"));
            }
            charge(self.remaining, if object.is_empty() { 1 } else { 2 })?;
            let value = access.next_value_seed(Strict {
                depth: self.depth + 1,
                remaining: self.remaining,
            })?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        charge(self.remaining, 2)?;
        let mut values = Vec::new();
        while let Some(value) = access.next_element_seed(Strict {
            depth: if values.len() >= MAX_ENUM {
                9
            } else {
                self.depth + 1
            },
            remaining: self.remaining,
        })? {
            if !values.is_empty() {
                charge(self.remaining, 1)?;
            }
            values.push(value);
        }
        Ok(Value::Array(values))
    }
}
impl<'de> Deserialize<'de> for ContextSchema {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let mut remaining = MAX_BYTES;
        let value = Strict {
            depth: 0,
            remaining: &mut remaining,
        }
        .deserialize(decoder)?;
        let wire: SchemaWire = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        let schema: Self = wire.into();
        schema.validate().map_err(serde::de::Error::custom)?;
        Ok(schema)
    }
}
impl<'de> Deserialize<'de> for ContextDefinition {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let mut remaining = MAX_BYTES;
        let value = Strict {
            depth: 0,
            remaining: &mut remaining,
        }
        .deserialize(decoder)?;
        let wire: DefinitionWire =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        let definition = Self {
            schema: wire.schema.into(),
            values: wire.values,
        };
        definition.validate().map_err(serde::de::Error::custom)?;
        Ok(definition)
    }
}

fn strict_json(bytes: &[u8]) -> Result<Value, ContextError> {
    if bytes.len() > MAX_BYTES {
        return Err(ContextError::Budget);
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let mut remaining = MAX_BYTES;
    let value = Strict {
        depth: 0,
        remaining: &mut remaining,
    }
    .deserialize(&mut decoder)
    .map_err(|_| ContextError::Syntax)?;
    decoder.end().map_err(|_| ContextError::Syntax)?;
    Ok(value)
}
