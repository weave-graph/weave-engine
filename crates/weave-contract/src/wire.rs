//! Core contract v1 wire boundary: strict JSON decoding with stable error codes.
//!
//! See `docs/contract/core-v1.md` ("Error codes"). Decoding is I/O-free and
//! checks, in this order: input size, strict JSON syntax (no duplicate object
//! keys, finite numbers), the declared `version` (programs only), then the
//! typed structure. A program whose version is unknown is never decoded
//! structurally, so a newer program is never partially reinterpreted.
use crate::{Program, LEGACY_VERSION, VERSION};
use serde::{
    de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use serde_json::{Map, Number, Value};
use std::fmt;

/// Every `Program.version` this contract decodes, newest first.
///
/// Each version keeps the exact semantics it was published with. Features
/// newer than a program's declared version are rejected at execution with
/// [`E_VERSION`].
pub const SUPPORTED_VERSIONS: &[&str] = &[
    VERSION,
    "0.22.0",
    "0.21.0",
    "0.20.0",
    "0.19.0",
    "0.18.0",
    "0.17.0",
    "0.16.0",
    "0.15.0",
    "0.14.0",
    "0.13.0",
    "0.12.0",
    "0.11.0",
    "0.10.0",
    "0.9.0",
    "0.8.0",
    "0.7.0",
    "0.6.0",
    "0.5.0",
    "0.4.0",
    "0.3.0",
    "0.2.0",
    LEGACY_VERSION,
];

/// Name of the consolidated core specification implemented by this crate.
pub const CORE_PROFILE: &str = "core-v1";

/// Largest encoded program or record accepted by [`decode_program`] and [`decode`].
pub const MAX_DECODE_BYTES: usize = 16 * 1024 * 1024;

/// The version is missing, not a string, unknown, or too old for a feature.
pub const E_VERSION: &str = "E_VERSION";
/// An object contains a field that its declared type does not define.
pub const E_UNKNOWN_FIELD: &str = "E_UNKNOWN_FIELD";
/// A tagged value (`op`, `kind`, enum string) names a variant that does not exist.
pub const E_UNKNOWN_VARIANT: &str = "E_UNKNOWN_VARIANT";
/// Malformed JSON, duplicate keys, wrong JSON types, missing fields or bounded-decoder limits.
pub const E_DECODE: &str = "E_DECODE";
/// The encoded input exceeds [`MAX_DECODE_BYTES`].
pub const E_BUDGET: &str = "E_BUDGET";

/// A stable, machine-readable decoding failure. Match on `code`, never on `message`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractError {
    pub code: String,
    pub message: String,
}
impl ContractError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ContractError {}

/// True when `version` is a contract version this crate decodes.
pub fn is_supported_version(version: &str) -> bool {
    SUPPORTED_VERSIONS.contains(&version)
}

/// Decode a [`Program`], checking its declared version before its structure.
pub fn decode_program(bytes: &[u8]) -> Result<Program, ContractError> {
    let value = strict_value(bytes)?;
    let version = match value.as_object().map(|object| object.get("version")) {
        None => {
            return Err(ContractError::new(
                E_DECODE,
                "program must be a JSON object",
            ))
        }
        Some(None) => return Err(ContractError::new(E_VERSION, "contract version required")),
        Some(Some(Value::String(version))) => version.clone(),
        Some(Some(_)) => {
            return Err(ContractError::new(
                E_VERSION,
                "contract version must be a string",
            ))
        }
    };
    if !is_supported_version(&version) {
        return Err(ContractError::new(
            E_VERSION,
            format!("unsupported contract version {}", bounded(&version)),
        ));
    }
    typed(value)
}

/// Decode any versionless contract record (for example `GraphData` or
/// `FactProvenance`) with the same strict syntax and error codes.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ContractError> {
    typed(strict_value(bytes)?)
}

/// Classify a structural decoding failure into a stable code.
pub fn classify(error: &serde_json::Error) -> ContractError {
    let message = error.to_string();
    // serde's derive emits these exact prefixes for `deny_unknown_fields`
    // and unknown enum tags; the fixture harness pins this behaviour.
    let code = if message.starts_with("unknown field") {
        E_UNKNOWN_FIELD
    } else if message.starts_with("unknown variant") {
        E_UNKNOWN_VARIANT
    } else {
        E_DECODE
    };
    ContractError::new(code, message)
}

fn typed<T: DeserializeOwned>(value: Value) -> Result<T, ContractError> {
    serde_json::from_value(value).map_err(|error| classify(&error))
}

fn bounded(text: &str) -> String {
    text.chars().take(64).collect()
}

fn strict_value(bytes: &[u8]) -> Result<Value, ContractError> {
    if bytes.len() > MAX_DECODE_BYTES {
        return Err(ContractError::new(E_BUDGET, "input exceeds 16 MiB"));
    }
    let StrictValue(value) = serde_json::from_slice(bytes)
        .map_err(|error| ContractError::new(E_DECODE, error.to_string()))?;
    Ok(value)
}

/// JSON value that rejects duplicate object keys instead of keeping the last one.
struct StrictValue(Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StrictVisitor)
    }
}
struct StrictVisitor;
impl<'de> Visitor<'de> for StrictVisitor {
    type Value = StrictValue;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("JSON without duplicate object keys")
    }
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Bool(value)))
    }
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(Number::from(value))))
    }
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(Number::from(value))))
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        Number::from_f64(value)
            .map(|n| StrictValue(Value::Number(n)))
            .ok_or_else(|| E::custom("finite JSON number required"))
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value.into())))
    }
    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value)))
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut values = vec![];
        while let Some(StrictValue(value)) = access.next_element()? {
            values.push(value);
        }
        Ok(StrictValue(Value::Array(values)))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = access.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom(format!(
                    "duplicate JSON object key {}",
                    bounded(&key)
                )));
            }
            let StrictValue(value) = access.next_value()?;
            values.insert(key, value);
        }
        Ok(StrictValue(Value::Object(values)))
    }
}
