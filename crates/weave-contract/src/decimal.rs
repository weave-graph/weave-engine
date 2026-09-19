//! Exact bounded decimal arithmetic and canonical protocol 0.12 scalar.
//! Values have <=18 significant coefficient digits and <=18 fractional places.
//! No operation rounds or converts through binary floating point.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};

const MAX_COEFFICIENT: i128 = 999_999_999_999_999_999;
const MAX_SCALE: i32 = 18;
const MAX_INPUT: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecimalError {
    Syntax,
    InputLimit,
    Precision,
    Scale,
    DivisionByZero,
    NonTerminating,
    NonCanonical,
}
impl fmt::Display for DecimalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DecimalError {}

/// Normalized exact decimal; private fields prevent invalid/noncanonical values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Decimal {
    coefficient: i128,
    scale: u8,
}
impl Decimal {
    pub const ZERO: Self = Self {
        coefficient: 0,
        scale: 0,
    };
    fn normalized(mut coefficient: i128, mut scale: i32) -> Result<Self, DecimalError> {
        if coefficient == 0 {
            return Ok(Self::ZERO);
        }
        while scale > 0 && coefficient % 10 == 0 {
            coefficient /= 10;
            scale -= 1;
        }
        if scale > MAX_SCALE {
            return Err(DecimalError::Scale);
        }
        while scale < 0 {
            if coefficient.abs() > MAX_COEFFICIENT / 10 {
                return Err(DecimalError::Precision);
            }
            coefficient *= 10;
            scale += 1;
        }
        if coefficient.abs() > MAX_COEFFICIENT {
            return Err(DecimalError::Precision);
        }
        Ok(Self {
            coefficient,
            scale: scale as u8,
        })
    }
    pub fn checked_add(self, rhs: Self) -> Result<Self, DecimalError> {
        let scale = self.scale.max(rhs.scale);
        // At most 18+18 digits, so these signed i128 intermediates cannot overflow.
        let left = self.coefficient * 10_i128.pow((scale - self.scale).into());
        let right = rhs.coefficient * 10_i128.pow((scale - rhs.scale).into());
        Self::normalized(left + right, scale.into())
    }
    pub fn checked_sub(self, rhs: Self) -> Result<Self, DecimalError> {
        self.checked_add(Self {
            coefficient: -rhs.coefficient,
            ..rhs
        })
    }
    pub fn checked_mul(self, rhs: Self) -> Result<Self, DecimalError> {
        Self::normalized(
            self.coefficient * rhs.coefficient,
            i32::from(self.scale) + i32::from(rhs.scale),
        )
    }
    pub fn checked_div(self, rhs: Self) -> Result<Self, DecimalError> {
        Self::quotient(
            self.coefficient,
            rhs.coefficient,
            i32::from(self.scale) - i32::from(rhs.scale),
        )
    }
    /// Exact rational scaling; cancellation precedes result precision checks.
    pub fn checked_mul_ratio(
        self,
        numerator: Self,
        denominator: Self,
    ) -> Result<Self, DecimalError> {
        Self::quotient(
            self.coefficient * numerator.coefficient,
            denominator.coefficient,
            i32::from(self.scale) + i32::from(numerator.scale) - i32::from(denominator.scale),
        )
    }
    pub fn is_positive(self) -> bool {
        self.coefficient > 0
    }
    fn quotient(
        mut numerator: i128,
        denominator: i128,
        mut scale: i32,
    ) -> Result<Self, DecimalError> {
        if denominator == 0 {
            return Err(DecimalError::DivisionByZero);
        }
        if numerator == 0 {
            return Ok(Self::ZERO);
        }
        let divisor = gcd(numerator.abs(), denominator.abs());
        numerator /= divisor;
        if denominator < 0 {
            numerator = -numerator;
        }
        let mut denominator = denominator.abs() / divisor;
        while numerator % 10 == 0 {
            numerator /= 10;
            scale -= 1;
        }
        let (mut twos, mut fives) = (0, 0);
        while denominator % 2 == 0 {
            denominator /= 2;
            twos += 1;
        }
        while denominator % 5 == 0 {
            denominator /= 5;
            fives += 1;
        }
        if denominator != 1 {
            return Err(DecimalError::NonTerminating);
        }
        let places = twos.max(fives);
        scale += places;
        if scale > MAX_SCALE {
            return Err(DecimalError::Scale);
        }
        for factor in std::iter::repeat_n(2, (places - twos) as usize)
            .chain(std::iter::repeat_n(5, (places - fives) as usize))
        {
            if numerator.abs() > MAX_COEFFICIENT / factor {
                return Err(DecimalError::Precision);
            }
            numerator *= factor;
        }
        Self::normalized(numerator, scale)
    }
}
fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
impl FromStr for Decimal {
    type Err = DecimalError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() > MAX_INPUT {
            return Err(DecimalError::InputLimit);
        }
        let bytes = text.as_bytes();
        let mut at = 0;
        let negative = bytes.first() == Some(&b'-');
        if negative || bytes.first() == Some(&b'+') {
            at += 1;
        }
        let mut digits = String::new();
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            digits.push(bytes[at] as char);
            at += 1;
        }
        if at == start {
            return Err(DecimalError::Syntax);
        }
        let mut fraction = 0_i32;
        if bytes.get(at) == Some(&b'.') {
            at += 1;
            let start = at;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                digits.push(bytes[at] as char);
                at += 1;
                fraction += 1;
            }
            if at == start {
                return Err(DecimalError::Syntax);
            }
        }
        let mut exponent = 0_i32;
        if matches!(bytes.get(at), Some(b'e' | b'E')) {
            at += 1;
            let negative_exponent = bytes.get(at) == Some(&b'-');
            if negative_exponent || bytes.get(at) == Some(&b'+') {
                at += 1;
            }
            let start = at;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                exponent = exponent
                    .checked_mul(10)
                    .and_then(|e| e.checked_add(i32::from(bytes[at] - b'0')))
                    .ok_or(DecimalError::InputLimit)?;
                if exponent > 128 {
                    return Err(DecimalError::InputLimit);
                }
                at += 1;
            }
            if start == at {
                return Err(DecimalError::Syntax);
            }
            if negative_exponent {
                exponent = -exponent;
            }
        }
        if at != bytes.len() {
            return Err(DecimalError::Syntax);
        }
        let digits = digits.trim_start_matches('0');
        if digits.is_empty() {
            return Ok(Self::ZERO);
        }
        let trimmed = digits.trim_end_matches('0');
        let scale = fraction - exponent - (digits.len() - trimmed.len()) as i32;
        if trimmed.len() > 18 {
            return Err(DecimalError::Precision);
        }
        let coefficient = trimmed
            .parse::<i128>()
            .map_err(|_| DecimalError::Precision)?;
        Self::normalized(if negative { -coefficient } else { coefficient }, scale)
    }
}
impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.coefficient < 0 {
            f.write_str("-")?;
        }
        let digits = self.coefficient.abs().to_string();
        let scale = usize::from(self.scale);
        if scale == 0 {
            return f.write_str(&digits);
        }
        if digits.len() <= scale {
            f.write_str("0.")?;
            for _ in digits.len()..scale {
                f.write_str("0")?;
            }
            f.write_str(&digits)
        } else {
            let split = digits.len() - scale;
            write!(f, "{}.{}", &digits[..split], &digits[split..])
        }
    }
}
impl Serialize for Decimal {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct DecimalVisitor;
        impl serde::de::Visitor<'_> for DecimalVisitor {
            type Value = Decimal;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a canonical bounded decimal string")
            }
            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Decimal, E> {
                let value: Decimal = text.parse().map_err(E::custom)?;
                if value.to_string() != text {
                    return Err(E::custom(DecimalError::NonCanonical));
                }
                Ok(value)
            }
        }
        deserializer.deserialize_str(DecimalVisitor)
    }
}
