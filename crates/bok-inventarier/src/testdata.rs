//! Gemensamma testdata: ett inventarium som skaffas mitt i ett år.

#![allow(clippy::unwrap_used)]

use chrono::NaiveDate;

use bok_core::{AccountNumber, Money};

use crate::asset::{AssetAccounts, NewAsset};

pub fn acc(n: i64) -> AccountNumber {
    AccountNumber::new(n).unwrap()
}

pub fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

pub fn kr(kronor: i64) -> Money {
    Money::from_ore(kronor * 100)
}

/// En dator för 36 000 kr, köpt 12 maj 2026, som skrivs av på 36 månader.
pub fn laptop() -> NewAsset {
    NewAsset {
        description: "Bärbar dator".into(),
        acquired: d(2026, 5, 12),
        cost: kr(36_000),
        life_months: 36,
        depreciation_start: d(2026, 5, 1),
        residual: Money::ZERO,
        accounts: AssetAccounts {
            asset: acc(1220),
            accumulated: acc(1229),
            expense: acc(7832),
        },
    }
}
