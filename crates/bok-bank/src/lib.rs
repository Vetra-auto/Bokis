//! Inläsning av kontoutdrag för Bokis.
//!
//! En betalning som syns i internetbanken räknas som mottagen uppgift
//! (BFNAR 2013:2, kommentaren till 3.3). Bankfilen sparas därför oförändrad i
//! underlagsarkivet, och den här modulen gör om den till [`BankRow`]er som
//! kan matchas mot verifikationer.
//!
//! - [`parse_camt053`] läser ISO 20022 camt.053 (version .02 och .08).
//! - [`parse_csv`] läser en CSV-export med ett angivet [`CsvFormat`].
//! - [`parse_amount`] tolkar svenska och internationella beloppsformat till öre.
//!
//! Varje rad får ett stabilt [`BankRow::entry_id`], så att samma rad inte
//! läses in två gånger när två utdrag överlappar.

mod camt;
mod csv;

use chrono::NaiveDate;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub use camt::parse_camt053;
pub use csv::{balances_from_rows, parse_csv, CsvFormat, RowOrder, TextEncoding};

/// Fel vid inläsning av bankfiler.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BankError {
    #[error("XML-fel: {0}")]
    Xml(String),
    #[error("filen innehåller inget kontoutdrag")]
    NoStatement,
    #[error("ogiltigt belopp {0:?}")]
    InvalidAmount(String),
    #[error("ogiltigt datum {0:?}")]
    InvalidDate(String),
    #[error("rad {line}: {error}")]
    Line { line: usize, error: String },
    #[error("posten saknar {0}")]
    Missing(&'static str),
}

/// En bokförd rad på ett bankkonto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BankRow {
    /// Kontots IBAN eller annan identitet.
    #[serde(rename = "konto")]
    pub account: String,
    #[serde(rename = "bokforingsdag")]
    pub booking_date: NaiveDate,
    #[serde(rename = "valutadag")]
    pub value_date: Option<NaiveDate>,
    /// Belopp i öre. Positivt in på kontot, negativt ut.
    #[serde(rename = "belopp_ore")]
    pub amount_ore: i64,
    #[serde(rename = "valuta")]
    pub currency: String,
    /// Meddelande eller beskrivning från banken.
    pub text: String,
    /// OCR, end-to-end-id eller bankens referens.
    #[serde(rename = "referens")]
    pub reference: Option<String>,
    /// Betalare för insättningar, mottagare för uttag.
    #[serde(rename = "motpart")]
    pub counterparty: Option<String>,
    /// Saldo efter raden, om banken anger det.
    #[serde(rename = "saldo_ore")]
    pub balance_ore: Option<i64>,
    /// Stabilt id för dubblettkontroll mellan överlappande utdrag.
    #[serde(rename = "post_id")]
    pub entry_id: String,
}

/// Ett kontoutdrag med rader och saldon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Statement {
    #[serde(rename = "konto")]
    pub account: String,
    #[serde(rename = "valuta")]
    pub currency: String,
    #[serde(rename = "ingaende_saldo_ore")]
    pub opening_balance_ore: Option<i64>,
    #[serde(rename = "utgaende_saldo_ore")]
    pub closing_balance_ore: Option<i64>,
    #[serde(rename = "rader")]
    pub rows: Vec<BankRow>,
}

impl Statement {
    /// `Some(true)` om ingående saldo plus raderna ger utgående saldo,
    /// `Some(false)` om det inte stämmer och `None` om saldon saknas.
    pub fn balances_match(&self) -> Option<bool> {
        let opening = self.opening_balance_ore?;
        let closing = self.closing_balance_ore?;
        let sum: i64 = self.rows.iter().map(|r| r.amount_ore).sum();
        Some(opening + sum == closing)
    }
}

/// Tolkar ett belopp till öre.
///
/// Godtar mellanslag, hårda mellanslag och apostrof som tusentalsavgränsare,
/// komma eller punkt som decimaltecken (det sista av dem räknas som
/// decimaltecken om båda finns), samt minus som `-`, `−` eller avslutande
/// minus. Fler än två decimaler nekas, eftersom de inte går att bokföra i öre.
pub fn parse_amount(text: &str) -> Result<i64, BankError> {
    parse_amount_with_decimal(text, None)
}

pub(crate) fn parse_amount_with_decimal(
    text: &str,
    decimal: Option<char>,
) -> Result<i64, BankError> {
    let err = || BankError::InvalidAmount(text.to_string());
    let mut s: String = text
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '\u{a0}' | '\u{202f}' | '\'' | '+'))
        .collect();
    if s.is_empty() {
        return Err(err());
    }
    let mut negative = false;
    for minus in ['-', '\u{2212}'] {
        if let Some(rest) = s.strip_prefix(minus) {
            negative = true;
            s = rest.to_string();
        } else if let Some(rest) = s.strip_suffix(minus) {
            negative = true;
            s = rest.to_string();
        }
    }
    let decimal_char = match decimal {
        Some(d) => Some(d),
        None => match (s.rfind(','), s.rfind('.')) {
            (Some(c), Some(p)) => Some(if c > p { ',' } else { '.' }),
            (Some(_), None) => Some(','),
            (None, Some(_)) => Some('.'),
            (None, None) => None,
        },
    };
    let (whole, fraction) = match decimal_char {
        Some(d) => match s.rsplit_once(d) {
            Some((w, f)) => (w.to_string(), f.to_string()),
            None => (s.clone(), String::new()),
        },
        None => (s.clone(), String::new()),
    };
    let whole: String = whole.chars().filter(|c| *c != ',' && *c != '.').collect();
    if whole.is_empty() && fraction.is_empty() {
        return Err(err());
    }
    if !whole.chars().all(|c| c.is_ascii_digit()) || !fraction.chars().all(|c| c.is_ascii_digit()) {
        return Err(err());
    }
    if fraction.len() > 2 {
        return Err(err());
    }
    let kronor: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| err())?
    };
    let ore: i64 = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<i64>().map_err(|_| err())? * 10,
        _ => fraction.parse().map_err(|_| err())?,
    };
    let amount = kronor
        .checked_mul(100)
        .and_then(|k| k.checked_add(ore))
        .ok_or_else(err)?;
    Ok(if negative { -amount } else { amount })
}

pub(crate) fn parse_date(text: &str, format: &str) -> Result<NaiveDate, BankError> {
    let t = text.trim();
    // DateTime-värden som "2026-11-02T10:15:00" kortas till datumet.
    let t = if format == "%Y-%m-%d" && t.len() > 10 {
        &t[..10]
    } else {
        t
    };
    NaiveDate::parse_from_str(t, format).map_err(|_| BankError::InvalidDate(text.to_string()))
}

/// Sätter stabila post-id:n. Identiska rader i samma fil skiljs åt med ett löpnummer.
pub(crate) fn assign_entry_ids(rows: &mut [BankRow], source: &str, bank_refs: &[Option<String>]) {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, row) in rows.iter_mut().enumerate() {
        let basis = match bank_refs.get(i).cloned().flatten() {
            Some(bank_ref) => format!("{source}|{}|ref|{bank_ref}", row.account),
            None => {
                let key = format!(
                    "{source}|{}|{}|{}|{}|{}|{}",
                    row.account,
                    row.booking_date,
                    row.amount_ore,
                    row.reference.as_deref().unwrap_or(""),
                    row.text,
                    row.balance_ore.map(|s| s.to_string()).unwrap_or_default()
                );
                let n = seen.entry(key.clone()).or_insert(0);
                *n += 1;
                format!("{key}|{n}")
            }
        };
        row.entry_id = hex::encode(Sha256::digest(basis.as_bytes()));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn swedish_amounts() {
        assert_eq!(parse_amount("1 234,56").unwrap(), 123_456);
        assert_eq!(parse_amount("-500,00").unwrap(), -50_000);
        assert_eq!(parse_amount("\u{2212}12,00").unwrap(), -1_200);
        assert_eq!(parse_amount("12,5").unwrap(), 1_250);
        assert_eq!(parse_amount("0,40").unwrap(), 40);
        assert_eq!(parse_amount("-0,40").unwrap(), -40);
        assert_eq!(parse_amount("1\u{a0}000").unwrap(), 100_000);
        assert_eq!(parse_amount("250-").unwrap(), -25_000);
    }

    #[test]
    fn international_amounts() {
        assert_eq!(parse_amount("1234.56").unwrap(), 123_456);
        assert_eq!(parse_amount("1,234.56").unwrap(), 123_456);
        assert_eq!(parse_amount("1.234,56").unwrap(), 123_456);
        assert_eq!(parse_amount("+75").unwrap(), 7_500);
    }

    #[test]
    fn invalid_amounts_are_refused() {
        assert!(parse_amount("12,345").is_err());
        assert!(parse_amount("abc").is_err());
        assert!(parse_amount("").is_err());
        assert!(parse_amount("-").is_err());
        assert!(parse_amount("1 2x").is_err());
    }
}
