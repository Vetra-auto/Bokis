//! Kontonummer i kontoplanen (fyra siffror, 1000–9999).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i64")]
pub struct AccountNumber(u16);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("kontonummer måste vara fyra siffror, 1000–9999 (fick {0})")]
pub struct AccountError(pub String);

impl AccountNumber {
    pub fn new(number: i64) -> Result<Self, AccountError> {
        if (1000..=9999).contains(&number) {
            Ok(Self(number as u16))
        } else {
            Err(AccountError(number.to_string()))
        }
    }

    pub fn value(self) -> u16 {
        self.0
    }

    /// Kontoklassen är första siffran (1 tillgångar, 2 eget kapital och
    /// skulder, 3–8 resultatkonton, 9 intern redovisning).
    pub fn class(self) -> u8 {
        (self.0 / 1000) as u8
    }

    /// Balanskonton (klass 1–2) har ingående och utgående balans.
    pub fn is_balance_account(self) -> bool {
        self.class() <= 2
    }

    /// Resultatkonton (klass 3 och uppåt) nollställs inte i bokföringen men
    /// redovisas som årets resultat (`#RES` i SIE).
    pub fn is_result_account(self) -> bool {
        !self.is_balance_account()
    }
}

impl TryFrom<i64> for AccountNumber {
    type Error = AccountError;
    fn try_from(value: i64) -> Result<Self, Self::Error> {
        AccountNumber::new(value)
    }
}

impl From<AccountNumber> for i64 {
    fn from(value: AccountNumber) -> Self {
        i64::from(value.0)
    }
}

impl FromStr for AccountNumber {
    type Err = AccountError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        if t.len() != 4 || !t.chars().all(|c| c.is_ascii_digit()) {
            return Err(AccountError(t.to_string()));
        }
        AccountNumber::new(t.parse::<i64>().map_err(|_| AccountError(t.to_string()))?)
    }
}

impl fmt::Display for AccountNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_is_enforced() {
        assert!(AccountNumber::new(1000).is_ok());
        assert!(AccountNumber::new(9999).is_ok());
        assert!(AccountNumber::new(999).is_err());
        assert!(AccountNumber::new(10000).is_err());
        assert!("193".parse::<AccountNumber>().is_err());
        assert!("19300".parse::<AccountNumber>().is_err());
        assert_eq!("1930".parse::<AccountNumber>().unwrap().value(), 1930);
    }

    #[test]
    fn balance_and_result_accounts() {
        let bank: AccountNumber = "1930".parse().unwrap();
        let skuld: AccountNumber = "2440".parse().unwrap();
        let intakt: AccountNumber = "3010".parse().unwrap();
        let skatt: AccountNumber = "8910".parse().unwrap();
        assert!(bank.is_balance_account() && skuld.is_balance_account());
        assert!(intakt.is_result_account() && skatt.is_result_account());
        assert_eq!(skuld.class(), 2);
    }
}
