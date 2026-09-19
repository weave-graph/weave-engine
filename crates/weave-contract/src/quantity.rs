//! Exact nominal quantities. Pure arithmetic does not attest conversion authority.
use crate::decimal::{Decimal, DecimalError};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuantityError {
    Descriptor,
    UnitMismatch,
    DimensionMismatch,
    InvalidFactor,
    Decimal(DecimalError),
}
impl fmt::Display for QuantityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for QuantityError {}
impl From<DecimalError> for QuantityError {
    fn from(value: DecimalError) -> Self {
        Self::Decimal(value)
    }
}

/// Exact nominal identity; familiar unit names do not establish compatibility.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct UnitDescriptor {
    dimension_id: String,
    unit_id: String,
    revision: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorWire {
    dimension_id: String,
    unit_id: String,
    revision: String,
}
impl UnitDescriptor {
    pub fn new(
        dimension_id: String,
        unit_id: String,
        revision: String,
    ) -> Result<Self, QuantityError> {
        if [&dimension_id, &unit_id, &revision]
            .iter()
            .any(|s| s.is_empty() || s.len() > 512 || s.chars().any(char::is_control))
        {
            return Err(QuantityError::Descriptor);
        }
        Ok(Self {
            dimension_id,
            unit_id,
            revision,
        })
    }
    pub fn dimension_id(&self) -> &str {
        &self.dimension_id
    }
    pub fn unit_id(&self) -> &str {
        &self.unit_id
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
}
impl<'de> Deserialize<'de> for UnitDescriptor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = DescriptorWire::deserialize(deserializer)?;
        Self::new(wire.dimension_id, wire.unit_id, wire.revision).map_err(serde::de::Error::custom)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Quantity {
    amount: Decimal,
    unit: UnitDescriptor,
}
impl Quantity {
    pub fn new(amount: Decimal, unit: UnitDescriptor) -> Self {
        Self { amount, unit }
    }
    pub fn amount(&self) -> Decimal {
        self.amount
    }
    pub fn unit(&self) -> &UnitDescriptor {
        &self.unit
    }
    pub fn checked_add(&self, rhs: &Self) -> Result<Self, QuantityError> {
        if self.unit != rhs.unit {
            return Err(QuantityError::UnitMismatch);
        }
        Ok(Self::new(
            self.amount.checked_add(rhs.amount)?,
            self.unit.clone(),
        ))
    }
    pub fn checked_sub(&self, rhs: &Self) -> Result<Self, QuantityError> {
        if self.unit != rhs.unit {
            return Err(QuantityError::UnitMismatch);
        }
        Ok(Self::new(
            self.amount.checked_sub(rhs.amount)?,
            self.unit.clone(),
        ))
    }
    pub fn checked_mul(&self, scalar: Decimal) -> Result<Self, QuantityError> {
        Ok(Self::new(
            self.amount.checked_mul(scalar)?,
            self.unit.clone(),
        ))
    }
    pub fn checked_div(&self, scalar: Decimal) -> Result<Self, QuantityError> {
        Ok(Self::new(
            self.amount.checked_div(scalar)?,
            self.unit.clone(),
        ))
    }
    pub fn convert(&self, conversion: &RationalConversion) -> Result<Self, QuantityError> {
        if self.unit != conversion.from {
            return Err(QuantityError::UnitMismatch);
        }
        Ok(Self::new(
            self.amount
                .checked_mul_ratio(conversion.numerator, conversion.denominator)?,
            conversion.to.clone(),
        ))
    }
}
/// Positive multiplicative factor only. No offset, similarity or authority inference.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RationalConversion {
    from: UnitDescriptor,
    to: UnitDescriptor,
    numerator: Decimal,
    denominator: Decimal,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversionWire {
    from: UnitDescriptor,
    to: UnitDescriptor,
    numerator: Decimal,
    denominator: Decimal,
}
impl RationalConversion {
    pub fn new(
        from: UnitDescriptor,
        to: UnitDescriptor,
        numerator: Decimal,
        denominator: Decimal,
    ) -> Result<Self, QuantityError> {
        if from.dimension_id != to.dimension_id {
            return Err(QuantityError::DimensionMismatch);
        }
        if !numerator.is_positive() || !denominator.is_positive() {
            return Err(QuantityError::InvalidFactor);
        }
        Ok(Self {
            from,
            to,
            numerator,
            denominator,
        })
    }
    pub fn source(&self) -> &UnitDescriptor {
        &self.from
    }
    pub fn target(&self) -> &UnitDescriptor {
        &self.to
    }
}
impl<'de> Deserialize<'de> for RationalConversion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ConversionWire::deserialize(deserializer)?;
        Self::new(wire.from, wire.to, wire.numerator, wire.denominator)
            .map_err(serde::de::Error::custom)
    }
}
