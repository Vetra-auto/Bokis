//! Belopp i öre. All aritmetik sker i heltal.

use std::fmt;
use std::ops::{Neg, Sub};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Ett belopp i öre (10050 = 100,50 kr).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Money(i64);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MoneyError {
    #[error("\"{0}\" är inget belopp (skriv till exempel 125,50)")]
    Invalid(String),
    #[error("\"{0}\" har fler än två decimaler")]
    TooManyDecimals(String),
    #[error("beloppet är för stort")]
    Overflow,
}

impl Money {
    pub const ZERO: Money = Money(0);

    pub const fn from_ore(ore: i64) -> Self {
        Self(ore)
    }

    pub const fn ore(self) -> i64 {
        self.0
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    pub fn checked_add(self, rhs: Money) -> Option<Money> {
        self.0.checked_add(rhs.0).map(Money)
    }

    pub fn checked_sub(self, rhs: Money) -> Option<Money> {
        self.0.checked_sub(rhs.0).map(Money)
    }

    /// Tolkar ett belopp i kronor: `125`, `125,50`, `125.5`, `-0,01`,
    /// `1 234,56`. Mer än två decimaler är ett fel, inte en avrundning.
    pub fn parse(input: &str) -> Result<Money, MoneyError> {
        let invalid = || MoneyError::Invalid(input.to_string());
        let trimmed = input.trim();
        let (negative, rest) = match trimmed.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        };
        let cleaned: String = rest
            .chars()
            .filter(|c| !matches!(c, ' ' | '\u{a0}' | '\u{202f}' | '_'))
            .collect();
        if cleaned.matches([',', '.']).count() > 1 {
            return Err(invalid());
        }
        let (whole, frac) = match cleaned.split_once([',', '.']) {
            Some((w, f)) => (w, f),
            None => (cleaned.as_str(), ""),
        };
        if whole.is_empty() && frac.is_empty() {
            return Err(invalid());
        }
        if !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
            return Err(invalid());
        }
        if frac.len() > 2 {
            return Err(MoneyError::TooManyDecimals(input.to_string()));
        }
        let kronor: i64 = if whole.is_empty() {
            0
        } else {
            whole.parse().map_err(|_| MoneyError::Overflow)?
        };
        let ore: i64 = match frac.len() {
            0 => 0,
            1 => frac.parse::<i64>().map_err(|_| invalid())? * 10,
            _ => frac.parse().map_err(|_| invalid())?,
        };
        let total = kronor
            .checked_mul(100)
            .and_then(|k| k.checked_add(ore))
            .ok_or(MoneyError::Overflow)?;
        Ok(Money(if negative { -total } else { total }))
    }

    /// Delar beloppet i tecken, kronor och öre. Tecknet bärs separat så att
    /// belopp mellan -0,99 och -0,01 kr behåller sitt minus.
    pub fn split(self) -> (bool, u64, u64) {
        let abs = self.0.unsigned_abs();
        (self.0 < 0, abs / 100, abs % 100)
    }
}

impl Sub for Money {
    type Output = Money;
    fn sub(self, rhs: Money) -> Money {
        Money(self.0 - rhs.0)
    }
}

impl Neg for Money {
    type Output = Money;
    fn neg(self) -> Money {
        Money(-self.0)
    }
}

/// `1234,56`, `-0,50`, `0,00`.
impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (negative, kronor, ore) = self.split();
        let sign = if negative { "-" } else { "" };
        write!(f, "{sign}{kronor},{ore:02}")
    }
}

/// Summerar belopp och säger ifrån vid överslag i stället för att slå runt.
pub fn checked_sum<I: IntoIterator<Item = Money>>(items: I) -> Option<Money> {
    items
        .into_iter()
        .try_fold(Money::ZERO, |acc, m| acc.checked_add(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_keeps_sign_of_small_negative_amounts() {
        assert_eq!(Money::from_ore(-1).to_string(), "-0,01");
        assert_eq!(Money::from_ore(-50).to_string(), "-0,50");
        assert_eq!(Money::from_ore(-99).to_string(), "-0,99");
        assert_eq!(Money::from_ore(-100).to_string(), "-1,00");
        assert_eq!(Money::from_ore(-5075).to_string(), "-50,75");
        assert_eq!(Money::from_ore(0).to_string(), "0,00");
        assert_eq!(Money::from_ore(10050).to_string(), "100,50");
        assert_eq!(
            Money::from_ore(i64::MIN).to_string(),
            "-92233720368547758,08"
        );
    }

    #[test]
    fn parse_accepts_swedish_and_plain_forms() {
        assert_eq!(Money::parse("125").unwrap().ore(), 12500);
        assert_eq!(Money::parse("125,50").unwrap().ore(), 12550);
        assert_eq!(Money::parse("125.5").unwrap().ore(), 12550);
        assert_eq!(Money::parse("1 234,56").unwrap().ore(), 123456);
        assert_eq!(Money::parse("-0,01").unwrap().ore(), -1);
        assert_eq!(Money::parse("-,5").unwrap().ore(), -50);
        assert_eq!(Money::parse("+7").unwrap().ore(), 700);
        assert_eq!(Money::parse("0,05").unwrap().ore(), 5);
    }

    #[test]
    fn parse_rejects_garbage_and_rounding() {
        assert!(matches!(
            Money::parse("12,345"),
            Err(MoneyError::TooManyDecimals(_))
        ));
        assert!(Money::parse("").is_err());
        assert!(Money::parse("-").is_err());
        assert!(Money::parse("1,2,3").is_err());
        assert!(Money::parse("1.2,3").is_err());
        assert!(Money::parse("12kr").is_err());
        assert!(Money::parse("99999999999999999999").is_err());
    }

    #[test]
    fn parse_and_display_round_trip() {
        for ore in [-123456, -100, -99, -1, 0, 1, 99, 100, 123456] {
            let m = Money::from_ore(ore);
            assert_eq!(Money::parse(&m.to_string()).unwrap(), m);
        }
    }

    #[test]
    fn checked_sum_detects_overflow() {
        assert_eq!(
            checked_sum([Money::from_ore(1), Money::from_ore(2)]),
            Some(Money::from_ore(3))
        );
        assert_eq!(
            checked_sum([Money::from_ore(i64::MAX), Money::from_ore(1)]),
            None
        );
    }
}
