//! Duplicate-aware preflight, including decoded keys inside opaque scalar/property JSON.
use serde::de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor};
use std::collections::BTreeSet;
use std::fmt;

pub(crate) fn check(bytes: &[u8], limit: usize) -> Result<(), &'static str> {
    if bytes.len() > limit {
        return Err("E_HOST_BUDGET");
    }
    let mut remaining = 1_000_000usize;
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    Scan {
        depth: 0,
        remaining: &mut remaining,
    }
    .deserialize(&mut decoder)
    .map_err(|e| {
        if e.to_string().contains("limit") {
            "E_HOST_BUDGET"
        } else {
            "E_HOST_INPUT"
        }
    })?;
    decoder.end().map_err(|_| "E_HOST_INPUT")
}
struct Scan<'a> {
    depth: usize,
    remaining: &'a mut usize,
}
impl<'de> DeserializeSeed<'de> for Scan<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        if self.depth > 120 || *self.remaining == 0 {
            return Err(D::Error::custom("JSON work limit"));
        }
        *self.remaining -= 1;
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Scan<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded duplicate-free JSON")
    }
    fn visit_bool<E: Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<(), A::Error> {
        while a
            .next_element_seed(Scan {
                depth: self.depth + 1,
                remaining: self.remaining,
            })?
            .is_some()
        {}
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<(), A::Error> {
        let mut keys = BTreeSet::new();
        while let Some(key) = a.next_key::<String>()? {
            if !keys.insert(key) {
                return Err(A::Error::custom("duplicate JSON key"));
            }
            a.next_value_seed(Scan {
                depth: self.depth + 1,
                remaining: self.remaining,
            })?;
        }
        Ok(())
    }
}
