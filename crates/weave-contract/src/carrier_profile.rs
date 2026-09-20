//! Strict bounded decoding for the new record/value alternative carriers.
//! Existing edge/assertion group decoding retains its historical profile.
use crate::Derivation;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use std::cell::Cell;
use std::marker::PhantomData;

pub(crate) const MAX_GROUPS: usize = 128;
const MAX_REFS: usize = 1000;

struct Refs<'a, T>(&'a Cell<usize>, PhantomData<T>);
struct Ref<'a, T>(&'a Cell<usize>, PhantomData<T>);
impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for Ref<'_, T> {
    type Value = T;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<T, D::Error> {
        let remaining = self
            .0
            .get()
            .checked_sub(1)
            .ok_or_else(|| de::Error::custom("carrier reference limit"))?;
        self.0.set(remaining);
        T::deserialize(d)
    }
}
impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for Refs<'_, T> {
    type Value = Vec<T>;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Vec<T>, D::Error> {
        d.deserialize_seq(self)
    }
}
impl<'de, T: Deserialize<'de>> Visitor<'de> for Refs<'_, T> {
    type Value = Vec<T>;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("bounded carrier references")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
        if seq.size_hint().is_some_and(|n| n > self.0.get()) {
            return Err(de::Error::custom("carrier reference limit"));
        }
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(Ref(self.0, PhantomData))? {
            values.push(value);
        }
        Ok(values)
    }
}
struct Group<'a>(&'a Cell<usize>);
impl<'de> DeserializeSeed<'de> for Group<'_> {
    type Value = Derivation;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Derivation, D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for Group<'_> {
    type Value = Derivation;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a strict bounded derivation")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Derivation, A::Error> {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Operator,
            Premises,
            NodePremises,
            SnapshotPremises,
            Parameters,
            InputSnapshots,
        }
        let (mut operator, mut premises, mut nodes, mut snapshots, mut parameters, mut inputs) =
            (None, None, None, None, None, None);
        while let Some(field) = map.next_key::<Field>()? {
            match field {
                Field::Operator => {
                    if operator.is_some() {
                        return Err(de::Error::duplicate_field("operator"));
                    }
                    operator = Some(map.next_value()?);
                }
                Field::Premises => {
                    if premises.is_some() {
                        return Err(de::Error::duplicate_field("premises"));
                    }
                    premises = Some(map.next_value_seed(Refs(self.0, PhantomData))?);
                }
                Field::NodePremises => {
                    if nodes.is_some() {
                        return Err(de::Error::duplicate_field("node_premises"));
                    }
                    nodes = Some(map.next_value_seed(Refs(self.0, PhantomData))?);
                }
                Field::SnapshotPremises => {
                    if snapshots.is_some() {
                        return Err(de::Error::duplicate_field("snapshot_premises"));
                    }
                    snapshots = Some(map.next_value_seed(Refs(self.0, PhantomData))?);
                }
                Field::Parameters => {
                    if parameters.is_some() {
                        return Err(de::Error::duplicate_field("parameters"));
                    }
                    parameters = Some(map.next_value()?);
                }
                Field::InputSnapshots => {
                    if inputs.is_some() {
                        return Err(de::Error::duplicate_field("input_snapshots"));
                    }
                    // Descriptive pins have a separate bound and confer no authority.
                    inputs = Some(map.next_value_seed(Refs(&Cell::new(MAX_REFS), PhantomData))?);
                }
            }
        }
        Ok(Derivation {
            operator: operator.ok_or_else(|| de::Error::missing_field("operator"))?,
            premises: premises.ok_or_else(|| de::Error::missing_field("premises"))?,
            node_premises: nodes.unwrap_or_default(),
            snapshot_premises: snapshots.unwrap_or_default(),
            parameters: parameters.unwrap_or_default(),
            input_snapshots: inputs.unwrap_or_default(),
        })
    }
}
pub(crate) fn bounded_derivations<'de, D: de::Deserializer<'de>>(
    d: D,
) -> Result<Vec<Derivation>, D::Error> {
    struct Groups;
    impl<'de> Visitor<'de> for Groups {
        type Value = Vec<Derivation>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("at most 128 alternatives and 1000 total gate references")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            if seq.size_hint().is_some_and(|n| n > MAX_GROUPS) {
                return Err(de::Error::custom("carrier group limit"));
            }
            let remaining = Cell::new(MAX_REFS);
            let mut groups = Vec::new();
            struct Next<'a> {
                refs: &'a Cell<usize>,
                count: usize,
            }
            impl<'de> DeserializeSeed<'de> for Next<'_> {
                type Value = Derivation;
                fn deserialize<D: de::Deserializer<'de>>(
                    self,
                    d: D,
                ) -> Result<Derivation, D::Error> {
                    if self.count == MAX_GROUPS {
                        return Err(de::Error::custom("carrier group limit"));
                    }
                    Group(self.refs).deserialize(d)
                }
            }
            while let Some(group) = seq.next_element_seed(Next {
                refs: &remaining,
                count: groups.len(),
            })? {
                groups.push(group);
            }
            Ok(groups)
        }
    }
    d.deserialize_seq(Groups)
}

/// Typed feature detection: literal property keys never enable this profile.
pub fn requires_v019(data: &crate::GraphData) -> bool {
    data.influence
        .as_ref()
        .is_some_and(|i| !i.derivations.is_empty())
        || data.nodes.iter().any(|n| !n.derivations.is_empty())
        || data.attachments.iter().any(|a| !a.derivations.is_empty())
        || data
            .edges
            .iter()
            .flat_map(|e| &e.derivations)
            .any(|d| !d.snapshot_premises.is_empty())
        || data
            .assertions
            .iter()
            .flat_map(|a| &a.derivations)
            .any(|d| !d.snapshot_premises.is_empty())
}
