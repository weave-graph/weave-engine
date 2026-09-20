//! Lossless SDK artifact selection. A fingerprint is descriptive, never authority.
use crate::{host::HostError, strict_json};
use serde::{Deserialize, Serialize};
use serde_json::{value::RawValue, Value};
use std::collections::BTreeMap;
use weave_contract::{CompiledHandlerTemplate, CompiledViewTemplate, Program};

/// Separate from the operational request cap: includes the SDK's envelope allowance.
pub const SDK_RESPONSE_LIMIT: usize = 16 * 1024 * 1024 + 4096;
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArtifactInventory {
    pub artifact_fingerprint: String,
    pub values: Vec<String>,
    pub view_templates: Vec<String>,
    pub handler_templates: Vec<String>,
}
/// Owns the exact entire successful SDK response, including unselected scalar values.
pub struct ArtifactBundle {
    bytes: Vec<u8>,
    inventory: ArtifactInventory,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Success<'a> {
    format: String,
    ok: bool,
    artifact_fingerprint: String,
    #[serde(borrow)]
    artifacts: &'a RawValue,
}
struct Artifacts<'a> {
    program: &'a RawValue,
    values: BTreeMap<String, &'a RawValue>,
    view_templates: BTreeMap<String, &'a RawValue>,
    handler_templates: Option<BTreeMap<String, &'a RawValue>>,
}
struct Key;
impl<'de> serde::de::DeserializeSeed<'de> for Key {
    type Value = String;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<String, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = String;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bounded artifact key")
            }
            fn visit_str<E: serde::de::Error>(self, key: &str) -> Result<String, E> {
                if key.is_empty() || key.len() > 512 {
                    return Err(E::custom("artifact key limit"));
                }
                Ok(key.to_owned())
            }
        }
        d.deserialize_str(V)
    }
}
struct InventoryMap<'a>(&'a mut usize);
impl<'de> serde::de::DeserializeSeed<'de> for InventoryMap<'_> {
    type Value = BTreeMap<String, &'de RawValue>;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de> serde::de::Visitor<'de> for InventoryMap<'_> {
    type Value = BTreeMap<String, &'de RawValue>;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("bounded artifact inventory")
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
        let mut map = BTreeMap::new();
        while let Some(key) = a.next_key_seed(Key)? {
            if *self.0 == 0 {
                return Err(serde::de::Error::custom("aggregate inventory limit"));
            }
            *self.0 -= 1;
            let value = a.next_value()?;
            if map.insert(key, value).is_some() {
                return Err(serde::de::Error::custom("duplicate artifact"));
            }
        }
        Ok(map)
    }
}
impl<'de> Deserialize<'de> for Artifacts<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Artifacts<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("complete SDK artifact object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut a: A,
            ) -> Result<Self::Value, A::Error> {
                let (mut program, mut values, mut views, mut handlers) = (None, None, None, None);
                let mut remaining = 1000;
                while let Some(key) = a.next_key_seed(Key)? {
                    match key.as_str() {
                        "program" if program.is_none() => program = Some(a.next_value()?),
                        "values" if values.is_none() => {
                            values = Some(a.next_value_seed(InventoryMap(&mut remaining))?)
                        }
                        "view_templates" if views.is_none() => {
                            views = Some(a.next_value_seed(InventoryMap(&mut remaining))?)
                        }
                        "handler_templates" if handlers.is_none() => {
                            handlers = Some(a.next_value_seed(InventoryMap(&mut remaining))?)
                        }
                        _ => {
                            return Err(serde::de::Error::custom(
                                "unknown or duplicate artifact field",
                            ))
                        }
                    }
                }
                Ok(Artifacts {
                    program: program.ok_or_else(|| serde::de::Error::missing_field("program"))?,
                    values: values.ok_or_else(|| serde::de::Error::missing_field("values"))?,
                    view_templates: views
                        .ok_or_else(|| serde::de::Error::missing_field("view_templates"))?,
                    handler_templates: handlers,
                })
            }
        }
        d.deserialize_map(V)
    }
}
fn invalid() -> HostError {
    HostError::new("E_HOST_ARTIFACT", "invalid complete compiler response")
}
fn decode<'a>(bytes: &'a [u8]) -> Result<(Success<'a>, Artifacts<'a>), HostError> {
    let success: Success = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let artifacts: Artifacts = serde_json::from_str(success.artifacts.get()).map_err(|e| {
        if e.to_string().contains("limit") {
            HostError::new("E_HOST_BUDGET", "artifact inventory limit exceeded")
        } else {
            invalid()
        }
    })?;
    Ok((success, artifacts))
}
impl ArtifactBundle {
    pub fn parse(bytes: &[u8]) -> Result<Self, HostError> {
        if bytes.len() > SDK_RESPONSE_LIMIT {
            return Err(HostError::new(
                "E_HOST_BUDGET",
                "compiler response exceeds byte limit",
            ));
        }
        // Bound inventory before the recursive pass retains any of its duplicate-key sets.
        // Fixed structs/manual maps reject duplicates here; opaque payloads remain borrowed.
        let (success, artifacts) = decode(bytes)?;
        strict_json::check(bytes, SDK_RESPONSE_LIMIT)
            .map_err(|c| HostError::new(c, "invalid compiler response bytes"))?;
        if success.format != "weave-compiler-response/1"
            || !success.ok
            || artifacts
                .handler_templates
                .as_ref()
                .is_some_and(BTreeMap::is_empty)
        {
            return Err(invalid());
        }
        let handlers = artifacts.handler_templates.as_ref();
        let _: Program = serde_json::from_str(artifacts.program.get()).map_err(|_| invalid())?;
        for (name, raw) in &artifacts.view_templates {
            let template: CompiledViewTemplate =
                serde_json::from_str(raw.get()).map_err(|_| invalid())?;
            if &template.name != name {
                return Err(invalid());
            }
            weave_contract::view_registration::validate_template(&template)
                .map_err(|_| invalid())?;
        }
        for (name, raw) in handlers.into_iter().flat_map(|m| m.iter()) {
            let template: CompiledHandlerTemplate =
                serde_json::from_str(raw.get()).map_err(|_| invalid())?;
            if &template.name != name {
                return Err(invalid());
            }
            weave_contract::handler_registration::validate_handler_template(&template)
                .map_err(|_| invalid())?;
        }
        // Value is safe only after the recursive duplicate-key pass above. Rust retains i64/u64.
        let value: Value = serde_json::from_str(success.artifacts.get()).map_err(|_| invalid())?;
        let fingerprint = weave_contract::identity::source_fingerprint(&serde_json::json!({
            "profile": if handlers.is_none() { "weave-compiled-artifacts-v1" } else { "weave-compiled-artifacts-v2" },
            "artifacts": value
        })).map_err(|_| invalid())?;
        if fingerprint != success.artifact_fingerprint {
            return Err(invalid());
        }
        let inventory = ArtifactInventory {
            artifact_fingerprint: fingerprint,
            values: artifacts.values.keys().cloned().collect(),
            view_templates: artifacts.view_templates.keys().cloned().collect(),
            handler_templates: handlers
                .into_iter()
                .flat_map(|m| m.keys().cloned())
                .collect(),
        };
        Ok(Self {
            bytes: bytes.to_vec(),
            inventory,
        })
    }
    pub fn original_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn inventory(&self) -> &ArtifactInventory {
        &self.inventory
    }
    pub fn program_bytes(&self) -> &[u8] {
        decode(&self.bytes)
            .expect("validated immutable bundle")
            .1
            .program
            .get()
            .as_bytes()
    }
    pub fn view_bytes(&self, name: &str) -> Option<&[u8]> {
        decode(&self.bytes)
            .ok()?
            .1
            .view_templates
            .get(name)
            .map(|&r| r.get().as_bytes())
    }
    pub fn handler_bytes(&self, name: &str) -> Option<&[u8]> {
        decode(&self.bytes)
            .ok()?
            .1
            .handler_templates?
            .get(name)
            .map(|&r| r.get().as_bytes())
    }
}
