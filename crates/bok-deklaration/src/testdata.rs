//! Gemensamma testdata: ett litet aktiebolags saldobalans för 2026.

#![allow(clippy::unwrap_used)]

use chrono::NaiveDate;

use bok_core::{AccountNumber, Money};

use crate::catalog::Catalog;
use crate::declaration::{AccountBalance, Input};

pub fn kr(kronor: i64) -> i64 {
    kronor * 100
}

pub fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

pub fn catalog() -> Catalog {
    Catalog::for_year_end(d(2026, 12, 31)).unwrap()
}

pub fn balance(account: i64, name: &str, ore: i64) -> AccountBalance {
    AccountBalance {
        account: AccountNumber::new(account).unwrap(),
        name: name.into(),
        balance: Money::from_ore(ore),
        sru: None,
    }
}

/// Utgående saldon, debet positivt. Årets resultat 131 600 kr är inte
/// bokfört mot eget kapital.
pub fn balances() -> Vec<AccountBalance> {
    vec![
        balance(1220, "Inventarier och verktyg", kr(36_000)),
        balance(1229, "Ackumulerade avskrivningar", -kr(8_000)),
        balance(1630, "Skattekonto", kr(1_000)),
        balance(1930, "Företagskonto", kr(150_000)),
        balance(2081, "Aktiekapital", -kr(25_000)),
        balance(2440, "Leverantörsskulder", -kr(10_000)),
        balance(2611, "Utgående moms", -kr(12_400)),
        balance(3001, "Försäljning", -kr(200_000)),
        balance(5420, "Programvaror", kr(30_000)),
        balance(6072, "Representation, ej avdragsgill", kr(10_000)),
        balance(7010, "Löner", kr(20_000)),
        balance(7832, "Avskrivningar på inventarier", kr(8_000)),
        balance(8314, "Skattefria ränteintäkter", -kr(100)),
        balance(8423, "Räntekostnader för skatter och avgifter", kr(500)),
    ]
}

pub fn input() -> Input {
    Input {
        company_name: "Exempelbolaget AB".into(),
        org_number: "559000-0005".into(),
        year_start: d(2026, 4, 27),
        year_end: d(2026, 12, 31),
        balances: balances(),
        manual: vec![],
    }
}
