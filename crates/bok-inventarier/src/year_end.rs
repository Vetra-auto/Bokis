//! Årets avskrivningar för hela registret, som verifikationsrader, och
//! registrets värden att stämma av mot huvudboken.
//!
//! Ett inventarium som avyttras under året skrivs av i avyttringens
//! verifikation (se [`crate::disposal`]) och kommer inte med här. Ett
//! felregistrerat inventarium kommer aldrig med.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use bok_core::voucher::Line;
use bok_core::{AccountNumber, Money};

use crate::asset::Asset;
use crate::plan::Plan;

/// Ett inventariums avskrivning för ett räkenskapsår.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepreciationRow {
    /// Numret i registret.
    pub asset_id: i64,
    /// Inventariets beskrivning.
    pub description: String,
    /// Årets avskrivning.
    pub amount: Money,
    /// Ackumulerad avskrivning vid årets slut enligt plan.
    pub accumulated: Money,
    /// Bokfört värde vid årets slut enligt plan.
    pub book_value: Money,
    /// Kostnadskontot (debet).
    pub expense: AccountNumber,
    /// Kontot för ackumulerade avskrivningar (kredit).
    pub accumulated_account: AccountNumber,
}

/// Sant om inventariet fanns kvar i företaget vid utgången av `at`.
fn held_at(asset: &Asset, at: NaiveDate) -> bool {
    asset.details.acquired <= at
        && match &asset.disposal {
            None => true,
            Some(d) if d.is_void() => false,
            Some(d) => d.date > at,
        }
}

/// Årets avskrivningar enligt plan för räkenskapsåret `start`–`end`, ett
/// inventarium per rad, i registrets ordning. Inventarier utan avskrivning
/// det året (helt avskrivna eller ännu inte i bruk) kommer inte med.
pub fn year_depreciation(
    assets: &[Asset],
    start: NaiveDate,
    end: NaiveDate,
) -> Vec<DepreciationRow> {
    assets
        .iter()
        .filter(|a| held_at(a, end))
        .filter_map(|a| {
            let plan = Plan::of(&a.details);
            let amount = plan.between(start, end);
            if amount.is_zero() {
                return None;
            }
            let accumulated = plan.accumulated_through(end);
            Some(DepreciationRow {
                asset_id: a.id,
                description: a.details.description.clone(),
                amount,
                accumulated,
                book_value: Money::from_ore(a.details.cost.ore() - accumulated.ore()),
                expense: a.details.accounts.expense,
                accumulated_account: a.details.accounts.accumulated,
            })
        })
        .collect()
}

/// Verifikationsraderna för årets avskrivningar: debet kostnad och kredit
/// ackumulerade avskrivningar, ett par per inventarium.
pub fn depreciation_lines(rows: &[DepreciationRow]) -> Vec<Line> {
    rows.iter()
        .flat_map(|r| {
            let text = format!("Avskrivning inventarie {}: {}", r.asset_id, r.description);
            [
                Line::debit(r.expense, r.amount).with_text(text.clone()),
                Line::credit(r.accumulated_account, r.amount).with_text(text),
            ]
        })
        .collect()
}

/// Registrets värden på ett konto vid ett datum, att stämma av mot
/// huvudboken. Beloppen har bokföringens tecken: anskaffningsvärden debet
/// (positiva) och ackumulerade avskrivningar kredit (negativa).
pub fn register_balances(assets: &[Asset], at: NaiveDate) -> BTreeMap<AccountNumber, Money> {
    let mut out: BTreeMap<AccountNumber, i64> = BTreeMap::new();
    for a in assets.iter().filter(|a| held_at(a, at)) {
        let accounts = a.details.accounts;
        *out.entry(accounts.asset).or_default() += a.details.cost.ore();
        let accumulated = Plan::of(&a.details).accumulated_through(at).ore();
        *out.entry(accounts.accumulated).or_default() -= accumulated;
    }
    out.into_iter()
        .map(|(k, v)| (k, Money::from_ore(v)))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::disposal::{Disposal, DisposalKind};
    use crate::testdata::{acc, d, kr, laptop};

    fn registered(id: i64, disposal: Option<Disposal>) -> Asset {
        Asset {
            id,
            details: laptop(),
            voucher: None,
            disposal,
        }
    }

    #[test]
    fn one_pair_of_lines_per_asset() {
        let assets = [registered(1, None), registered(2, None)];
        let rows = year_depreciation(&assets, d(2026, 1, 1), d(2026, 12, 31));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].amount, kr(8_000));
        assert_eq!(rows[0].book_value, kr(28_000));
        let lines = depreciation_lines(&rows);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].account, acc(7832));
        assert_eq!(lines[0].debit, kr(8_000));
        assert_eq!(lines[1].account, acc(1229));
        assert_eq!(lines[1].credit, kr(8_000));
        assert!(lines[0].text.as_deref().unwrap().contains("inventarie 1"));
    }

    #[test]
    fn disposed_and_void_assets_are_left_out() {
        let sold = Disposal {
            date: d(2026, 10, 1),
            kind: DisposalKind::Sold { price: kr(1) },
            note: None,
        };
        let void = Disposal {
            date: d(2026, 6, 1),
            kind: DisposalKind::Void,
            note: None,
        };
        let later = Disposal {
            date: d(2027, 2, 1),
            kind: DisposalKind::Scrapped,
            note: None,
        };
        let assets = [
            registered(1, Some(sold)),
            registered(2, Some(void)),
            registered(3, Some(later)),
        ];
        let rows = year_depreciation(&assets, d(2026, 1, 1), d(2026, 12, 31));
        assert_eq!(rows.iter().map(|r| r.asset_id).collect::<Vec<_>>(), [3]);
    }

    #[test]
    fn fully_depreciated_assets_give_no_row() {
        let assets = [registered(1, None)];
        assert!(year_depreciation(&assets, d(2030, 1, 1), d(2030, 12, 31)).is_empty());
        assert!(year_depreciation(&assets, d(2025, 1, 1), d(2025, 12, 31)).is_empty());
    }

    #[test]
    fn register_values_per_account() {
        let assets = [registered(1, None), registered(2, None)];
        let b = register_balances(&assets, d(2026, 12, 31));
        assert_eq!(b[&acc(1220)], kr(72_000));
        assert_eq!(b[&acc(1229)], kr(-16_000));
        assert!(register_balances(&assets, d(2026, 4, 30)).is_empty());
    }
}
