//! Actual native artifact version one: a nondeterministic sample and pinned input.
use serde_json::{json, Value};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
pub fn compute(event: &str, input: &Value) -> (String, Value) {
    let mut random = RandomState::new().build_hasher();
    random.write(event.as_bytes());
    let sample = random.finish().to_string();
    let value = json!({"sample":sample,"input":input});
    (sample, value)
}
