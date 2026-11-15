//! Verifikationsserier och referenser som "A12".

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Serierna börjar om på 1 varje räkenskapsår.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Series {
    /// Löpande bokföring.
    A,
    /// Bokslut.
    B,
    /// Ingående balans.
    I,
    /// Rättelser.
    R,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SeriesError {
    #[error("okänd serie \"{0}\" (A, B, I eller R)")]
    UnknownSeries(String),
    #[error("\"{0}\" är ingen verifikation (skriv till exempel A12)")]
    InvalidRef(String),
}

impl Series {
    pub const ALL: [Series; 4] = [Series::A, Series::B, Series::I, Series::R];

    pub fn as_str(self) -> &'static str {
        match self {
            Series::A => "A",
            Series::B => "B",
            Series::I => "I",
            Series::R => "R",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Series::A => "löpande bokföring",
            Series::B => "bokslut",
            Series::I => "ingående balans",
            Series::R => "rättelser",
        }
    }
}

impl FromStr for Series {
    type Err = SeriesError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "A" | "a" => Ok(Series::A),
            "B" | "b" => Ok(Series::B),
            "I" | "i" => Ok(Series::I),
            "R" | "r" => Ok(Series::R),
            other => Err(SeriesError::UnknownSeries(other.to_string())),
        }
    }
}

impl fmt::Display for Series {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// En verifikation inom ett räkenskapsår, till exempel A12.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VoucherRef {
    pub series: Series,
    pub number: i64,
}

impl FromStr for VoucherRef {
    type Err = SeriesError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        let invalid = || SeriesError::InvalidRef(t.to_string());
        let mut chars = t.chars();
        let first = chars.next().ok_or_else(invalid)?;
        let series: Series = first.to_string().parse().map_err(|_| invalid())?;
        let digits = chars.as_str();
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(invalid());
        }
        let number: i64 = digits.parse().map_err(|_| invalid())?;
        if number < 1 {
            return Err(invalid());
        }
        Ok(VoucherRef { series, number })
    }
}

impl fmt::Display for VoucherRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.series, self.number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_voucher_refs() {
        let r: VoucherRef = "A12".parse().unwrap();
        assert_eq!(
            r,
            VoucherRef {
                series: Series::A,
                number: 12
            }
        );
        assert_eq!(r.to_string(), "A12");
        assert_eq!("r4".parse::<VoucherRef>().unwrap().series, Series::R);
        assert!("A".parse::<VoucherRef>().is_err());
        assert!("A0".parse::<VoucherRef>().is_err());
        assert!("X1".parse::<VoucherRef>().is_err());
        assert!("A1b".parse::<VoucherRef>().is_err());
    }
}
