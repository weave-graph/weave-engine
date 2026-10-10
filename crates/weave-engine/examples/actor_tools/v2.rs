//! Actual native artifact version two, preserving the sample/input state ABI.
use serde_json::{json, Value};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
pub fn compute(event: &str, input: &Value) -> (String, Value) {
    let mut random = RandomState::new().build_hasher();
    random.write(b"actor-version-two:");
    random.write(event.as_bytes());
    let sample = format!("v2:{}", random.finish());
    let value = json!({"sample":sample,"input":input});
    (sample, value)
}
